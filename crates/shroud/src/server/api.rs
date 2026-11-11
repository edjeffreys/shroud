//! Requests on `control.sock` act through the same tab and pane code as keys do.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use shroud_core::input::Chord;
use shroud_core::input::commands::Command;
use shroud_core::layout::{Axis, FloatRect, PaneId};
use shroud_core::protocol::api::{self, Dir, Event, FloatSpec, Input, Request, Target, parse_request};
use shroud_core::protocol::tags;
use shroud_core::shell::Phase;

use super::Server;
use super::control_conn::{ControlConn, Wait};
use super::keymaps::target_command;

// A title only ever fills one border line.
const MAX_TITLE: usize = 200;

// For shells that finish a command but never draw another prompt.
pub(super) const WAIT_FOR_PROMPT: Duration = Duration::from_millis(500);
// Without OSC 133 the command-finished mark never comes, so a wait would hang forever.
// Generous because a themed prompt can take a second or two to start.
pub(super) const WAIT_FOR_INTEGRATION: Duration = Duration::from_secs(10);

type Located = (usize, usize, PaneId);

const MESSAGE_FOR: Duration = Duration::from_secs(4);

impl Server {
    pub(super) fn accept_control(&mut self) {
        while let Ok((stream, _)) = self.control_listener.accept() {
            match ControlConn::new(stream) {
                Ok(c) => self.controls.push(c),
                Err(e) => eprintln!("shroud: accept control: {e}"),
            }
        }
    }

    pub(super) fn read_control(&mut self, i: usize) -> Result<(), ()> {
        let mut lines = Vec::new();
        let eof = self.controls[i].read(&mut lines).map_err(|_| ())?;
        for line in lines {
            let (id, request) = parse_request(&line);
            match request {
                Ok(r) => self.request(i, id, r),
                Err(e) => self.controls[i].send(&api::error(id.as_ref(), &e)),
            }
        }
        self.invalidate();
        if eof {
            let _ = self.controls[i].flush();
            return Err(());
        }
        Ok(())
    }

