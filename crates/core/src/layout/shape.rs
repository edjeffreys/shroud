//! The tree is binary, so an n-part split becomes nested pairs whose ratios keep each share.

use super::tree::{Axis, Index, Kind, Node, PaneId, Split, Tree};

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Shape<T> {
    Leaf(T),
    /// A part's size is a fraction of the split; unsized parts share what's left.
    Split {
        axis: Axis,
        parts: Vec<(Option<f32>, Shape<T>)>,
    },
}

impl<T> Shape<T> {
    pub fn leaves(&self) -> Vec<&T> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    pub fn map<U>(self, f: &mut impl FnMut(T) -> U) -> Shape<U> {
        match self {
            Shape::Leaf(t) => Shape::Leaf(f(t)),
            Shape::Split { axis, parts } => {
                Shape::Split { axis, parts: parts.into_iter().map(|(s, p)| (s, p.map(f))).collect() }
            }
        }
    }

    fn collect<'a>(&'a self, out: &mut Vec<&'a T>) {
        match self {
            Shape::Leaf(t) => out.push(t),
            Shape::Split { parts, .. } => parts.iter().for_each(|(_, p)| p.collect(out)),
        }
    }
}

impl Tree {
    /// An empty split has no pane to hold, so it is `None`.
    pub fn from_shape(shape: &Shape<PaneId>) -> Option<Tree> {
        let mut tree = Tree { nodes: Vec::new(), root: 0 };
        tree.root = tree.build(shape, None)?;
        Some(tree)
    }

    fn build(&mut self, shape: &Shape<PaneId>, parent: Option<Index>) -> Option<Index> {
        match shape {
            Shape::Leaf(p) => Some(self.push(Node { parent, kind: Kind::Leaf(*p) })),
            Shape::Split { axis, parts } => {
                let shares = shares(parts.iter().map(|(s, _)| *s));
                let parts: Vec<_> = parts.iter().map(|(_, p)| p).zip(shares).collect();
                self.build_parts(*axis, &parts, parent)
            }
        }
    }

    fn build_parts(&mut self, axis: Axis, parts: &[(&Shape<PaneId>, f32)], parent: Option<Index>) -> Option<Index> {
        match parts {
            [] => None,
            [(only, _)] => self.build(only, parent),
            [(first, share), rest @ ..] => {
                // Not `alloc`: it would hand this placeholder's slot to the first child.
                let me = self.push(Node { parent, kind: Kind::Free });
                let a = self.build(first, Some(me))?;
                let b = self.build_parts(axis, rest, Some(me))?;
                let total: f32 = share + rest.iter().map(|(_, s)| s).sum::<f32>();
                let ratio = if total > 0.0 { share / total } else { 0.5 };
                self.nodes[me].kind = Kind::Split(Split { axis, ratio, a, b });
                Some(me)
            }
        }
    }

    /// Every part sized, and same-direction pairs merged back into one split.
    pub fn to_shape(&self) -> Shape<PaneId> {
        self.shape_of(self.root)
    }

    fn shape_of(&self, at: Index) -> Shape<PaneId> {
        match &self.nodes[at].kind {
            Kind::Split(s) => {
                let mut parts = Vec::new();
                for (child, share) in [(s.a, s.ratio), (s.b, 1.0 - s.ratio)] {
                    match self.shape_of(child) {
                        Shape::Split { axis, parts: inner } if axis == s.axis => {
                            parts.extend(inner.into_iter().map(|(size, p)| (size.map(|f| f * share), p)));
                        }
                        other => parts.push((Some(share), other)),
                    }
                }
                Shape::Split { axis: s.axis, parts }
            }
            Kind::Leaf(p) => Shape::Leaf(*p),
            Kind::Free => Shape::Split { axis: Axis::Horizontal, parts: Vec::new() },
        }
    }

    fn push(&mut self, node: Node) -> Index {
        self.nodes.push(node);
        self.nodes.len() - 1
    }
}

fn shares(sizes: impl Iterator<Item = Option<f32>> + Clone) -> Vec<f32> {
    let given: f32 = sizes.clone().flatten().sum();
    let unsized_count = sizes.clone().filter(Option::is_none).count();
    let rest = if unsized_count > 0 { (1.0 - given).max(0.0) / unsized_count as f32 } else { 0.0 };
    sizes.map(|s| s.unwrap_or(rest)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Rect;

    fn widths(shape: &Shape<PaneId>, w: u16) -> Vec<u16> {
        Tree::from_shape(shape).unwrap().layout(Rect::new(0, 0, w, 10)).iter().map(|p| p.rect.w).collect()
    }

    #[test]
    fn three_unsized_parts_share_equally() {
        let s = Shape::Split { axis: Axis::Horizontal, parts: (1..=3).map(|p| (None, Shape::Leaf(p))).collect() };
        assert_eq!(widths(&s, 32), vec![10, 10, 10]);
    }

    #[test]
    fn sized_parts_take_their_share_and_the_rest_splits_what_is_left() {
        let s = Shape::Split {
            axis: Axis::Horizontal,
            parts: vec![(Some(0.5), Shape::Leaf(1)), (None, Shape::Leaf(2)), (None, Shape::Leaf(3))],
        };
        assert_eq!(widths(&s, 43), vec![21, 10, 10]);
    }

    #[test]
    fn nesting_keeps_leaf_order() {
        let inner = Shape::Split { axis: Axis::Vertical, parts: vec![(None, Shape::Leaf(2)), (None, Shape::Leaf(3))] };
        let s = Shape::Split { axis: Axis::Horizontal, parts: vec![(None, Shape::Leaf(1)), (Some(0.3), inner)] };
        let order: Vec<_> =
            Tree::from_shape(&s).unwrap().layout(Rect::new(0, 0, 80, 24)).iter().map(|p| p.pane).collect();
        assert_eq!(order, vec![1, 2, 3]);
        assert_eq!(s.leaves(), vec![&1, &2, &3]);
    }

    #[test]
    fn to_shape_round_trips_and_flattens() {
        let inner = Shape::Split { axis: Axis::Vertical, parts: vec![(None, Shape::Leaf(2)), (None, Shape::Leaf(3))] };
        let s = Shape::Split {
            axis: Axis::Horizontal,
            parts: vec![(Some(0.2), Shape::Leaf(1)), (Some(0.3), inner), (None, Shape::Leaf(4))],
        };
        let tree = Tree::from_shape(&s).unwrap();
        let back = tree.to_shape();
        let Shape::Split { axis: Axis::Horizontal, parts } = &back else { panic!("{back:?}") };
        assert_eq!(parts.len(), 3, "the nested horizontal pairs merge back into one split");
        assert!((parts[0].0.unwrap() - 0.2).abs() < 1e-4 && (parts[2].0.unwrap() - 0.5).abs() < 1e-4);
        let area = Rect::new(0, 0, 80, 24);
        assert_eq!(Tree::from_shape(&back).unwrap().layout(area), tree.layout(area));
    }

    #[test]
    fn an_empty_split_builds_nothing() {
        assert!(Tree::from_shape(&Shape::Split { axis: Axis::Vertical, parts: vec![] }).is_none());
    }
}
