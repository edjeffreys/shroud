//! A plugin-drawn status bar. Never waited on: frames draw the last bar sent, else the built-in one.

use serde_json::{Value, json};
use shroud_core::protocol::api::{self, Event, SpanSpec};
use shroud_core::render::Shape;
use shroud_core::statusbar::{Badge, Mode, Status};

use super::Server;
use super::client_conn::{Bar, ClientConn};
use super::control_conn::ControlConn;
use super::overlays::span;

pub fn state(s: &Status, width: u16) -> Value {
    let badge = |b: Badge| match b {
        Badge::Failed => "failed",
        Badge::Done => "done",
    };
    let mode = mode_name(s.mode);
    let selection = match s.mode {
        Mode::Visual(Some(Shape::Chars)) => Some("chars"),
        Mode::Visual(Some(Shape::Lines)) => Some("lines"),
        Mode::Visual(Some(Shape::Block)) => Some("block"),
        _ => None,
    };
    let tabs: Vec<Value> =
        s.tabs.iter().map(|t| json!({"name": t.name, "active": t.active, "badge": t.badge.map(badge)})).collect();
    let badges: Vec<Value> = s.badges.iter().map(|b| json!({"pane": b.pane, "badge": badge(b.badge)})).collect();
    json!({
        "session": s.session, "mode": mode, "selection": selection, "zoomed": s.zoomed, "broadcast": s.broadcast, "tabs": tabs, "message": s.message,
        "alert": s.alert, "clock": s.clock, "cwd": s.cwd, "branch": s.branch, "badges": badges, "width": width,
    })
}

pub fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Normal => "normal",
        Mode::Leader => "leader",
        Mode::Jump => "jump",
        Mode::Visual(_) => "visual",
    }
}

/// Called from render, which already has the state, so it's never built twice.
pub fn tell_plugin(c: &mut ClientConn, controls: &mut [ControlConn], owner: u64, state: Value) {
    if c.bar_state.as_ref() == Some(&state) {
        return;
    }
    let line = api::event(&Event::Status { client: c.id, state: state.clone() });
    c.bar_state = Some(state);
    if let Some(ctl) = controls.iter_mut().find(|x| x.id == owner) {
        ctl.send(&line);
    }
}

impl Server {
    pub(super) fn take_status(&mut self, owner: u64) -> Result<Value, String> {
        if self.status_owner.is_some_and(|o| o != owner) {
            return Err("another connection already draws the status bar".into());
        }
        self.status_owner = Some(owner);
        self.forget_bars();
        Ok(json!({}))
    }

    pub(super) fn release_status(&mut self, owner: u64) {
        if self.status_owner == Some(owner) {
            self.status_owner = None;
            self.forget_bars();
        }
    }

    pub(super) fn set_status(
        &mut self,
        owner: u64,
        client: u64,
        left: Vec<SpanSpec>,
        right: Vec<SpanSpec>,
        fill: Option<String>,
    ) -> Result<Value, String> {
        if self.status_owner != Some(owner) {
            return Err("take the status bar with status_take first".into());
        }
        let c = self.clients.iter_mut().find(|c| c.id == client).ok_or_else(|| format!("no client {client}"))?;
        c.bar = Some(Bar {
            left: left.into_iter().map(span).collect(),
            right: right.into_iter().map(span).collect(),
            fill,
        });
        c.needs_frame = true;
        Ok(json!({}))
    }

    fn forget_bars(&mut self) {
        for c in &mut self.clients {
            c.bar = None;
            c.bar_state = None;
        }
        self.invalidate();
    }
}
