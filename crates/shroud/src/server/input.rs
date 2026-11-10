//! Routes decoded client input: the switcher if open, then keybinds, then the focused pane or
//! the pane under the mouse.

use shroud_core::input::{Action, Event, MouseAction, MouseButton, MouseEvent, Outcome};
use shroud_core::layout::PaneId;
use shroud_core::protocol::{ExitReason, ServerMsg};
use shroud_core::render::passthrough;
use shroud_core::switcher::Switcher;

use super::Server;
use super::session::Session;

const WHEEL_LINES: isize = 3;

impl Server {
    pub(super) fn event(&mut self, i: usize, ev: Event) {
        if let (Event::Key(k), Some(_)) = (&ev, &self.clients[i].switcher) {
            self.switcher_key(i, k);
            self.invalidate();
            return;
        }
        if let Event::Key(k) = &ev
            && (self.modal_key(i, k) || self.visual_key(i, k))
        {
            self.invalidate();
            return;
        }
        match ev {
            Event::Key(k) => {
                let quiet_before = !self.clients[i].keybinds.is_pending() && !self.clients[i].pending_reported;
                match self.clients[i].keybinds.handle(&k, &self.settings.bindings) {
                    // A held command pane takes Enter and q before its shell does.
                    Outcome::Passthrough if self.held_key(i, &k) => self.reap(),
                    Outcome::Passthrough => {
                        let mut moved = true;
                        self.with_focused(i, |p| moved = p.key(&k));
                        if quiet_before && !moved {
                            return;
                        }
                    }
                    Outcome::Action(a) => self.action(i, a),
                    Outcome::Plugin { name, count } => self.run_plugin_command(i, name, count),
                    Outcome::Shortcut(s) => self.run_shortcut(i, s),
                    Outcome::Pending | Outcome::Consumed => {}
                }
                if i < self.clients.len() {
                    if !self.clients[i].keybinds.is_jumping() {
                        self.clients[i].jump = None;
                    }
                    self.key_sequence_moved(i);
                }
            }
            Event::Mouse(m) => self.mouse(i, m),
            Event::Paste(data) => self.with_focused(i, |p| p.paste(&data)),
            Event::Focus(gained) => self.with_focused(i, |p| p.focus(gained)),
            Event::ColorScheme { dark } => self.color_scheme_changed(i, dark),
            Event::OuterColor { slot, rgb } => self.outer_color(slot, rgb),
            Event::GraphemeClustering(on) => self.outer_graphemes(on),
        }
        self.invalidate();
    }

    fn with_focused(&mut self, i: usize, f: impl FnOnce(&mut super::pane::Pane)) {
        if let Some(p) = self.session_of(i).and_then(|s| s.tab_mut().focused_pane()) {
            f(p);
        }
    }

    pub(super) fn action(&mut self, i: usize, action: Action) {
        let Some(session) = self.session_of(i) else { return };
        let before = session.tab().focused;
        match action {
            Action::Detach => self.clients[i].exit(ExitReason::Detached),
            Action::SendLeader => {
                let leader = self.settings.bindings.leader.to_event();
                self.with_focused(i, |p| {
                    p.key(&leader);
                });
            }
            Action::JumpPrompt(n) => self.jump(i, n),
            Action::LeaveJump => self.leave_jump(i),
            Action::CopyJumped { command } => self.copy_jumped(i, command),
            Action::VisualJumped => self.visual_jumped(i),
            Action::CopyLastOutput => self.copy_last(i, false),
            Action::CopyLastCommand => self.copy_last(i, true),
            Action::ToggleFrameTime => {
                self.frame_time = if self.frame_time.is_some() { None } else { Some(String::new()) }
            }
            Action::OpenSwitcher => self.open_switcher(i),
            Action::NewSession => self.clients[i].switcher = Some(Switcher::create()),
            Action::RenameSession => {
                let name = session.name.clone();
                self.clients[i].switcher = Some(Switcher::rename(&name));
            }
            Action::LastSession => {
                if !self.last_session(i)
                    && let Some(s) = self.session_of(i)
                {
                    s.status.say("no other session");
                }
            }
            Action::RenameTab => {
                let name = session.tab().name.clone();
                self.clients[i].switcher = Some(Switcher::rename_tab(name.as_deref()));
            }
            Action::LastTab => {
                if !session.last_tab() {
                    session.status.say("no other tab yet");
                }
            }
            Action::Visual(s) => self.enter_visual(i, s),
            Action::ToggleZoom => session.tab_mut().toggle_zoom(),
            Action::FloatOrTile => {
                if let Err(why) = session.tab_mut().float_or_tile() {
                    session.status.say(why);
                }
            }
            Action::NextSession => self.step_session(i, 1),
            Action::PrevSession => self.step_session(i, -1),
            Action::NewTab => {
                if let Err(e) = session.new_tab() {
                    session.status.say(format!("new tab: {e}"));
                }
            }
            Action::MoveToNewTab => {
                if !session.move_to_new_tab() {
                    session.status.say("only one pane in this tab");
                }
            }
            Action::NextTab => session.step(1),
            Action::PrevTab => session.step(-1),
            Action::GotoTab(n) => session.goto(n as usize - 1),
            other => {
                session.tab_mut().apply(other);
            }
        }
        self.reap();
        self.focus_changed(i, before);
    }

