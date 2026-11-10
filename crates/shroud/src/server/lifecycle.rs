//! The CLI, the switcher and leader keys all come through here, so they behave the same.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use shroud_core::protocol::{ExitReason, ServerMsg, Size, VERSION};

use super::client_conn::{Request, Want};
use super::session::{Session, SessionId};
use super::{CONFIG_MESSAGE_FOR, Server};
use crate::{config_file, layout_child};

const MAX_NAME: usize = 64;

impl Server {
    pub(super) fn hello(&mut self, i: usize, size: Size, version: u16, want: Want) {
        if version != VERSION {
            self.clients[i].exit(ExitReason::VersionMismatch { server: VERSION });
            return;
        }
        self.clients[i].send(ServerMsg::Welcome { version: VERSION });
        self.query_outer(i);
        self.clients[i].size = Some(size);
        let target = match want {
            Want::Recent => match self.sessions.iter().max_by_key(|s| s.last_used) {
                Some(s) => Ok(s.id),
                None => self.create("default", &home(), size),
            },
            Want::Named(name) => self
                .sessions
                .iter()
                .find(|s| s.name == name)
                .map(|s| s.id)
                .ok_or_else(|| format!("no session named {name}")),
            Want::New { name, cwd } => {
                let name = if name.is_empty() { self.free_name() } else { name };
                let cwd = if cwd.is_empty() { home() } else { PathBuf::from(cwd) };
                self.create(&name, &cwd, size)
            }
        };
        match target {
            Ok(id) => self.attach(i, id),
            Err(why) => self.refuse(i, &why),
        }
    }

    pub(super) fn control(&mut self, i: usize, version: u16, request: Request) {
        if version != VERSION {
            self.clients[i].exit(ExitReason::VersionMismatch { server: VERSION });
            return;
        }
        let reply = match request {
            Request::List => Ok(self.listing()),
            Request::Kill(name) => self.id_of(&name).map(|id| {
                self.kill(id);
                format!("closed {name}")
            }),
            Request::Rename { from, to } => {
                self.id_of(&from).and_then(|id| self.rename(id, &to)).map(|()| format!("renamed {from} to {to}"))
            }
            Request::StopServer { restart } => {
                self.done = Some(if restart { ExitReason::Restart } else { ExitReason::ServerShutdown });
                Ok(if restart { "restarting the server" } else { "stopping the server" }.to_string())
            }
        };
        let (ok, text) = match &reply {
            Ok(t) => (true, t.as_str()),
            Err(t) => (false, t.as_str()),
        };
        self.clients[i].send(ServerMsg::Reply { ok, text });
        self.clients[i].exit(if ok { ExitReason::Detached } else { ExitReason::Refused });
    }

    pub(super) fn attach(&mut self, i: usize, id: SessionId) {
        if let Some(s) = self.sessions.iter_mut().find(|s| s.id == id) {
            s.last_used = Instant::now();
        }
        let c = &mut self.clients[i];
        c.session = Some(id);
        c.switcher = None;
        c.prev = None;
        self.resize_sessions();
    }

    pub(super) fn create(&mut self, name: &str, cwd: &Path, size: Size) -> Result<SessionId, String> {
        let name = valid_name(name)?;
        if self.id_of(&name).is_ok() {
            return Err(format!("a session named {name} already exists"));
        }
        let cwd = if cwd.is_dir() { cwd.to_path_buf() } else { home() };
        let id = self.next_session;
        let mut note = self.config_note();
        let layout = match config_file::layout_for(&cwd) {
            None => Vec::new(),
            Some((path, text)) => layout_child::parse(&text).unwrap_or_else(|p| {
                note = Some(format!("{}: {p}", path.display()));
                Vec::new()
            }),
        };
        let session = match Session::with_layout(id, name.clone(), &cwd, size, self.launch.clone(), &layout) {
            Ok(s) => s,
            Err(e) if !layout.is_empty() => {
                note = Some(format!("layout: {e}"));
                Session::new(id, name, &cwd, size, self.launch.clone()).map_err(|e: io::Error| e.to_string())?
            }
            Err(e) => return Err(e.to_string()),
        };
        self.sessions.push(session);
        if let (Some(n), Some(s)) = (note, self.sessions.last_mut()) {
            s.status.say_for(n, CONFIG_MESSAGE_FOR);
        }
        self.next_session += 1;
        Ok(id)
    }

