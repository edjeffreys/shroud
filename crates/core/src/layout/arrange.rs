//! Behaviour follows nvim's `CTRL-W` commands; `:help CTRL-W` settles edge cases.

use super::rect::Rect;
use super::tree::{Axis, Direction, Index, Kind, Node, PaneId, Split, Tree};

impl Tree {
    pub fn grow(&mut self, area: Rect, target: PaneId, axis: Axis, delta: i32) -> bool {
        let Some(leaf) = self.find(target) else { return false };
        let mut child = leaf;
        while let Some(node) = self.nodes[child].parent {
            if let Kind::Split(s) = self.nodes[node].kind
                && s.axis == axis
            {
                let toward_a = s.a == child;
                self.shift(area, node, if toward_a { delta } else { -delta });
                return true;
            }
            child = node;
        }
        false
    }

    pub fn maximize(&mut self, area: Rect, target: PaneId, axis: Axis) {
        let Some(leaf) = self.find(target) else { return };
        let mut child = leaf;
        while let Some(node) = self.nodes[child].parent {
            if let Kind::Split(s) = self.nodes[node].kind
                && s.axis == axis
            {
                self.shift(area, node, if s.a == child { i32::MAX / 2 } else { i32::MIN / 2 });
            }
            child = node;
        }
    }

    pub fn equalize(&mut self) {
        for i in 0..self.nodes.len() {
            if let Kind::Split(s) = self.nodes[i].kind {
                let (a, b) = (self.weight(s.a, s.axis) as f32, self.weight(s.b, s.axis) as f32);
                if let Kind::Split(s) = &mut self.nodes[i].kind {
                    s.ratio = a / (a + b);
                }
            }
        }
    }

    pub fn move_to_edge(&mut self, target: PaneId, dir: Direction) -> bool {
        if self.len() < 2 || self.remove(target).is_err() {
            return false;
        }
        let axis = dir.axis();
        let others = self.weight(self.root, axis) as f32;
        let old_root = self.root;
        let leaf = self.alloc(Node { parent: None, kind: Kind::Leaf(target) });
        let first = matches!(dir, Direction::Left | Direction::Up);
        let (a, b) = if first { (leaf, old_root) } else { (old_root, leaf) };
        let share = 1.0 / (others + 1.0);
        let ratio = if first { share } else { 1.0 - share };
        let root = self.alloc(Node { parent: None, kind: Kind::Split(Split { axis, ratio, a, b }) });
        self.nodes[leaf].parent = Some(root);
        self.nodes[old_root].parent = Some(root);
        self.root = root;
        true
    }

    /// Returns the pane now in `target`'s old place: in nvim, focus stays put, not with the pane.
    pub fn exchange(&mut self, target: PaneId) -> Option<PaneId> {
        let leaf = self.find(target)?;
        let parent = self.nodes[leaf].parent?;
        let Kind::Split(s) = self.nodes[parent].kind else { return None };
        let sibling = if s.a == leaf { s.b } else { s.a };
        let other_leaf = self.find(self.first_leaf(sibling))?;
        let Kind::Leaf(other) = self.nodes[other_leaf].kind else { return None };
        self.nodes[leaf].kind = Kind::Leaf(other);
        self.nodes[other_leaf].kind = Kind::Leaf(target);
        Some(other)
    }

    /// Refuses, as nvim does, when a sibling is split the other way.
    pub fn rotate(&mut self, target: PaneId) -> bool {
        let Some(leaf) = self.find(target) else { return false };
        let Some(parent) = self.nodes[leaf].parent else { return false };
        let Kind::Split(s) = self.nodes[parent].kind else { return false };
        let mut top = parent;
        while let Some(p) = self.nodes[top].parent {
            match self.nodes[p].kind {
                Kind::Split(ps) if ps.axis == s.axis => top = p,
                _ => break,
            }
        }
        let mut leaves = Vec::new();
        if !self.run_leaves(top, s.axis, &mut leaves) || leaves.len() < 2 {
            return false;
        }
        let panes: Vec<PaneId> = leaves.iter().map(|&i| self.pane_at(i)).collect();
        for (k, &i) in leaves.iter().enumerate() {
            self.nodes[i].kind = Kind::Leaf(panes[(k + panes.len() - 1) % panes.len()]);
        }
        true
    }

