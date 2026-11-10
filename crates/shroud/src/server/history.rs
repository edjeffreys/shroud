//! Compression runs in bounded steps on quiet panes only, so a keystroke never waits behind it.

use std::time::{Duration, Instant};

use super::Server;

const IDLE: Duration = Duration::from_secs(2);
const BUDGET: Duration = Duration::from_millis(1);

impl Server {
    pub(super) fn sync_history(&mut self) {
        let bytes = self.settings.scrollback_bytes();
        for s in &mut self.sessions {
            s.panes_mut().for_each(|(_, p)| p.set_scrollback(bytes));
        }
    }

    pub(super) fn compress_idle(&mut self) {
        let start = Instant::now();
        for s in &mut self.sessions {
            for (_, p) in s.panes_mut() {
                if p.compress_due(IDLE).is_none_or(|at| at > start) {
                    continue;
                }
                while p.compress_step() {
                    if start.elapsed() >= BUDGET {
                        return;
                    }
                }
            }
        }
    }

    pub(super) fn compress_deadline(&self) -> Option<Instant> {
        self.sessions.iter().flat_map(|s| s.panes().filter_map(|(_, p)| p.compress_due(IDLE))).min()
    }
}
