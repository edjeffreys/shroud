//! For shroud's own chrome only; pane text arrives already laid out by libghostty-vt.

use unicode_width::UnicodeWidthChar;

use super::cell::{Cell, Style, Width};
use super::grid::Grid;

pub struct Writer<'g> {
    grid: &'g mut Grid,
    y: u16,
    pub x: u16,
    end: u16,
}

impl<'g> Writer<'g> {
    /// Writes stop at `end`, so callers can't spill into a neighbour.
    pub fn new(grid: &'g mut Grid, y: u16, x: u16, end: u16) -> Writer<'g> {
        Writer { grid, y, x, end }
    }

    pub fn text(&mut self, s: &str, style: Style) {
        for ch in s.chars() {
            self.char(ch, style);
        }
    }

    /// A path's tail is the part that says where you are.
    pub fn text_tail(&mut self, s: &str, width: u16, style: Style) {
        let full = str_width(s);
        if full <= width {
            return self.text(s, style);
        }
        self.char('…', style);
        let mut skip = full - width + 1;
        for ch in s.chars() {
            let cw = char_width(ch);
            if skip > 0 {
                skip = skip.saturating_sub(cw.max(1));
                continue;
            }
            self.char(ch, style);
        }
    }

    pub fn number(&mut self, n: u16, style: Style) {
        let mut digits = [0u8; 5];
        let mut i = digits.len();
        let mut n = n;
        loop {
            i -= 1;
            digits[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        for &d in &digits[i..] {
            self.char(d as char, style);
        }
    }

    pub fn char(&mut self, ch: char, style: Style) {
        let w = char_width(ch);
        if w == 0 || self.x.saturating_add(w) > self.end {
            return;
        }
        let mut buf = [0u8; 4];
        let width = if w == 2 { Width::Wide } else { Width::Narrow };
        self.grid.set(self.x, self.y, Cell::new(ch.encode_utf8(&mut buf), width, style));
        if w == 2 {
            let mut spacer = Cell::SPACER;
            spacer.style = style;
            self.grid.set(self.x + 1, self.y, spacer);
        }
        self.x += w;
    }
}

pub fn char_width(ch: char) -> u16 {
    ch.width().unwrap_or(0) as u16
}

pub fn str_width(s: &str) -> u16 {
    s.chars().map(char_width).fold(0u16, u16::saturating_add)
}
