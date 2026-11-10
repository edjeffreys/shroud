//! OSC 8 URIs are interned server-wide; this drops the ones no pane shows any more.

use std::collections::HashSet;

use shroud_core::render::NO_LINK;

use super::Server;

const LINKS_BEFORE_COLLECTING: usize = 1024;

impl Server {
    // Only pane grids can bring an id back to the screen, so only they keep a link alive.
    pub(super) fn collect_links(&mut self) {
        if self.links.len() < (2 * self.links_kept).max(LINKS_BEFORE_COLLECTING) {
            return;
        }
        let mut live = HashSet::new();
        for (_, p) in self.sessions.iter().flat_map(|s| s.panes()) {
            for y in 0..p.grid.rows() {
                live.extend(p.grid.row(y).iter().map(|c| c.link).filter(|&l| l != NO_LINK));
            }
        }
        self.links.retain(&live);
        self.links_kept = self.links.len();
    }
}
