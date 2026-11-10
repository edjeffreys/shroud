//! libghostty-vt wants physical (W3C) key codes; shroud's events carry kitty codes,
//! so this maps one to the other before encoding in the pane's own keyboard mode.

use std::ptr;

use shroud_core::input::{KeyAction, KeyEvent, Mods, key};

use crate::ffi::*;
use crate::terminal::{Terminal, encode_into};
use crate::{Error, check};

pub struct KeyEncoder {
    enc: GhosttyKeyEncoder,
    ev: GhosttyKeyEvent,
}

impl std::fmt::Debug for KeyEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyEncoder").finish_non_exhaustive()
    }
}

impl KeyEncoder {
    pub fn new() -> Result<KeyEncoder, Error> {
        let mut k = KeyEncoder { enc: ptr::null_mut(), ev: ptr::null_mut() };
        // SAFETY: out-pointers valid; Drop frees what succeeded.
        unsafe {
            check(ghostty_key_encoder_new(ptr::null(), &mut k.enc))?;
            check(ghostty_key_event_new(ptr::null(), &mut k.ev))?;
        }
        Ok(k)
    }

    pub fn encode(&mut self, term: &Terminal, ev: &KeyEvent, out: &mut Vec<u8>) {
        let text = if ev.text.is_empty() { implied_text(ev) } else { ev.text.clone() };
        // SAFETY: handles are live; `text` outlives the encode call that borrows it.
        unsafe {
            // Modes change with every write, so sync right before encoding.
            ghostty_key_encoder_set_from_terminal(self.enc, term.raw());
            ghostty_key_event_set_action(self.ev, action(ev.action));
            ghostty_key_event_set_key(self.ev, physical(ev.base.unwrap_or(ev.code)));
            ghostty_key_event_set_mods(self.ev, mods(ev.mods));
            ghostty_key_event_set_consumed_mods(self.ev, consumed(ev, &text));
            ghostty_key_event_set_utf8(self.ev, GhosttyString { ptr: text.as_ptr(), len: text.len() });
            let unshifted = if is_functional(ev.code) { 0 } else { ev.code };
            ghostty_key_event_set_unshifted_codepoint(self.ev, unshifted);
            encode_into(out, |buf, len, written| ghostty_key_encoder_encode(self.enc, self.ev, buf, len, written));
        }
    }
}

impl Drop for KeyEncoder {
    fn drop(&mut self) {
        // SAFETY: freed once.
        unsafe {
            ghostty_key_event_free(self.ev);
            ghostty_key_encoder_free(self.enc);
        }
    }
}

// Ghostty reports a macOS Option that composed text (`#` is Option+3 on UK) as Alt; unless it's
// marked consumed, kitty-mode panes get an Alt chord instead of the text.
fn consumed(ev: &KeyEvent, text: &str) -> u16 {
    if text.is_empty() {
        return 0;
    }
    let mut out = 0;
    if ev.mods.contains(Mods::SHIFT) && (ev.shifted.is_some() || composed(ev, text)) {
        out |= GHOSTTY_MODS_SHIFT;
    }
    if ev.mods.contains(Mods::ALT) && !ev.mods.contains(Mods::CTRL) && composed(ev, text) {
        out |= GHOSTTY_MODS_ALT;
    }
    out as u16
}

fn composed(ev: &KeyEvent, text: &str) -> bool {
    let own = |cp: Option<u32>| cp.and_then(char::from_u32).is_some_and(|c| text.chars().eq([c]));
    !is_functional(ev.code) && !text.chars().any(char::is_control) && !own(Some(ev.code)) && !own(ev.shifted)
}

// Terminals without the associated-text flag omit it; legacy encoding still needs it.
fn implied_text(ev: &KeyEvent) -> String {
    if is_functional(ev.code)
        || ev.mods.contains(Mods::CTRL)
        || ev.mods.contains(Mods::ALT)
        || ev.mods.contains(Mods::SUPER)
    {
        return String::new();
    }
    let cp = if ev.mods.contains(Mods::SHIFT) { ev.shifted.unwrap_or(ev.code) } else { ev.code };
    match cp {
        key::ENTER => "\r".into(),
        key::TAB => "\t".into(),
        key::BACKSPACE | key::ESCAPE => String::new(),
        _ => char::from_u32(cp).filter(|c| !c.is_control()).map(String::from).unwrap_or_default(),
    }
}

fn is_functional(code: u32) -> bool {
    (0xE000..=0xF8FF).contains(&code) || matches!(code, key::ENTER | key::TAB | key::BACKSPACE | key::ESCAPE)
}

