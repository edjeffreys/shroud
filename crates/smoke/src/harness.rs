//! Each section gets its own directory, server and clients, so sections can run at once.
//! Waits end as soon as their condition holds; their timeouts only matter when something is broken.

use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use shroud_core::protocol::socket_path;
use shroud_smoke::pty::Pty;
use shroud_vt::Terminal;

pub const LEADER: &[u8] = b"\x1b[32;5u";
pub const ESC: &[u8] = b"\x1b[27u";
/// Stands in for shell integration's output mark; the smoke shell only sends prompt marks.
pub const OUTPUT_START: &[u8] = b"printf '\\033]133;C\\007'; ";
pub const WAIT: Duration = Duration::from_secs(4);
const COLS: u16 = 80;
const ROWS: u16 = 24;

pub struct Env {
    pub bin: PathBuf,
    pub tmp: PathBuf,
    pub config: PathBuf,
    vars: Vec<(String, String)>,
}

impl Env {
    pub fn new(bin: &Path, section: &str) -> Env {
        // Short, so the socket path fits and shroud never falls back to the shared /tmp/shroud-$UID.
        let tmp = PathBuf::from(format!("/tmp/sm-{section}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let config = tmp.join("config/shroud/config.toml");
        fs::create_dir_all(config.parent().unwrap()).expect("smoke directory");
        let mut vars: Vec<(String, String)> = std::env::vars()
            .filter(|(k, _)| {
                !["GHOSTTY_RESOURCES_DIR", "SHROUD", "SHROUD_PANE"].contains(&k.as_str())
                    && !k.starts_with("ZELLIJ")
                    && !k.starts_with("TMUX")
            })
            .collect();
        let set = [
            ("TMPDIR", tmp.display().to_string()),
            // Each section's server lives under its own directory, whichever variable shroud reads.
            ("XDG_RUNTIME_DIR", tmp.display().to_string()),
            ("SHELL", "/bin/bash".into()),
            ("TERM", "xterm-ghostty".into()),
            ("PS1", "$ ".into()),
            ("BASH_SILENCE_DEPRECATION_WARNING", "1".into()),
            // macOS's bash 3.2 has no PS0, so this stands in for shell integration's prompt marks.
            ("PROMPT_COMMAND", "printf \"\\033]133;D;%s\\007\\033]133;A\\007\" $?".into()),
            ("XDG_CONFIG_HOME", tmp.join("config").display().to_string()),
            ("XDG_DATA_HOME", tmp.join("data").display().to_string()),
        ];
        for (k, v) in set {
            vars.retain(|(x, _)| x != k);
            vars.push((k.into(), v));
        }
        Env { bin: bin.to_path_buf(), tmp, config, vars }
    }

    pub fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(&self.bin);
        c.args(args).env_clear().envs(self.vars.iter().map(|(k, v)| (k, v)));
        c
    }

    pub fn cli(&self, args: &[&str]) -> (i32, String) {
        match self.command(args).stdin(Stdio::null()).output() {
            Ok(o) => (
                o.status.code().unwrap_or(-1),
                format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)),
            ),
            Err(e) => (-1, e.to_string()),
        }
    }

    pub fn runtime(&self) -> PathBuf {
        let tmp = self.tmp.display().to_string();
        socket_path::runtime_dir(Some(&tmp), Some(&tmp), rustix::process::getuid().as_raw())
    }

    /// The process holding this section's socket, so sections running at once can't mix theirs up.
    pub fn server_pid(&self) -> Option<u32> {
        let sock = socket_path::server_socket(&self.runtime());
        let out = Command::new("lsof").arg("-t").arg(sock).output().ok()?;
        String::from_utf8_lossy(&out.stdout).split_whitespace().filter_map(|p| p.parse().ok()).find(|&p| is_server(p))
    }

    pub fn write_config(&self, text: &str) {
        fs::write(&self.config, text).expect("config");
    }

    pub fn remove_config(&self) {
        let _ = fs::remove_file(&self.config);
    }

    pub fn spawn(&self, cwd: Option<&Path>, args: &[&str]) -> Client {
        Client::spawn(self, cwd, args)
    }

    pub fn stop_server(&self) {
        self.cli(&["server", "stop"]);
    }
}

fn is_server(pid: u32) -> bool {
    Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("__server"))
}

pub fn cpu_seconds(pid: u32) -> f64 {
    let out = Command::new("ps")
        .args(["-o", "cputime=", "-p", &pid.to_string()])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let mut secs = 0.0;
    for part in out.split(':') {
        secs = secs * 60.0 + part.parse::<f64>().unwrap_or(0.0);
    }
    secs
}

/// One attached `shroud`, as a terminal would run it, with its output parsed into a screen.
pub struct Client {
    pty: Pty,
    term: Terminal,
    effects: Vec<shroud_vt::Effect>,
    pub raw: Vec<u8>,
}

impl Client {
    fn spawn(env: &Env, cwd: Option<&Path>, args: &[&str]) -> Client {
        let mut cmd = env.command(args);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        let pty = Pty::spawn(cmd, COLS, ROWS).expect("spawn shroud");
        Client { pty, term: Terminal::new(COLS, ROWS).expect("terminal"), effects: Vec::new(), raw: Vec::new() }
    }

