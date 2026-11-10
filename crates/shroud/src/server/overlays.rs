//! Overlays belong to the control connection that opened them and vanish with it, so a
//! plugin that dies can't leave a box on screen.

use shroud_core::input::{Chord, KeyAction, KeyEvent, Mods, key};
use shroud_core::overlay::{Anchor, Overlay, Span};
use shroud_core::protocol::api::{self, AnchorSpec, Event, LineSpec, OverlaySpec, SpanSpec};

use super::Server;
use super::session::SessionId;

#[derive(Debug)]
pub struct Placed {
    pub id: u32,
    pub owner: u64,
    pub session: SessionId,
    pub client: Option<u64>,
    pub overlay: Overlay,
}

impl Placed {
    pub fn shown_to(&self, session: SessionId, client: u64) -> bool {
        self.session == session && self.client.is_none_or(|c| c == client)
    }
}

impl Server {
    pub(super) fn open_overlay(
        &mut self,
        owner: u64,
        session: Option<&str>,
        client: Option<u64>,
        spec: OverlaySpec,
    ) -> Result<u32, String> {
        let session = match session {
            Some(name) => {
                self.sessions.iter().find(|s| s.name == name).ok_or_else(|| format!("no session named {name}"))?
            }
            None => self.sessions.iter().max_by_key(|s| s.last_used).ok_or("no sessions")?,
        }
        .id;
        self.next_overlay += 1;
        self.overlays.push(Placed { id: self.next_overlay, owner, session, client, overlay: overlay(spec) });
        Ok(self.next_overlay)
    }

    pub(super) fn update_overlay(&mut self, owner: u64, id: u32, spec: OverlaySpec) -> Result<(), String> {
        let p = self.owned(owner, id)?;
        p.overlay = overlay(spec);
        Ok(())
    }

    pub(super) fn close_overlay(&mut self, owner: u64, id: u32) -> Result<(), String> {
        self.owned(owner, id)?;
        self.overlays.retain(|p| p.id != id);
        Ok(())
    }

    pub(super) fn drop_overlays_of(&mut self, owner: u64) {
        let before = self.overlays.len();
        self.overlays.retain(|p| p.owner != owner);
        if self.overlays.len() != before {
            self.invalidate();
        }
    }

    // Escape always closes, so a broken plugin can't hold the keyboard.
    pub(super) fn modal_key(&mut self, client: usize, k: &KeyEvent) -> bool {
        let Some(session) = self.clients[client].session else { return false };
        let me = self.clients[client].id;
        let Some(i) = self.overlays.iter().rposition(|p| p.shown_to(session, me) && p.overlay.modal) else {
            return false;
        };
        if k.action == KeyAction::Release {
            return true;
        }
        let (id, owner) = (self.overlays[i].id, self.overlays[i].owner);
        let event = if k.code == key::ESCAPE && k.mods == Mods::NONE {
            self.overlays.remove(i);
            Event::OverlayClosed { overlay: id }
        } else {
            let Some(chord) = Chord::from_event(k) else { return true };
            Event::OverlayKey { overlay: id, key: chord.to_string() }
        };
        if let Some(c) = self.controls.iter_mut().find(|c| c.id == owner) {
            c.send(&api::event(&event));
        }
        true
    }

    fn owned(&mut self, owner: u64, id: u32) -> Result<&mut Placed, String> {
        match self.overlays.iter_mut().find(|p| p.id == id) {
            Some(p) if p.owner == owner => Ok(p),
            Some(_) => Err(format!("overlay {id} belongs to another connection")),
            None => Err(format!("no overlay {id}")),
        }
    }
}

pub(super) fn span(s: SpanSpec) -> Span {
    Span { text: s.text, style: s.style, bg: s.bg, bold: s.bold, italic: s.italic }
}

fn overlay(spec: OverlaySpec) -> Overlay {
    let lines = spec
        .lines
        .into_iter()
        .map(|l| match l {
            LineSpec::Text(text) => vec![Span { text, ..Span::default() }],
            LineSpec::Spans(spans) => spans.into_iter().map(span).collect(),
        })
        .collect();
    let anchor = match spec.anchor {
        AnchorSpec::Centre => Anchor::Centre,
        AnchorSpec::TopLeft => Anchor::TopLeft,
        AnchorSpec::TopRight => Anchor::TopRight,
        AnchorSpec::BottomLeft => Anchor::BottomLeft,
        AnchorSpec::BottomRight => Anchor::BottomRight,
        AnchorSpec::Cursor => Anchor::Cursor,
    };
    Overlay { title: spec.title, lines, anchor, modal: spec.modal, ..Overlay::default() }
}