    fn request(&mut self, i: usize, id: Option<Value>, request: Request) {
        let reply = match request {
            Request::List => Ok(self.list()),
            Request::Subscribe => {
                self.controls[i].subscribed = true;
                Ok(json!({}))
            }
            Request::Split { pane, dir, cwd, command, focus, wait, float, close_on_exit, keep } => {
                if (wait || close_on_exit || keep) && command.is_none() {
                    Err("wait, close_on_exit and keep need a command".into())
                } else if keep && close_on_exit {
                    Err("keep holds the pane when its command ends, and close_on_exit closes it: choose one".into())
                } else {
                    // `exit` returns the command's status, so `wait` still reports it.
                    let typed = command.clone().map(|c| if close_on_exit { format!("{c}; exit") } else { c });
                    let typed = if keep { None } else { typed };
                    let opened = self.split_for_api(pane, dir, cwd, typed.as_deref(), focus, float);
                    if let (Ok(new), true, Some(c)) = (&opened, keep, &command) {
                        self.make_command_pane(*new, c, true);
                    }
                    match opened {
                        Ok(new) if wait => {
                            self.controls[i].waits.push(Wait {
                                id,
                                pane: new,
                                started: Instant::now(),
                                finished: None,
                            });
                            return;
                        }
                        Ok(new) => Ok(json!({"pane": new})),
                        Err(e) => Err(e),
                    }
                }
            }
            Request::Send { pane, input } => self.send_input(pane, &input).map(|()| json!({})),
            Request::OverlayOpen { session, client, spec } => {
                let owner = self.controls[i].id;
                self.open_overlay(owner, session.as_deref(), client, spec).map(|id| json!({"overlay": id}))
            }
            Request::OverlayUpdate { overlay, spec } => {
                self.update_overlay(self.controls[i].id, overlay, spec).map(|()| json!({}))
            }
            Request::OverlayClose { overlay } => self.close_overlay(self.controls[i].id, overlay).map(|()| json!({})),
            Request::RegisterCommand { name, key } => self.register_command(self.controls[i].id, name, key),
            Request::Bind { keys, command, direct } => self.bind(self.controls[i].id, &keys, command, direct),
            Request::Bindings => Ok(self.bindings_json()),
            Request::Layout { session, tab } => self.layout_of(session.as_deref(), tab),
            Request::ApplyLayout { session, layout } => self.apply_layout(session.as_deref(), layout),
            Request::SetLayout { session, tab, layout } => self.set_layout(session.as_deref(), tab, layout),
            Request::SessionNew { name, cwd } => self.session_new(name, cwd),
            Request::SessionKill { session } => self.id_of(&session).map(|id| {
                self.kill(id);
                json!({})
            }),
            Request::SessionRename { session, to } => self
                .session_index(session.as_deref())
                .map(|si| self.sessions[si].id)
                .and_then(|id| self.rename(id, &to))
                .map(|()| json!({})),
            Request::TabNew { session } => self.tab_new(session.as_deref()),
            Request::Message { session, text } => self.session_index(session.as_deref()).map(|si| {
                let text: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(200).collect();
                self.sessions[si].status.say_for(text, MESSAGE_FOR);
                json!({})
            }),
            Request::PluginHello { token } => self.plugin_hello(self.controls[i].id, &token),
            Request::PluginRunning { plugin } => return self.plugin_running(self.controls[i].id, plugin),
            Request::PluginError { plugin, message } if self.plugin_host.is_host(self.controls[i].id) => {
                self.plugin_alert(format!("plugin {plugin}: {message}"));
                Ok(json!({}))
            }
            Request::PluginError { .. } => Err("only the plugin host reports plugin errors".into()),
            Request::StatusTake => self.take_status(self.controls[i].id),
            Request::StatusRelease => {
                self.release_status(self.controls[i].id);
                Ok(json!({}))
            }
            Request::StatusSet { client, left, right, fill } => {
                self.set_status(self.controls[i].id, client, left, right, fill)
            }
            Request::PluginsReload => {
                self.reload_plugins();
                Ok(json!({}))
            }
            Request::PluginStatus => Ok(self.plugin_status()),
            Request::Output { pane } => self.locate(pane).and_then(|(s, t, p)| {
                let pane = &self.sessions[s].tabs[t].panes[&p];
                pane.last_output()
                    .map(|text| json!({"text": text}))
                    .ok_or_else(|| format!("pane {p} has no finished command output"))
            }),
            Request::PaneTitle { pane, title } => self.locate(pane).map(|(s, t, p)| {
                let pane = self.sessions[s].tabs[t].panes.get_mut(&p).expect("located");
                pane.label = title.map(|t| t.chars().take(MAX_TITLE).collect());
                self.invalidate();
                json!({})
            }),
            Request::Tag { pane, key, value } => self.locate(pane).and_then(|(s, t, p)| {
                let pane = self.sessions[s].tabs[t].panes.get_mut(&p).expect("located");
                if pane.tags.get(&key) != value.as_ref() {
                    tags::set(&mut pane.tags, &key, value.clone())?;
                    self.broadcast(&Event::TagChanged { pane: p, key, value });
                }
                Ok(json!({}))
            }),
            Request::Capture { pane, history } => self.locate(pane).and_then(|(s, t, p)| {
                let pane = &self.sessions[s].tabs[t].panes[&p];
                pane.capture(history).map(|text| json!({"text": text})).ok_or_else(|| format!("pane {p} has no text"))
            }),
            Request::Run { command, client, session } => {
                self.run_command(command, client, session.as_deref()).map(|()| json!({}))
            }
            Request::Close { pane } => self.locate(pane).map(|(s, t, p)| {
                self.sessions[s].tabs[t].remove(p);
                self.reap();
                json!({})
            }),
            Request::Focus { pane } => self.locate(pane).map(|(s, t, p)| {
                let session = &mut self.sessions[s];
                session.goto(t);
                session.tab_mut().focus_pane(p);
                json!({})
            }),
        };
        let line = match reply {
            Ok(fields) => api::ok(id.as_ref(), fields),
            Err(e) => api::error(id.as_ref(), &e),
        };
        self.controls[i].send(&line);
    }

    /// No pane means the focused pane of the most recently used session.
    fn locate(&self, pane: Option<u32>) -> Result<Located, String> {
        match pane {
            Some(p) => self
                .sessions
                .iter()
                .enumerate()
                .find_map(|(s, session)| session.tabs.iter().position(|t| t.panes.contains_key(&p)).map(|t| (s, t, p)))
                .ok_or_else(|| format!("no pane {p}")),
            None => {
                let (s, session) =
                    self.sessions.iter().enumerate().max_by_key(|(_, s)| s.last_used).ok_or("no sessions")?;
                Ok((s, session.active, session.tab().focused))
            }
        }
    }

