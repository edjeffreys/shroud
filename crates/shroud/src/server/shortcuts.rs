//! Keys that run programs. A command pane is an ordinary shell with the command typed in: it
//! closes when the command succeeds and holds when it fails (or always, with `keep`). A job has
//! no pane and reports to the status bar. Jobs are reaped with the panes, on SIGCHLD, so the
//! server stays single-threaded; their output goes to an unlinked file, which can't fill up and
//! block them the way a pipe nobody reads would.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};
use shroud_core::input::KeyEvent;
use shroud_core::input::shortcut::{self, Ending, HeldKey, Open, Program, Shortcut};
use shroud_core::layout::{Axis, FloatRect, PaneId};

use super::session::SessionId;
use super::{CONFIG_MESSAGE_FOR, Server};

// The summary is one line; the tail of the output always holds it.
const TAIL: u64 = 4096;
// After SIGTERM at the timeout, how long a job gets before SIGKILL.
const KILL_AFTER: Duration = Duration::from_secs(5);
// Long enough to read after the command ends, short of nagging.
const HELD_MESSAGE_FOR: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct CommandPane {
    pub command: String,
    /// Hold even when the command succeeds.
    pub keep: bool,
    pub held: bool,
}

#[derive(Debug)]
pub struct Job {
    child: Child,
    out: File,
    label: String,
    session: SessionId,
    timeout: Duration,
    deadline: Instant,
    stopped: bool,
}

impl Server {
    pub(super) fn run_shortcut(&mut self, i: usize, shortcut: Shortcut) {
        let Some(id) = self.clients[i].session else { return };
        let Some(s) = self.sessions.iter().position(|s| s.id == id) else { return };
        let before = self.sessions[s].tab().focused;
        let result = match shortcut {
            Shortcut::Run { command, open, keep, cwd } => {
                self.open_command_pane(s, &command, open, keep, cwd.as_deref()).map(|_| ())
            }
            Shortcut::Exec { program, cwd, timeout } => self.start_job(s, program, cwd.as_deref(), timeout),
        };
        if let Err(e) = result {
            self.sessions[s].status.say_for(e, CONFIG_MESSAGE_FOR);
        }
        self.focus_changed(i, before);
    }

    fn open_command_pane(
        &mut self,
        s: usize,
        command: &str,
        open: Open,
        keep: bool,
        cwd: Option<&str>,
    ) -> Result<PaneId, String> {
        let session = &mut self.sessions[s];
        let cwd = resolve(session.tab().focused_cwd(), cwd)?;
        let cwd = cwd.as_deref();
        let opened = match open {
            Open::Float => session.tab_mut().new_float(FloatRect::CENTRED, cwd, true),
            Open::Right | Open::Down => {
                let axis = if open == Open::Right { Axis::Horizontal } else { Axis::Vertical };
                let tab = session.tab_mut();
                let target = tab.focused;
                tab.split_pane(target, axis, cwd, true)
            }
            Open::Tab => session.new_tab_at(cwd),
        };
        let id = opened.map_err(|e| format!("{command}: {e}"))?;
        self.make_command_pane(id, command, keep);
        Ok(id)
    }

    /// Types `command` into the pane and watches for its end.
    pub(super) fn make_command_pane(&mut self, id: PaneId, command: &str, keep: bool) {
        let Some((pane, _)) = self.sessions.iter_mut().find_map(|s| s.pane_mut(id)) else { return };
        pane.type_line(command);
        pane.command_pane = Some(CommandPane { command: command.to_string(), keep, held: false });
    }

    /// Every finished command comes here; only a command pane's own command counts.
    pub(super) fn command_pane_finished(&mut self, id: PaneId, exit: Option<i32>) {
        let Some(session) = self.sessions.iter_mut().find(|s| s.panes().any(|(p, _)| *p == id)) else { return };
        let message = {
            let Some((pane, _)) = session.pane_mut(id) else { return };
            let Some(cp) = pane.command_pane.as_mut().filter(|c| !c.held) else { return };
            if exit == Some(0) && !cp.keep {
                // Reaped like a pane whose shell exited.
                pane.exited = true;
                return;
            }
            cp.held = true;
            shortcut::held_message(&cp.command, exit)
        };
        session.status.say_for(message, HELD_MESSAGE_FOR);
    }