    fn run_leaves(&self, i: Index, axis: Axis, out: &mut Vec<Index>) -> bool {
        match self.nodes[i].kind {
            Kind::Leaf(_) => {
                out.push(i);
                true
            }
            Kind::Split(s) if s.axis == axis => self.run_leaves(s.a, axis, out) && self.run_leaves(s.b, axis, out),
            _ => false,
        }
    }

    fn pane_at(&self, i: Index) -> PaneId {
        match self.nodes[i].kind {
            Kind::Leaf(p) => p,
            _ => unreachable!("run_leaves only collects leaves"),
        }
    }

    // Panes side by side along `axis`, so `=` gives each pane, not each subtree, an equal share.
    fn weight(&self, i: Index, axis: Axis) -> usize {
        match self.nodes[i].kind {
            Kind::Leaf(_) | Kind::Free => 1,
            Kind::Split(s) if s.axis == axis => self.weight(s.a, axis) + self.weight(s.b, axis),
            Kind::Split(s) => self.weight(s.a, axis).max(self.weight(s.b, axis)),
        }
    }

    fn shift(&mut self, area: Rect, node: Index, cells: i32) {
        let r = self.rect_of(area, node);
        let Kind::Split(s) = &mut self.nodes[node].kind else { return };
        let span = if s.axis == Axis::Horizontal { r.w } else { r.h };
        if span < 3 {
            return;
        }
        let usable = (span - 1) as f32;
        let min = 1.0 / usable;
        s.ratio = (s.ratio + cells as f32 / usable).clamp(min, 1.0 - min);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { x: 0, y: 0, w: 81, h: 25 };

    fn widths(t: &Tree) -> Vec<(PaneId, u16)> {
        t.layout(SCREEN).iter().map(|p| (p.pane, p.rect.w)).collect()
    }

    #[test]
    fn grow_and_shrink_the_current_pane() {
        let mut t = Tree::new(1);
        t.split(1, Axis::Horizontal, 2).unwrap();
        assert!(t.grow(SCREEN, 2, Axis::Horizontal, 10));
        assert_eq!(widths(&t), vec![(1, 30), (2, 50)]);
        assert!(t.grow(SCREEN, 1, Axis::Horizontal, -5));
        assert_eq!(widths(&t), vec![(1, 25), (2, 55)]);
        assert!(!t.grow(SCREEN, 1, Axis::Vertical, 1));
    }

    #[test]
    fn equalize_counts_panes_not_subtrees() {
        let mut t = Tree::new(1);
        t.split(1, Axis::Horizontal, 2).unwrap();
        t.split(2, Axis::Horizontal, 3).unwrap();
        t.equalize();
        let w = widths(&t);
        assert!(w.iter().all(|(_, w)| (25..=27).contains(w)), "{w:?}");
    }

    #[test]
    fn maximize_leaves_one_cell_for_others() {
        let mut t = Tree::new(1);
        t.split(1, Axis::Horizontal, 2).unwrap();
        t.maximize(SCREEN, 1, Axis::Horizontal);
        assert_eq!(widths(&t), vec![(1, 79), (2, 1)]);
    }

    #[test]
    fn move_to_edge_spans_the_full_side() {
        let mut t = Tree::new(1);
        t.split(1, Axis::Horizontal, 2).unwrap();
        t.split(2, Axis::Horizontal, 3).unwrap();
        assert!(t.move_to_edge(3, Direction::Up));
        let ps = t.layout(SCREEN);
        let top = ps.iter().find(|p| p.pane == 3).unwrap().rect;
        assert_eq!((top.y, top.w), (0, 81));
        assert!(ps.iter().filter(|p| p.pane != 3).all(|p| p.rect.y > 0));
    }

    #[test]
    fn exchange_swaps_with_the_sibling() {
        let mut t = Tree::new(1);
        t.split(1, Axis::Horizontal, 2).unwrap();
        assert_eq!(t.exchange(1), Some(2));
        assert_eq!(widths(&t)[0].0, 2);
    }

    #[test]
    fn rotate_a_row_and_refuse_mixed_splits() {
        let mut t = Tree::new(1);
        t.split(1, Axis::Horizontal, 2).unwrap();
        t.split(2, Axis::Horizontal, 3).unwrap();
        assert!(t.rotate(1));
        let order: Vec<_> = t.layout(SCREEN).iter().map(|p| p.pane).collect();
        assert_eq!(order, vec![3, 1, 2]);
        t.split(1, Axis::Vertical, 4).unwrap();
        assert!(!t.rotate(3));
    }
}
