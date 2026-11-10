//! Percentages of the tab, so a float keeps its shape across resizes. Never in the tree.

use super::rect::Rect;

// Room for a border and at least one cell of content.
const MIN_OUTER: u16 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FloatRect {
    pub x: u8,
    pub y: u8,
    pub w: u8,
    pub h: u8,
}

impl FloatRect {
    pub const CENTRED: FloatRect = FloatRect { x: 10, y: 10, w: 80, h: 80 };

    pub fn new(x: u8, y: u8, w: u8, h: u8) -> Option<FloatRect> {
        (w > 0 && h > 0 && u16::from(x) + u16::from(w) <= 100 && u16::from(y) + u16::from(h) <= 100)
            .then_some(FloatRect { x, y, w, h })
    }

    pub fn outer(&self, area: Rect) -> Rect {
        let pct = |len: u16, p: u8| (u32::from(len) * u32::from(p) / 100) as u16;
        let w = pct(area.w, self.w).max(MIN_OUTER).min(area.w);
        let h = pct(area.h, self.h).max(MIN_OUTER).min(area.h);
        let x = (area.x + pct(area.w, self.x)).min(area.right().saturating_sub(w));
        let y = (area.y + pct(area.h, self.y)).min(area.bottom().saturating_sub(h));
        Rect { x, y, w, h }
    }

    pub fn inner(&self, area: Rect) -> Rect {
        let o = self.outer(area);
        Rect { x: o.x + 1, y: o.y + 1, w: o.w.saturating_sub(2).max(1), h: o.h.saturating_sub(2).max(1) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centred_float_scales_with_the_tab() {
        let area = Rect::new(0, 0, 100, 50);
        assert_eq!(FloatRect::CENTRED.outer(area), Rect::new(10, 5, 80, 40));
        assert_eq!(FloatRect::CENTRED.inner(area), Rect::new(11, 6, 78, 38));
    }

    #[test]
    fn tiny_tabs_keep_a_usable_float_inside_them() {
        let area = Rect::new(0, 0, 4, 3);
        let o = FloatRect { x: 90, y: 90, w: 5, h: 5 }.outer(area);
        assert_eq!(o, Rect::new(1, 0, 3, 3));
        assert_eq!(FloatRect::CENTRED.inner(Rect::new(0, 0, 2, 2)).w, 1);
    }

    #[test]
    fn out_of_range_specs_are_refused() {
        assert!(FloatRect::new(50, 0, 60, 10).is_none());
        assert!(FloatRect::new(0, 0, 0, 10).is_none());
        assert_eq!(FloatRect::new(0, 0, 100, 100), Some(FloatRect { x: 0, y: 0, w: 100, h: 100 }));
    }
}
