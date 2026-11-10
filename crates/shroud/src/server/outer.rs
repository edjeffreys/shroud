//! What the outer terminal is like (colours, light/dark, grapheme widths), passed down
//! to every pane's terminal. With several clients attached, the latest reply wins.

use shroud_core::input::{ColorSlot, OuterColors, Rgb};
use shroud_core::protocol::ServerMsg;

use super::Server;

impl Server {
    pub(super) fn query_outer(&mut self, client: usize) {
        let mut q = OuterColors::query();
        q.extend_from_slice(b"\x1b[?2027$p");
        self.clients[client].send(ServerMsg::Output(&q));
    }

    // Pane widths must match the outer terminal's, or a cluster it draws as one cell
    // shifts the rest of the row.
    pub(super) fn outer_graphemes(&mut self, on: bool) {
        if self.outer_graphemes != on {
            self.outer_graphemes = on;
            self.outer_gen += 1;
        }
    }

    pub(super) fn outer_color(&mut self, slot: ColorSlot, rgb: Rgb) {
        if self.outer_colors.set(slot, rgb) {
            self.outer_gen += 1;
        }
    }

    pub(super) fn color_scheme_changed(&mut self, client: usize, dark: bool) {
        let changed = self.outer_dark.replace(dark) != Some(dark);
        if changed {
            self.outer_gen += 1;
            for s in &mut self.sessions {
                s.panes_mut().for_each(|(_, p)| p.report_color_scheme(dark));
            }
        }
        // A theme switch changes the palette too, not just the scheme.
        self.query_outer(client);
    }

    // Generations rather than pushing on change: new panes pick the colours up here too.
    pub(super) fn sync_outer(&mut self) {
        let (colors, dark, graphemes, gen_) =
            (&self.outer_colors, self.outer_dark, self.outer_graphemes, self.outer_gen);
        for s in &mut self.sessions {
            for (_, p) in s.panes_mut().filter(|(_, p)| p.outer_gen != gen_) {
                p.set_outer(colors, dark, graphemes);
                p.outer_gen = gen_;
            }
        }
    }
}
