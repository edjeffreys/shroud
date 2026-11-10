//! Selections show by inverting cells, so pane colours are never changed.

use super::cell::{Cell, Underline};
use super::grid::Grid;
use crate::layout::rect::Rect;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Chars,
    Lines,
    Block,
}

/// `i64` rows, since a selection can run off the top or bottom of the pane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Highlight {
    pub from: (u16, i64),
    pub to: (u16, i64),
    pub shape: Shape,
}

pub fn apply(grid: &mut Grid, rect: Rect, h: &Highlight) {
    for_cells(grid, rect, h, |cell| cell.style.inverse = !cell.style.inverse);
}

/// Search matches are underlined, not inverted, so they stay apart from the selection.
pub fn underline(grid: &mut Grid, rect: Rect, h: &Highlight) {
    for_cells(grid, rect, h, |cell| cell.style.underline = Underline::Single);
}

fn for_cells(grid: &mut Grid, rect: Rect, h: &Highlight, mut f: impl FnMut(&mut Cell)) {
    if rect.w == 0 {
        return;
    }
    let last = rect.w - 1;
    for ly in 0..rect.h {
        let y = ly as i64;
        if y < h.from.1 || y > h.to.1 || rect.y + ly >= grid.rows() {
            continue;
        }
        let (x0, x1) = match h.shape {
            Shape::Lines => (0, last),
            Shape::Block => (h.from.0.min(h.to.0), h.from.0.max(h.to.0)),
            Shape::Chars => (if y == h.from.1 { h.from.0 } else { 0 }, if y == h.to.1 { h.to.0 } else { last }),
        };
        let row = grid.row_mut(rect.y + ly);
        for x in x0..=x1.min(last) {
            if let Some(cell) = row.get_mut((rect.x + x) as usize) {
                f(cell);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inverted(g: &Grid, y: u16) -> String {
        (0..g.cols()).map(|x| if g.get(x, y).style.inverse { '#' } else { '.' }).collect()
    }

    #[test]
    fn shapes_cover_the_right_cells_and_clip_to_the_pane() {
        let rect = Rect::new(1, 0, 4, 3);
        let mut g = Grid::new(6, 3);
        apply(&mut g, rect, &Highlight { from: (2, 0), to: (1, 1), shape: Shape::Chars });
        assert_eq!([inverted(&g, 0), inverted(&g, 1), inverted(&g, 2)], ["...##.", ".##...", "......"]);
        let mut g = Grid::new(6, 3);
        apply(&mut g, rect, &Highlight { from: (3, -5), to: (1, 1), shape: Shape::Block });
        assert_eq!([inverted(&g, 0), inverted(&g, 1), inverted(&g, 2)], ["..###.", "..###.", "......"]);
        let mut g = Grid::new(6, 3);
        apply(&mut g, rect, &Highlight { from: (3, 2), to: (0, 9), shape: Shape::Lines });
        assert_eq!(inverted(&g, 2), ".####.");
    }
}
