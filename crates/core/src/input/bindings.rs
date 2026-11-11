//! The defaults are nvim's `CTRL-W` keys; config merges over them, so a user file only says what differs.

use super::chord::Chord;
use super::commands::{Command, command};

#[derive(Clone, Debug)]
pub struct Bindings {
    pub leader: Chord,
    leader_keys: Vec<(Vec<Chord>, Command)>,
    direct: Vec<(Chord, Command)>,
    /// Sequences config set to `"none"`; a plugin's default key never claims them.
    unbound: Vec<Vec<Chord>>,
}

#[derive(Clone, Debug)]
pub enum Lookup {
    Found(Command),
    Prefix,
    Unbound,
}

const DEFAULT_LEADER: &str = "ctrl+space";

const DEFAULTS: &[(&str, &str)] = &[
    ("v", "vsplit"),
    ("s", "split"),
    ("S", "split"),
    ("n", "split"),
    ("h", "focus-left"),
    ("j", "focus-down"),
    ("k", "focus-up"),
    ("l", "focus-right"),
    ("w", "focus-next"),
    ("W", "focus-prev"),
    ("p", "focus-last-used"),
    ("t", "focus-first"),
    ("b", "focus-last"),
    ("c", "close"),
    ("q", "close"),
    ("o", "only"),
    ("+", "grow-height"),
    ("-", "shrink-height"),
    (">", "grow-width"),
    ("<", "shrink-width"),
    ("=", "equalize"),
    ("_", "maximize-height"),
    ("|", "maximize-width"),
    ("H", "move-left"),
    ("J", "move-down"),
    ("K", "move-up"),
    ("L", "move-right"),
    ("x", "exchange"),
    ("r", "rotate"),
    ("[", "prompt-prev"),
    ("]", "prompt-next"),
    ("y", "copy-output"),
    ("Y", "copy-command"),
    ("N", "new-tab"),
    ("T", "move-to-new-tab"),
    ("g t", "next-tab"),
    ("g T", "prev-tab"),
    (";", "switcher"),
    // nvim's CTRL-W leaves these free; ( ) and $ are tmux's session keys.
    ("C", "new-session"),
    ("$", "rename-session"),
    (")", "next-session"),
    ("(", "prev-session"),
    ("d", "detach"),
    ("g p", "frame-time"),
    // nvim's CTRL-W f opens the file under the cursor, which means nothing in a multiplexer.
    ("f", "toggle-floats"),
    ("F", "new-float"),
    ("e", "float-or-tile"),
    // tmux's zoom and window rename; nvim's CTRL-W z only closes the preview window.
    ("z", "zoom"),
    // tmux's synchronize-panes has no key; nvim's CTRL-W leaves B free.
    ("B", "broadcast"),
    (",", "rename-tab"),
    // nvim's g<Tab> is the last accessed tab, and CTRL-^ the alternate file.
    ("g tab", "last-tab"),
    ("'", "last-session"),
    ("V", "visual"),
    // nvim's `v` and `CTRL-V` here are splits; `gv` reselects, the nearest free meaning.
    ("g v", "visual-char"),
    ("g V", "visual-line"),
    ("g ctrl+v", "visual-block"),
    ("1", "goto-tab 1"),
    ("2", "goto-tab 2"),
    ("3", "goto-tab 3"),
    ("4", "goto-tab 4"),
    ("5", "goto-tab 5"),
    ("6", "goto-tab 6"),
    ("7", "goto-tab 7"),
    ("8", "goto-tab 8"),
    ("9", "goto-tab 9"),
];

impl Default for Bindings {
    fn default() -> Bindings {
        let mut b = Bindings {
            leader: Chord::parse(DEFAULT_LEADER).expect("default leader parses"),
            leader_keys: Vec::new(),
            direct: Vec::new(),
            unbound: Vec::new(),
        };
        for (keys, name) in DEFAULTS {
            b.bind_leader(parse_sequence(keys).expect("default keys parse"), command(name));
        }
        b
    }
}

impl Bindings {
    pub fn bind_leader(&mut self, keys: Vec<Chord>, cmd: Option<Command>) {
        self.leader_keys.retain(|(k, _)| *k != keys);
        match cmd {
            Some(cmd) => self.leader_keys.push((keys, cmd)),
            None => self.unbound.push(keys),
        }
    }

