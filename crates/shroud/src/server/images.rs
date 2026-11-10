//! Ids are per client, so clients never clash and one detaching frees only its own copies.

use std::collections::{HashMap, HashSet};

use shroud_core::layout::{PaneId, Rect};
use shroud_core::render::images::{self, Placed, REMOVE_ALL};

use super::tab::{Tab, View};

#[derive(Debug, Default)]
pub struct ClientImages {
    sent: HashMap<(PaneId, u32), (u32, u64)>,
    placed: HashMap<(u32, u32), Placed>,
    next_id: u32,
}

impl ClientImages {
    pub fn is_empty(&self) -> bool {
        self.sent.is_empty()
    }

    pub fn reset(&mut self, out: &mut Vec<u8>) {
        if !self.sent.is_empty() || !self.placed.is_empty() {
            out.extend_from_slice(REMOVE_ALL);
        }
        self.sent.clear();
        self.placed.clear();
    }

    /// `covered` lies over every pane: overlays and the switcher.
    pub fn sync(&mut self, out: &mut Vec<u8>, tab: &Tab, views: &[View], covered: &[Rect]) {
        let mut cmds = Vec::new();
        let mut want: HashMap<(u32, u32), Placed> = HashMap::new();
        let mut used = HashSet::new();
        for (i, v) in views.iter().enumerate() {
            let Some(pane) = tab.panes.get(&v.pane) else { continue };
            if pane.images.is_empty() {
                continue;
            }
            let above: Vec<Rect> =
                covered.iter().copied().chain(views[i + 1..].iter().filter_map(|w| w.float)).collect();
            let mut nth: HashMap<u32, u32> = HashMap::new();
            for p in &pane.images {
                let Some(placed) = images::place(p, v.rect, tab.cell_px(), &above) else { continue };
                let key = (v.pane, p.image);
                let id = match self.sent.get(&key) {
                    Some(&(id, generation)) if generation == p.generation => id,
                    known => {
                        let Some(img) = pane.image(p.image) else { continue };
                        let id = known.map(|&(id, _)| id).unwrap_or_else(|| {
                            self.next_id = self.next_id.wrapping_add(1).max(1);
                            self.next_id
                        });
                        images::transmit(&mut cmds, id, img.width, img.height, img.alpha, &img.pixels);
                        self.sent.insert(key, (id, p.generation));
                        id
                    }
                };
                used.insert(key);
                let n = nth.entry(p.image).or_default();
                *n += 1;
                want.insert((id, *n), placed);
            }
        }
        for &(id, pid) in self.placed.keys() {
            if !want.contains_key(&(id, pid)) {
                images::remove_placement(&mut cmds, id, pid);
            }
        }
        for (&(id, pid), p) in &want {
            if self.placed.get(&(id, pid)) != Some(p) {
                images::put(&mut cmds, id, pid, p);
            }
        }
        self.sent.retain(|key, &mut (id, _)| {
            let keep = used.contains(key);
            if !keep {
                images::remove_image(&mut cmds, id);
            }
            keep
        });
        self.placed = want;
        if !cmds.is_empty() {
            out.extend_from_slice(b"\x1b7");
            out.extend_from_slice(&cmds);
            out.extend_from_slice(b"\x1b8");
        }
    }
}
