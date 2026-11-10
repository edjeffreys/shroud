//! Must never panic or stall on malformed input: it decodes untrusted bytes, and the fuzz
//! targets hold it to that.

use super::event::{Event, KeyAction, KeyEvent, Mods, MouseAction, MouseButton, MouseEvent, key};
use super::outer_colors::parse_report;

// Unterminated OSC/DCS or paste beyond this is dropped rather than buffered forever.
const MAX_PENDING: usize = 1 << 20;

#[derive(Default, Debug)]
pub struct Decoder {
    buf: Vec<u8>,
    paste: Option<Vec<u8>>,
}

enum Step {
    Event(Event, usize),
    Skip(usize),
    PasteStart(usize),
    Incomplete,
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder::default()
    }

    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Event>) {
        self.buf.extend_from_slice(bytes);
        let mut pos = 0;
        while pos < self.buf.len() {
            if let Some(paste) = &mut self.paste {
                const END: &[u8] = b"\x1b[201~";
                let rest = &self.buf[pos..];
                if let Some(i) = find(rest, END) {
                    paste.extend_from_slice(&rest[..i]);
                    out.push(Event::Paste(self.paste.take().unwrap_or_default()));
                    pos += i + END.len();
                    continue;
                }
                let keep = (1..END.len()).rev().find(|&n| rest.ends_with(&END[..n])).unwrap_or(0);
                paste.extend_from_slice(&rest[..rest.len() - keep]);
                if paste.len() > MAX_PENDING {
                    out.push(Event::Paste(self.paste.take().unwrap_or_default()));
                }
                pos = self.buf.len() - keep;
                break;
            }
            match step(&self.buf[pos..]) {
                Step::Event(ev, n) => {
                    out.push(ev);
                    pos += n;
                }
                Step::PasteStart(n) => {
                    self.paste = Some(Vec::new());
                    pos += n;
                }
                Step::Skip(n) => pos += n,
                Step::Incomplete => break,
            }
        }
        self.buf.drain(..pos);
        if self.buf.len() > MAX_PENDING {
            self.buf.clear();
        }
    }

    pub fn has_pending(&self) -> bool {
        !self.buf.is_empty() && self.paste.is_none()
    }

    /// Call after a short idle: a lone ESC is indistinguishable from a split sequence until then.
    pub fn flush(&mut self, out: &mut Vec<Event>) {
        if self.paste.is_some() {
            return;
        }
        let pending = std::mem::take(&mut self.buf);
        if pending.first() == Some(&0x1b) {
            out.push(Event::Key(KeyEvent::press(key::ESCAPE, Mods::NONE)));
            self.feed(&pending[1..], out);
        }
    }
}

fn step(b: &[u8]) -> Step {
    match b[0] {
        0x1b => escape(b),
        _ => plain(b),
    }
}

fn plain(b: &[u8]) -> Step {
    let c = b[0];
    let ev = match c {
        0x0d => KeyEvent::press(key::ENTER, Mods::NONE),
        0x09 => KeyEvent::press(key::TAB, Mods::NONE),
        0x7f => KeyEvent::press(key::BACKSPACE, Mods::NONE),
        0x08 => KeyEvent::press(key::BACKSPACE, Mods::CTRL),
        0x00 => KeyEvent::press(key::SPACE, Mods::CTRL),
        0x01..=0x1a => KeyEvent::press(u32::from(c - 1 + b'a'), Mods::CTRL),
        0x1c..=0x1f => KeyEvent::press(u32::from(c - 0x1c + b'\\'), Mods::CTRL),
        _ => return text(b),
    };
    Step::Event(Event::Key(ev), 1)
}

fn text(b: &[u8]) -> Step {
    let len = match b[0] {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => return Step::Skip(1),
    };
    if b.len() < len {
        return Step::Incomplete;
    }
    let Ok(s) = std::str::from_utf8(&b[..len]) else { return Step::Skip(1) };
    let ch = s.chars().next().unwrap_or('\u{FFFD}');
    Step::Event(Event::Key(char_key(ch, Mods::NONE)), len)
}

fn char_key(ch: char, mods: Mods) -> KeyEvent {
    let lower = ch.to_lowercase().next().unwrap_or(ch);
    let mut ev = KeyEvent { code: lower as u32, mods, text: ch.to_string(), ..KeyEvent::default() };
    if lower != ch {
        ev.shifted = Some(ch as u32);
        ev.mods = mods | Mods::SHIFT;
    }
    ev
}

