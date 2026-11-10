//! State events come from comparing snapshots each loop, like pane events, so every way a
//! thing can change is covered by one rule. Nothing is computed while nobody listens.

use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};

use shroud_core::protocol::api::Event;

use super::Server;
use super::plugin_bar::mode_name;
use super::session::SessionId;

// Long enough that a fluent sequence never flashes a hint.
pub(super) const KEY_PENDING_AFTER: Duration = Duration::from_millis(300);

#[derive(Debug, Default, PartialEq)]
pub struct Watched {
    sessions: BTreeMap<SessionId, SessionState>,
    clients: BTreeMap<u64, SessionId>,
    modes: BTreeMap<u64, &'static str>,
}

#[derive(Debug, PartialEq, Clone)]
struct SessionState {
    name: String,
    focused: u32,
    tab: usize,
    layouts: Vec<u64>,
}

impl Server {
    pub(super) fn watch_changes(&mut self) {
        if !self.controls.iter().any(|c| c.subscribed) {
            self.watched = None;
            return;
        }
        let now = self.snapshot_state();
        let Some(before) = self.watched.replace(now) else { return };
        let now = self.watched.as_ref().expect("just set");
        let mut events = Vec::new();
        for (id, old) in &before.sessions {
            match now.sessions.get(id) {
                None => events.push(Event::SessionClosed { session: old.name.clone() }),
                Some(new) => diff_session(old, new, &mut events),
            }
        }
        for (id, new) in &now.sessions {
            if !before.sessions.contains_key(id) {
                events.push(Event::SessionOpened { session: new.name.clone() });
            }
        }
        for (client, session) in &now.clients {
            if before.clients.get(client) != Some(session)
                && let Some(s) = now.sessions.get(session)
            {
                events.push(Event::ClientAttached { client: *client, session: s.name.clone() });
            }
        }
        for (client, mode) in &now.modes {
            if before.modes.get(client).is_some_and(|m| m != mode) {
                events.push(Event::ModeChanged { client: *client, mode: mode.to_string() });
            }
        }
        for client in before.clients.keys().filter(|c| !now.clients.contains_key(c)) {
            events.push(Event::ClientDetached { client: *client });
        }
        for e in &events {
            self.broadcast(e);
        }
    }

    pub(super) fn key_sequence_moved(&mut self, client: usize) {
        let c = &mut self.clients[client];
        if c.keybinds.is_pending() {
            // Once hints are showing they follow each key, rather than vanishing for 300 ms.
            let now = Instant::now();
            c.pending_since =
                Some(if c.pending_reported { now.checked_sub(KEY_PENDING_AFTER).unwrap_or(now) } else { now });
            c.pending_reported = false;
            return;
        }
        let was_reported = c.pending_reported;
        c.pending_since = None;
        c.pending_reported = false;
        if was_reported {
            let id = c.id;
            self.broadcast(&Event::KeyDone { client: id });
        }
    }

    pub(super) fn report_pending_keys(&mut self) {
        let now = Instant::now();
        let leader = self.settings.bindings.leader.to_string();
        let mut events = Vec::new();
        for c in self.clients.iter_mut() {
            let due = c.pending_since.is_some_and(|t| now >= t + KEY_PENDING_AFTER) && !c.pending_reported;
            let (Some(keys), true) = (c.keybinds.pending_keys(), due) else { continue };
            let Some(session) = c.session.and_then(|id| self.sessions.iter().find(|s| s.id == id)) else { continue };
            c.pending_reported = true;
            let keys = std::iter::once(leader.clone()).chain(keys.iter().map(|k| k.to_string())).collect();
            events.push(Event::KeyPending { client: c.id, session: session.name.clone(), keys });
        }
        for e in &events {
            self.broadcast(e);
        }
    }

    pub(super) fn key_pending_deadline(&self) -> Option<Instant> {
        self.clients
            .iter()
            .filter(|c| !c.pending_reported)
            .filter_map(|c| c.pending_since)
            .map(|t| t + KEY_PENDING_AFTER)
            .min()
    }

    fn snapshot_state(&self) -> Watched {
        let sessions = self
            .sessions
            .iter()
            .map(|s| {
                let layouts = s
                    .tabs
                    .iter()
                    .map(|t| {
                        let mut h = DefaultHasher::new();
                        for v in t.views() {
                            (v.pane, v.rect.x, v.rect.y, v.rect.w, v.rect.h, v.float.is_some()).hash(&mut h);
                        }
                        h.finish()
                    })
                    .collect();
                (s.id, SessionState { name: s.name.clone(), focused: s.tab().focused, tab: s.active, layouts })
            })
            .collect();
        let clients = self.clients.iter().filter_map(|c| Some((c.id, c.session?))).collect();
        let modes = self.clients.iter().filter(|c| c.session.is_some()).map(|c| (c.id, mode_name(c.mode()))).collect();
        Watched { sessions, clients, modes }
    }
}

fn diff_session(old: &SessionState, new: &SessionState, events: &mut Vec<Event>) {
    if old.name != new.name {
        events.push(Event::SessionRenamed { from: old.name.clone(), to: new.name.clone() });
    }
    let session = new.name.clone();
    if old.tab != new.tab {
        events.push(Event::TabChanged { session: session.clone(), tab: new.tab + 1 });
    }
    if old.focused != new.focused {
        events.push(Event::FocusChanged { session: session.clone(), pane: new.focused });
    }
    for (i, sig) in new.layouts.iter().enumerate() {
        if old.layouts.get(i).is_some_and(|o| o != sig) {
            events.push(Event::LayoutChanged { session: session.clone(), tab: i + 1 });
        }
    }
}
