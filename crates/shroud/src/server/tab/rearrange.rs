//! A tab rearranged in place: its panes keep running and only move. New panes start only
//! after the whole arrangement checks out.

use std::collections::HashSet;
use std::path::Path;

use shroud_core::config::PaneSpec;
use shroud_core::layout::rearrange::check;
use shroud_core::layout::{FloatRect, PaneId, Shape, Slot, Tree};

use super::super::pane::{self, Pane};
use super::{Float, Tab, resolve};

impl Tab {
    pub fn rearrange(
        &mut self,
        shape: Shape<Slot<PaneSpec>>,
        floats: Vec<(FloatRect, Slot<PaneSpec>)>,
        base: &Path,
    ) -> Result<(), String> {
        let tiles: Vec<PaneId> = self.tree.to_shape().leaves().into_iter().copied().collect();
        let float_ids: Vec<PaneId> = self.floats.iter().map(|f| f.pane).collect();
        check(&tiles, &float_ids, &shape, &floats.iter().map(|(_, s)| s).collect::<Vec<_>>())?;

        let mut started: Vec<(PaneId, Pane)> = Vec::new();
        let mut failed = None;
        let mut place = |slot: Slot<PaneSpec>| match slot {
            Slot::Pane(id) => id,
            Slot::New(spec) => {
                let id = pane::next_id();
                match resolve(base, spec.cwd.as_deref())
                    .and_then(|cwd| Pane::spawn(id, self.area.w.max(1), self.area.h.max(1), Some(&cwd), &self.launch))
                {
                    Ok(mut p) => {
                        if let Some(cmd) = &spec.command {
                            p.type_line(cmd);
                        }
                        p.tags = spec.tags;
                        started.push((id, p));
                    }
                    Err(e) => failed = Some(e.to_string()),
                }
                id
            }
        };
        let ids = shape.map(&mut place);
        let floats: Vec<(FloatRect, PaneId)> = floats.into_iter().map(|(at, s)| (at, place(s))).collect();
        if let Some(e) = failed {
            return Err(format!("a new pane didn't start: {e}"));
        }
        let tree = Tree::from_shape(&ids).ok_or("a layout needs at least one tile")?;

        self.panes.extend(started);
        let tiled: HashSet<PaneId> = ids.leaves().into_iter().copied().collect();
        self.floats.retain(|f| !tiled.contains(&f.pane));
        for (at, id) in floats {
            match self.floats.iter_mut().find(|f| f.pane == id) {
                Some(f) => f.at = at,
                None => self.floats.push(Float { pane: id, at }),
            }
        }
        self.tree = tree;
        self.zoomed = None;
        self.relayout();
        Ok(())
    }
}
