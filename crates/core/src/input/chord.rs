//! Shift folds into the character, so `H` and `shift+h` are one chord.

use super::event::{KeyEvent, Mods, key};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Key {
    Char(char),
    Named(u32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Chord {
    pub mods: Mods,
    pub key: Key,
}

const NAMES: &[(&str, u32)] = &[
    ("space", key::SPACE),
    ("enter", key::ENTER),
    ("tab", key::TAB),
    ("esc", key::ESCAPE),
    ("escape", key::ESCAPE),
    ("backspace", key::BACKSPACE),
    ("delete", key::DELETE),
    ("insert", key::INSERT),
    ("left", key::LEFT),
    ("right", key::RIGHT),
    ("up", key::UP),
    ("down", key::DOWN),
    ("home", key::HOME),
    ("end", key::END),
    ("pageup", key::PAGE_UP),
    ("pagedown", key::PAGE_DOWN),
];

const MODS: &[(&str, Mods)] =
    &[("ctrl", Mods::CTRL), ("alt", Mods::ALT), ("shift", Mods::SHIFT), ("super", Mods::SUPER), ("cmd", Mods::SUPER)];

impl Chord {
    pub const fn plain(c: char) -> Chord {
        Chord { mods: Mods::NONE, key: Key::Char(c) }
    }

    pub fn parse(spec: &str) -> Result<Chord, String> {
        let (mod_part, key_part) = match spec.strip_suffix("++") {
            Some(m) => (m, "+"),
            None if spec == "+" => ("", "+"),
            None => spec.rsplit_once('+').unwrap_or(("", spec)),
        };
        let mut mods = Mods::NONE;
        for m in mod_part.split('+').filter(|m| !m.is_empty()) {
            let (_, bit) = MODS
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(m))
                .ok_or_else(|| format!("unknown modifier `{m}` in `{spec}`"))?;
            mods = mods | *bit;
        }
        let mut chars = key_part.chars();
        let key = match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_graphic() => Key::Char(c),
            _ => named(key_part).ok_or_else(|| format!("unknown key `{key_part}` in `{spec}`"))?,
        };
        Ok(Chord { mods, key }.normalised())
    }

    pub fn from_event(ev: &KeyEvent) -> Option<Chord> {
        let mods = Mods(ev.mods.without_locks().0 & (Mods::CTRL.0 | Mods::ALT.0 | Mods::SHIFT.0 | Mods::SUPER.0));
        let code = ev.base.unwrap_or(ev.code);
        if NAMES.iter().any(|&(_, c)| c == ev.code) || (key::F1..=key::F35).contains(&ev.code) {
            return Some(Chord { mods, key: Key::Named(ev.code) }.normalised());
        }
        let c = if !mods.contains(Mods::CTRL)
            && !mods.contains(Mods::ALT)
            && let Some(t) = ev.text.chars().next().filter(char::is_ascii_graphic)
        {
            t
        } else if mods.contains(Mods::SHIFT) {
            let c = char::from_u32(ev.shifted.unwrap_or(code))?;
            // Without associated text, shift+letter still has to mean the capital.
            c.to_ascii_uppercase()
        } else {
            char::from_u32(code)?
        };
        Some(Chord { mods, key: Key::Char(c) }.normalised())
    }

    /// What the pane receives when the leader is pressed twice.
    pub fn to_event(self) -> KeyEvent {
        match self.key {
            Key::Named(code) => KeyEvent::press(code, self.mods),
            Key::Char(c) => {
                let plain = !self.mods.contains(Mods::CTRL)
                    && !self.mods.contains(Mods::ALT)
                    && !self.mods.contains(Mods::SUPER);
                let shift = if c.is_ascii_uppercase() { Mods::SHIFT } else { Mods::NONE };
                KeyEvent {
                    text: if plain { c.to_string() } else { String::new() },
                    ..KeyEvent::press(c.to_ascii_lowercase() as u32, self.mods | shift)
                }
            }
        }
    }

    /// nvim reads CTRL-<letter> and arrows after CTRL-W as the plain letters.
    pub fn nvim_alias(self) -> Option<Chord> {
        match self.key {
            Key::Named(key::LEFT) => Some(Chord::plain('h')),
            Key::Named(key::DOWN) => Some(Chord::plain('j')),
            Key::Named(key::UP) => Some(Chord::plain('k')),
            Key::Named(key::RIGHT) => Some(Chord::plain('l')),
            Key::Char(c) if self.mods == Mods::CTRL && c.is_ascii_lowercase() => Some(Chord::plain(c)),
            _ => None,
        }
    }

    pub fn digit(self) -> Option<u32> {
        match self.key {
            Key::Char(c) if self.mods == Mods::NONE => c.to_digit(10),
            _ => None,
        }
    }

    fn normalised(mut self) -> Chord {
        if let Key::Char(c) = self.key
            && self.mods.contains(Mods::SHIFT)
        {
            self.mods = Mods(self.mods.0 & !Mods::SHIFT.0);
            self.key = Key::Char(c.to_ascii_uppercase());
        }
        if let Key::Char(' ') = self.key {
            self.key = Key::Named(key::SPACE);
        }
        self
    }
}

