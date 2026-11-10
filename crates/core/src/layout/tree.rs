//! Splits leave a one-cell gap between children, where borders are drawn.

use super::rect::Rect;

pub type PaneId = u32;

pub const MAX_PANES: usize = 64;

/// Named for how children are laid out (tmux's `-h`), not the divider's direction.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize, serde::Serialize)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub fn axis(self) -> Axis {
        match self {
            Direction::Left | Direction::Right => Axis::Horizontal,
            Direction::Up | Direction::Down => Axis::Vertical,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Placement {
    pub pane: PaneId,
    pub rect: Rect,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TreeError {
    NotFound,
    TooManyPanes,
}

pub(super) type Index = usize;

#[derive(Clone, Debug)]
pub(super) struct Node {
    pub(super) parent: Option<Index>,
    pub(super) kind: Kind,
}

#[derive(Clone, Debug)]
pub(super) enum Kind {
    Leaf(PaneId),
    Split(Split),
    Free,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Split {
    pub(super) axis: Axis,
    pub(super) ratio: f32,
    pub(super) a: Index,
    pub(super) b: Index,
}

#[derive(Clone, Debug)]
pub struct Tree {
    pub(super) nodes: Vec<Node>,
    pub(super) root: Index,
}

impl Tree {
    pub fn new(first: PaneId) -> Tree {
        Tree { nodes: vec![Node { parent: None, kind: Kind::Leaf(first) }], root: 0 }
    }

    pub fn len(&self) -> usize {
        self.nodes.iter().filter(|n| matches!(n.kind, Kind::Leaf(_))).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, pane: PaneId) -> bool {
        self.find(pane).is_some()
    }

    pub fn split(&mut self, target: PaneId, axis: Axis, new: PaneId) -> Result<(), TreeError> {
        if self.len() >= MAX_PANES {
            return Err(TreeError::TooManyPanes);
        }
        let leaf = self.find(target).ok_or(TreeError::NotFound)?;
        let a = self.alloc(Node { parent: Some(leaf), kind: Kind::Leaf(target) });
        let b = self.alloc(Node { parent: Some(leaf), kind: Kind::Leaf(new) });
        // Reusing the leaf's slot for the split keeps the parent's child index valid.
        self.nodes[leaf].kind = Kind::Split(Split { axis, ratio: 0.5, a, b });
        Ok(())
    }

    pub fn remove(&mut self, target: PaneId) -> Result<Option<PaneId>, TreeError> {
        let leaf = self.find(target).ok_or(TreeError::NotFound)?;
        let Some(parent) = self.nodes[leaf].parent else {
            self.nodes[leaf].kind = Kind::Free;
            return Ok(None);
        };
        let Kind::Split(s) = self.nodes[parent].kind else { unreachable!("parent is a split") };
        let sibling = if s.a == leaf { s.b } else { s.a };
        // Moving the sibling into the parent's slot spares the grandparent an update.
        self.nodes[parent].kind = std::mem::replace(&mut self.nodes[sibling].kind, Kind::Free);
        if let Kind::Split(ps) = self.nodes[parent].kind {
            self.nodes[ps.a].parent = Some(parent);
            self.nodes[ps.b].parent = Some(parent);
        }
        self.nodes[leaf].kind = Kind::Free;
        Ok(Some(self.first_leaf(parent)))
    }

    pub fn layout(&self, area: Rect) -> Vec<Placement> {
        let mut out = Vec::with_capacity(self.len());
        self.place(self.root, area, &mut out);
        out
    }

    fn place(&self, i: Index, area: Rect, out: &mut Vec<Placement>) {
        match self.nodes[i].kind {
            Kind::Free => {}
            Kind::Leaf(pane) => out.push(Placement { pane, rect: area }),
            Kind::Split(s) => {
                let [a, b] = divide(area, s);
                self.place(s.a, a, out);
                self.place(s.b, b, out);
            }
        }
    }

    pub(super) fn rect_of(&self, area: Rect, target: Index) -> Rect {
        let mut path = vec![target];
        while let Some(p) = self.nodes[*path.last().unwrap()].parent {
            path.push(p);
        }
        let mut r = area;
        for pair in path.windows(2).rev() {
            let (child, parent) = (pair[0], pair[1]);
            let Kind::Split(s) = self.nodes[parent].kind else { unreachable!() };
            let [a, b] = divide(r, s);
            r = if s.a == child { a } else { b };
        }
        r
    }

    pub(super) fn find(&self, pane: PaneId) -> Option<Index> {
        self.nodes.iter().position(|n| matches!(n.kind, Kind::Leaf(p) if p == pane))
    }

    pub(super) fn first_leaf(&self, mut i: Index) -> PaneId {
        loop {
            match self.nodes[i].kind {
                Kind::Leaf(p) => return p,
                Kind::Split(s) => i = s.a,
                Kind::Free => unreachable!("free node reachable from root"),
            }
        }
    }

    pub(super) fn alloc(&mut self, node: Node) -> Index {
        if let Some(i) = self.nodes.iter().position(|n| matches!(n.kind, Kind::Free)) {
            self.nodes[i] = node;
            i
        } else {
            self.nodes.push(node);
            self.nodes.len() - 1
        }
    }
}

pub(super) fn divide(area: Rect, s: Split) -> [Rect; 2] {
    let span = if s.axis == Axis::Horizontal { area.w } else { area.h };
    let usable = span.saturating_sub(1);
    let mut first = (usable as f32 * s.ratio).round() as u16;
    if usable >= 2 {
        first = first.clamp(1, usable - 1);
    }
    let second = usable.saturating_sub(first);
    match s.axis {
        Axis::Horizontal => {
            [Rect::new(area.x, area.y, first, area.h), Rect::new(area.x + first + 1, area.y, second, area.h)]
        }
        Axis::Vertical => {
            [Rect::new(area.x, area.y, area.w, first), Rect::new(area.x, area.y + first + 1, area.w, second)]
        }
    }
}

pub fn neighbour(placements: &[Placement], from: PaneId, dir: Direction) -> Option<PaneId> {
    let src = placements.iter().find(|p| p.pane == from)?.rect;
    let overlap = |a0: u16, a1: u16, b0: u16, b1: u16| a1.min(b1) as i32 - a0.max(b0) as i32;
    placements
        .iter()
        .filter(|p| p.pane != from)
        .filter_map(|p| {
            let r = p.rect;
            let (dist, ov) = match dir {
                Direction::Left => (src.x as i32 - r.right() as i32, overlap(src.y, src.bottom(), r.y, r.bottom())),
                Direction::Right => (r.x as i32 - src.right() as i32, overlap(src.y, src.bottom(), r.y, r.bottom())),
                Direction::Up => (src.y as i32 - r.bottom() as i32, overlap(src.x, src.right(), r.x, r.right())),
                Direction::Down => (r.y as i32 - src.bottom() as i32, overlap(src.x, src.right(), r.x, r.right())),
            };
            (dist >= 0 && ov > 0).then_some((dist, -ov, p.pane))
        })
        .min()
        .map(|(_, _, pane)| pane)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { x: 0, y: 0, w: 81, h: 24 };

    #[test]
    fn split_leaves_a_one_cell_divider() {
        let mut t = Tree::new(1);
        t.split(1, Axis::Horizontal, 2).unwrap();
        let ps = t.layout(SCREEN);
        assert_eq!(ps[0].rect, Rect::new(0, 0, 40, 24));
        assert_eq!(ps[1].rect, Rect::new(41, 0, 40, 24));
    }

    #[test]
    fn remove_promotes_the_sibling_subtree() {
        let mut t = Tree::new(1);
        t.split(1, Axis::Horizontal, 2).unwrap();
        t.split(2, Axis::Vertical, 3).unwrap();
        assert_eq!(t.remove(1), Ok(Some(2)));
        let ps = t.layout(SCREEN);
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[0].rect.w, 81);
        assert_eq!(t.remove(2), Ok(Some(3)));
        assert_eq!(t.remove(3), Ok(None));
        assert!(t.is_empty());
    }

    #[test]
    fn neighbour_picks_the_overlapping_pane() {
        let mut t = Tree::new(1);
        t.split(1, Axis::Horizontal, 2).unwrap();
        t.split(2, Axis::Vertical, 3).unwrap();
        let ps = t.layout(SCREEN);
        assert_eq!(neighbour(&ps, 1, Direction::Right), Some(2));
        assert_eq!(neighbour(&ps, 2, Direction::Down), Some(3));
        assert_eq!(neighbour(&ps, 3, Direction::Left), Some(1));
        assert_eq!(neighbour(&ps, 1, Direction::Left), None);
    }
}