fn escape(b: &[u8]) -> Step {
    let Some(&kind) = b.get(1) else { return Step::Incomplete };
    match kind {
        b'[' => csi(b),
        b'O' => ss3(b),
        b']' | b'P' | b'_' | b'^' | b'X' => string(b),
        0x1b => Step::Event(Event::Key(KeyEvent::press(key::ESCAPE, Mods::NONE)), 1),
        _ => match plain(&b[1..]) {
            Step::Event(Event::Key(mut k), n) => {
                k.mods = k.mods | Mods::ALT;
                Step::Event(Event::Key(k), n + 1)
            }
            Step::Incomplete => Step::Incomplete,
            _ => Step::Skip(2),
        },
    }
}

// Only OSC colour reports mean anything here; every other string is skipped.
fn string(b: &[u8]) -> Step {
    for i in 2..b.len() {
        let len = match b[i] {
            0x07 => i + 1,
            0x1b if b.get(i + 1) == Some(&b'\\') => i + 2,
            _ => continue,
        };
        return match (b[1], parse_report(&b[2..i])) {
            (b']', Some((slot, rgb))) => Step::Event(Event::OuterColor { slot, rgb }, len),
            _ => Step::Skip(len),
        };
    }
    Step::Incomplete
}

fn ss3(b: &[u8]) -> Step {
    let Some(&f) = b.get(2) else { return Step::Incomplete };
    match legacy_final(f) {
        Some(code) => Step::Event(Event::Key(KeyEvent::press(code, Mods::NONE)), 3),
        None => Step::Skip(3),
    }
}

fn legacy_final(f: u8) -> Option<u32> {
    Some(match f {
        b'A' => key::UP,
        b'B' => key::DOWN,
        b'C' => key::RIGHT,
        b'D' => key::LEFT,
        b'H' => key::HOME,
        b'F' => key::END,
        b'E' => key::KP_BEGIN,
        b'P' => key::f(1),
        b'Q' => key::f(2),
        b'R' => key::f(3),
        b'S' => key::f(4),
        _ => return None,
    })
}

fn csi(b: &[u8]) -> Step {
    let mut i = 2;
    while i < b.len() && (0x20..=0x3f).contains(&b[i]) {
        i += 1;
    }
    let Some(&fin) = b.get(i) else { return Step::Incomplete };
    if !(0x40..=0x7e).contains(&fin) {
        return Step::Skip(i);
    }
    let len = i + 1;
    let body = &b[2..i];
    let ev = match (body.first(), fin) {
        (Some(b'<'), b'M' | b'm') => sgr_mouse(&body[1..], fin == b'M'),
        (Some(b'?'), b'n') if body == b"?997;1" || body == b"?997;2" => {
            Some(Event::ColorScheme { dark: body == b"?997;1" })
        }
        (Some(b'?'), b'y') if body.starts_with(b"?2027;") && body.ends_with(b"$") => {
            Some(Event::GraphemeClustering(matches!(&body[6..body.len() - 1], b"1" | b"3")))
        }
        (Some(b'?' | b'>' | b'='), _) => None,
        (_, b'I') if body.is_empty() => Some(Event::Focus(true)),
        (_, b'O') if body.is_empty() => Some(Event::Focus(false)),
        (_, b'u') => kitty_u(body),
        (_, b'~') => tilde(body),
        (_, b'Z') => Some(Event::Key(KeyEvent::press(key::TAB, Mods::SHIFT))),
        _ => legacy_final(fin).and_then(|code| with_mods(code, body)),
    };
    if body == b"200" && fin == b'~' {
        return Step::PasteStart(len);
    }
    match ev {
        Some(ev) => Step::Event(ev, len),
        None => Step::Skip(len),
    }
}

fn kitty_u(body: &[u8]) -> Option<Event> {
    let mut fields = body.split(|&c| c == b';');
    let mut codes = fields.next()?.split(|&c| c == b':');
    let code = num(codes.next()?)?;
    let shifted = codes.next().and_then(num);
    let base = codes.next().and_then(num);
    let (mods, action) = mods_field(fields.next());
    let text = fields
        .next()
        .map(|t| t.split(|&c| c == b':').filter_map(num).filter_map(char::from_u32).collect())
        .unwrap_or_default();
    Some(Event::Key(KeyEvent { code, shifted, base, mods, action, text }))
}

