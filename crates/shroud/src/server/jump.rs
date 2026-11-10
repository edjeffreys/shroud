//! Jump mode's selected command belongs to a client, as visual mode's cursor does, while the
//! scrolling it causes is the pane's.

use shroud_core::layout::PaneId;
use shroud_core::render::{Highlight, Shape};
use shroud_core::visual::{Pos, Visual};

use super::Server;

const LABEL_COMMAND: usize = 40;

#[derive(Debug)]
pub struct Jump {
    pub pane: PaneId,
    prompts: Vec<u32>,
    at: usize,
    prompt: u32,
    input: Option<((u16, u32), (u16, u32))>,
    pub label: String,
}

impl Jump {
    /// The command line only; without input marks, the prompt's row.
    pub fn highlight(&self, top: u32) -> Highlight {
        let local = |(x, y): (u16, u32)| (x, y as i64 - top as i64);
        match self.input {
            Some((from, to)) => Highlight { from: local(from), to: local(to), shape: Shape::Chars },
            None => Highlight { from: local((0, self.prompt)), to: local((0, self.prompt)), shape: Shape::Lines },
        }
    }
}

impl Server {
    /// Starts at the last finished command, so the first `[` picks what just happened.
    pub(super) fn jump(&mut self, i: usize, delta: i32) {
        let previous = self.clients[i].jump.take();
        let Some(session) = self.session_of(i) else { return };
        let id = session.tab().focused;
        let Some(pane) = session.tab_mut().panes.get_mut(&id) else { return };
        let mut j = match previous {
            Some(j) if j.pane == id => j,
            _ => {
                let t = pane.history().0;
                let Some(last) = t.last_finished_prompt() else {
                    session.status.say("no commands to jump to");
                    self.clients[i].keybinds.stop_jumping();
                    return;
                };
                let mut prompts = t.prompt_rows();
                prompts.retain(|&p| p <= last);
                Jump { pane: id, at: prompts.len(), prompts, prompt: last, input: None, label: String::new() }
            }
        };
        if j.prompts.is_empty() {
            self.clients[i].keybinds.stop_jumping();
            return;
        }
        j.at = (j.at as i64 + delta as i64).clamp(0, j.prompts.len() as i64 - 1) as usize;
        j.prompt = j.prompts[j.at];
        let t = pane.history().0;
        let rows = t.command_at(j.prompt);
        let command = t.command_text(&rows);
        j.input = rows.input;
        let mut label = format!("{}/{}", j.at + 1, j.prompts.len());
        if let Some(c) = &command {
            let short: String = c.chars().take(LABEL_COMMAND).collect();
            label += &format!("  {short}{}", if c.chars().count() > LABEL_COMMAND { "…" } else { "" });
        }
        match pane.exit_of(j.prompt, &command) {
            Some(0) => label += "  ✓",
            Some(code) => label += &format!("  ✗{code}"),
            None => {}
        }
        j.label = label;
        pane.scroll_to_row(j.prompt);
        self.clients[i].jump = Some(j);
    }

    pub(super) fn leave_jump(&mut self, i: usize) {
        let Some(j) = self.clients[i].jump.take() else { return };
        if let Some(pane) = self.session_of(i).and_then(|s| s.tab_mut().panes.get_mut(&j.pane)) {
            pane.scroll_to_bottom();
        }
    }

    pub(super) fn copy_jumped(&mut self, i: usize, command: bool) {
        let Some(j) = self.clients[i].jump.take() else { return };
        let Some(session) = self.session_of(i) else { return };
        let Some(pane) = session.tab_mut().panes.get_mut(&j.pane) else { return };
        let t = pane.history().0;
        let rows = t.command_at(j.prompt);
        let output = t.output_text(&rows);
        let text = match (command, t.command_text(&rows)) {
            (true, Some(cmd)) => Some(match output {
                Some(out) => format!("$ {cmd}\n{out}"),
                None => format!("$ {cmd}"),
            }),
            _ => output,
        };
        pane.scroll_to_bottom();
        match text {
            Some(t) => self.copy_text(i, &t),
            None => session.status.say("nothing to copy"),
        }
    }

    pub(super) fn visual_jumped(&mut self, i: usize) {
        let Some(j) = self.clients[i].jump.take() else { return };
        let Some(session) = self.session_of(i) else { return };
        let Some(pane) = session.tab_mut().panes.get_mut(&j.pane) else { return };
        let t = pane.history().0;
        let rows = t.command_at(j.prompt);
        let vp = pane.viewport();
        let mut v = Visual::new(Pos { x: 0, y: j.prompt }, vp.top, vp.rows);
        if let Some((from, to)) = rows.output {
            v.drag(Pos { x: 0, y: from }, Pos { x: vp.cols.saturating_sub(1), y: to });
        }
        pane.scroll_to_row(v.top);
        self.clients[i].visual = Some((j.pane, v));
    }
}
