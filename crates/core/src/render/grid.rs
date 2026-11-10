//! Must not know about terminals, panes or escape codes.

use std::sync::atomic::{AtomicU64, Ordering};

use super::cell::{Cell, Width};
use crate::layout::rect::Rect;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Every write bumps its row's generation, so a frame row built from unchanged rows can skip
/// the cell-by-cell comparison. The id tells grids apart, since generations restart per grid.
#[derive(Debug)]
pub struct Grid {
    cols: u16,
    rows: u16,
    cells: Vec<Cell>,
    id: u64,
    tick: u64,
    gens: Vec<u64>,
}

impl Clone for Grid {
    fn clone(&self) -> Grid {
        Grid {
            cells: self.cells.clone(),
            gens: self.gens.clone(),
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            ..*self
        }
    }
}

impl PartialEq for Grid {
    fn eq(&self, o: &Grid) -> bool {
        (self.cols, self.rows) == (o.cols, o.rows) && self.cells == o.cells
    }
}

impl Grid {
    pub fn new(cols: u16, rows: u16) -> Grid {
        Grid {
            cols,
            rows,
            cells: vec![Cell::BLANK; cols as usize * rows as usize],
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            tick: 0,
            gens: vec![0; rows as usize],
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn generation(&self, y: u16) -> u64 {
        self.gens[y as usize]
    }

    fn touch(&mut self, y: u16) {
        self.tick += 1;
        self.gens[y as usize] = self.tick;
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Contents are cleared; callers redraw after a resize anyway.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        if (cols, rows) != (self.cols, self.rows) {
            *self = Grid::new(cols, rows);
        }
    }

    pub fn row(&self, y: u16) -> &[Cell] {
        let start = y as usize * self.cols as usize;
        &self.cells[start..start + self.cols as usize]
    }

    pub fn row_mut(&mut self, y: u16) -> &mut [Cell] {
        self.touch(y);
        let start = y as usize * self.cols as usize;
        &mut self.cells[start..start + self.cols as usize]
    }

    pub fn get(&self, x: u16, y: u16) -> &Cell {
        &self.cells[y as usize * self.cols as usize + x as usize]
    }

    pub fn set(&mut self, x: u16, y: u16, cell: Cell) {
        self.touch(y);
        self.cells[y as usize * self.cols as usize + x as usize] = cell;
    }

    pub fn fill(&mut self, r: Rect, cell: Cell) {
        for y in r.y..r.bottom().min(self.rows) {
            let from = (r.x as usize).min(self.cols as usize);
            let to = (r.right() as usize).min(self.cols as usize);
            self.row_mut(y)[from..to].fill(cell);
        }
    }

    pub fn clear(&mut self) {
        self.cells.fill(Cell::BLANK);
        (0..self.rows).for_each(|y| self.touch(y));
    }

    pub fn blit(&mut self, src: &Grid, dst: Rect) {
        let w = dst.w.min(src.cols).min(self.cols.saturating_sub(dst.x)) as usize;
        let h = dst.h.min(src.rows).min(self.rows.saturating_sub(dst.y));
        for y in 0..h {
            let to = &mut self.row_mut(dst.y + y)[dst.x as usize..][..w];
            to.copy_from_slice(&src.row(y)[..w]);
            // A wide cell cut by the right edge would bleed past the pane.
            if let Some(last) = to.last_mut()
                && last.width == Width::Wide
            {
                *last = Cell::BLANK;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::cell::Style;

    #[test]
    fn blit_clips_and_blanks_a_cut_wide_cell() {
        let mut dst = Grid::new(4, 2);
        let mut src = Grid::new(3, 1);
        src.set(0, 0, Cell::new("a", Width::Narrow, Style::PLAIN));
        src.set(1, 0, Cell::new("世", Width::Wide, Style::PLAIN));
        src.set(2, 0, Cell::SPACER);
        dst.blit(&src, Rect { x: 0, y: 1, w: 2, h: 1 });
        assert_eq!(dst.get(0, 1).text(), "a");
        assert!(dst.get(1, 1).is_blank());
    }

    #[test]
    fn writes_bump_only_their_row_and_clones_are_told_apart() {
        let mut g = Grid::new(3, 2);
        let (a, b) = (g.generation(0), g.generation(1));
        g.set(1, 1, Cell::SPACER);
        assert_eq!(g.generation(0), a);
        assert_ne!(g.generation(1), b);
        g.row_mut(0);
        assert_ne!(g.generation(0), a);
        let c = g.clone();
        assert_ne!(c.id(), g.id());
        assert_eq!(c, g);
    }
}
