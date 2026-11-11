//! Config names for actions. Each command owns what a count means, so rebinding `w` keeps `3 w`.

use super::keybinds::{Action, Selecting};
use super::shortcut::Shortcut;
use crate::layout::{Axis, Direction};

pub type Build = fn(Option<u16>) -> Action;

/// What a key runs. Plugin commands are names, since plugins register them at runtime.
#[derive(Clone, Debug)]
pub enum Command {
    /// `count` is fixed by the binding (`goto-tab 3`) and beats a typed one.
    Builtin {
        name: &'static str,
        build: Build,
        count: Option<u16>,
    },
    Plugin(String),
    /// A program to run, from a table rather than a name.
    Shortcut(Shortcut),
}

impl Command {
    pub fn name(&self) -> &str {
        match self {
            Command::Builtin { name, .. } => name,
            Command::Plugin(name) => name,
            // No dot, so it can never be mistaken for a plugin command.
            Command::Shortcut(_) => "shortcut",
        }
    }

    /// As config spells it, count included; a shortcut shows what it runs.
    pub fn label(&self) -> String {
        match self {
            Command::Builtin { name, count: Some(n), .. } => format!("{name} {n}"),
            Command::Shortcut(s) => s.label(),
            other => other.name().to_string(),
        }
    }
}

/// `plugin.command`: the dot keeps plugin names from ever colliding with built-ins.
pub fn is_plugin_name(name: &str) -> bool {
    let ok = |part: &str| {
        !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    };
    matches!(name.split_once('.'), Some((plugin, cmd)) if ok(plugin) && cmd.split('.').all(ok)) && name.len() <= 64
}

fn n(count: Option<u16>) -> i32 {
    count.unwrap_or(1) as i32
}

pub const COMMANDS: &[(&str, Build)] = &[
    ("vsplit", |_| Action::Split(Axis::Horizontal)),
    ("split", |_| Action::Split(Axis::Vertical)),
    ("focus-left", |_| Action::Focus(Direction::Left)),
    ("focus-down", |_| Action::Focus(Direction::Down)),
    ("focus-up", |_| Action::Focus(Direction::Up)),
    ("focus-right", |_| Action::Focus(Direction::Right)),
    ("focus-next", |c| c.map_or(Action::FocusNext, Action::FocusNth)),
    ("focus-prev", |c| c.map_or(Action::FocusPrev, Action::FocusNth)),
    ("focus-last-used", |_| Action::FocusPrevious),
    ("focus-first", |_| Action::FocusFirst),
    ("focus-last", |_| Action::FocusLastPane),
    ("close", |_| Action::Close),
    ("only", |_| Action::Only),
    ("grow-height", |c| Action::Grow(Axis::Vertical, n(c))),
    ("shrink-height", |c| Action::Grow(Axis::Vertical, -n(c))),
    ("grow-width", |c| Action::Grow(Axis::Horizontal, n(c))),
    ("shrink-width", |c| Action::Grow(Axis::Horizontal, -n(c))),
    ("equalize", |_| Action::Equalize),
    ("maximize-height", |c| c.map_or(Action::Maximize(Axis::Vertical), |c| Action::SetSize(Axis::Vertical, c))),
    ("maximize-width", |c| c.map_or(Action::Maximize(Axis::Horizontal), |c| Action::SetSize(Axis::Horizontal, c))),
    ("move-left", |_| Action::MoveToEdge(Direction::Left)),
    ("move-down", |_| Action::MoveToEdge(Direction::Down)),
    ("move-up", |_| Action::MoveToEdge(Direction::Up)),
    ("move-right", |_| Action::MoveToEdge(Direction::Right)),
    ("exchange", |_| Action::Exchange),
    ("rotate", |_| Action::Rotate),
    ("prompt-prev", |c| Action::JumpPrompt(-n(c))),
    ("prompt-next", |c| Action::JumpPrompt(n(c))),
    ("copy-output", |_| Action::CopyLastOutput),
    ("copy-command", |_| Action::CopyLastCommand),
    ("new-tab", |_| Action::NewTab),
    ("move-to-new-tab", |_| Action::MoveToNewTab),
    ("next-tab", |c| c.map_or(Action::NextTab, Action::GotoTab)),
    ("prev-tab", |_| Action::PrevTab),
    ("switcher", |_| Action::OpenSwitcher),
    ("new-session", |_| Action::NewSession),
    ("rename-session", |_| Action::RenameSession),
    ("next-session", |_| Action::NextSession),
    ("prev-session", |_| Action::PrevSession),
    ("detach", |_| Action::Detach),
    ("send-leader", |_| Action::SendLeader),
    ("frame-time", |_| Action::ToggleFrameTime),
    ("toggle-floats", |_| Action::ToggleFloats),
    ("new-float", |_| Action::NewFloat),
    ("goto-tab", |c| Action::GotoTab(c.unwrap_or(1))),
    ("last-tab", |_| Action::LastTab),
    ("last-session", |_| Action::LastSession),
    ("zoom", |_| Action::ToggleZoom),
    ("broadcast", |_| Action::ToggleBroadcast),
    ("float-or-tile", |_| Action::FloatOrTile),
    ("rename-tab", |_| Action::RenameTab),
    ("visual", |_| Action::Visual(Selecting::Nothing)),
    ("visual-char", |_| Action::Visual(Selecting::Chars)),
    ("visual-line", |_| Action::Visual(Selecting::Lines)),
    ("visual-block", |_| Action::Visual(Selecting::Block)),
];

