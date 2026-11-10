//! Visual mode belongs to a client but scrolls a pane, which every client of the session sees.

use shroud_core::input::{Chord, KeyAction, KeyEvent, Selecting};
use shroud_core::render::Shape;
use shroud_core::visual::{self, Outcome, Pos, Text, Visual};
use shroud_vt::Terminal;

use super::Server;

pub struct History<'a>(pub &'a Terminal);

impl Text for History<'_> {
    fn rows(&self) -> u32 {
        self.0.viewport().total
    }

    fn cols(&self) -> u16 {
        self.0.viewport().cols
    }

    fn line(&self, y: u32) -> Vec<char> {
        self.0.history_line(y)
    }

    fn is_prompt(&self, y: u32) -> bool {
        self.0.is_prompt_row(y)
    }

    fn all_lines(&self) -> Vec<Vec<char>> {
        self.0.history_rows().iter().map(|r| visual::columns(r)).collect()
    }
}

impl Server {
    pub(super) fn enter_visual(&mut self, i: usize, selecting: Selecting) {
        let Some(session) = self.session_of(i) else { return };
        let tab = session.tab();
        let id = tab.focused;
        let Some(pane) = tab.panes.get(&id) else { return };
        let vp = pane.viewport();
        let bottom = vp.top + vp.rows as u32;
        let cursor = match pane.history().0.cursor_in_history() {
            Some((x, y)) if (vp.top..bottom).contains(&y) => Pos { x, y },
            _ => Pos { x: 0, y: bottom.saturating_sub(1) },
        };
        let mut v = Visual::new(cursor, vp.top, vp.rows);
        match selecting {
            Selecting::Nothing => {}
            Selecting::Chars => v.select(Shape::Chars),
            Selecting::Lines => v.select(Shape::Lines),
            Selecting::Block => v.select(Shape::Block),
        }
        self.clients[i].visual = Some((id, v));
    }

    /// True when visual mode took the key. The leader leaves it, so window commands still work.
    pub(super) fn visual_key(&mut self, i: usize, k: &KeyEvent) -> bool {
        if self.clients[i].visual.is_none() {
            return false;
        }
        if k.action == KeyAction::Release {
            // Keybinds swallowed the entering key and must see its release.
            let _ = self.clients[i].keybinds.handle(k, &self.settings.bindings);
            return true;
        }
        if Chord::from_event(k) == Some(self.settings.bindings.leader) {
            self.leave_visual(i);
            return false;
        }
        let Some((id, mut v)) = self.clients[i].visual.take() else { return false };
        let Some(session) = self.session_of(i) else { return true };
        let Some(pane) = session.tab_mut().panes.get_mut(&id) else { return true };
        let outcome = v.key(k, &pane.history());
        match outcome {
            Outcome::Stay => {
                pane.scroll_to_row(v.top);
                self.clients[i].visual = Some((id, v));
            }
            Outcome::Say(m) => {
                pane.scroll_to_row(v.top);
                session.status.say(m);
                self.clients[i].visual = Some((id, v));
            }
            Outcome::Exit => pane.scroll_to_bottom(),
            Outcome::Copy(sel) => self.copy_selection(i, id, sel),
        }
        true
    }

    pub(super) fn leave_visual(&mut self, i: usize) {
        let Some((id, _)) = self.clients[i].visual.take() else { return };
        if let Some(pane) = self.session_of(i).and_then(|s| s.tab_mut().panes.get_mut(&id)) {
            pane.scroll_to_bottom();
        }
    }
}