    pub(super) fn rename(&mut self, id: SessionId, to: &str) -> Result<(), String> {
        let to = valid_name(to)?;
        if self.sessions.iter().any(|s| s.name == to && s.id != id) {
            return Err(format!("a session named {to} already exists"));
        }
        if let Some(s) = self.sessions.iter_mut().find(|s| s.id == id) {
            s.name = to;
        }
        self.invalidate();
        Ok(())
    }

    /// Dropping a session hangs up its shells.
    pub(super) fn kill(&mut self, id: SessionId) {
        self.sessions.retain(|s| s.id != id);
        self.sessions_ended(&[id]);
    }

    pub(super) fn sessions_ended(&mut self, ended: &[SessionId]) {
        let fallback = self.sessions.iter().max_by_key(|s| s.last_used).map(|s| s.id);
        for i in 0..self.clients.len() {
            if self.clients[i].session.is_some_and(|id| ended.contains(&id)) {
                match fallback {
                    Some(id) => self.attach(i, id),
                    None => {
                        self.clients[i].session = None;
                        self.clients[i].exit(ExitReason::LastPaneClosed);
                    }
                }
            }
        }
        if self.sessions.is_empty() {
            self.done = Some(ExitReason::LastPaneClosed);
        }
        self.resize_sessions();
    }

    /// Creation order, so `(` and `)` cycle predictably.
    pub(super) fn step_session(&mut self, i: usize, by: isize) {
        let Some(current) = self.clients[i].session else { return };
        let Some(at) = self.sessions.iter().position(|s| s.id == current) else { return };
        let n = self.sessions.len() as isize;
        let next = self.sessions[(at as isize + by).rem_euclid(n) as usize].id;
        if next != current {
            self.attach(i, next);
        }
    }

    /// The most recently entered other session.
    pub(super) fn last_session(&mut self, i: usize) -> bool {
        let current = self.clients[i].session;
        let last = self.sessions.iter().filter(|s| Some(s.id) != current).max_by_key(|s| s.last_used).map(|s| s.id);
        last.inspect(|&id| self.attach(i, id)).is_some()
    }

    pub(super) fn id_of(&self, name: &str) -> Result<SessionId, String> {
        self.sessions.iter().find(|s| s.name == name).map(|s| s.id).ok_or_else(|| format!("no session named {name}"))
    }

    fn refuse(&mut self, i: usize, why: &str) {
        self.clients[i].send(ServerMsg::Reply { ok: false, text: why });
        self.clients[i].exit(ExitReason::Refused);
    }

    fn listing(&self) -> String {
        let mut by_recent: Vec<&Session> = self.sessions.iter().collect();
        by_recent.sort_by_key(|s| std::cmp::Reverse(s.last_used));
        by_recent
            .iter()
            .map(|s| {
                let clients = self.clients.iter().filter(|c| c.session == Some(s.id)).count();
                let tabs = s.tabs.len();
                let attached = if clients > 0 { " (attached)" } else { "" };
                format!("{}: {tabs} tab{}{attached}", s.name, if tabs == 1 { "" } else { "s" })
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `default` first, then numbers, as tmux numbers unnamed sessions.
    pub(super) fn free_name(&self) -> String {
        let taken = |n: &str| self.sessions.iter().any(|s| s.name == n);
        if !taken("default") {
            return "default".into();
        }
        (1..).map(|n: u32| n.to_string()).find(|n| !taken(n)).unwrap_or_default()
    }
}

fn valid_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a session needs a name".into());
    }
    if name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(format!("session names are up to {MAX_NAME} printable characters"));
    }
    Ok(name.to_string())
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}