    fn run_command(&mut self, target: Target, client: Option<u64>, session: Option<&str>) -> Result<(), String> {
        let command = target_command(target)?.ok_or("`none` is for unbinding, not running")?;
        let i = match client {
            Some(id) => self.clients.iter().position(|c| c.id == id).ok_or_else(|| format!("no client {id}"))?,
            None => {
                let sid = self.sessions[self.session_index(session)?].id;
                self.clients
                    .iter()
                    .position(|c| c.session == Some(sid))
                    .ok_or("no client is attached to that session")?
            }
        };
        match command {
            Command::Builtin { build, count, .. } => self.action(i, build(count)),
            Command::Plugin(name) => self.run_plugin_command(i, name, None),
            Command::Shortcut(s) => self.run_shortcut(i, s),
        }
        Ok(())
    }

    fn split_for_api(
        &mut self,
        pane: Option<u32>,
        dir: Dir,
        cwd: Option<String>,
        command: Option<&str>,
        focus: bool,
        float: Option<FloatSpec>,
    ) -> Result<PaneId, String> {
        let (s, t, target) = self.locate(pane)?;
        let tab = &mut self.sessions[s].tabs[t];
        let cwd = match cwd {
            Some(c) if PathBuf::from(&c).is_dir() => Some(PathBuf::from(c)),
            Some(c) => return Err(format!("{c} is not a directory")),
            None => tab.cwd_of(target),
        };
        let new = match float {
            Some(f) => {
                let at = FloatRect::new(f.x, f.y, f.w, f.h)
                    .ok_or("a float must fit inside the tab: x + w and y + h at most 100")?;
                tab.new_float(at, cwd.as_deref(), focus).map_err(|e| format!("float: {e}"))?
            }
            None => {
                let axis = if dir == Dir::Right { Axis::Horizontal } else { Axis::Vertical };
                tab.split_pane(target, axis, cwd.as_deref(), focus).map_err(|e| format!("split: {e}"))?
            }
        };
        if let (Some(cmd), Some(p)) = (command, tab.panes.get_mut(&new)) {
            p.type_line(cmd);
        }
        if focus {
            self.sessions[s].goto(t);
        }
        Ok(new)
    }

    fn send_input(&mut self, pane: Option<u32>, input: &[Input]) -> Result<(), String> {
        let (s, t, p) = self.locate(pane)?;
        // Every key must parse before anything is sent, so a typo never half-types a command.
        let mut keys = Vec::new();
        for item in input {
            if let Input::Key(k) = item {
                keys.push(Chord::parse(k)?.to_event());
            }
        }
        let pane = self.sessions[s].tabs[t].panes.get_mut(&p).ok_or_else(|| format!("no pane {p}"))?;
        let mut keys = keys.into_iter();
        for item in input {
            match item {
                Input::Text(text) => pane.type_text(text),
                Input::Key(_) => {
                    pane.key(&keys.next().expect("parsed above"));
                }
            }
        }
        Ok(())
    }

    fn list(&self) -> Value {
        let sessions: Vec<Value> = self
            .sessions
            .iter()
            .map(|s| {
                let tabs: Vec<Value> = s
                    .tabs
                    .iter()
                    .enumerate()
                    .map(|(ti, t)| {
                        let ids = t.placements().into_iter().map(|pl| pl.pane).chain(t.floats.iter().map(|f| f.pane));
                        let panes: Vec<Value> = ids
                            .filter_map(|id| {
                                let p = t.panes.get(&id)?;
                                Some(json!({
                                    "id": id,
                                    "focused": id == t.focused,
                                    "float": t.is_float(id),
                                    "cwd": p.shell.cwd,
                                    "title": p.title,
                                    "label": p.label,
                                    "running": p.shell.phase == Phase::Running,
                                    "process": p.process(),
                                    "last_exit": p.shell.last_exit,
                                    "tags": p.tags,
                                }))
                            })
                            .collect();
                        json!({"active": ti == s.active, "broadcast": t.broadcast, "panes": panes})
                    })
                    .collect();
                json!({"name": s.name, "tabs": tabs})
            })
            .collect();
        json!({"sessions": sessions})
    }

