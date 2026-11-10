//! Each job waits on its own thread, so a slow program never holds up a handler.
//! A job is its own process group, so a signal reaches the children that hold its pipes too.

use std::collections::HashMap;
use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};

#[derive(Default, Debug)]
pub struct Spec {
    pub argv: Vec<String>,
    pub cwd: Option<String>,
    pub env: HashMap<String, String>,
    pub stdin: Option<String>,
    pub timeout: Option<Duration>,
}

#[derive(Debug, PartialEq)]
pub struct Done {
    pub id: u64,
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub struct Jobs {
    tx: Sender<Done>,
    rx: Receiver<Done>,
    /// Written once per finished job, so the host's loop wakes for it.
    wake: Option<UnixStream>,
    pids: HashMap<u64, Pid>,
    deadlines: Vec<(Instant, u64)>,
}

impl Default for Jobs {
    fn default() -> Jobs {
        let (tx, rx) = channel();
        Jobs { tx, rx, wake: None, pids: HashMap::new(), deadlines: Vec::new() }
    }
}

impl Jobs {
    pub fn set_wake(&mut self, wake: UnixStream) {
        self.wake = Some(wake);
    }

    pub fn start(&mut self, id: u64, spec: Spec) -> io::Result<u32> {
        let (program, args) = spec.argv.split_first().ok_or_else(|| io::Error::other("the command is empty"))?;
        let mut cmd = Command::new(program);
        cmd.args(args).envs(&spec.env).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
        cmd.stdin(if spec.stdin.is_some() { Stdio::piped() } else { Stdio::null() });
        if let Some(dir) = &spec.cwd {
            cmd.current_dir(dir);
        }
        let mut child = cmd.spawn()?;
        let pid = child.id();
        if let (Some(mut pipe), Some(text)) = (child.stdin.take(), spec.stdin) {
            // Else a program that writes before reading stdin deadlocks.
            std::thread::spawn(move || {
                let _ = pipe.write_all(text.as_bytes());
            });
        }
        let tx = self.tx.clone();
        let wake = self.wake.as_ref().and_then(|w| w.try_clone().ok());
        std::thread::spawn(move || {
            let done = match child.wait_with_output() {
                Ok(o) => Done {
                    id,
                    code: o.status.code(),
                    signal: o.status.signal(),
                    stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
                },
                Err(e) => Done { id, code: None, signal: None, stdout: String::new(), stderr: e.to_string() },
            };
            let _ = tx.send(done);
            if let Some(mut w) = wake {
                let _ = w.write_all(b"!");
            }
        });
        if let Some(p) = Pid::from_raw(pid as i32) {
            self.pids.insert(id, p);
        }
        if let Some(t) = spec.timeout {
            self.deadlines.push((Instant::now() + t, id));
        }
        Ok(pid)
    }

    /// A finished job's group id may have been reused, so it isn't signalled.
    pub fn kill(&self, id: u64, signal: Signal) -> bool {
        self.pids.get(&id).is_some_and(|&p| kill_process_group(p, signal).is_ok())
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.deadlines.iter().map(|(at, _)| *at).min()
    }

