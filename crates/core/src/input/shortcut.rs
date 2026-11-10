//! Keys that run a program. One parser serves config.toml, the API's `bind` and Lua's
//! `keymap`, so all three accept the same table; running it is the server's job.

use std::time::Duration;

use serde_json::{Map, Value};

use super::event::{KeyAction, KeyEvent, Mods, key};
use super::keybinds::is_modifier;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Shortcut {
    /// Typed into a new pane's shell, which closes when the command succeeds and holds otherwise.
    Run { command: String, open: Open, keep: bool, cwd: Option<String> },
    /// No pane: the result goes to the status bar.
    Exec { program: Program, cwd: Option<String>, timeout: Duration },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Open {
    Float,
    Right,
    Down,
    Tab,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Program {
    /// Run directly, without a shell.
    Argv(Vec<String>),
    /// Run with `/bin/sh -c`.
    Shell(String),
}

/// How a background command ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ending {
    Exit(i32),
    Signal,
    TimedOut(Duration),
}

/// What a key does to a held command pane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HeldKey {
    Rerun,
    Close,
    /// The pane becomes a plain shell, and the key goes to it.
    Release,
    /// Leaves the hold as it is: releases and modifiers on their own.
    Ignore,
}

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_TIMEOUT_SECS: u64 = 24 * 60 * 60;
// The status bar is one row; more than this is cut by the bar anyway.
const MAX_SUMMARY: usize = 200;

impl Shortcut {
    pub fn from_json(v: &Value) -> Result<Shortcut, String> {
        let Value::Object(t) = v else { return Err("a shortcut is a table with `run` or `exec`".into()) };
        let cwd = string(t, "cwd")?;
        match (t.get("run"), t.get("exec")) {
            (Some(_), Some(_)) => Err("a shortcut has `run` or `exec`, not both".into()),
            (None, None) => Err("a shortcut needs `run` (a pane) or `exec` (no pane)".into()),
            (Some(run), None) => {
                only(t, &["run", "open", "keep", "cwd"], "`run`")?;
                let command = match run {
                    Value::String(s) if !s.trim().is_empty() && !s.contains(['\n', '\r']) => s.clone(),
                    _ => return Err("`run` is a command line, typed into the new pane's shell".into()),
                };
                let open = match string(t, "open")?.as_deref() {
                    None | Some("float") => Open::Float,
                    Some("right") => Open::Right,
                    Some("down") => Open::Down,
                    Some("tab") => Open::Tab,
                    Some(other) => {
                        return Err(format!("`open` is \"float\", \"right\", \"down\" or \"tab\", not \"{other}\""));
                    }
                };
                let keep = match t.get("keep") {
                    None => false,
                    Some(Value::Bool(b)) => *b,
                    Some(_) => return Err("`keep` is true or false".into()),
                };
                Ok(Shortcut::Run { command, open, keep, cwd })
            }
            (None, Some(exec)) => {
                only(t, &["exec", "timeout", "cwd"], "`exec`")?;
                let program = match exec {
                    Value::String(s) if !s.trim().is_empty() => Program::Shell(s.clone()),
                    Value::Array(a) if !a.is_empty() && a.iter().all(|x| x.as_str().is_some_and(|s| !s.is_empty())) => {
                        Program::Argv(a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    }
                    _ => {
                        return Err(
                            "`exec` is a program and its arguments, [\"git\", \"pull\"], or a line for sh".into()
                        );
                    }
                };
                let timeout = match t.get("timeout") {
                    None => DEFAULT_TIMEOUT,
                    Some(n) => match n.as_u64() {
                        Some(s) if (1..=MAX_TIMEOUT_SECS).contains(&s) => Duration::from_secs(s),
                        _ => return Err(format!("`timeout` is seconds, from 1 to {MAX_TIMEOUT_SECS}")),
                    },
                };
                Ok(Shortcut::Exec { program, cwd, timeout })
            }
        }
    }

    /// What hints and `bindings` show for the key.
    pub fn label(&self) -> String {
        match self {
            Shortcut::Run { command, .. } => command.clone(),
            Shortcut::Exec { program: Program::Argv(argv), .. } => argv.join(" "),
            Shortcut::Exec { program: Program::Shell(line), .. } => line.clone(),
        }
    }
}

/// `git pull ✓ Already up to date.`: the last line it printed says the most.
pub fn summary(label: &str, ending: Ending, output: &str) -> String {
    let head = match ending {
        Ending::Exit(0) => format!("{label} ✓"),
        Ending::Exit(n) => format!("{label} ✗{n}"),
        Ending::Signal => format!("{label} ✗ killed"),
        Ending::TimedOut(after) => format!("{label} ✗ stopped after {}s", after.as_secs()),
    };
    let text = match last_line(output) {
        Some(line) => format!("{head} {line}"),
        None => head,
    };
    text.chars().take(MAX_SUMMARY).collect()
}

/// Shown when a command pane holds.
pub fn held_message(command: &str, exit: Option<i32>) -> String {
    let status = match exit {
        Some(0) => "finished".to_string(),
        Some(n) => format!("exited {n}"),
        None => "ended".to_string(),
    };
    format!("{command} {status} · Enter reruns · q closes")
}

pub fn held_key(ev: &KeyEvent) -> HeldKey {
    if ev.action == KeyAction::Release || is_modifier(ev.code) {
        return HeldKey::Ignore;
    }
    if ev.mods.without_locks() == Mods::NONE {
        match ev.code {
            key::ENTER => return HeldKey::Rerun,
            c if c == 'q' as u32 => return HeldKey::Close,
            _ => {}
        }
    }
    HeldKey::Release
}

fn string(t: &Map<String, Value>, name: &str) -> Result<Option<String>, String> {
    match t.get(name) {
        None => Ok(None),
        Some(Value::String(s)) if !s.is_empty() => Ok(Some(s.clone())),
        Some(_) => Err(format!("`{name}` must be a string")),
    }
}

fn only(t: &Map<String, Value>, allowed: &[&str], with: &str) -> Result<(), String> {
    match t.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!(
            "`{k}` doesn't go with {with}; it takes {}",
            allowed.iter().map(|a| format!("`{a}`")).collect::<Vec<_>>().join(", ")
        )),
        None => Ok(()),
    }
}