    pub fn send(&mut self, bytes: &[u8]) {
        let _ = self.pty.write(bytes);
    }

    pub fn since(&self, at: usize) -> &[u8] {
        &self.raw[at.min(self.raw.len())..]
    }

    pub fn mark(&self) -> usize {
        self.raw.len()
    }

    fn pump(&mut self, wait: Duration) {
        let at = self.raw.len();
        if self.pty.read(wait, &mut self.raw) > 0 {
            self.term.write(&self.raw[at..]);
            self.term.drain_effects(&mut self.effects);
            self.effects.clear();
        }
    }

    #[track_caller]
    pub fn wait(&mut self, timeout: Duration, cond: impl Fn(&Client) -> bool) -> bool {
        let ok = self.wait_quietly(timeout, cond);
        if !ok {
            eprintln!("{}: wait timed out", std::panic::Location::caller());
        }
        ok
    }

    /// For retries, where a timeout is expected.
    pub fn wait_quietly(&mut self, timeout: Duration, cond: impl Fn(&Client) -> bool) -> bool {
        let end = Instant::now() + timeout;
        loop {
            if cond(self) {
                return true;
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            self.pump(left.min(Duration::from_millis(20)));
        }
    }

    /// A login shell redraws as it starts, so typing before its prompt can lose the output.
    #[track_caller]
    pub fn ready(&mut self) -> bool {
        self.wait(WAIT, |c| c.rows().iter().any(|r| r.ends_with('$')))
    }

    #[track_caller]
    pub fn wait_screen(&mut self, text: &str) -> bool {
        self.wait(WAIT, |c| c.screen().contains(text))
    }

    #[track_caller]
    pub fn wait_gone(&mut self, text: &str) -> bool {
        self.wait(WAIT, |c| !c.screen().contains(text))
    }

    #[track_caller]
    pub fn wait_line(&mut self, text: &str) -> bool {
        self.wait(WAIT, |c| c.has_line(text))
    }

    #[track_caller]
    pub fn wait_raw(&mut self, at: usize, needle: &[u8]) -> bool {
        self.wait(WAIT, |c| contains(c.since(at), needle))
    }

    /// For steps with nothing to look for: returns once output has stopped for a moment.
    pub fn settle(&mut self) {
        let end = Instant::now() + Duration::from_secs(2);
        let mut quiet_since = Instant::now();
        let mut seen = self.raw.len();
        while Instant::now() < end && quiet_since.elapsed() < Duration::from_millis(120) {
            self.pump(Duration::from_millis(20));
            if self.raw.len() != seen {
                seen = self.raw.len();
                quiet_since = Instant::now();
            }
        }
    }

    /// Reads for exactly `d`, for checks that are about time passing.
    pub fn idle(&mut self, d: Duration) {
        let end = Instant::now() + d;
        while Instant::now() < end {
            self.pump(end.saturating_duration_since(Instant::now()).min(Duration::from_millis(50)));
        }
    }

    pub fn rows(&self) -> Vec<String> {
        let vp = self.term.viewport();
        (vp.top..vp.top + vp.rows as u32)
            .map(|y| {
                self.term.history_line(y).into_iter().filter(|&c| c != '\0').collect::<String>().trim_end().to_string()
            })
            .collect()
    }

    pub fn screen(&self) -> String {
        self.rows().join("\n")
    }

    pub fn status(&self) -> String {
        self.rows().pop().unwrap_or_default()
    }

    /// A row that reads exactly `text`, so a typed command doesn't count as its own output.
    pub fn has_line(&self, text: &str) -> bool {
        self.rows().iter().any(|r| r == text)
    }

    pub fn exit_code(&mut self) -> Option<i32> {
        self.pty.child.try_wait().ok().flatten().map(|s| s.code().unwrap_or_else(|| 128 + s.signal().unwrap_or(0)))
    }

    #[track_caller]
    pub fn wait_exit(&mut self, timeout: Duration) -> Option<i32> {
        let end = Instant::now() + timeout;
        loop {
            self.pump(Duration::from_millis(20));
            if let Some(c) = self.exit_code() {
                return Some(c);
            }
            if Instant::now() >= end {
                eprintln!("{}: the client didn't exit", std::panic::Location::caller());
                return None;
            }
        }
    }
}

pub fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

pub fn has_clock(line: &str) -> bool {
    line.as_bytes().windows(5).any(|w| {
        w[0].is_ascii_digit() && w[1].is_ascii_digit() && w[2] == b':' && w[3].is_ascii_digit() && w[4].is_ascii_digit()
    })
}

pub fn eventually(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    loop {
        if cond() {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

#[derive(Default)]
pub struct Report {
    pub lines: Vec<(bool, String)>,
}

impl Report {
    pub fn check(&mut self, name: &str, ok: bool) {
        self.lines.push((ok, name.to_string()));
    }

    pub fn check_or(&mut self, name: &str, ok: bool, detail: impl std::fmt::Debug) {
        let line = if ok { name.to_string() } else { format!("{name} ({detail:?})") };
        self.lines.push((ok, line));
    }

    pub fn failed(&self) -> bool {
        self.lines.iter().any(|(ok, _)| !ok)
    }
}