fn action(a: KeyAction) -> GhosttyKeyAction {
    match a {
        KeyAction::Press => GHOSTTY_KEY_ACTION_PRESS,
        KeyAction::Repeat => GHOSTTY_KEY_ACTION_REPEAT,
        KeyAction::Release => GHOSTTY_KEY_ACTION_RELEASE,
    }
}

fn mods(m: Mods) -> GhosttyMods {
    let mut out = 0u32;
    for (from, to) in [
        (Mods::SHIFT, GHOSTTY_MODS_SHIFT),
        (Mods::CTRL, GHOSTTY_MODS_CTRL),
        (Mods::ALT, GHOSTTY_MODS_ALT),
        (Mods::SUPER, GHOSTTY_MODS_SUPER),
        (Mods::CAPS_LOCK, GHOSTTY_MODS_CAPS_LOCK),
        (Mods::NUM_LOCK, GHOSTTY_MODS_NUM_LOCK),
    ] {
        if m.contains(from) {
            out |= to;
        }
    }
    out as GhosttyMods
}

fn physical(code: u32) -> GhosttyKey {
    let Some(c) = char::from_u32(code) else { return GHOSTTY_KEY_UNIDENTIFIED };
    match c {
        'a'..='z' => GHOSTTY_KEY_A + (code - 'a' as u32),
        '0'..='9' => GHOSTTY_KEY_DIGIT_0 + (code - '0' as u32),
        '`' => GHOSTTY_KEY_BACKQUOTE,
        '\\' => GHOSTTY_KEY_BACKSLASH,
        '[' => GHOSTTY_KEY_BRACKET_LEFT,
        ']' => GHOSTTY_KEY_BRACKET_RIGHT,
        ',' => GHOSTTY_KEY_COMMA,
        '=' => GHOSTTY_KEY_EQUAL,
        '-' => GHOSTTY_KEY_MINUS,
        '.' => GHOSTTY_KEY_PERIOD,
        '\'' => GHOSTTY_KEY_QUOTE,
        ';' => GHOSTTY_KEY_SEMICOLON,
        '/' => GHOSTTY_KEY_SLASH,
        ' ' => GHOSTTY_KEY_SPACE,
        _ => match code {
            key::ENTER => GHOSTTY_KEY_ENTER,
            key::TAB => GHOSTTY_KEY_TAB,
            key::BACKSPACE => GHOSTTY_KEY_BACKSPACE,
            key::ESCAPE => GHOSTTY_KEY_ESCAPE,
            key::INSERT => GHOSTTY_KEY_INSERT,
            key::DELETE => GHOSTTY_KEY_DELETE,
            key::LEFT => GHOSTTY_KEY_ARROW_LEFT,
            key::RIGHT => GHOSTTY_KEY_ARROW_RIGHT,
            key::UP => GHOSTTY_KEY_ARROW_UP,
            key::DOWN => GHOSTTY_KEY_ARROW_DOWN,
            key::PAGE_UP => GHOSTTY_KEY_PAGE_UP,
            key::PAGE_DOWN => GHOSTTY_KEY_PAGE_DOWN,
            key::HOME => GHOSTTY_KEY_HOME,
            key::END => GHOSTTY_KEY_END,
            key::CAPS_LOCK => GHOSTTY_KEY_CAPS_LOCK,
            key::SCROLL_LOCK => GHOSTTY_KEY_SCROLL_LOCK,
            key::NUM_LOCK => GHOSTTY_KEY_NUM_LOCK,
            key::PRINT_SCREEN => GHOSTTY_KEY_PRINT_SCREEN,
            key::PAUSE => GHOSTTY_KEY_PAUSE,
            key::MENU => GHOSTTY_KEY_CONTEXT_MENU,
            c if (key::F1..key::F1 + 25).contains(&c) => GHOSTTY_KEY_F1 + (c - key::F1),
            c if (key::KP_0..=key::KP_9).contains(&c) => GHOSTTY_KEY_NUMPAD_0 + (c - key::KP_0),
            key::KP_DECIMAL => GHOSTTY_KEY_NUMPAD_DECIMAL,
            key::KP_DIVIDE => GHOSTTY_KEY_NUMPAD_DIVIDE,
            key::KP_MULTIPLY => GHOSTTY_KEY_NUMPAD_MULTIPLY,
            key::KP_SUBTRACT => GHOSTTY_KEY_NUMPAD_SUBTRACT,
            key::KP_ADD => GHOSTTY_KEY_NUMPAD_ADD,
            key::KP_ENTER => GHOSTTY_KEY_NUMPAD_ENTER,
            key::KP_EQUAL => GHOSTTY_KEY_NUMPAD_EQUAL,
            key::KP_SEPARATOR => GHOSTTY_KEY_NUMPAD_SEPARATOR,
            key::KP_LEFT => GHOSTTY_KEY_NUMPAD_LEFT,
            key::KP_RIGHT => GHOSTTY_KEY_NUMPAD_RIGHT,
            key::KP_UP => GHOSTTY_KEY_NUMPAD_UP,
            key::KP_DOWN => GHOSTTY_KEY_NUMPAD_DOWN,
            key::KP_PAGE_UP => GHOSTTY_KEY_NUMPAD_PAGE_UP,
            key::KP_PAGE_DOWN => GHOSTTY_KEY_NUMPAD_PAGE_DOWN,
            key::KP_HOME => GHOSTTY_KEY_NUMPAD_HOME,
            key::KP_END => GHOSTTY_KEY_NUMPAD_END,
            key::KP_INSERT => GHOSTTY_KEY_NUMPAD_INSERT,
            key::KP_DELETE => GHOSTTY_KEY_NUMPAD_DELETE,
            key::KP_BEGIN => GHOSTTY_KEY_NUMPAD_BEGIN,
            key::LEFT_SHIFT => GHOSTTY_KEY_SHIFT_LEFT,
            key::LEFT_CONTROL => GHOSTTY_KEY_CONTROL_LEFT,
            key::LEFT_ALT => GHOSTTY_KEY_ALT_LEFT,
            key::LEFT_SUPER => GHOSTTY_KEY_META_LEFT,
            key::RIGHT_SHIFT => GHOSTTY_KEY_SHIFT_RIGHT,
            key::RIGHT_CONTROL => GHOSTTY_KEY_CONTROL_RIGHT,
            key::RIGHT_ALT => GHOSTTY_KEY_ALT_RIGHT,
            key::RIGHT_SUPER => GHOSTTY_KEY_META_RIGHT,
            _ => GHOSTTY_KEY_UNIDENTIFIED,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(term: &Terminal, ev: KeyEvent) -> Vec<u8> {
        let mut out = Vec::new();
        KeyEncoder::new().unwrap().encode(term, &ev, &mut out);
        out
    }

    #[test]
    fn follows_the_panes_keyboard_mode() {
        let mut t = Terminal::new(80, 24).unwrap();
        let ctrl_w = KeyEvent::press('w' as u32, Mods::CTRL);
        assert_eq!(encode(&t, ctrl_w.clone()), b"\x17");
        t.write(b"\x1b[>1u");
        assert_eq!(encode(&t, ctrl_w), b"\x1b[119;5u");
    }

    #[test]
    fn cursor_keys_follow_application_mode() {
        let mut t = Terminal::new(80, 24).unwrap();
        let up = KeyEvent::press(key::UP, Mods::NONE);
        assert_eq!(encode(&t, up.clone()), b"\x1b[A");
        t.write(b"\x1b[?1h");
        assert_eq!(encode(&t, up), b"\x1bOA");
    }

    #[test]
    fn plain_and_shifted_text() {
        let t = Terminal::new(80, 24).unwrap();
        assert_eq!(encode(&t, KeyEvent::press('a' as u32, Mods::NONE)), b"a");
        let upper =
            KeyEvent { shifted: Some('A' as u32), text: "A".into(), ..KeyEvent::press('a' as u32, Mods::SHIFT) };
        assert_eq!(encode(&t, upper), b"A");
        assert_eq!(encode(&t, KeyEvent::press(key::ENTER, Mods::NONE)), b"\r");
    }

    #[test]
    fn option_composed_text_reaches_kitty_panes() {
        let mut t = Terminal::new(80, 24).unwrap();
        t.write(b"\x1b[>1u");
        let hash = KeyEvent { text: "#".into(), ..KeyEvent::press('3' as u32, Mods::ALT) };
        assert_eq!(encode(&t, hash), b"#");
        let euro = KeyEvent { text: "€".into(), ..KeyEvent::press('2' as u32, Mods::ALT | Mods::SHIFT) };
        assert_eq!(encode(&t, euro), "€".as_bytes());
        // Option-as-Alt sends the key's own character, so it stays a chord.
        let alt_3 = KeyEvent { text: "3".into(), ..KeyEvent::press('3' as u32, Mods::ALT) };
        assert_eq!(encode(&t, alt_3), b"\x1b[51;3u");
        assert_eq!(encode(&t, KeyEvent::press('3' as u32, Mods::ALT)), b"\x1b[51;3u");
    }
}
