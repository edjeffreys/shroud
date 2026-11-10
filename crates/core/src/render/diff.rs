//! Frames are wrapped in mode 2026 so Ghostty never shows half of one, and end with a
//! plain pen so no frame depends on the previous one's SGR state.

use super::cell::{Cell, Style, Width};
use super::frame::{Cursor, CursorShape, Frame};
use super::link::{LinkId, Links, NO_LINK};
use super::num::push_u16;
use super::sgr;

const SYNC_BEGIN: &[u8] = b"\x1b[?2026h";
const SYNC_END: &[u8] = b"\x1b[?2026l";

pub fn diff(out: &mut Vec<u8>, prev: Option<&Frame>, next: &Frame, links: &Links) {
    let prev = prev.filter(|p| p.grid.cols() == next.grid.cols() && p.grid.rows() == next.grid.rows());
    let start = out.len();
    out.extend_from_slice(SYNC_BEGIN);
    let body = out.len();

    if prev.is_none() {
        out.extend_from_slice(b"\x1b[0m\x1b[H\x1b[2J");
    }
    let mut w =
        Writer { out, links, cols: next.grid.cols(), pos: None, pen: Style::PLAIN, link: NO_LINK, moved: false };
    for y in 0..next.grid.rows() {
        let row = next.grid.row(y);
        if let Some(p) = prev
            && next.key(y).is_some_and(|k| p.key(y) == Some(k))
        {
            continue;
        }
        let old = prev.map(|p| p.grid.row(y));
        if old == Some(row) {
            continue;
        }
        for (x, cell) in row.iter().enumerate() {
            if cell.width == Width::Spacer {
                continue;
            }
            let unchanged = match old {
                Some(o) => *cell == o[x] && (cell.width != Width::Wide || row.get(x + 1) == o.get(x + 1)),
                None => cell.is_blank(),
            };
            if !unchanged {
                w.put(x as u16, y, cell);
            }
        }
    }
    w.reset_pen();
    let moved = w.moved;

    cursor(out, prev.map(|p| p.cursor), next.cursor, moved);
    if prev.is_none_or(|p| p.title != next.title) {
        title(out, &next.title);
    }

    if out.len() == body {
        out.truncate(start);
    } else {
        out.extend_from_slice(SYNC_END);
    }
}

struct Writer<'a> {
    out: &'a mut Vec<u8>,
    links: &'a Links,
    cols: u16,
    // None after writing the last column: the terminal is in pending-wrap, so always move.
    pos: Option<(u16, u16)>,
    pen: Style,
    link: LinkId,
    moved: bool,
}

impl Writer<'_> {
    fn put(&mut self, x: u16, y: u16, cell: &Cell) {
        if self.pos != Some((x, y)) {
            if self.pos.is_some_and(|(_, py)| py == y) {
                csi(self.out, &[x + 1], b'G');
            } else {
                csi(self.out, &[y + 1, x + 1], b'H');
            }
            self.moved = true;
        }
        sgr::transition(self.out, &self.pen, &cell.style);
        self.pen = cell.style;
        self.set_link(cell.link);
        let text = cell.text();
        self.out.extend_from_slice(if text.is_empty() { b" " } else { text.as_bytes() });
        let next = x + if cell.width == Width::Wide { 2 } else { 1 };
        // A terminal that sizes a cluster differently would shift the rest of the row.
        let cluster = text.chars().nth(1).is_some();
        self.pos = (next < self.cols && !cluster).then_some((next, y));
    }

    fn reset_pen(&mut self) {
        if self.pen != Style::PLAIN {
            self.out.extend_from_slice(b"\x1b[0m");
            self.pen = Style::PLAIN;
        }
        self.set_link(NO_LINK);
    }

    // An id whose URI was collected is written unlinked rather than dropped.
    fn set_link(&mut self, id: LinkId) {
        let uri = self.links.uri(id).unwrap_or("");
        let id = if uri.is_empty() { NO_LINK } else { id };
        if id == self.link {
            return;
        }
        self.out.extend_from_slice(b"\x1b]8;;");
        self.out.extend_from_slice(uri.as_bytes());
        self.out.extend_from_slice(b"\x1b\\");
        self.link = id;
    }
}

fn cursor(out: &mut Vec<u8>, prev: Option<Cursor>, next: Cursor, moved: bool) {
    if prev.is_none_or(|p| p.shape != next.shape || p.blink != next.blink) {
        let steady = u16::from(!next.blink);
        let code = match next.shape {
            CursorShape::Default => 0,
            CursorShape::Block => 1 + steady,
            CursorShape::Underline => 3 + steady,
            CursorShape::Bar => 5 + steady,
        };
        out.extend_from_slice(b"\x1b[");
        push_u16(out, code);
        out.extend_from_slice(b" q");
    }
    if next.visible && (moved || prev.is_none_or(|p| (p.x, p.y) != (next.x, next.y))) {
        csi(out, &[next.y + 1, next.x + 1], b'H');
    }
    if prev.is_none_or(|p| p.visible != next.visible) {
        out.extend_from_slice(if next.visible { b"\x1b[?25h" } else { b"\x1b[?25l" });
    }
}