impl std::fmt::Display for Chord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (name, bit) in MODS.iter().filter(|(n, _)| *n != "cmd") {
            if self.mods.contains(*bit) {
                write!(f, "{name}+")?;
            }
        }
        match self.key {
            Key::Char(c) => write!(f, "{c}"),
            Key::Named(code) if (key::F1..=key::F35).contains(&code) => write!(f, "f{}", code - key::F1 + 1),
            Key::Named(code) => f.write_str(NAMES.iter().find(|(_, c)| *c == code).map_or("?", |(n, _)| n)),
        }
    }
}

fn named(name: &str) -> Option<Key> {
    let lower = name.to_ascii_lowercase();
    if let Some(&(_, code)) = NAMES.iter().find(|(n, _)| *n == lower) {
        return Some(Key::Named(code));
    }
    let n: u32 = lower.strip_prefix('f')?.parse().ok()?;
    (1..=35).contains(&n).then(|| Key::Named(key::f(n)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(mods: Mods, c: char) -> Chord {
        Chord { mods, key: Key::Char(c) }
    }

    #[test]
    fn parses_specs() {
        assert_eq!(Chord::parse("ctrl+space"), Ok(Chord { mods: Mods::CTRL, key: Key::Named(key::SPACE) }));
        assert_eq!(Chord::parse("H"), Ok(ch(Mods::NONE, 'H')));
        assert_eq!(Chord::parse("shift+h"), Ok(ch(Mods::NONE, 'H')));
        assert_eq!(Chord::parse("Ctrl+Alt+h"), Ok(ch(Mods::CTRL | Mods::ALT, 'h')));
        assert_eq!(Chord::parse("+"), Ok(ch(Mods::NONE, '+')));
        assert_eq!(Chord::parse("ctrl++"), Ok(ch(Mods::CTRL, '+')));
        assert_eq!(Chord::parse("f5"), Ok(Chord { mods: Mods::NONE, key: Key::Named(key::f(5)) }));
        assert!(Chord::parse("hyper+h").is_err());
        assert!(Chord::parse("ctrl+banana").is_err());
        assert!(Chord::parse("").is_err());
    }

    #[test]
    fn events_match_their_specs() {
        let cases: &[(KeyEvent, &str)] = &[
            (KeyEvent::press(key::SPACE, Mods::CTRL), "ctrl+space"),
            (KeyEvent { text: "H".into(), ..KeyEvent::press('h' as u32, Mods::SHIFT) }, "H"),
            (KeyEvent::press('h' as u32, Mods::SHIFT), "H"),
            (KeyEvent::press('h' as u32, Mods::CTRL), "ctrl+h"),
            (
                KeyEvent {
                    shifted: Some('+' as u32),
                    base: Some('=' as u32),
                    ..KeyEvent::press('=' as u32, Mods::SHIFT)
                },
                "+",
            ),
            (KeyEvent { base: Some('v' as u32), text: "м".into(), ..KeyEvent::press('м' as u32, Mods::NONE) }, "v"),
            (KeyEvent { text: "å".into(), base: Some('a' as u32), ..KeyEvent::press('a' as u32, Mods::ALT) }, "alt+a"),
            (KeyEvent::press(key::LEFT, Mods::NONE), "left"),
            (KeyEvent::press('a' as u32, Mods::CAPS_LOCK), "a"),
        ];
        for (ev, spec) in cases {
            assert_eq!(Chord::from_event(ev), Chord::parse(spec).ok(), "{spec}");
        }
    }

    #[test]
    fn display_round_trips() {
        for spec in ["ctrl+space", "H", "ctrl+alt+h", "+", "f12", "esc", "super+left"] {
            assert_eq!(Chord::parse(spec).unwrap().to_string(), spec);
        }
    }

    #[test]
    fn to_event_round_trips() {
        for spec in ["ctrl+space", "ctrl+a", "H", "alt+x", "f3"] {
            let c = Chord::parse(spec).unwrap();
            assert_eq!(Chord::from_event(&c.to_event()), Some(c), "{spec}");
        }
    }

    #[test]
    fn nvim_aliases() {
        assert_eq!(Chord::parse("ctrl+h").unwrap().nvim_alias(), Some(Chord::plain('h')));
        assert_eq!(Chord::parse("down").unwrap().nvim_alias(), Some(Chord::plain('j')));
        assert_eq!(Chord::parse("h").unwrap().nvim_alias(), None);
    }
}
