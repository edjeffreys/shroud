//! Must not allocate: runs every frame for every client.

use std::hash::{DefaultHasher, Hash, Hasher};

use super::borders;
use super::cell::Cell;
use super::frame::{Cursor, Frame};
use super::grid::Grid;
use super::highlight::{self, Highlight};
use super::text::Writer;
use crate::layout::rect::Rect;
use crate::layout::tree::MAX_PANES;
use crate::theme::Theme;

#[derive(Clone, Copy)]
pub struct PaneView<'a> {
    /// For a float, its inner rect; `float` holds the frame around it.
    pub rect: Rect,
    pub grid: &'a Grid,
    /// Pane-local.
    pub cursor: Cursor,
    pub float: Option<Rect>,
    pub highlight: Option<Highlight>,
    /// Drawn on the border above the pane, or a float's own frame.
    pub title: Option<&'a str>,
    pub matches: &'a [Highlight],
}

pub fn compose(frame: &mut Frame, area: Rect, panes: &[PaneView], focused: Option<usize>, theme: &Theme) {
    frame.grid.clear();
    let mut rects = [Rect::default(); MAX_PANES];
    let (mut n, mut tiled_focus) = (0, None);
    for (i, p) in panes.iter().enumerate().filter(|(_, p)| p.float.is_none()).take(MAX_PANES) {
        frame.grid.blit(p.grid, p.rect);
        mark(&mut frame.grid, p);
        if focused == Some(i) {
            tiled_focus = Some(n);
        }
        rects[n] = p.rect;
        n += 1;
    }
    borders::draw(&mut frame.grid, area, &rects[..n], tiled_focus, theme);
    for (i, p) in panes.iter().enumerate().filter(|(_, p)| p.float.is_none()) {
        if let Some(t) = p.title
            && p.rect.y > area.y
        {
            title(&mut frame.grid, p.rect.y - 1, p.rect.x, p.rect.right(), t, theme.title(focused == Some(i)));
        }
    }
    for (i, p) in panes.iter().enumerate() {
        if let Some(outer) = p.float {
            frame.grid.fill(outer, Cell::BLANK);
            borders::frame_box(&mut frame.grid, outer, theme.border(focused == Some(i)));
            if let Some(t) = p.title {
                title(
                    &mut frame.grid,
                    outer.y,
                    outer.x + 1,
                    outer.right().saturating_sub(1),
                    t,
                    theme.title(focused == Some(i)),
                );
            }
            frame.grid.blit(p.grid, p.rect);
            mark(&mut frame.grid, p);
        }
    }
    frame.cursor = match focused.map(|f| (f, &panes[f])) {
        Some((f, p)) => {
            let x = p.cursor.x.saturating_add(p.rect.x);
            let y = p.cursor.y.saturating_add(p.rect.y);
            // Later views draw on top, so a cursor beneath one would show through it.
            let covered = panes[f + 1..].iter().any(|q| q.float.is_some_and(|o| o.contains(x, y)));
            Cursor { x, y, visible: p.cursor.visible && p.rect.contains(x, y) && !covered, ..p.cursor }
        }
        None => Cursor { visible: false, ..Cursor::default() },
    };
    key_rows(frame, area, panes, focused, theme);
}

fn mark(grid: &mut Grid, p: &PaneView) {
    for m in p.matches {
        highlight::underline(grid, p.rect, m);
    }
    if let Some(h) = &p.highlight {
        highlight::apply(grid, p.rect, h);
    }
}

/// Inside `from..to` on row `y`, one cell clear of each end so the border's corners show.
fn title(grid: &mut Grid, y: u16, from: u16, to: u16, text: &str, style: crate::render::cell::Style) {
    if to < from + 4 {
        return;
    }
    let mut w = Writer::new(grid, y, from + 1, to - 1);
    w.char(' ', style);
    w.text(text, style);
    w.char(' ', style);
}