    pub fn finished(&mut self, now: Instant) -> Vec<Done> {
        let (due, later): (Vec<_>, Vec<_>) = self.deadlines.drain(..).partition(|(at, _)| *at <= now);
        self.deadlines = later;
        for (_, id) in due {
            self.kill(id, Signal::TERM);
        }
        let done: Vec<Done> = self.rx.try_iter().collect();
        for d in &done {
            self.pids.remove(&d.id);
            self.deadlines.retain(|(_, id)| *id != d.id);
        }
        done
    }
}

impl Drop for Jobs {
    fn drop(&mut self) {
        for &p in self.pids.values() {
            let _ = kill_process_group(p, Signal::KILL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for(jobs: &mut Jobs) -> Done {
        let start = Instant::now();
        loop {
            if let Some(d) = jobs.finished(Instant::now()).pop() {
                return d;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "the job never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn sh(script: &str) -> Spec {
        Spec { argv: vec!["/bin/sh".into(), "-c".into(), script.into()], ..Spec::default() }
    }

    #[test]
    fn output_status_stdin_cwd_and_env_come_back() {
        let mut jobs = Jobs::default();
        let mut spec = sh("cat; echo \"$PWD $X\" >&2; exit 3");
        spec.stdin = Some("in".into());
        spec.cwd = Some("/".into());
        spec.env.insert("X".into(), "y".into());
        jobs.start(7, spec).unwrap();
        assert_eq!(
            wait_for(&mut jobs),
            Done { id: 7, code: Some(3), signal: None, stdout: "in".into(), stderr: "/ y\n".into() }
        );
    }

    #[test]
    fn a_timeout_terminates_and_a_finished_job_cant_be_killed() {
        let mut jobs = Jobs::default();
        let mut spec = sh("sleep 10");
        spec.timeout = Some(Duration::from_millis(20));
        jobs.start(1, spec).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(wait_for(&mut jobs).signal, Some(Signal::TERM.as_raw()));
        assert!(!jobs.kill(1, Signal::TERM));
    }

    #[test]
    fn a_timeout_reaches_children_holding_the_pipes() {
        // `&` forces a child on every sh; macOS's would otherwise exec `sleep` and hide this.
        let mut jobs = Jobs::default();
        let mut spec = sh("sleep 10 & wait");
        spec.timeout = Some(Duration::from_millis(20));
        jobs.start(1, spec).unwrap();
        assert_eq!(wait_for(&mut jobs).signal, Some(Signal::TERM.as_raw()));
    }

    #[test]
    fn a_missing_program_fails_to_start() {
        let mut jobs = Jobs::default();
        assert!(jobs.start(1, Spec { argv: vec!["/no/such/program".into()], ..Spec::default() }).is_err());
        assert!(jobs.start(2, Spec::default()).is_err());
    }
}

#[cfg(test)]
mod lua_tests {
    use std::time::{Duration, Instant};

    use serde_json::{Value, json};

    use crate::host::Host;
    use crate::host::tests::{Shared, TestDir, find, plugin, start_with};

    fn run(test: &str, init: &str) -> (Host, Shared, TestDir) {
        let d = plugin(test, "sys", "[commands]\ngo = {}\n", init);
        let (mut h, out) = start_with(vec![json!({"dir": d, "opts": {}})], &[], None);
        out.take();
        h.line(r#"{"event":"command","name":"sys.go","client":1,"session":"s","count":1}"#);
        (h, out, d)
    }

    fn until_sent(h: &mut Host, out: &Shared, cmd: &str) -> Vec<Value> {
        let start = Instant::now();
        let mut seen = Vec::new();
        while start.elapsed() < Duration::from_secs(5) {
            h.tick(Instant::now());
            seen.extend(out.take());
            if seen.iter().any(|l| l["cmd"] == cmd) {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("no {cmd} in {seen:?}");
    }

    #[test]
    fn wait_yields_the_handler_until_the_program_ends() {
        let init = "shroud.command('go', function()\n local r = shroud.system({'/bin/sh', '-c', 'printf hi; exit 2'}):wait()\n shroud.message{text = r.stdout .. r.code}\nend)\nreturn {}";
        let (mut h, out, _dir) = run("system-wait", init);
        assert!(!out.take().iter().any(|l| l["cmd"] == "message"), "the handler is still waiting");
        assert_eq!(find(&until_sent(&mut h, &out, "message"), "message")["text"], "hi2");
    }

    #[test]
    fn on_exit_runs_as_the_plugin_and_kill_signals_the_job() {
        let init = "shroud.command('go', function()\n local j = shroud.system({'/bin/sleep', '10'}, function(r) shroud.message{text = 'sig' .. r.signal} end)\n j:kill()\nend)\nreturn {}";
        let (mut h, out, _dir) = run("system-exit", init);
        let lines = until_sent(&mut h, &out, "message");
        assert_eq!(find(&lines, "message")["text"], "sig15");
        assert!(lines.contains(&json!({"cmd": "plugin_running", "plugin": "sys"})));
    }

    #[test]
    fn a_job_that_finished_before_wait_still_returns_its_result() {
        let init = "shroud.command('go', function()\n local j = shroud.system({'/bin/echo', 'early'})\n shroud.after(100, function() shroud.message{text = j:wait().stdout} end)\nend)\nreturn {}";
        let (mut h, out, _dir) = run("system-early", init);
        assert_eq!(find(&until_sent(&mut h, &out, "message"), "message")["text"], "early\n");
    }
}
