//! Runs at most once per paced tick, so per-frame cost here is the latency budget.

use std::time::Instant;

use shroud_core::layout::Rect;
use shroud_core::overlay::draw_bar;
use shroud_core::pace::Decision;
use shroud_core::protocol::{ServerMsg, Size};
use shroud_core::render::{Cursor, CursorShape, Frame, PaneView, compose, diff};
use shroud_core::statusbar::{self, PaneBadge, Status};

use super::tab::pane_area;
use super::{Server, plugin_bar};

const SYNC_END: &[u8] = b"\x1b[?2026l";

impl Server {
    /// The earliest moment a pane's mode-2026 hold runs out over output it held back.
    pub(super) fn hold_deadline(&self) -> Option<Instant> {
        self.sessions.iter().flat_map(|s| s.panes().filter_map(|(_, p)| p.hold_due())).min()
    }

    /// Ends expired holds and asks for the frame they held back. Clearing the hold also
    /// clears its deadline, so the loop doesn't spin on one that's already past.
    pub(super) fn expire_holds(&mut self) {
        let mut expired = false;
        for s in &mut self.sessions {
            s.panes_mut().for_each(|(_, p)| expired |= p.expire_hold());
        }
        if expired {
            self.invalidate();
        }
    }

    pub(super) fn maybe_render(&mut self) {
        let now = self.start.elapsed();
        let (dirty, backpressured) = self.frame_pressure();
        if self.pacer.decide(now, dirty, backpressured) != Decision::RenderNow {
            return;
        }
        let started = Instant::now();
        let frame_time = self.frame_time.clone();
        let mut sent = false;
        for session in &mut self.sessions {
            if !self.clients.iter().any(|c| c.session == Some(session.id) && c.needs_frame && !c.wants_write()) {
                continue;
            }
            session.tab_mut().panes.values_mut().for_each(|p| p.snapshot(&mut self.links));
            let tabs = session.tab_entries();
            let tab = session.tab();
            let placements = tab.placements();
            let drawn = tab.views();
            let focused = drawn.iter().position(|v| v.pane == tab.focused);
            let views: Vec<PaneView> = drawn
                .iter()
                .filter_map(|v| {
                    tab.panes.get(&v.pane).map(|pane| {
                        let title = tab
                            .frames()
                            .then(|| pane.label.as_deref().unwrap_or(&pane.title))
                            .filter(|t| !t.is_empty());
                        PaneView {
                            rect: v.rect,
                            grid: &pane.grid,
                            cursor: pane.cursor,
                            float: v.float,
                            highlight: None,
                            title,
                            matches: &[],
                        }
                    })
                })
                .collect();
            let title = tab.panes.get(&tab.focused).map(|p| p.title.clone()).unwrap_or_default();
            let badges: Vec<PaneBadge> = placements
                .iter()
                .enumerate()
                .filter_map(|(i, p)| Some(PaneBadge { pane: i as u16 + 1, badge: tab.panes.get(&p.pane)?.badge? }))
                .collect();
            let area = tab.area();
            let st = &session.status;
            let broadcasting = tab.broadcast.then(|| self.settings.theme.broadcasting());
            let theme = broadcasting.as_ref().unwrap_or(&self.settings.theme);
            for c in self.clients.iter_mut().filter(|c| c.session == Some(session.id)) {
                if c.size.is_none() || !c.needs_frame || c.wants_write() {
                    continue;
                }
                // Visual mode ends when its pane loses focus, wherever that happened.
                if c.visual.as_ref().is_some_and(|(id, _)| *id != tab.focused) {
                    c.visual = None;
                }
                if c.jump.as_ref().is_some_and(|j| j.pane != tab.focused) {
                    c.jump = None;
                }
                let mut search = None;
                let mut own = None;
                let found;
                if let Some((id, v)) = &mut c.visual
                    && let (Some(at), Some(pane)) = (drawn.iter().position(|d| d.pane == *id), tab.panes.get(id))
                {
                    let top = pane.viewport().top;
                    v.resize(drawn[at].rect.h);
                    found = v.matches(&pane.history(), top);
                    let mut mine = views.clone();
                    mine[at].highlight = v.highlight(top);
                    mine[at].matches = &found;
                    mine[at].cursor = match v.local_cursor(top) {
                        Some((x, y)) => Cursor { x, y, visible: true, shape: CursorShape::Block, blink: false },
                        None => Cursor { visible: false, ..Cursor::default() },
                    };
                    search = v.prompt();
                    own = Some(mine);
                } else if let Some(j) = &c.jump
                    && let (Some(at), Some(pane)) =
                        (drawn.iter().position(|d| d.pane == j.pane), tab.panes.get(&j.pane))
                {
                    let mut mine = views.clone();
                    mine[at].highlight = Some(j.highlight(pane.viewport().top));
                    search = Some(j.label.clone());
                    own = Some(mine);
                }
                compose(&mut c.next, area, own.as_deref().unwrap_or(&views), focused, theme);
                if area.h < c.next.grid.rows() {
                    let status = Status {
                        mode: c.mode(),
                        session: Some(&session.name),
                        tabs: &tabs,
                        zoomed: tab.is_zoomed(),
                        broadcast: tab.broadcast,
                        cwd: st.cwd.as_deref(),
                        branch: st.branch.as_deref(),
                        message: search.as_deref().or(st.message()),
                        badges: &badges,
                        alert: if !self.problems.is_empty() {
                            Some("config")
                        } else {
                            self.plugin_host.alert.as_ref().map(|_| "plugin")
                        },
                        frame_time: frame_time.as_deref().filter(|f| !f.is_empty()),
                        clock: &st.clock,
                    };
                    match self.status_owner {
                        Some(owner) => {
                            plugin_bar::tell_plugin(
                                c,
                                &mut self.controls,
                                owner,
                                plugin_bar::state(&status, c.next.grid.cols()),
                            );
                            match &c.bar {
                                Some(bar) => draw_bar(
                                    &mut c.next.grid,
                                    area.h,
                                    &bar.left,
                                    &bar.right,
                                    bar.fill.as_deref(),
                                    &self.settings.theme,
                                ),
                                None => statusbar::draw(&mut c.next.grid, area.h, &status, &self.settings.theme),
                            }
                        }
                        None => statusbar::draw(&mut c.next.grid, area.h, &status, &self.settings.theme),
                    }
                }
                let cursor = (c.next.cursor.x, c.next.cursor.y);
                let mut covered = Vec::new();
                for p in self.overlays.iter().filter(|p| p.shown_to(session.id, c.id)) {
                    p.overlay.draw(&mut c.next.grid, area, cursor, &self.settings.theme);
                    covered.push(p.overlay.rect(area, cursor));
                }
                if let Some(sw) = &c.switcher {
                    let over = pane_area_of(&c.next);
                    c.next.cursor = sw.draw(&mut c.next.grid, over, &self.settings.theme);
                    covered.extend(sw.view(over).map(|(o, _)| o.rect(over, (0, 0))));
                }
                c.next.title.clone_from(&title);
                self.out.clear();
                if c.prev.is_none() {
                    c.images.reset(&mut self.out);
                }
                diff(&mut self.out, c.prev.as_ref(), &c.next, &self.links);
                let mut pictures = Vec::new();
                c.images.sync(&mut pictures, tab, &drawn, &covered);
                // Inside the synchronized update, so images move with the text around them.
                match self.out.len().checked_sub(SYNC_END.len()).filter(|&at| self.out[at..] == *SYNC_END) {
                    Some(at) => {
                        self.out.splice(at..at, pictures);
                    }
                    None => self.out.extend_from_slice(&pictures),
                }
                if !self.out.is_empty() {
                    c.send(ServerMsg::Output(&self.out));
                    sent = true;
                }
                match &mut c.prev {
                    Some(prev) => std::mem::swap(prev, &mut c.next),
                    None => c.prev = Some(c.next.clone()),
                }
                c.needs_frame = false;
            }
        }
        // A keypress invalidates before its echo arrives; an empty frame mustn't make the
        // echo wait out the frame interval.
        if sent {
            self.pacer.rendered(now);
        }
        // Shown in the next frame; redrawing for it alone would render forever.
        if let Some(f) = &mut self.frame_time {
            *f = format!("{:.2}ms", started.elapsed().as_secs_f64() * 1000.0);
        }
        self.collect_links();
    }
}

fn pane_area_of(frame: &Frame) -> Rect {
    pane_area(Size { cols: frame.grid.cols(), rows: frame.grid.rows(), px_w: 0, px_h: 0 })
}