    pub(super) fn command_finished(&mut self, pane: PaneId, exit: Option<i32>) {
        self.broadcast(&Event::CommandFinished { pane, exit });
        let now = Instant::now();
        for w in self.controls.iter_mut().flat_map(|c| c.waits.iter_mut()) {
            if w.pane == pane && w.finished.is_none() {
                w.finished = Some((exit, now));
            }
        }
    }

    pub(super) fn answer_waits(&mut self, prompted: Option<PaneId>) {
        let now = Instant::now();
        let silent = self.silent_panes();
        let ready =
            |w: &Wait| w.finished.is_some_and(|(_, at)| Some(w.pane) == prompted || now >= at + WAIT_FOR_PROMPT);
        let unmarked = |w: &Wait| silent.contains(&w.pane) && now >= w.started + WAIT_FOR_INTEGRATION;
        if !self.controls.iter().any(|c| c.waits.iter().any(|w| ready(w) || unmarked(w))) {
            return;
        }
        let mut answers = Vec::new();
        for (ci, c) in self.controls.iter_mut().enumerate() {
            answers.extend(c.waits.extract_if(.., |w| ready(w) || unmarked(w)).map(|w| (ci, w)));
        }
        for (ci, w) in answers {
            let Some((exit, _)) = w.finished else {
                let msg = format!("pane {} has no shell integration, so its command's end can't be seen", w.pane);
                self.controls[ci].send(&api::error(w.id.as_ref(), &msg));
                continue;
            };
            let output = self
                .sessions
                .iter()
                .find_map(|s| s.panes().find(|(p, _)| **p == w.pane))
                .and_then(|(_, p)| p.last_output());
            self.controls[ci].send(&api::ok(w.id.as_ref(), json!({"pane": w.pane, "exit": exit, "output": output})));
        }
    }

    pub(super) fn wait_deadline(&self) -> Option<Instant> {
        let silent = self.silent_panes();
        self.controls
            .iter()
            .flat_map(|c| &c.waits)
            .filter_map(|w| match w.finished {
                Some((_, at)) => Some(at + WAIT_FOR_PROMPT),
                None => silent.contains(&w.pane).then_some(w.started + WAIT_FOR_INTEGRATION),
            })
            .min()
    }

    /// Waited-on panes that have sent no OSC 133 mark yet.
    fn silent_panes(&self) -> Vec<PaneId> {
        let waited: Vec<PaneId> = self.controls.iter().flat_map(|c| c.waits.iter().map(|w| w.pane)).collect();
        if waited.is_empty() {
            return waited;
        }
        self.sessions
            .iter()
            .flat_map(|s| s.panes())
            .filter(|(id, p)| waited.contains(id) && p.shell.phase == Phase::Unknown)
            .map(|(id, _)| *id)
            .collect()
    }

    // Diffing ids covers keys, exits and `session kill` alike.
    pub(super) fn panes_changed(&mut self) {
        let now: BTreeMap<PaneId, usize> = self
            .sessions
            .iter()
            .enumerate()
            .flat_map(|(s, session)| session.panes().map(move |(id, _)| (*id, s)))
            .collect();
        let opened: Vec<Event> = now
            .iter()
            .filter(|(id, _)| !self.known_panes.contains_key(id))
            .map(|(&pane, &s)| Event::PaneOpened { pane, session: self.sessions[s].name.clone() })
            .collect();
        let closed: Vec<PaneId> = self.known_panes.keys().filter(|id| !now.contains_key(id)).copied().collect();
        self.known_panes = now;
        for e in &opened {
            self.broadcast(e);
        }
        for pane in closed {
            self.broadcast(&Event::PaneClosed { pane });
            for c in &mut self.controls {
                for w in c.waits.extract_if(.., |w| w.pane == pane).collect::<Vec<_>>() {
                    c.send(&match w.finished {
                        Some((exit, _)) => api::ok(w.id.as_ref(), json!({"pane": pane, "exit": exit, "output": null})),
                        None => api::error(w.id.as_ref(), &format!("pane {pane} closed before its command finished")),
                    });
                }
            }
        }
    }

    pub(super) fn broadcast(&mut self, e: &Event) {
        if !self.controls.iter().any(|c| c.subscribed) {
            return;
        }
        let line = api::event(e);
        self.controls.iter_mut().filter(|c| c.subscribed).for_each(|c| c.send(&line));
    }
}
