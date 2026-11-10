//! Frame: everything one client should see — a grid, the cursor, the title.
//! The unit that `diff` compares.

use super::grid::Grid;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
    pub shape: CursorShape,
    pub blink: bool,
}

impl Default for Cursor {
    fn default() -> Cursor {
        Cursor { x: 0, y: 0, visible: true, shape: CursorShape::Default, blink: true }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CursorShape {
    #[default]
    Default,
    Block,
    Underline,
    Bar,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub grid: Grid,
    pub cursor: Cursor,
    pub title: String,
    /// What `compose` built each row from; equal keys mean equal rows.
    keys: Vec<Option<u64>>,
    /// Each row's generation when composed: anything drawn on it since voids its key.
    composed: Vec<u64>,
}

impl Frame {
    pub fn new(cols: u16, rows: u16) -> Frame {
        Frame {
            grid: Grid::new(cols, rows),
            cursor: Cursor::default(),
            title: String::new(),
            keys: vec![None; rows as usize],
            composed: vec![0; rows as usize],
        }
    }

    pub(super) fn set_key(&mut self, y: u16, key: Option<u64>) {
        let y_ = y as usize;
        if y_ < self.keys.len() {
            self.keys[y_] = key;
            self.composed[y_] = self.grid.generation(y);
        }
    }

    pub(super) fn key(&self, y: u16) -> Option<u64> {
        let y_ = y as usize;
        self.keys.get(y_).copied().flatten().filter(|_| self.composed[y_] == self.grid.generation(y))
    }
}