    /// With `command`, the command line comes first, as `$ cmd`, then whatever it printed. A
    /// shell that doesn't mark its input (OSC 133 B) gives just the output.
    fn copy_last(&mut self, i: usize, command: bool) {
        let Some(session) = self.session_of(i) else { return };
        let pane = session.tab_mut().focused_pane();
        let text = match (command, pane) {
            (false, Some(p)) => p.last_output(),
            (true, Some(p)) => match (p.last_command(), p.last_output()) {
                (Some(cmd), Some(out)) => Some(format!("$ {cmd}\n{out}")),
                (Some(cmd), None) => Some(format!("$ {cmd}")),
                (None, out) => out,
            },
            (_, None) => None,
        };
        let Some(text) = text else {
            session.status.say(if command { "nothing to copy" } else { "no output to copy" });
            return;
        };
        self.copy_text(i, &text);
    }

    /// Through the client's terminal (OSC 52), so it lands on the clipboard of the machine
    /// the user is sitting at, even over ssh.
    pub(super) fn copy_text(&mut self, i: usize, text: &str) {
        let lines = text.lines().count();
        if let Some(session) = self.session_of(i) {
            session.status.say(format!("copied {lines} line{}", if lines == 1 { "" } else { "s" }));
        }
        let mut osc = Vec::new();
        passthrough::clipboard(&mut osc, text.as_bytes());
        self.clients[i].send(ServerMsg::Output(&osc));
    }

    /// The pane that lost focus may be in another tab.
    pub(super) fn focus_changed(&mut self, i: usize, before: PaneId) {
        let Some(session) = self.session_of(i) else { return };
        let now = session.tab().focused;
        if now == before {
            return;
        }
        if let Some((p, _)) = session.pane_mut(before) {
            p.focus(false);
        }
        if let Some((p, _)) = session.pane_mut(now) {
            p.focus(true);
        }
    }

    fn mouse(&mut self, i: usize, m: MouseEvent) {
        let owner = self.mouse_owner;
        let Some(session) = self.session_of(i) else { return };
        let before = session.tab().focused;
        if self.select_mouse(i, m) {
            self.invalidate();
        } else if let Some(session) = self.session_of(i) {
            self.mouse_owner = mouse_in(session, owner, m);
        }
        self.focus_changed(i, before);
    }
}

/// A drag stays with the pane it started in until release.
fn mouse_in(session: &mut Session, owner: Option<PaneId>, m: MouseEvent) -> Option<PaneId> {
    let tab = session.tab_mut();
    let views = tab.views();
    // Topmost first, so a float takes clicks over the tiles beneath it.
    let under = views.iter().rev().find(|v| v.float.unwrap_or(v.rect).contains(m.x, m.y));
    let target = match (m.action, owner) {
        (MouseAction::Motion | MouseAction::Release, Some(o)) => views.iter().find(|v| v.pane == o),
        _ => under,
    };
    let Some(target) = target.copied() else { return owner };
    let mut owner = owner;
    if m.action == MouseAction::Press && !is_wheel(m.button) {
        tab.focus_pane(target.pane);
        owner = Some(target.pane);
    }
    if m.action == MouseAction::Release {
        owner = None;
    }
    let local = MouseEvent {
        x: m.x.saturating_sub(target.rect.x).min(target.rect.w.saturating_sub(1)),
        y: m.y.saturating_sub(target.rect.y).min(target.rect.h.saturating_sub(1)),
        ..m
    };
    if let Some(p) = tab.panes.get_mut(&target.pane)
        && !p.mouse(&local)
        && m.action == MouseAction::Press
    {
        match m.button {
            Some(MouseButton::WheelUp) => p.scroll(-WHEEL_LINES),
            Some(MouseButton::WheelDown) => p.scroll(WHEEL_LINES),
            _ => {}
        }
    }
    owner
}

fn is_wheel(b: Option<MouseButton>) -> bool {
    matches!(b, Some(MouseButton::WheelUp | MouseButton::WheelDown | MouseButton::WheelLeft | MouseButton::WheelRight))
}
