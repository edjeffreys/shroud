//! Cells stay `Copy` with inline text so diff can compare rows cheaply.

use super::color::Color;
use super::link::{LinkId, NO_LINK};

#[derive(Clone, Copy, Eq, Debug, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub underline_color: Color,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub underline: Underline,
}

impl Style {
    pub const PLAIN: Style = Style {
        fg: Color::Default,
        bg: Color::Default,
        underline_color: Color::Default,
        bold: false,
        italic: false,
        faint: false,
        blink: false,
        inverse: false,
        invisible: false,
        strikethrough: false,
        overline: false,
        underline: Underline::None,
    };

    pub fn fg(color: Color) -> Style {
        Style { fg: color, ..Style::PLAIN }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Width {
    #[default]
    Narrow,
    Wide,
    /// Second half of a wide grapheme; never written, the terminal fills it.
    Spacer,
}

/// Inline UTF-8 so cells are `Copy` and rows compare with one memcmp-like pass.
pub const MAX_TEXT: usize = 30;

#[derive(Clone, Copy, Eq, Debug, Default)]
pub struct Cell {
    pub style: Style,
    pub width: Width,
    pub link: LinkId,
    len: u8,
    text: [u8; MAX_TEXT],
}

// `&` rather than `&&`: diff compares every cell of every frame, and a run of
// short-circuit branches cost more than the comparisons they skip.
impl PartialEq for Style {
    #[inline]
    fn eq(&self, o: &Self) -> bool {
        (self.fg == o.fg)
            & (self.bg == o.bg)
            & (self.underline_color == o.underline_color)
            & (self.bold == o.bold)
            & (self.italic == o.italic)
            & (self.faint == o.faint)
            & (self.blink == o.blink)
            & (self.inverse == o.inverse)
            & (self.invisible == o.invisible)
            & (self.strikethrough == o.strikethrough)
            & (self.overline == o.overline)
            & (self.underline == o.underline)
    }
}

impl PartialEq for Cell {
    #[inline]
    fn eq(&self, o: &Self) -> bool {
        (self.style == o.style)
            & (self.width == o.width)
            & (self.link == o.link)
            & (self.len == o.len)
            & (self.text == o.text)
    }
}

impl Cell {
    pub const BLANK: Cell =
        Cell { style: Style::PLAIN, width: Width::Narrow, link: NO_LINK, len: 0, text: [0; MAX_TEXT] };

    pub const SPACER: Cell = Cell { width: Width::Spacer, ..Cell::BLANK };

    pub fn new(text: &str, width: Width, style: Style) -> Cell {
        let mut c = Cell { style, width, ..Cell::BLANK };
        c.set_text(text);
        c
    }

    /// Clusters longer than MAX_TEXT become U+FFFD with their width kept, so layout never shifts.
    pub fn set_text(&mut self, text: &str) {
        let src = if text.len() <= MAX_TEXT { text } else { "\u{FFFD}" };
        self.text = [0; MAX_TEXT];
        self.text[..src.len()].copy_from_slice(src.as_bytes());
        self.len = src.len() as u8;
    }

    /// Empty for a blank cell.
    pub fn text(&self) -> &str {
        // Only ever filled from &str, cut on whole-string boundaries.
        std::str::from_utf8(&self.text[..self.len as usize]).unwrap_or("\u{FFFD}")
    }

    pub fn is_blank(&self) -> bool {
        *self == Cell::BLANK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn over_long_grapheme_is_replaced_width_kept() {
        let c = Cell::new(&"a".repeat(40), Width::Wide, Style::PLAIN);
        assert_eq!(c.text(), "\u{FFFD}");
        assert_eq!(c.width, Width::Wide);
    }
}