/// The last line with text, as a terminal would leave it: after the last `\r`, without
/// escape sequences or control characters, so a progress bar or colour can't garble the bar.
fn last_line(output: &str) -> Option<String> {
    output.lines().rev().map(clean).find(|l| !l.is_empty())
}

fn clean(line: &str) -> String {
    let line = line.rsplit('\r').find(|part| !part.trim().is_empty()).unwrap_or("");
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // CSI runs to a final byte in @..~; anything else, drop the next character too.
            if chars.next_if_eq(&'[').is_some() {
                while chars.next().is_some_and(|c| !('@'..='~').contains(&c)) {}
            } else {
                chars.next();
            }
        } else if !c.is_control() {
            out.push(c);
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn run_takes_a_command_and_defaults_to_a_float_that_closes() {
        let s = Shortcut::from_json(&json!({"run": "nvim"})).unwrap();
        assert_eq!(s, Shortcut::Run { command: "nvim".into(), open: Open::Float, keep: false, cwd: None });
        let s =
            Shortcut::from_json(&json!({"run": "cargo test", "open": "down", "keep": true, "cwd": "~/src"})).unwrap();
        assert_eq!(
            s,
            Shortcut::Run { command: "cargo test".into(), open: Open::Down, keep: true, cwd: Some("~/src".into()) }
        );
        assert_eq!(s.label(), "cargo test");
    }

    #[test]
    fn exec_takes_argv_or_a_line_for_sh() {
        let s = Shortcut::from_json(&json!({"exec": ["git", "pull"]})).unwrap();
        assert_eq!(
            s,
            Shortcut::Exec {
                program: Program::Argv(vec!["git".into(), "pull".into()]),
                cwd: None,
                timeout: DEFAULT_TIMEOUT
            }
        );
        assert_eq!(s.label(), "git pull");
        let s = Shortcut::from_json(&json!({"exec": "make && make install", "timeout": 30})).unwrap();
        assert_eq!(
            s,
            Shortcut::Exec {
                program: Program::Shell("make && make install".into()),
                cwd: None,
                timeout: Duration::from_secs(30)
            }
        );
    }

    #[test]
    fn mistakes_say_what_is_wrong() {
        let err = |v: Value| Shortcut::from_json(&v).unwrap_err();
        assert!(err(json!("nvim")).contains("table"));
        assert!(err(json!({})).contains("needs"));
        assert!(err(json!({"run": "a", "exec": ["b"]})).contains("not both"));
        assert!(err(json!({"run": ""})).contains("command line"));
        assert!(err(json!({"run": "a\nb"})).contains("command line"));
        assert!(err(json!({"run": "a", "open": "left"})).contains("\"left\""));
        assert!(err(json!({"run": "a", "keep": "yes"})).contains("true or false"));
        assert!(err(json!({"run": "a", "timeout": 3})).contains("`timeout` doesn't go with `run`"));
        assert!(err(json!({"exec": []})).contains("program"));
        assert!(err(json!({"exec": ["git", 3]})).contains("program"));
        assert!(err(json!({"exec": ["x"], "keep": true})).contains("`keep` doesn't go with `exec`"));
        assert!(err(json!({"exec": ["x"], "timeout": 0})).contains("seconds"));
        assert!(err(json!({"run": "a", "cwd": 3})).contains("`cwd` must be a string"));
    }

    #[test]
    fn summaries_end_with_the_last_line_printed() {
        assert_eq!(
            summary("git pull", Ending::Exit(0), "From x\nAlready up to date.\n\n"),
            "git pull ✓ Already up to date."
        );
        assert_eq!(
            summary("git pull", Ending::Exit(1), "fatal: not a git repository\n"),
            "git pull ✗1 fatal: not a git repository"
        );
        assert_eq!(summary("true", Ending::Exit(0), ""), "true ✓");
        assert_eq!(summary("sleep 9", Ending::TimedOut(Duration::from_secs(5)), ""), "sleep 9 ✗ stopped after 5s");
        assert_eq!(summary("x", Ending::Signal, ""), "x ✗ killed");
        assert_eq!(summary("x", Ending::Exit(0), &"y".repeat(500)).chars().count(), MAX_SUMMARY);
    }

    #[test]
    fn progress_and_colour_never_reach_the_bar() {
        assert_eq!(
            summary("x", Ending::Exit(0), "Receiving 10%\rReceiving 100%, done.\r\n"),
            "x ✓ Receiving 100%, done."
        );
        assert_eq!(summary("x", Ending::Exit(0), "\x1b[32mok\x1b[0m\t\x07\n"), "x ✓ ok");
        assert_eq!(summary("x", Ending::Exit(0), "done\r\n\x1b[K\n"), "x ✓ done");
    }

    #[test]
    fn held_panes_rerun_on_enter_and_close_on_q() {
        assert_eq!(held_key(&KeyEvent::press(key::ENTER, Mods::NONE)), HeldKey::Rerun);
        assert_eq!(held_key(&KeyEvent::press('q' as u32, Mods::NONE)), HeldKey::Close);
        assert_eq!(held_key(&KeyEvent::press('q' as u32, Mods::CTRL)), HeldKey::Release);
        assert_eq!(held_key(&KeyEvent::press('l' as u32, Mods::NONE)), HeldKey::Release);
        assert_eq!(held_key(&KeyEvent::press(key::LEFT_SHIFT, Mods::SHIFT)), HeldKey::Ignore);
        assert_eq!(
            held_key(&KeyEvent { action: KeyAction::Release, ..KeyEvent::press(key::ENTER, Mods::NONE) }),
            HeldKey::Ignore
        );
        assert_eq!(held_message("cargo test", Some(101)), "cargo test exited 101 · Enter reruns · q closes");
        assert_eq!(held_message("make", Some(0)), "make finished · Enter reruns · q closes");
    }
}
