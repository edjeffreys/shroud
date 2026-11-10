//! Selecting with the mouse in panes whose program doesn't use it. The selection is visual mode's,
//! so it draws and copies the same way; programs that track the mouse still get every event.

use std::time::{Duration, Instant};

use shroud_core::input::{MouseAction, MouseButton, MouseEvent};
use shroud_core::layout::PaneId;
use shroud_core::render::Shape;
use shroud_core::visual::{Pos, Selection, Visual};

use super::Server;

const MULTI_CLICK: Duration = Duration::from_millis(400);
const WHEEL_LINES: i64 = 3;

#[derive(Clone, Copy, Debug)]
pub struct Drag {
    pane: PaneId,
    from: Pos,
    moved: bool,
}

enum Copy {
    Text(String),
    Selection(PaneId, Selection),
}

#[derive(Clone, Copy, Debug)]
pub struct Click {
    at: Instant,
    pane: PaneId,
    pos: Pos,
    count: u8,
}

impl Server {
    /// True when the event was a selection's, so the pane must not see it.
    pub(super) fn select_mouse(&mut self, i: usize, m: MouseEvent) -> bool {
        let drag = self.clients[i].drag;
        let wheel = match m.button {
            Some(MouseButton::WheelUp) => -WHEEL_LINES,
            Some(MouseButton::WheelDown) => WHEEL_LINES,
            _ => 0,
        };
        let left = m.button == Some(MouseButton::Left);
        match (m.action, drag) {
            (MouseAction::Press, _) if left || wheel != 0 => {}
            (MouseAction::Motion, Some(_)) if left => {}
            (MouseAction::Release, Some(_)) => {}
            _ => return false,
        }
        // Taken out so the session can be borrowed alongside; put back below.
        let mut visual = self.clients[i].visual.take();
        let mut click = self.clients[i].last_click;
        let mut next_drag = None;
        let (handled, copy) = self.pick(i, m, drag, wheel, &mut visual, &mut click, &mut next_drag);
        let c = &mut self.clients[i];
        c.visual = visual;
        c.last_click = click;
        c.drag = next_drag;
        match copy {
            Some(Copy::Text(t)) => self.copy_text(i, &t),
            Some(Copy::Selection(id, sel)) => self.copy_selection(i, id, sel),
            None => {}
        }
        handled
    }

    #[allow(clippy::too_many_arguments)]
    fn pick(
        &mut self,
        i: usize,
        m: MouseEvent,
        drag: Option<Drag>,
        wheel: i64,
        visual: &mut Option<(PaneId, Visual)>,
        click: &mut Option<Click>,
        next_drag: &mut Option<Drag>,
    ) -> (bool, Option<Copy>) {
        let Some(session) = self.session_of(i) else { return (false, None) };
        let tab = session.tab_mut();
        let views = tab.views();
        let view = match drag {
            Some(d) => views.iter().find(|v| v.pane == d.pane),
            None => views.iter().rev().find(|v| v.float.unwrap_or(v.rect).contains(m.x, m.y)),
        };
        let Some(view) = view.copied() else { return (drag.is_some(), None) };
        let tracked = tab.panes.get(&view.pane).is_none_or(|p| p.tracks_mouse());
        if drag.is_none() && tracked {
            *next_drag = drag;
            return (false, None);
        }
        if m.action == MouseAction::Press && wheel == 0 {
            tab.focus_pane(view.pane);
        }
        let Some(pane) = tab.panes.get_mut(&view.pane) else { return (false, None) };
        let vp = pane.viewport();
        let r = view.rect;
        let pos = Pos {
            x: m.x.saturating_sub(r.x).min(r.w.saturating_sub(1)),
            y: vp.top + m.y.saturating_sub(r.y).min(r.h.saturating_sub(1)) as u32,
        };
        if wheel != 0 {
            *next_drag = drag;
            return match visual {
                Some((id, v)) if *id == view.pane => {
                    v.scroll(&pane.history(), wheel);
                    pane.scroll_to_row(v.top);
                    (true, None)
                }
                _ => (false, None),
            };
        }
        match m.action {
            MouseAction::Press => {
                let now = Instant::now();
                let count = match *click {
                    Some(c) if c.pane == view.pane && c.pos == pos && now.duration_since(c.at) < MULTI_CLICK => {
                        c.count % 3 + 1
                    }
                    _ => 1,
                };
                *click = Some(Click { at: now, pane: view.pane, pos, count });
                let text = match count {
                    2 => pane.history().0.word_at(pos.x, pos.y),
                    3 => pane.history().0.line_at(pos.x, pos.y),
                    _ => {
                        *next_drag = Some(Drag { pane: view.pane, from: pos, moved: false });
                        return (true, None);
                    }
                };
                (true, text.map(Copy::Text))
            }
            MouseAction::Motion => {
                let Some(mut d) = drag else { return (true, None) };
                if !d.moved && pos == d.from {
                    *next_drag = Some(d);
                    return (true, None);
                }
                d.moved = true;
                *next_drag = Some(d);
                let v = match visual {
                    Some((id, v)) if *id == d.pane => v,
                    _ => &mut visual.insert((d.pane, Visual::new(pos, vp.top, vp.rows))).1,
                };
                v.drag(d.from, pos);
                pane.scroll_to_row(v.top);
                (true, None)
            }
            MouseAction::Release => {
                let Some(d) = drag.filter(|d| d.moved) else { return (true, None) };
                let sel = match visual.take() {
                    Some((id, v)) if id == d.pane => v.selection(),
                    other => {
                        *visual = other;
                        None
                    }
                };
                (true, sel.map(|s| Copy::Selection(d.pane, s)))
            }
        }
    }

    /// Leaves the pane scrolled to the bottom, as leaving visual mode does.
    pub(super) fn copy_selection(&mut self, i: usize, id: PaneId, sel: Selection) {
        let Some(session) = self.session_of(i) else { return };
        let Some(pane) = session.tab_mut().panes.get_mut(&id) else { return };
        let text = pane.history().0.text_between(
            (sel.start.x, sel.start.y),
            (sel.end.x, sel.end.y),
            sel.shape == Shape::Block,
        );
        pane.scroll_to_bottom();
        match text.filter(|t| !t.is_empty()) {
            Some(t) => self.copy_text(i, &t),
            None => session.status.say("nothing to copy"),
        }
    }
}
