//! Checked up front, so a bad arrangement changes nothing.

use std::collections::HashSet;

use super::shape::Shape;
use super::tree::PaneId;

#[derive(Clone, Debug, PartialEq)]
pub enum Slot<S> {
    Pane(PaneId),
    New(S),
}

/// Floats left out stay put; tiles left out are an error.
pub fn check<S>(
    tiles: &[PaneId],
    floats: &[PaneId],
    shape: &Shape<Slot<S>>,
    new_floats: &[&Slot<S>],
) -> Result<(), String> {
    let placed: Vec<&Slot<S>> = shape.leaves().into_iter().chain(new_floats.iter().copied()).collect();
    if shape.leaves().is_empty() {
        return Err("a layout needs at least one tile".into());
    }
    let mut seen = HashSet::new();
    for slot in &placed {
        if let Slot::Pane(id) = slot {
            if !tiles.contains(id) && !floats.contains(id) {
                return Err(format!("pane {id} isn't in this tab"));
            }
            if !seen.insert(*id) {
                return Err(format!("pane {id} is placed twice"));
            }
        }
    }
    let missing: Vec<String> = tiles.iter().filter(|id| !seen.contains(id)).map(|id| id.to_string()).collect();
    if !missing.is_empty() {
        return Err(format!("the layout leaves out pane {}: place every tile, or close it first", missing.join(", ")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Axis;

    fn split(parts: Vec<Slot<()>>) -> Shape<Slot<()>> {
        Shape::Split { axis: Axis::Horizontal, parts: parts.into_iter().map(|s| (None, Shape::Leaf(s))).collect() }
    }

    #[test]
    fn panes_can_swap_turn_float_and_join_new_ones() {
        assert_eq!(check(&[1, 2], &[3], &split(vec![Slot::Pane(2), Slot::Pane(1), Slot::New(())]), &[]), Ok(()));
        assert_eq!(check(&[1, 2], &[3], &split(vec![Slot::Pane(3), Slot::Pane(1)]), &[&Slot::Pane(2)]), Ok(()));
    }

    #[test]
    fn a_bad_arrangement_says_why() {
        let e = |r: Result<(), String>| r.unwrap_err();
        assert!(e(check(&[1, 2], &[], &split(vec![Slot::Pane(1)]), &[])).contains("leaves out pane 2"));
        assert!(e(check(&[1], &[], &split(vec![Slot::Pane(1), Slot::Pane(1)]), &[])).contains("placed twice"));
        assert!(
            e(check(&[1], &[], &split(vec![Slot::Pane(1), Slot::Pane(9)]), &[])).contains("pane 9 isn't in this tab")
        );
        assert!(e(check(&[1], &[], &split(vec![]), &[&Slot::Pane(1)])).contains("at least one tile"));
    }
}
