//! Any cell no pane covers is a border; its neighbours pick the junction glyph.

use super::cell::{Cell, Style, Width};
use super::grid::Grid;
use crate::layout::rect::Rect;
use crate::layout::tree::MAX_PANES;
use crate::theme::Theme;

pub fn draw(grid: &mut Grid, area: Rect, panes: &[Rect], focused: Option<usize>, theme: &Theme) {
    let is_border = |x: i32, y: i32| -> bool {
        if x < 0 || y < 0 || x > u16::MAX as i32 || y > u16::MAX as i32 {
            return false;
        }
        let (x, y) = (x as u16, y as u16);
        area.contains(x, y) && !panes.iter().any(|p| p.contains(x, y))
    };
    // Only the gaps between panes are visited: testing every cell made this the costliest
    // part of a one-key frame.
    let mut spans = [(0u16, 0u16); MAX_PANES];
    for y in area.y..area.bottom() {
        let mut n = 0;
        for p in panes.iter().filter(|p| y >= p.y && y < p.bottom()).take(MAX_PANES) {
            spans[n] = (p.x.max(area.x), p.right().min(area.right()));
            n += 1;
        }
        spans[..n].sort_unstable();
        let mut x = area.x;
        for &(from, to) in spans[..n].iter().chain([(area.right(), area.right())].iter()) {
            for gap in x..from.max(x) {
                let (ix, iy) = (gap as i32, y as i32);
                let mask = usize::from(is_border(ix, iy - 1))
                    | usize::from(is_border(ix, iy + 1)) << 1
                    | usize::from(is_border(ix - 1, iy)) << 2
                    | usize::from(is_border(ix + 1, iy)) << 3;
                let active = focused.is_some_and(|f| touches(panes[f], ix, iy));
                grid.set(gap, y, Cell::new(GLYPHS[mask], Width::Narrow, theme.border(active)));
            }
            x = x.max(to);
        }
    }
}

// Index bits: up, down, left, right.
const GLYPHS: [&str; 16] = ["│", "│", "│", "│", "─", "┘", "┐", "┤", "─", "└", "┌", "├", "─", "┴", "┬", "┼"];

/// A float's own frame, drawn over whatever is beneath it.
pub fn frame_box(grid: &mut Grid, outer: Rect, style: Style) {
    if outer.w < 2 || outer.h < 2 {
        return;
    }
    let (r, b) = (outer.right() - 1, outer.bottom() - 1);
    for x in outer.x..=r {
        let (top, bottom) = match x {
            _ if x == outer.x => ("┌", "└"),
            _ if x == r => ("┐", "┘"),
            _ => ("─", "─"),
        };
        put(grid, x, outer.y, top, style);
        put(grid, x, b, bottom, style);
    }
    for y in outer.y + 1..b {
        put(grid, outer.x, y, "│", style);
        put(grid, r, y, "│", style);
    }
}

fn put(grid: &mut Grid, x: u16, y: u16, glyph: &str, style: Style) {
    if x < grid.cols() && y < grid.rows() {
        grid.set(x, y, Cell::new(glyph, Width::Narrow, style));
    }
}

// Diagonals count, so corner junctions light up too.
fn touches(r: Rect, x: i32, y: i32) -> bool {
    x >= r.x as i32 - 1 && x <= r.right() as i32 && y >= r.y as i32 - 1 && y <= r.bottom() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_float_frame_is_a_closed_box() {
        let mut g = Grid::new(5, 4);
        frame_box(&mut g, Rect::new(1, 0, 4, 3), Theme::default().border(true));
        let row = |y| (0..5).map(|x| if g.get(x, y).is_blank() { " " } else { g.get(x, y).text() }).collect::<String>();
        assert_eq!(row(0), " ┌──┐");
        assert_eq!(row(1), " │  │");
        assert_eq!(row(2), " └──┘");
        assert_eq!(g.get(1, 0).style.fg, Theme::default().border_active);
    }

    #[test]
    fn vertical_divider_with_a_t_junction() {
        let mut g = Grid::new(5, 3);
        let panes = [Rect { x: 0, y: 0, w: 2, h: 3 }, Rect { x: 3, y: 0, w: 2, h: 1 }, Rect { x: 3, y: 2, w: 2, h: 1 }];
        let theme = Theme::default();
        draw(&mut g, Rect { x: 0, y: 0, w: 5, h: 3 }, &panes, Some(0), &theme);
        assert_eq!(g.get(2, 0).text(), "│");
        assert_eq!(g.get(2, 1).text(), "├");
        assert_eq!(g.get(4, 1).text(), "─");
        assert_eq!(g.get(2, 0).style.fg, theme.border_active);
        assert_eq!(g.get(4, 1).style.fg, theme.border_inactive);
    }
}