    /// True when a held command pane took the key; the caller reaps, since `q` closes it.
    pub(super) fn held_key(&mut self, i: usize, k: &KeyEvent) -> bool {
        let Some(session) = self.session_of(i) else { return false };
        let Some(pane) = session.tab_mut().focused_pane() else { return false };
        let Some(command) = pane.command_pane.as_ref().filter(|c| c.held).map(|c| c.command.clone()) else {
            return false;
        };
        match shortcut::held_key(k) {
            HeldKey::Rerun => {
                pane.type_line(&command);
                if let Some(cp) = &mut pane.command_pane {
                    cp.held = false;
                }
                session.status.say(format!("{command} …"));
                true
            }
            HeldKey::Close => {
                pane.exited = true;
                true
            }
            HeldKey::Release => {
                pane.command_pane = None;
                false
            }
            HeldKey::Ignore => false,
        }
    }

    fn start_job(&mut self, s: usize, program: Program, cwd: Option<&str>, timeout: Duration) -> Result<(), String> {
        let label = Shortcut::Exec { program: program.clone(), cwd: None, timeout }.label();
        let session = &mut self.sessions[s];
        let cwd = resolve(session.tab().focused_cwd(), cwd)?;
        let fail = |e: io::Error| format!("{label}: {e}");
        let out = output_file(self.cfg.socket.parent().unwrap_or(Path::new("/tmp"))).map_err(fail)?;
        let mut cmd = match &program {
            Program::Argv(argv) => {
                let mut c = Command::new(&argv[0]);
                c.args(&argv[1..]);
                c
            }
            Program::Shell(line) => {
                let mut c = Command::new("/bin/sh");
                c.arg("-c").arg(line);
                c
            }
        };
        if let Some(dir) = &cwd {
            cmd.current_dir(dir);
        }
        // Its own process group, so a timeout stops what it started too. SHROUD tells a
        // `shroud` it runs not to try attaching to a terminal it doesn't have.
        cmd.stdin(Stdio::null())
            .stdout(out.try_clone().map_err(fail)?)
            .stderr(out.try_clone().map_err(fail)?)
            .env("SHROUD", "1")
            .env_remove("SHROUD_LIBVT_DIR")
            .process_group(0);
        let child = cmd.spawn().map_err(fail)?;
        session.status.say_for(format!("{label} …"), timeout.min(CONFIG_MESSAGE_FOR));
        self.jobs.push(Job {
            child,
            out,
            label,
            session: session.id,
            timeout,
            deadline: Instant::now() + timeout,
            stopped: false,
        });
        Ok(())
    }

    /// From `reap`: reports jobs that ended and stops the ones past their timeout.
    pub(super) fn reap_jobs(&mut self) {
        let now = Instant::now();
        let mut i = 0;
        while i < self.jobs.len() {
            let job = &mut self.jobs[i];
            let status = match job.child.try_wait() {
                Ok(None) => {
                    if now >= job.deadline {
                        let signal = if job.stopped { Signal::KILL } else { Signal::TERM };
                        let _ = kill_process_group(Pid::from_child(&job.child), signal);
                        job.stopped = true;
                        job.deadline = now + KILL_AFTER;
                    }
                    i += 1;
                    continue;
                }
                Ok(Some(status)) => status.code(),
                Err(_) => None,
            };
            let mut job = self.jobs.remove(i);
            let ending = match status {
                _ if job.stopped => Ending::TimedOut(job.timeout),
                Some(code) => Ending::Exit(code),
                None => Ending::Signal,
            };
            let text = tail(&mut job.out).unwrap_or_default();
            if let Some(s) = self.sessions.iter_mut().find(|s| s.id == job.session) {
                s.status.say_for(shortcut::summary(&job.label, ending, &text), CONFIG_MESSAGE_FOR);
            }
            self.invalidate();
        }
    }

    pub(super) fn job_deadline(&self) -> Option<Instant> {
        self.jobs.iter().map(|j| j.deadline).min()
    }
}

/// Relative to the focused pane's directory, as the pane's own shell would read it.
fn resolve(base: Option<PathBuf>, cwd: Option<&str>) -> Result<Option<PathBuf>, String> {
    let home = || std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
    let path = match cwd {
        None => return Ok(base),
        Some("~") => home(),
        Some(c) => match c.strip_prefix("~/") {
            Some(rest) => home().join(rest),
            None => base.unwrap_or_else(home).join(c),
        },
    };
    if path.is_dir() { Ok(Some(path)) } else { Err(format!("{} is not a directory", path.display())) }
}

fn output_file(dir: &Path) -> io::Result<File> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let path = dir.join(format!("job-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let file = File::options().read(true).write(true).create_new(true).mode(0o600).open(&path)?;
    std::fs::remove_file(&path)?;
    Ok(file)
}

fn tail(out: &mut File) -> io::Result<String> {
    let len = out.seek(SeekFrom::End(0))?;
    out.seek(SeekFrom::Start(len.saturating_sub(TAIL)))?;
    let mut bytes = Vec::new();
    out.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