fn tilde(body: &[u8]) -> Option<Event> {
    let mut fields = body.split(|&c| c == b';');
    let code = match num(fields.next()?)? {
        2 => key::INSERT,
        3 => key::DELETE,
        5 => key::PAGE_UP,
        6 => key::PAGE_DOWN,
        7 => key::HOME,
        8 => key::END,
        n @ 11..=15 => key::f(n - 10),
        n @ 17..=21 => key::f(n - 11),
        n @ 23..=26 => key::f(n - 12),
        n @ 28..=29 => key::f(n - 13),
        n @ 31..=34 => key::f(n - 14),
        _ => return None,
    };
    let (mods, action) = mods_field(fields.next());
    Some(Event::Key(KeyEvent { code, mods, action, ..KeyEvent::default() }))
}

fn with_mods(code: u32, body: &[u8]) -> Option<Event> {
    let mut fields = body.split(|&c| c == b';');
    let first = fields.next().unwrap_or_default();
    // A first param other than 1 means this isn't a key (e.g. a cursor-position report).
    if !first.is_empty() && first != b"1" {
        return None;
    }
    let (mods, action) = mods_field(fields.next());
    Some(Event::Key(KeyEvent { code, mods, action, ..KeyEvent::default() }))
}

fn mods_field(field: Option<&[u8]>) -> (Mods, KeyAction) {
    let Some(field) = field else { return (Mods::NONE, KeyAction::Press) };
    let mut parts = field.split(|&c| c == b':');
    let mods = parts.next().and_then(num).map_or(0, |m| m.saturating_sub(1));
    let action = match parts.next().and_then(num) {
        Some(2) => KeyAction::Repeat,
        Some(3) => KeyAction::Release,
        _ => KeyAction::Press,
    };
    (Mods(mods.min(255) as u8), action)
}

fn sgr_mouse(body: &[u8], press: bool) -> Option<Event> {
    let mut f = body.split(|&c| c == b';');
    let cb = num(f.next()?)?;
    let x = num(f.next()?)?.saturating_sub(1).min(u16::MAX as u32) as u16;
    let y = num(f.next()?)?.saturating_sub(1).min(u16::MAX as u32) as u16;
    let mut mods = Mods::NONE;
    if cb & 4 != 0 {
        mods = mods | Mods::SHIFT;
    }
    if cb & 8 != 0 {
        mods = mods | Mods::ALT;
    }
    if cb & 16 != 0 {
        mods = mods | Mods::CTRL;
    }
    let motion = cb & 32 != 0;
    let button = match (cb & 0b1100_0011, motion) {
        (0, _) => Some(MouseButton::Left),
        (1, _) => Some(MouseButton::Middle),
        (2, _) => Some(MouseButton::Right),
        (3, true) => None,
        (3, false) => Some(MouseButton::Left),
        (64, _) => Some(MouseButton::WheelUp),
        (65, _) => Some(MouseButton::WheelDown),
        (66, _) => Some(MouseButton::WheelLeft),
        (67, _) => Some(MouseButton::WheelRight),
        (n, _) => Some(MouseButton::Other(n as u8)),
    };
    let action = if motion {
        MouseAction::Motion
    } else if press {
        MouseAction::Press
    } else {
        MouseAction::Release
    };
    Some(Event::Mouse(MouseEvent { action, button, x, y, mods }))
}

