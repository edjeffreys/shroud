//! Boxes a plugin draws over the panes, with no PTY behind them. Their text is untrusted,
//! so control characters never reach a cell, and every colour comes from a theme token.

use crate::layout::rect::Rect;
use crate::render::borders::frame_box;
use crate::render::cell::{Cell, Style};
use crate::render::grid::Grid;
use crate::render::text::{Writer, str_width};
use crate::theme::Theme;

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Span {
    pub text: String,
    /// The text colour: a theme token such as `badge.error`, or a colour value. Anything
    /// else falls back to plain text.
    pub style: Option<String>,
    pub bg: Option<String>,
    pub bold: bool,
    pub italic: bool,
}

impl Span {
    fn look(&self, theme: &Theme, under: Style) -> Style {
        let pick = |name: &Option<String>, or: _| name.as_deref().and_then(|n| theme.resolve(n)).unwrap_or(or);
        Style {
            fg: pick(&self.style, under.fg),
            bg: pick(&self.bg, under.bg),
            bold: self.bold,
            italic: self.italic,
            ..Style::PLAIN
        }
    }
}

/// A status bar a plugin drew: `left` from the left edge, `right` ending at the right one,
/// which wins where they meet.
pub fn draw_bar(grid: &mut Grid, y: u16, left: &[Span], right: &[Span], fill: Option<&str>, theme: &Theme) {
    if y >= grid.rows() {
        return;
    }
    let mut base = theme.status();
    if let Some(bg) = fill.and_then(|f| theme.resolve(f)) {
        base.bg = bg;
    }
    for cell in grid.row_mut(y) {
        *cell = Cell::new("", crate::render::cell::Width::Narrow, base);
    }
    let cols = grid.cols();
    let right_w = right.iter().map(|s| drawn_width(&s.text)).fold(0u16, u16::saturating_add).min(cols);
    let mut w = Writer::new(grid, y, 0, cols.saturating_sub(right_w));
    for s in left {
        safe_text(&mut w, &s.text, s.look(theme, base));
    }
    let mut w = Writer::new(grid, y, cols - right_w, cols);
    for s in right {
        safe_text(&mut w, &s.text, s.look(theme, base));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Anchor {
    #[default]
    Centre,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// Below the cursor, or above it when there's no room.
    Cursor,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Overlay {
    pub title: Option<String>,
    pub lines: Vec<Vec<Span>>,
    pub anchor: Anchor,
    pub modal: bool,
    /// The inner width, instead of fitting the content.
    pub width: Option<u16>,
    /// A theme token; `overlay.border` when unset.
    pub border: Option<String>,
}

impl Overlay {
    pub fn rect(&self, area: Rect, cursor: (u16, u16)) -> Rect {
        let content = self
            .lines
            .iter()
            .map(|l| l.iter().map(|s| drawn_width(&s.text)).fold(0u16, u16::saturating_add))
            .max()
            .unwrap_or(0);
        let title = self.title.as_deref().map_or(0, |t| drawn_width(t) + 2);
        let w = self.width.unwrap_or(content.max(title)).saturating_add(2).clamp(3, area.w.max(1));
        let h = (self.lines.len().min(u16::MAX as usize - 2) as u16 + 2).clamp(3, area.h.max(1));
        let (left, top) = (area.x, area.y);
        let (right, bottom) = (area.right().saturating_sub(w), area.bottom().saturating_sub(h));
        let (x, y) = match self.anchor {
            Anchor::Centre => (left + (area.w - w) / 2, top + (area.h - h) / 2),
            Anchor::TopLeft => (left, top),
            Anchor::TopRight => (right, top),
            Anchor::BottomLeft => (left, bottom),
            Anchor::BottomRight => (right, bottom),
            Anchor::Cursor => {
                let below = cursor.1.saturating_add(1);
                let y = if below.saturating_add(h) <= area.bottom() { below } else { cursor.1.saturating_sub(h) };
                (cursor.0.clamp(left, right), y.clamp(top, bottom))
            }
        };
        Rect { x, y, w, h }
    }

    pub fn draw(&self, grid: &mut Grid, area: Rect, cursor: (u16, u16), theme: &Theme) {
        let r = self.rect(area, cursor);
        grid.fill(r, Cell::BLANK);
        let border = Style::fg(self.border.as_deref().and_then(|b| theme.resolve(b)).unwrap_or(theme.overlay_border));
        frame_box(grid, r, border);
        let inner_end = r.right().saturating_sub(1);
        if let Some(t) = &self.title {
            let mut w = Writer::new(grid, r.y, r.x + 1, inner_end);
            w.text(" ", border);
            safe_text(&mut w, t, Style { bold: true, ..Style::fg(theme.overlay_title) });
            w.text(" ", border);
        }
        for (i, line) in self.lines.iter().enumerate().take(r.h.saturating_sub(2) as usize) {
            let mut w = Writer::new(grid, r.y + 1 + i as u16, r.x + 1, inner_end);
            for span in line {
                safe_text(&mut w, &span.text, span.look(theme, Style::PLAIN));
            }
        }
    }
}

// Control characters are drawn as spaces, so they're measured as one.
fn drawn_width(s: &str) -> u16 {
    s.chars()
        .map(|c| if c.is_control() { 1 } else { str_width(c.encode_utf8(&mut [0; 4])) })
        .fold(0u16, u16::saturating_add)
}

fn safe_text(w: &mut Writer, s: &str, style: Style) {
    for ch in s.chars() {
        let ch = if ch.is_control() { ' ' } else { ch };
        w.text(ch.encode_utf8(&mut [0; 4]), style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::color::Color;

    fn span(t: &str) -> Span {
        Span { text: t.into(), ..Span::default() }
    }

    fn row(g: &Grid, y: u16) -> String {
        (0..g.cols()).map(|x| if g.get(x, y).is_blank() { " " } else { g.get(x, y).text() }).collect()
    }

    #[test]
    fn sized_to_content_and_anchored() {
        let o = Overlay {
            lines: vec![vec![span("ab"), span("cd")], vec![span("x")]],
            anchor: Anchor::BottomRight,
            ..Overlay::default()
        };
        assert_eq!(o.rect(Rect::new(0, 0, 20, 10), (0, 0)), Rect::new(14, 6, 6, 4));
        let c = Overlay { anchor: Anchor::Centre, ..o.clone() };
        assert_eq!(c.rect(Rect::new(0, 0, 20, 10), (0, 0)), Rect::new(7, 3, 6, 4));
    }

    #[test]
    fn cursor_anchor_flips_above_near_the_bottom() {
        let o = Overlay { lines: vec![vec![span("hint")]], anchor: Anchor::Cursor, ..Overlay::default() };
        assert_eq!(o.rect(Rect::new(0, 0, 20, 10), (3, 2)).y, 3);
        assert_eq!(o.rect(Rect::new(0, 0, 20, 10), (3, 8)).y, 5);
    }

    #[test]
    fn draws_title_text_and_strips_control_characters() {
        let o = Overlay { title: Some("keys".into()), lines: vec![vec![span("a\x1b[2Jb")]], ..Overlay::default() };
        let mut g = Grid::new(12, 3);
        o.draw(&mut g, Rect::new(0, 0, 12, 3), (0, 0), &Theme::default());
        assert_eq!(row(&g, 0), "  ┌ keys ┐  ");
        assert_eq!(row(&g, 1), "  │a [2Jb│  ");
        assert!(!row(&g, 1).contains('\x1b'));
    }

    #[test]
    fn spans_take_theme_tokens_and_unknown_ones_are_plain() {
        let o = Overlay {
            lines: vec![vec![
                Span { text: "x".into(), style: Some("badge.error".into()), bold: true, ..Span::default() },
                Span { text: "y".into(), style: Some("nope".into()), ..Span::default() },
            ]],
            ..Overlay::default()
        };
        let mut g = Grid::new(6, 3);
        o.draw(&mut g, Rect::new(0, 0, 6, 3), (0, 0), &Theme::default());
        let x = g.get(2, 1);
        assert_eq!(x.text(), "x");
        assert_eq!(x.style.fg, Theme::default().badge_error);
        assert!(x.style.bold);
        assert_eq!(g.get(3, 1).style.fg, crate::render::color::Color::Default);
    }

    #[test]
    fn a_plugin_bar_has_colours_and_right_wins_where_they_meet() {
        let left = [
            Span {
                text: " work ".into(),
                style: Some("0".into()),
                bg: Some("#82aaff".into()),
                bold: true,
                ..Span::default()
            },
            span("tabs here"),
        ];
        let right = [Span { text: "10:14".into(), italic: true, ..Span::default() }];
        let mut g = Grid::new(16, 2);
        draw_bar(&mut g, 1, &left, &right, None, &Theme::default());
        assert_eq!(row(&g, 1), " work tabs 10:14");
        let w = g.get(1, 1);
        assert_eq!((w.style.fg, w.style.bg, w.style.bold), (Color::Palette(0), Color::Rgb(0x82, 0xaa, 0xff), true));
        assert!(g.get(11, 1).style.italic);
        assert_eq!(g.get(7, 1).style.bg, Theme::default().status_bg, "unstyled spans keep the bar's colours");
        draw_bar(&mut g, 1, &left, &right, Some("#1a1a1a"), &Theme::default());
        assert_eq!(
            (g.get(7, 1).style.bg, g.get(10, 1).style.bg),
            (Color::Rgb(0x1a, 0x1a, 0x1a), Color::Rgb(0x1a, 0x1a, 0x1a)),
            "fill shows behind plain spans and gaps"
        );
    }
}