/// `name` or `name N`; only built-ins take a count.
pub fn command(spec: &str) -> Option<Command> {
    let (name, count) = match spec.split_once(' ') {
        Some((name, n)) => (name, Some(n.trim().parse::<u16>().ok().filter(|n| (1..=9999).contains(n))?)),
        None => (spec, None),
    };
    if is_plugin_name(name) {
        return count.is_none().then(|| Command::Plugin(name.to_string()));
    }
    COMMANDS.iter().find(|(n, _)| *n == name).map(|&(name, build)| Command::Builtin { name, build, count })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_names_need_a_dot_and_plain_characters() {
        assert!(is_plugin_name("sessions.save"));
        assert!(is_plugin_name("my-plugin.do_it.now"));
        for bad in ["save", ".save", "sessions.", "Sessions.save", "a..b", "a.b c"] {
            assert!(!is_plugin_name(bad), "{bad}");
        }
        assert!(matches!(command("sessions.save"), Some(Command::Plugin(n)) if n == "sessions.save"));
        assert!(matches!(command("vsplit"), Some(Command::Builtin { name: "vsplit", .. })));
    }

    #[test]
    fn names_are_unique() {
        for (i, (a, _)) in COMMANDS.iter().enumerate() {
            assert!(COMMANDS[i + 1..].iter().all(|(b, _)| a != b), "{a} listed twice");
        }
    }

    #[test]
    fn counts_belong_to_the_command() {
        let run = |name: &str, count| match command(name) {
            Some(Command::Builtin { build, .. }) => build(count),
            other => panic!("{other:?}"),
        };
        assert_eq!(run("focus-next", Some(3)), Action::FocusNth(3));
        assert_eq!(run("focus-next", None), Action::FocusNext);
        assert_eq!(run("shrink-width", Some(5)), Action::Grow(Axis::Horizontal, -5));
        assert!(command("nope").is_none());
    }

    #[test]
    fn a_binding_can_fix_the_count() {
        let Some(Command::Builtin { build, count, .. }) = command("goto-tab 3") else { panic!() };
        assert_eq!(build(count), Action::GotoTab(3));
        assert_eq!(command("goto-tab 3").unwrap().label(), "goto-tab 3");
        for bad in ["goto-tab 0", "goto-tab x", "goto-tab 3 4", "sessions.save 2"] {
            assert!(command(bad).is_none(), "{bad}");
        }
    }
}