fn num(s: &[u8]) -> Option<u32> {
    if s.is_empty() || s.len() > 9 {
        return None;
    }
    s.iter().try_fold(0u32, |acc, &c| c.is_ascii_digit().then(|| acc * 10 + u32::from(c - b'0')))
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(chunks: &[&[u8]]) -> Vec<Event> {
        let mut d = Decoder::new();
        let mut out = Vec::new();
        for c in chunks {
            d.feed(c, &mut out);
        }
        out
    }

    fn key(code: u32, mods: Mods) -> Event {
        Event::Key(KeyEvent::press(code, mods))
    }

    #[test]
    fn kitty_ctrl_w_and_release() {
        assert_eq!(decode(&[b"\x1b[119;5u"]), vec![key('w' as u32, Mods::CTRL)]);
        let ev = decode(&[b"\x1b[119;5:3u"]);
        assert!(matches!(&ev[0], Event::Key(k) if k.action == KeyAction::Release));
    }

    #[test]
    fn kitty_alternates_and_text() {
        let ev = decode(&[b"\x1b[97:65;2;65u"]);
        let Event::Key(k) = &ev[0] else { panic!() };
        assert_eq!((k.code, k.shifted, k.mods, k.text.as_str()), (97, Some(65), Mods::SHIFT, "A"));
    }

    #[test]
    fn legacy_bytes_normalise_to_kitty_codes() {
        assert_eq!(decode(&[b"\x17"]), vec![key('w' as u32, Mods::CTRL)]);
        assert_eq!(decode(&[b"\x1b[1;5A"]), vec![key(key::UP, Mods::CTRL)]);
        assert_eq!(decode(&[b"\x1bOP"]), vec![key(key::f(1), Mods::NONE)]);
        assert_eq!(decode(&[b"\x1b[15~"]), vec![key(key::f(5), Mods::NONE)]);
        assert_eq!(decode(&[b"\x1bx"]), vec![Event::Key(KeyEvent { mods: Mods::ALT, ..char_key('x', Mods::NONE) })]);
    }

    #[test]
    fn sequences_split_across_reads() {
        assert_eq!(decode(&[b"\x1b[11", b"9;5u"]), vec![key('w' as u32, Mods::CTRL)]);
        assert_eq!(decode(&[b"\xe4", b"\xb8\x96"]).len(), 1);
    }

    #[test]
    fn lone_escape_waits_for_flush() {
        let mut d = Decoder::new();
        let mut out = Vec::new();
        d.feed(b"\x1b", &mut out);
        assert!(out.is_empty() && d.has_pending());
        d.flush(&mut out);
        assert_eq!(out, vec![key(key::ESCAPE, Mods::NONE)]);
    }

    #[test]
    fn bracketed_paste_survives_a_split_terminator() {
        let ev = decode(&[b"\x1b[200~hi\x1b[2", b"01~x"]);
        assert_eq!(ev[0], Event::Paste(b"hi".to_vec()));
        assert!(matches!(&ev[1], Event::Key(k) if k.text == "x"));
    }

    #[test]
    fn sgr_mouse_and_focus() {
        let ev = decode(&[b"\x1b[<0;10;5M\x1b[<65;1;1M\x1b[I"]);
        assert_eq!(
            ev[0],
            Event::Mouse(MouseEvent {
                action: MouseAction::Press,
                button: Some(MouseButton::Left),
                x: 9,
                y: 4,
                mods: Mods::NONE
            })
        );
        assert!(matches!(ev[1], Event::Mouse(MouseEvent { button: Some(MouseButton::WheelDown), .. })));
        assert_eq!(ev[2], Event::Focus(true));
    }

    #[test]
    fn colour_reports_with_either_terminator() {
        use crate::input::ColorSlot;
        let evs = decode(&[b"\x1b]11;rgb:0000/0000/ffff\x1b\\\x1b]4;3;rgb:ff/00/00\x07x"]);
        assert_eq!(evs[0], Event::OuterColor { slot: ColorSlot::Background, rgb: [0, 0, 255] });
        assert_eq!(evs[1], Event::OuterColor { slot: ColorSlot::Palette(3), rgb: [255, 0, 0] });
        assert_eq!(evs.len(), 3);
        assert_eq!(decode(&[b"\x1b]2;title\x07"]), vec![]);
    }

    #[test]
    fn grapheme_clustering_mode_report() {
        assert_eq!(decode(&[b"\x1b[?2027;1$y"]), vec![Event::GraphemeClustering(true)]);
        assert_eq!(
            decode(&[b"\x1b[?2027;3$y\x1b[?2027;0$y\x1b[?2027;2$y"]),
            vec![Event::GraphemeClustering(true), Event::GraphemeClustering(false), Event::GraphemeClustering(false)]
        );
    }

    #[test]
    fn colour_scheme_report() {
        assert_eq!(decode(&[b"\x1b[?997;2n"]), vec![Event::ColorScheme { dark: false }]);
    }

    #[test]
    fn garbage_never_panics() {
        let junk: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
        let mut d = Decoder::new();
        let mut out = Vec::new();
        d.feed(&junk, &mut out);
        d.feed(b"\x1b[999999999999;;;:::u\x1b[<;;M\x1b]unterminated", &mut out);
        d.flush(&mut out);
    }
}
