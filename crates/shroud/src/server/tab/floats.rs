//! Floats sit beside the tree, so moving a pane between the two is just re-homing its id.

use std::io;
use std::path::Path;

use shroud_core::layout::{Axis, Direction, FloatRect, PaneId, Placement, neighbour};

use super::super::pane::{self, Pane};
use super::{Float, Tab};

impl Tab {
    pub fn is_float(&self, id: PaneId) -> bool {
        self.floats.iter().any(|f| f.pane == id)
    }

    pub fn new_float(&mut self, at: FloatRect, cwd: Option<&Path>, focus: bool) -> io::Result<PaneId> {
        let id = pane::next_id();
        let r = at.inner(self.area);
        let pane = Pane::spawn(id, r.w, r.h, cwd, &self.launch)?;
        self.panes.insert(id, pane);
        self.floats.push(Float { pane: id, at });
        self.floats_shown = true;
        if focus {
            self.set_focus(id);
        }
        self.relayout();
        Ok(id)
    }

    /// With no floats yet this opens one, so the key is useful from the start.
    pub fn toggle_floats(&mut self) -> io::Result<()> {
        if self.floats.is_empty() {
            let cwd = self.focused_cwd();
            return self.new_float(FloatRect::CENTRED, cwd.as_deref(), true).map(|_| ());
        }
        self.floats_shown = !self.floats_shown;
        if self.floats_shown {
            let top = self.floats[self.floats.len() - 1].pane;
            self.set_focus(top);
        } else if self.is_float(self.focused) {
            self.focus_tiled();
        }
        Ok(())
    }

    /// Floats cover the tiles, so geometry only says which float is next; past the last one
    /// in that direction, focus drops back to the tiles with the floats out of the way.
    pub(super) fn focus_from_float(&mut self, dir: Direction) {
        let floats: Vec<Placement> =
            self.floats.iter().map(|f| Placement { pane: f.pane, rect: f.at.outer(self.area) }).collect();
        match neighbour(&floats, self.focused, dir) {
            Some(id) => self.set_focus(id),
            None => {
                self.floats_shown = false;
                self.focus_tiled();
            }
        }
    }

    pub(super) fn focus_tiled(&mut self) {
        let tiled =
            self.previous.filter(|p| self.tree.contains(*p)).or_else(|| self.placements().first().map(|p| p.pane));
        if let Some(id) = tiled {
            self.set_focus(id);
        }
    }

    /// A tab must keep one tile, so its last one can't float.
    pub fn float_or_tile(&mut self) -> Result<(), &'static str> {
        let id = self.focused;
        if let Some(i) = self.floats.iter().position(|f| f.pane == id) {
            let target = self
                .previous
                .filter(|p| self.tree.contains(*p))
                .or_else(|| self.placements().first().map(|p| p.pane))
                .ok_or("no tile to join")?;
            self.tree.split(target, Axis::Horizontal, id).map_err(|_| "no room for another tile")?;
            self.floats.remove(i);
        } else {
            // Removing the last leaf empties the tree, so refuse before touching it.
            if self.tree.len() < 2 {
                return Err("a tab needs one tiled pane");
            }
            let next = self.tree.remove(id).ok().flatten().ok_or("a tab needs one tiled pane")?;
            self.floats.push(Float { pane: id, at: FloatRect::CENTRED });
            self.floats_shown = true;
            self.previous = Some(next);
        }
        self.zoomed = None;
        self.relayout();
        Ok(())
    }
}
