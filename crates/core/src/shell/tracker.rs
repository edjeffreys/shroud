//! Per-pane prompt, command and cwd state from OSC 133 and OSC 7. Prompt rows themselves
//! stay in libghostty-vt.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Phase {
    #[default]
    Unknown,
    Prompt,
    Input,
    Running,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    PromptStart,
    InputStart,
    OutputStart,
    CommandEnd { exit: Option<i32> },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Finished {
    pub exit: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tracker {
    pub phase: Phase,
    pub cwd: Option<String>,
    pub last_exit: Option<i32>,
    pub commands_finished: u64,
}

impl Tracker {
    pub fn new() -> Tracker {
        Tracker::default()
    }

    pub fn mark(&mut self, mark: Mark) -> Option<Finished> {
        let was_running = self.phase == Phase::Running;
        self.phase = match mark {
            Mark::PromptStart => Phase::Prompt,
            Mark::InputStart => Phase::Input,
            Mark::OutputStart => Phase::Running,
            Mark::CommandEnd { .. } => Phase::Prompt,
        };
        match mark {
            Mark::CommandEnd { exit } if was_running => {
                self.last_exit = exit;
                self.commands_finished += 1;
                Some(Finished { exit })
            }
            _ => None,
        }
    }

    pub fn set_cwd(&mut self, value: &str) {
        let decoded = if let Some(rest) = value.strip_prefix("file://") {
            percent_decode(after_host(rest))
        } else if let Some(rest) = value.strip_prefix("kitty-shell-cwd://") {
            // Kitty's scheme carries the raw path; decoding would corrupt a literal `%`.
            after_host(rest).to_string()
        } else {
            value.to_string()
        };
        if decoded.starts_with('/') {
            self.cwd = Some(decoded);
        }
    }
}

fn after_host(rest: &str) -> &str {
    rest.find('/').map_or("", |i| &rest[i..])
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_cycle_reports_its_exit_once() {
        let mut t = Tracker::new();
        assert_eq!(t.mark(Mark::PromptStart), None);
        t.mark(Mark::InputStart);
        t.mark(Mark::OutputStart);
        assert_eq!(t.mark(Mark::CommandEnd { exit: Some(2) }), Some(Finished { exit: Some(2) }));
        assert_eq!(t.mark(Mark::CommandEnd { exit: Some(0) }), None);
        assert_eq!((t.last_exit, t.commands_finished), (Some(2), 1));
    }

    #[test]
    fn osc7_file_and_kitty_forms_both_parse() {
        let mut t = Tracker::new();
        t.set_cwd("file://mac.local/Users/ed/My%20Projects");
        assert_eq!(t.cwd.as_deref(), Some("/Users/ed/My Projects"));
        t.set_cwd("kitty-shell-cwd://mac.local/Users/ed/100%");
        assert_eq!(t.cwd.as_deref(), Some("/Users/ed/100%"));
        t.set_cwd("relative/ignored");
        assert_eq!(t.cwd.as_deref(), Some("/Users/ed/100%"));
        t.set_cwd("file://mac.local/Users/ed/My%20Projects");
        assert_eq!(t.cwd.as_deref(), Some("/Users/ed/My Projects"));
    }
}