/// Everything `compose` reads, per row: the layout and theme, and the generation of each pane
/// row drawn there. A highlighted row isn't keyed, so it is always compared.
fn key_rows(frame: &mut Frame, area: Rect, panes: &[PaneView], focused: Option<usize>, theme: &Theme) {
    let mut scene = DefaultHasher::new();
    (area, focused, theme).hash(&mut scene);
    for p in panes {
        (p.rect, p.float, p.grid.id(), p.title).hash(&mut scene);
    }
    let scene = scene.finish();
    for y in 0..frame.grid.rows() {
        let mut h = DefaultHasher::new();
        scene.hash(&mut h);
        let mut keyed = true;
        for p in panes {
            let outer = p.float.unwrap_or(p.rect);
            if y < outer.y || y >= outer.bottom() {
                continue;
            }
            if p.highlight.is_some() || !p.matches.is_empty() {
                keyed = false;
                break;
            }
            let row = y.wrapping_sub(p.rect.y);
            if y >= p.rect.y && row < p.rect.h && row < p.grid.rows() {
                (p.grid.id(), p.grid.generation(row)).hash(&mut h);
            }
        }
        frame.set_key(y, keyed.then(|| h.finish()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::cell::{Style, Width};
    use crate::render::color::Color;
    use crate::render::diff::diff;
    use crate::render::visible::visible;

    fn fill(g: &mut Grid, text: &str, style: Style) {
        for (x, ch) in text.chars().enumerate() {
            g.set(x as u16, 0, Cell::new(ch.encode_utf8(&mut [0; 4]), Width::Narrow, style));
        }
    }

    fn two_panes(theme: &Theme) -> Vec<u8> {
        let mut left = Grid::new(4, 3);
        let mut right = Grid::new(4, 3);
        fill(&mut left, "$ ls", Style::PLAIN);
        // Pane colours must survive any theme.
        fill(&mut right, "err", Style::fg(Color::Palette(1)));
        let panes = [
            PaneView {
                rect: Rect { x: 0, y: 0, w: 4, h: 3 },
                grid: &left,
                cursor: Cursor { x: 2, ..Cursor::default() },
                float: None,
                highlight: None,
                title: None,
                matches: &[],
            },
            PaneView {
                rect: Rect { x: 5, y: 0, w: 4, h: 3 },
                grid: &right,
                cursor: Cursor::default(),
                float: None,
                highlight: None,
                title: None,
                matches: &[],
            },
        ];
        let mut frame = Frame::new(9, 3);
        compose(&mut frame, Rect { x: 0, y: 0, w: 9, h: 3 }, &panes, Some(0), theme);
        let mut out = Vec::new();
        diff(&mut out, None, &frame, &crate::render::Links::default());
        out
    }

    fn view(g: &Grid, highlight: Option<Highlight>) -> [PaneView<'_>; 1] {
        [PaneView {
            rect: Rect { x: 0, y: 0, w: 4, h: 3 },
            grid: g,
            cursor: Cursor::default(),
            float: None,
            highlight,
            title: None,
            matches: &[],
        }]
    }

    #[test]
    fn row_keys_change_only_where_something_drawn_there_changed() {
        let area = Rect { x: 0, y: 0, w: 4, h: 3 };
        let mut pane = Grid::new(4, 3);
        let theme = Theme::default();
        let (mut a, mut b) = (Frame::new(4, 3), Frame::new(4, 3));
        compose(&mut a, area, &view(&pane, None), Some(0), &theme);
        compose(&mut b, area, &view(&pane, None), Some(0), &theme);
        assert!((0..3).all(|y| a.key(y).is_some() && a.key(y) == b.key(y)));

        pane.set(0, 1, Cell::new("x", Width::Narrow, Style::PLAIN));
        compose(&mut b, area, &view(&pane, None), Some(0), &theme);
        assert_eq!((a.key(0) == b.key(0), a.key(1) == b.key(1), a.key(2) == b.key(2)), (true, false, true));
        let mut out = Vec::new();
        diff(&mut out, Some(&a), &b, &crate::render::Links::default());
        assert!(visible(&out).contains('x'));

        b.grid.set(0, 2, Cell::BLANK);
        assert_eq!(b.key(2), None, "drawn on after compose");
        compose(
            &mut b,
            area,
            &view(&pane, None),
            Some(0),
            &Theme { border_active: Color::Default, ..Theme::default() },
        );
        assert_ne!(a.key(0), b.key(0), "the theme is part of every key");
        let h = Highlight { from: (0, 0), to: (1, 0), shape: highlight::Shape::Lines };
        compose(&mut b, area, &view(&pane, Some(h)), Some(0), &theme);
        assert_eq!(b.key(0), None);
    }

    #[test]
    fn a_float_covers_the_tiles_and_hides_their_cursor() {
        let mut tile = Grid::new(6, 4);
        fill(&mut tile, "under!", Style::PLAIN);
        let mut top = Grid::new(2, 1);
        fill(&mut top, "hi", Style::PLAIN);
        let panes = [
            PaneView {
                rect: Rect::new(0, 0, 6, 4),
                grid: &tile,
                cursor: Cursor { x: 2, ..Cursor::default() },
                float: None,
                highlight: None,
                title: None,
                matches: &[],
            },
            PaneView {
                rect: Rect::new(2, 1, 2, 1),
                grid: &top,
                cursor: Cursor::default(),
                float: Some(Rect::new(1, 0, 4, 3)),
                highlight: None,
                title: None,
                matches: &[],
            },
        ];
        let mut frame = Frame::new(6, 4);
        compose(&mut frame, Rect::new(0, 0, 6, 4), &panes, Some(0), &Theme::default());
        let row = |y| {
            (0..6)
                .map(|x| if frame.grid.get(x, y).is_blank() { " " } else { frame.grid.get(x, y).text() })
                .collect::<String>()
        };
        assert_eq!(row(0), "u┌──┐!");
        assert_eq!(row(1), " │hi│ ");
        assert!(!frame.cursor.visible, "the tiled cursor sits under the float");
        compose(&mut frame, Rect::new(0, 0, 6, 4), &panes, Some(1), &Theme::default());
        assert_eq!((frame.cursor.x, frame.cursor.y, frame.cursor.visible), (2, 1, true));
    }

    #[test]
    fn golden_two_panes_default_theme() {
        insta::assert_snapshot!(visible(&two_panes(&Theme::default())));
    }

    #[test]
    fn golden_custom_theme_changes_only_border_bytes() {
        let theme = Theme { border_active: Color::Rgb(255, 0, 128), ..Theme::default() };
        let out = two_panes(&theme);
        insta::assert_snapshot!(visible(&out));
        assert!(out.windows(8).any(|w| w == b"\x1b[31merr"));
    }

    fn row(frame: &Frame, y: u16) -> String {
        (0..frame.grid.cols()).map(|x| frame.grid.get(x, y).text().chars().next().unwrap_or(' ')).collect()
    }

    #[test]
    fn titles_sit_on_the_border_above_and_change_the_key() {
        let (a, b) = (Grid::new(8, 2), Grid::new(8, 2));
        let view = |title| {
            [
                PaneView {
                    rect: Rect::new(1, 1, 8, 2),
                    grid: &a,
                    cursor: Cursor::default(),
                    float: None,
                    highlight: None,
                    title,
                    matches: &[],
                },
                PaneView {
                    rect: Rect::new(10, 1, 8, 2),
                    grid: &b,
                    cursor: Cursor::default(),
                    float: None,
                    highlight: None,
                    title: Some("a long title"),
                    matches: &[],
                },
            ]
        };
        let area = Rect::new(0, 0, 19, 4);
        let mut framed = Frame::new(19, 4);
        compose(&mut framed, area, &view(Some("vim")), Some(0), &Theme::default());
        assert_eq!(row(&framed, 0), "┌─ vim ──┬─ a lon─┐", "clipped one cell short of each corner");
        assert_eq!(row(&framed, 3), "└────────┴────────┘");
        let mut untitled = Frame::new(19, 4);
        compose(&mut untitled, area, &view(None), Some(0), &Theme::default());
        assert_ne!(framed.key(0), untitled.key(0), "a new title must redraw its row");
    }
}