    /// A plugin's suggested key, taken only if nothing has it, nothing it would hide or be
    /// hidden by, and config didn't clear it.
    pub fn bind_plugin_default(&mut self, keys: Vec<Chord>, name: &str) -> Result<(), String> {
        if let Lookup::Found(c) = self.after_leader(&keys)
            && c.name() == name
        {
            return Ok(());
        }
        let taken = !matches!(self.after_leader(&keys), Lookup::Unbound)
            || (1..keys.len()).any(|n| matches!(self.after_leader(&keys[..n]), Lookup::Found(_)))
            || self.unbound.contains(&keys);
        if taken {
            return Err(format!(
                "`{}` is already in use",
                keys.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" ")
            ));
        }
        self.leader_keys.push((keys, Command::Plugin(name.to_string())));
        Ok(())
    }

    pub fn leader_entries(&self) -> impl Iterator<Item = (&[Chord], &Command)> {
        self.leader_keys.iter().map(|(k, c)| (&k[..], c))
    }

    pub fn direct_entries(&self) -> impl Iterator<Item = (Chord, &Command)> {
        self.direct.iter().map(|(k, c)| (*k, c))
    }

    pub fn bind_direct(&mut self, chord: Chord, cmd: Option<Command>) {
        self.direct.retain(|(c, _)| *c != chord);
        if let Some(cmd) = cmd {
            self.direct.push((chord, cmd));
        }
    }

    pub fn after_leader(&self, keys: &[Chord]) -> Lookup {
        if let Some((_, cmd)) = self.leader_keys.iter().find(|(k, _)| k == keys) {
            return Lookup::Found(cmd.clone());
        }
        if self.leader_keys.iter().any(|(k, _)| k.starts_with(keys)) { Lookup::Prefix } else { Lookup::Unbound }
    }

    pub fn direct(&self, chord: Chord) -> Option<&Command> {
        self.direct.iter().find(|(c, _)| *c == chord).map(|(_, cmd)| cmd)
    }

    /// An exact match fires at once, so a longer sequence starting with it can never be typed.
    pub fn shadowed(&self) -> Vec<(Vec<Chord>, Vec<Chord>)> {
        let mut out = Vec::new();
        for (short, _) in &self.leader_keys {
            for (long, _) in &self.leader_keys {
                if long.len() > short.len() && long.starts_with(short) {
                    out.push((short.clone(), long.clone()));
                }
            }
        }
        out
    }
}

pub fn parse_sequence(spec: &str) -> Result<Vec<Chord>, String> {
    let keys = spec.split_whitespace().map(Chord::parse).collect::<Result<Vec<_>, _>>()?;
    if keys.is_empty() {
        return Err("empty key".into());
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::keybinds::Action;
    use crate::layout::Axis;

    fn found(b: &Bindings, keys: &str) -> Option<Action> {
        match b.after_leader(&parse_sequence(keys).unwrap()) {
            Lookup::Found(Command::Builtin { build, count, .. }) => Some(build(count)),
            _ => None,
        }
    }

    #[test]
    fn defaults_parse_and_nothing_is_shadowed() {
        let b = Bindings::default();
        assert_eq!(found(&b, "v"), Some(Action::Split(Axis::Horizontal)));
        assert_eq!(found(&b, "g t"), Some(Action::NextTab));
        assert!(matches!(b.after_leader(&parse_sequence("g").unwrap()), Lookup::Prefix));
        assert!(b.shadowed().is_empty());
        assert_eq!(found(&b, "7"), Some(Action::GotoTab(7)));
    }

    #[test]
    fn rebinding_replaces_and_none_unbinds() {
        let mut b = Bindings::default();
        b.bind_leader(parse_sequence("v").unwrap(), command("split"));
        assert_eq!(found(&b, "v"), Some(Action::Split(Axis::Vertical)));
        b.bind_leader(parse_sequence("v").unwrap(), None);
        assert!(matches!(b.after_leader(&parse_sequence("v").unwrap()), Lookup::Unbound));
    }

    #[test]
    fn plugin_defaults_take_only_free_keys() {
        let mut b = Bindings::default();
        assert!(b.bind_plugin_default(parse_sequence("g s").unwrap(), "sessions.save").is_ok());
        assert!(
            matches!(b.after_leader(&parse_sequence("g s").unwrap()), Lookup::Found(Command::Plugin(n)) if n == "sessions.save")
        );
        assert!(
            b.bind_plugin_default(parse_sequence("g s").unwrap(), "sessions.save").is_ok(),
            "re-registering is fine"
        );
        assert!(b.bind_plugin_default(parse_sequence("g s").unwrap(), "other.cmd").is_err());
        assert!(b.bind_plugin_default(parse_sequence("v").unwrap(), "other.cmd").is_err(), "a built-in key");
        assert!(b.bind_plugin_default(parse_sequence("g").unwrap(), "other.cmd").is_err(), "would hide g t");
        assert!(b.bind_plugin_default(parse_sequence("v x").unwrap(), "other.cmd").is_err(), "hidden by v");
        b.bind_leader(parse_sequence("z").unwrap(), None);
        assert!(b.bind_plugin_default(parse_sequence("z").unwrap(), "other.cmd").is_err(), "config cleared it");
    }

    #[test]
    fn g_ctrl_v_is_its_own_key_not_g_v() {
        let b = Bindings::default();
        assert!(
            matches!(b.after_leader(&parse_sequence("g ctrl+v").unwrap()), Lookup::Found(c) if c.name() == "visual-block")
        );
        assert!(
            matches!(b.after_leader(&parse_sequence("g v").unwrap()), Lookup::Found(c) if c.name() == "visual-char")
        );
    }

    #[test]
    fn shadowing_is_reported() {
        let mut b = Bindings::default();
        b.bind_leader(parse_sequence("g").unwrap(), command("close"));
        assert_eq!(b.shadowed().len(), 7, "g t, g T, g p, g tab, g v, g V and g ctrl+v");
    }
}
