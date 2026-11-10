#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub fn new(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    pub fn right(&self) -> u16 {
        self.x + self.w
    }

    pub fn bottom(&self) -> u16 {
        self.y + self.h
    }

    /// Never smaller than one cell, so a tiny window still lays out.
    pub fn inset(&self, by: u16) -> Rect {
        Rect {
            x: self.x + by.min(self.w / 2),
            y: self.y + by.min(self.h / 2),
            w: self.w.saturating_sub(2 * by).max(1),
            h: self.h.saturating_sub(2 * by).max(1),
        }
    }
}
