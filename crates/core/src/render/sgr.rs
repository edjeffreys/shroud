//! Style transitions as SGR bytes. With `theme`, the only code that knows colour encodings.

use super::cell::{Style, Underline};
use super::color::Color;
use super::num::push_u16;

pub fn transition(out: &mut Vec<u8>, from: &Style, to: &Style) {
    if from == to {
        return;
    }
    let mut p = Params::new(out);
    if *to == Style::PLAIN || reset_is_cheaper(from, to) {
        p.add(b"0");
        add_all(&mut p, &Style::PLAIN, to);
    } else {
        add_all(&mut p, from, to);
    }
    p.finish();
}

fn reset_is_cheaper(from: &Style, to: &Style) -> bool {
    let removed = [
        from.bold && !to.bold,
        from.faint && !to.faint,
        from.italic && !to.italic,
        from.blink && !to.blink,
        from.inverse && !to.inverse,
        from.invisible && !to.invisible,
        from.strikethrough && !to.strikethrough,
        from.overline && !to.overline,
        from.underline != Underline::None && to.underline == Underline::None,
        from.fg != Color::Default && to.fg == Color::Default,
        from.bg != Color::Default && to.bg == Color::Default,
    ];
    removed.iter().filter(|r| **r).count() >= 2
}

fn add_all(p: &mut Params, from: &Style, to: &Style) {
    // 22 clears bold and faint together, so re-add whichever survives.
    if (from.bold && !to.bold) || (from.faint && !to.faint) {
        p.add(b"22");
        if to.bold {
            p.add(b"1");
        }
        if to.faint {
            p.add(b"2");
        }
    } else {
        if to.bold && !from.bold {
            p.add(b"1");
        }
        if to.faint && !from.faint {
            p.add(b"2");
        }
    }
    flag(p, from.italic, to.italic, b"3", b"23");
    flag(p, from.blink, to.blink, b"5", b"25");
    flag(p, from.inverse, to.inverse, b"7", b"27");
    flag(p, from.invisible, to.invisible, b"8", b"28");
    flag(p, from.strikethrough, to.strikethrough, b"9", b"29");
    flag(p, from.overline, to.overline, b"53", b"55");
    if from.underline != to.underline {
        p.add(match to.underline {
            Underline::None => b"24",
            Underline::Single => b"4",
            Underline::Double => b"4:2",
            Underline::Curly => b"4:3",
            Underline::Dotted => b"4:4",
            Underline::Dashed => b"4:5",
        });
    }
    if from.fg != to.fg {
        color(p, to.fg, Target::Fg);
    }
    if from.bg != to.bg {
        color(p, to.bg, Target::Bg);
    }
    if from.underline_color != to.underline_color {
        color(p, to.underline_color, Target::Underline);
    }
}

fn flag(p: &mut Params, was: bool, is: bool, on: &[u8], off: &[u8]) {
    if was != is {
        p.add(if is { on } else { off });
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Target {
    Fg,
    Bg,
    Underline,
}

fn color(p: &mut Params, c: Color, target: Target) {
    let (default, extended): (&[u8], &[u8]) = match target {
        Target::Fg => (b"39", b"38"),
        Target::Bg => (b"49", b"48"),
        Target::Underline => (b"59", b"58"),
    };
    match c {
        Color::Default => p.add(default),
        // Short forms for 0–15 so the user's 16-colour theme maps exactly.
        Color::Palette(i) if i < 16 && target != Target::Underline => {
            let base: u16 = match (target, i < 8) {
                (Target::Fg, true) => 30,
                (Target::Fg, false) => 90,
                (_, true) => 40,
                (_, false) => 100,
            };
            p.add_num(base + (i & 7) as u16);
        }
        Color::Palette(i) => {
            p.add(extended);
            p.raw(b";5;");
            push_u16(p.out, i as u16);
        }
        Color::Rgb(r, g, b) => {
            p.add(extended);
            p.raw(b";2;");
            push_u16(p.out, r as u16);
            p.raw(b";");
            push_u16(p.out, g as u16);
            p.raw(b";");
            push_u16(p.out, b as u16);
        }
    }
}

struct Params<'a> {
    out: &'a mut Vec<u8>,
    any: bool,
}

impl<'a> Params<'a> {
    fn new(out: &'a mut Vec<u8>) -> Params<'a> {
        out.extend_from_slice(b"\x1b[");
        Params { out, any: false }
    }

    fn sep(&mut self) {
        if self.any {
            self.out.push(b';');
        }
        self.any = true;
    }

    fn add(&mut self, s: &[u8]) {
        self.sep();
        self.out.extend_from_slice(s);
    }

    fn add_num(&mut self, n: u16) {
        self.sep();
        push_u16(self.out, n);
    }

    fn raw(&mut self, s: &[u8]) {
        self.out.extend_from_slice(s);
    }

    fn finish(self) {
        if self.any {
            self.out.push(b'm');
        } else {
            let len = self.out.len() - 2;
            self.out.truncate(len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(from: Style, to: Style) -> String {
        let mut out = Vec::new();
        transition(&mut out, &from, &to);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn palette_short_forms_and_extended() {
        assert_eq!(t(Style::PLAIN, Style::fg(Color::Palette(1))), "\x1b[31m");
        assert_eq!(
            t(Style::PLAIN, Style { fg: Color::Palette(9), bg: Color::Palette(4), ..Style::PLAIN }),
            "\x1b[91;44m"
        );
        assert_eq!(t(Style::PLAIN, Style::fg(Color::Palette(200))), "\x1b[38;5;200m");
        assert_eq!(t(Style::PLAIN, Style { bg: Color::Rgb(1, 2, 3), ..Style::PLAIN }), "\x1b[48;2;1;2;3m");
    }

    #[test]
    fn default_colour_is_emitted_as_default() {
        let to = Style { bold: true, ..Style::PLAIN };
        assert_eq!(t(Style::fg(Color::Palette(1)), to), "\x1b[1;39m");
    }

    #[test]
    fn bold_off_keeps_faint() {
        let from = Style { bold: true, faint: true, ..Style::PLAIN };
        assert_eq!(t(from, Style { faint: true, ..Style::PLAIN }), "\x1b[22;2m");
    }

    #[test]
    fn many_removals_use_reset() {
        let from = Style { italic: true, inverse: true, fg: Color::Palette(2), ..Style::PLAIN };
        assert_eq!(t(from, Style { bold: true, ..Style::PLAIN }), "\x1b[0;1m");
        assert_eq!(t(Style::fg(Color::Palette(2)), Style::PLAIN), "\x1b[0m");
    }

    #[test]
    fn underline_styles_use_colon_subparams() {
        let to = Style { underline: Underline::Curly, underline_color: Color::Palette(1), ..Style::PLAIN };
        assert_eq!(t(Style::PLAIN, to), "\x1b[4:3;58;5;1m");
    }
}