fn title(out: &mut Vec<u8>, t: &str) {
    out.extend_from_slice(b"\x1b]2;");
    // Control bytes would end or corrupt the OSC.
    out.extend(t.bytes().map(|b| if b < 0x20 || b == 0x7f { b'?' } else { b }));
    out.extend_from_slice(b"\x1b\\");
}

fn csi(out: &mut Vec<u8>, params: &[u16], fin: u8) {
    out.extend_from_slice(b"\x1b[");
    for (i, p) in params.iter().enumerate() {
        if i > 0 {
            out.push(b';');
        }
        push_u16(out, *p);
    }
    out.push(fin);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::color::Color;

    fn render(prev: Option<&Frame>, next: &Frame) -> String {
        let mut out = Vec::new();
        diff(&mut out, prev, next, &Links::default());
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn identical_frames_produce_nothing() {
        let a = Frame::new(4, 2);
        assert_eq!(render(Some(&a), &a.clone()), "");
    }

    #[test]
    fn single_changed_cell_moves_styles_writes_restores() {
        let a = Frame::new(4, 2);
        let mut b = a.clone();
        b.grid.set(2, 1, Cell::new("x", Width::Narrow, Style::fg(Color::Palette(1))));
        assert_eq!(render(Some(&a), &b), "\x1b[?2026h\x1b[2;3H\x1b[31mx\x1b[0m\x1b[1;1H\x1b[?2026l");
    }

    #[test]
    fn adjacent_cells_need_no_move_and_wide_advances_two() {
        let mut a = Frame::new(5, 1);
        a.cursor.visible = false;
        let mut b = a.clone();
        b.grid.set(0, 0, Cell::new("世", Width::Wide, Style::PLAIN));
        b.grid.set(1, 0, Cell::SPACER);
        b.grid.set(2, 0, Cell::new("a", Width::Narrow, Style::PLAIN));
        assert_eq!(render(Some(&a), &b), "\x1b[?2026h\x1b[1;1H世a\x1b[?2026l");
    }

    #[test]
    fn full_redraw_clears_and_skips_blanks() {
        let mut b = Frame::new(3, 1);
        b.grid.set(1, 0, Cell::new("z", Width::Narrow, Style::PLAIN));
        b.title = "t".into();
        assert_eq!(
            render(None, &b),
            "\x1b[?2026h\x1b[0m\x1b[H\x1b[2J\x1b[1;2Hz\x1b[0 q\x1b[1;1H\x1b[?25h\x1b]2;t\x1b\\\x1b[?2026l"
        );
    }

    #[test]
    fn links_open_around_their_cells_and_close_before_the_frame_ends() {
        let mut links = Links::default();
        let id = links.intern("https://shroud.dev");
        let a = Frame::new(5, 1);
        let mut b = a.clone();
        for (x, ch) in ["a", "b", "c"].iter().enumerate() {
            let mut c = Cell::new(ch, Width::Narrow, Style::PLAIN);
            c.link = if x < 2 { id } else { NO_LINK };
            b.grid.set(x as u16, 0, c);
        }
        let mut out = Vec::new();
        diff(&mut out, Some(&a), &b, &links);
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\x1b[1;1H\x1b]8;;https://shroud.dev\x1b\\ab\x1b]8;;\x1b\\c"), "{s:?}");
        let mut out = Vec::new();
        diff(&mut out, Some(&b), &b.clone(), &links);
        assert!(out.is_empty());
    }

    #[test]
    fn a_cluster_is_followed_by_an_explicit_move() {
        let mut a = Frame::new(5, 1);
        a.cursor.visible = false;
        let mut b = a.clone();
        b.grid.set(0, 0, Cell::new("👍🏽", Width::Wide, Style::PLAIN));
        b.grid.set(1, 0, Cell::SPACER);
        b.grid.set(2, 0, Cell::new("a", Width::Narrow, Style::PLAIN));
        b.grid.set(3, 0, Cell::new("世", Width::Wide, Style::PLAIN));
        b.grid.set(4, 0, Cell::SPACER);
        assert_eq!(render(Some(&a), &b), "\x1b[?2026h\x1b[1;1H👍🏽\x1b[1;3Ha世\x1b[?2026l");
    }

    #[test]
    fn last_column_forces_explicit_move() {
        let a = Frame::new(2, 2);
        let mut b = a.clone();
        b.grid.set(1, 0, Cell::new("a", Width::Narrow, Style::PLAIN));
        b.grid.set(0, 1, Cell::new("b", Width::Narrow, Style::PLAIN));
        assert!(render(Some(&a), &b).contains("\x1b[1;2Ha\x1b[2;1Hb"));
    }
}
