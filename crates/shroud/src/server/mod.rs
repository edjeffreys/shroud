//! The per-user server: owns every session and PTY and renders for every client. Single-threaded.

mod api;
mod client_conn;
mod control_conn;
mod event_loop;
mod events;
mod history;
mod images;
mod input;
mod jump;
mod keymaps;
mod launch;
mod layout_api;
mod lifecycle;
mod links;
mod mouse_select;
mod outer;
mod overlays;
mod pane;
mod plugin_bar;
mod plugin_commands;
mod plugin_host;
mod reload;
mod render;
mod session;
mod shortcuts;
mod status;
mod switch;
mod tab;
mod visual;

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::os::fd::AsFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use shroud_core::config::{self, Problem};
use shroud_core::input::OuterColors;
use shroud_core::layout::PaneId;
use shroud_core::pace::{Decision, Pacer};
use shroud_core::protocol::api::Event;
use shroud_core::protocol::{ExitReason, ServerMsg, Size};
use shroud_core::render::{Frame, Links};
use shroud_core::statusbar::Badge;

pub use plugin_host::{SOCKET_ENV, TOKEN_ENV};

use crate::config_file;
use client_conn::{ClientConn, Incoming};
use control_conn::ControlConn;
use event_loop::{Interest, Token, wait};
use launch::Launch;
use pane::Outward;
use session::{Session, SessionId};

const ESC_TIMEOUT: Duration = Duration::from_millis(25);
// Long enough to read a config problem after saving the file.
pub(super) const CONFIG_MESSAGE_FOR: Duration = Duration::from_secs(8);
// Until a client reports its size; it does so immediately after hello.
pub(super) const DEFAULT_SIZE: Size = Size { cols: 80, rows: 24, px_w: 0, px_h: 0 };

#[derive(Debug)]
pub struct Options {
    pub socket: PathBuf,
    pub control: PathBuf,
}

struct Server {
    cfg: Options,
    listener: UnixListener,
    control_listener: UnixListener,
    controls: Vec<ControlConn>,
    known_panes: BTreeMap<PaneId, usize>,
    outer_colors: OuterColors,
    links: Links,
    plugin_commands: Vec<plugin_commands::PluginCommand>,
    keymaps: Vec<keymaps::Keymap>,
    jobs: Vec<shortcuts::Job>,
    /// The bindings config gave, before plugin defaults and `bind`s.
    config_bindings: shroud_core::input::Bindings,
    plugin_host: plugin_host::PluginHost,
    status_owner: Option<u64>,
    watched: Option<events::Watched>,
    overlays: Vec<overlays::Placed>,
    next_overlay: u32,
    frame_time: Option<String>,
    links_kept: usize,
    outer_dark: Option<bool>,
    outer_gen: u64,
    outer_graphemes: bool,
    signals: UnixStream,
    terminate: Arc<AtomicBool>,
    clients: Vec<ClientConn>,
    esc_deadline: Vec<Option<Instant>>,
    sessions: Vec<Session>,
    next_session: SessionId,
    pacer: Pacer,
    start: Instant,
    settings: config::Config,
    problems: Vec<Problem>,
    watch: config_file::Watch,
    launch: Launch,
    mouse_owner: Option<PaneId>,
    out: Vec<u8>,
    done: Option<ExitReason>,
}

pub fn run(cfg: Options) -> io::Result<()> {
    let _ = std::fs::remove_file(&cfg.socket);
    let listener = UnixListener::bind(&cfg.socket)?;
    listener.set_nonblocking(true)?;
    let _ = std::fs::remove_file(&cfg.control);
    let control_listener = UnixListener::bind(&cfg.control)?;
    control_listener.set_nonblocking(true)?;
    let (signals, sig_write) = UnixStream::pair()?;
    signals.set_nonblocking(true)?;
    let terminate = Arc::new(AtomicBool::new(false));
    for sig in [signal_hook::consts::SIGCHLD, signal_hook::consts::SIGTERM, signal_hook::consts::SIGHUP] {
        signal_hook::low_level::pipe::register(sig, sig_write.try_clone()?)?;
    }
    for sig in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGHUP] {
        signal_hook::flag::register(sig, terminate.clone())?;
    }
    let mut s = Server {
        cfg,
        listener,
        control_listener,
        controls: Vec::new(),
        known_panes: BTreeMap::new(),
        outer_colors: OuterColors::default(),
        links: Links::default(),
        plugin_commands: Vec::new(),
        keymaps: Vec::new(),
        jobs: Vec::new(),
        config_bindings: Default::default(),
        plugin_host: plugin_host::PluginHost::default(),
        status_owner: None,
        watched: None,
        overlays: Vec::new(),
        next_overlay: 0,
        frame_time: None,
        links_kept: 0,
        outer_dark: None,
        outer_gen: 1,
        outer_graphemes: true,
        signals,
        terminate,
        clients: Vec::new(),
        esc_deadline: Vec::new(),
        sessions: Vec::new(),
        next_session: 1,
        pacer: Pacer::new(),
        start: Instant::now(),
        settings: config::Config::default(),
        problems: Vec::new(),
        watch: config_file::Watch::new(config_file::settings_path()),
        launch: Launch::detect(None),
        mouse_owner: None,
        out: Vec::new(),
        done: None,
    };
    s.poll_config(true);
    let result = s.run();
    let _ = std::fs::remove_file(&s.cfg.socket);
    let _ = std::fs::remove_file(&s.cfg.control);
    result
}

impl Server {
    fn run(&mut self) -> io::Result<()> {
        while self.done.is_none() {
            let now = self.start.elapsed();
            let timeout = self.timeout(now);
            let ready = {
                let mut interests = vec![
                    Interest { fd: self.listener.as_fd(), token: Token::Listener, write: false },
                    Interest { fd: self.control_listener.as_fd(), token: Token::ControlListener, write: false },
                    Interest { fd: self.signals.as_fd(), token: Token::Signal, write: false },
                ];
                for (i, c) in self.clients.iter().enumerate() {
                    interests.push(Interest { fd: c.stream.as_fd(), token: Token::Client(i), write: c.wants_write() });
                }
                for (i, c) in self.controls.iter().enumerate() {
                    interests.push(Interest { fd: c.stream.as_fd(), token: Token::Control(i), write: c.wants_write() });
                }
                for (id, p) in self.sessions.iter().flat_map(Session::panes) {
                    interests.push(Interest { fd: p.fd(), token: Token::Pane(*id), write: p.wants_write() });
                }
                wait(&interests, timeout)?
            };
            let mut dead = Vec::new();
            let mut dead_controls = Vec::new();
            for r in ready {
                match r.token {
                    Token::Listener => self.accept(),
                    Token::ControlListener => self.accept_control(),
                    Token::Control(i) => {
                        if (r.readable && self.read_control(i).is_err())
                            || (r.writable && self.controls[i].flush().is_err())
                        {
                            dead_controls.push(i);
                        }
                    }
                    Token::Signal => self.signal(),
                    Token::Client(i) => {
                        if (r.readable && self.read_client(i).is_err())
                            || (r.writable && self.clients[i].flush().is_err())
                        {
                            dead.push(i);
                        }
                    }
                    Token::Pane(id) if r.readable || r.hangup => self.read_pane(id),
                    Token::Pane(_) => {}
                }
            }
            self.flush_escapes();
            self.poll_config(false);
            self.reap();
            self.watch_plugins();
            self.panes_changed();
            self.watch_changes();
            self.report_pending_keys();
            self.sync_outer();
            self.sync_history();
            self.answer_waits(None);
            self.refresh_status();
            for s in &mut self.sessions {
                s.panes_mut().for_each(|(_, p)| p.flush());
            }
            self.expire_holds();
            self.maybe_render();
            self.compress_idle();
            for (i, c) in self.clients.iter_mut().enumerate() {
                if c.flush().is_err() && !dead.contains(&i) {
                    dead.push(i);
                }
            }
            self.drop_clients(dead);
            for (i, c) in self.controls.iter_mut().enumerate() {
                if c.flush().is_err() && !dead_controls.contains(&i) {
                    dead_controls.push(i);
                }
            }
            dead_controls.sort_unstable();
            dead_controls.dedup();
            for i in dead_controls.into_iter().rev() {
                let gone = self.controls.remove(i);
                self.drop_overlays_of(gone.id);
                self.drop_commands_of(gone.id);
                self.drop_keymaps_of(gone.id);
                self.release_status(gone.id);
            }
        }
        let reason = self.done.unwrap_or(ExitReason::ServerShutdown);
        for c in &mut self.clients {
            c.exit(reason);
            let _ = c.stream.set_nonblocking(false);
            let _ = c.flush();
        }
        Ok(())
    }

    fn timeout(&mut self, now: Duration) -> Option<Duration> {
        let mut t: Option<Duration> = None;
        let (dirty, backpressured) = self.frame_pressure();
        match self.pacer.decide(now, dirty, backpressured) {
            Decision::RenderAt(at) => t = Some(at.saturating_sub(now)),
            Decision::RenderNow => t = Some(Duration::ZERO),
            Decision::Idle => {}
        }
        let status = self
            .sessions
            .iter()
            .filter_map(|s| s.status.deadline())
            .chain([self.watch.next_check])
            .chain(self.wait_deadline())
            .chain(self.key_pending_deadline())
            .chain(self.plugin_host.deadline())
            .chain(self.compress_deadline())
            .chain(self.hold_deadline())
            .chain(self.job_deadline());
        for d in self.esc_deadline.iter().flatten().copied().chain(status) {
            let left = d.saturating_duration_since(Instant::now());
            t = Some(t.map_or(left, |x| x.min(left)));
        }
        t
    }

    fn frame_pressure(&self) -> (bool, bool) {
        let waiting: Vec<_> = self.clients.iter().filter(|c| c.session.is_some() && c.needs_frame).collect();
        (!waiting.is_empty(), !waiting.is_empty() && waiting.iter().all(|c| c.wants_write()))
    }

    fn accept(&mut self) {
        while let Ok((stream, _)) = self.listener.accept() {
            match ClientConn::new(stream) {
                Ok(c) => {
                    self.clients.push(c);
                    self.esc_deadline.push(None);
                }
                Err(e) => eprintln!("shroud: accept: {e}"),
            }
        }
    }

    fn signal(&mut self) {
        let mut buf = [0u8; 64];
        while matches!(self.signals.read(&mut buf), Ok(n) if n > 0) {}
        if self.terminate.load(Ordering::Relaxed) {
            self.done = Some(ExitReason::ServerShutdown);
        }
        self.reap();
    }

    fn read_client(&mut self, i: usize) -> io::Result<()> {
        let mut incoming = Vec::new();
        self.clients[i].read(&mut incoming)?;
        if self.clients[i].decoder.has_pending() {
            self.esc_deadline[i] = Some(Instant::now() + ESC_TIMEOUT);
        }
        for msg in incoming {
            match msg {
                Incoming::Hello { size, version, want } => self.hello(i, size, version, want),
                Incoming::Control { version, request } => self.control(i, version, request),
                Incoming::Resize(size) => {
                    self.clients[i].size = Some(size);
                    self.resize_sessions();
                }
                Incoming::Events(events) => events.into_iter().for_each(|ev| self.event(i, ev)),
                Incoming::Detach => self.clients[i].exit(ExitReason::Detached),
            }
        }
        Ok(())
    }

    fn session_of(&mut self, i: usize) -> Option<&mut Session> {
        let id = self.clients[i].session?;
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    fn resize_sessions(&mut self) {
        for s in &mut self.sessions {
            let sizes = self.clients.iter().filter(|c| c.session == Some(s.id)).filter_map(|c| c.size);
            if let Some(size) = sizes.reduce(|a, b| Size {
                cols: a.cols.min(b.cols),
                rows: a.rows.min(b.rows),
                px_w: a.px_w.min(b.px_w),
                px_h: a.px_h.min(b.px_h),
            }) {
                s.resize(size);
            }
        }
        for c in &mut self.clients {
            if let Some(s) = c.size
                && (c.next.grid.cols(), c.next.grid.rows()) != (s.cols, s.rows)
            {
                c.next = Frame::new(s.cols, s.rows);
                c.prev = None;
            }
        }
        self.invalidate();
    }

    fn read_pane(&mut self, id: PaneId) {
        let Some(session) = self.sessions.iter_mut().find(|s| s.panes().any(|(p, _)| *p == id)) else { return };
        let session_id = session.id;
        let Some((pane, watched)) = session.pane_mut(id) else { return };
        let mut out = Outward::default();
        let finished_before = pane.shell.commands_finished;
        pane.read(&mut out);
        let finished = pane.shell.commands_finished != finished_before;
        let exits = std::mem::take(&mut out.exits);
        let prompted = out.prompted;
        if out.echo {
            self.pacer.expedite();
        }
        let mut events = Vec::new();
        if out.bell {
            events.push(Event::Bell { pane: id });
        }
        if out.title {
            events.push(Event::TitleChanged { pane: id, title: pane.title.clone() });
        }
        if out.cwd
            && let Some(cwd) = &pane.shell.cwd
        {
            events.push(Event::CwdChanged { pane: id, cwd: cwd.clone() });
        }
        if out.started {
            events.push(Event::OutputStarted { pane: id });
        }
        if !watched {
            for badge in out.finished {
                // A failure stays visible until seen, even if a later command succeeds.
                if pane.badge != Some(Badge::Failed) {
                    pane.badge = Some(badge);
                }
            }
        } else if finished {
            let cwd = pane.shell.cwd.clone();
            session.status.set_cwd(cwd.as_deref(), true);
        }
        if !out.bytes.is_empty() {
            for c in self.clients.iter_mut().filter(|c| c.session == Some(session_id)) {
                c.send(ServerMsg::Output(&out.bytes));
            }
        }
        for exit in exits {
            self.command_finished(id, exit);
            self.command_pane_finished(id, exit);
        }
        if prompted {
            self.answer_waits(Some(id));
        }
        for e in &events {
            self.broadcast(e);
        }
        self.invalidate();
    }

    fn reap(&mut self) {
        self.reap_jobs();
        let mut ended = Vec::new();
        self.sessions.retain_mut(|s| {
            let alive = s.reap();
            if !alive {
                ended.push(s.id);
            }
            alive
        });
        if !ended.is_empty() {
            self.sessions_ended(&ended);
        }
    }

    fn flush_escapes(&mut self) {
        let now = Instant::now();
        for i in 0..self.clients.len() {
            if self.esc_deadline[i].is_some_and(|d| d <= now) {
                self.esc_deadline[i] = None;
                let mut events = Vec::new();
                self.clients[i].decoder.flush(&mut events);
                events.into_iter().for_each(|ev| self.event(i, ev));
            }
        }
    }

    fn invalidate(&mut self) {
        self.clients.iter_mut().for_each(|c| c.needs_frame = true);
    }

    fn refresh_status(&mut self) {
        let mut changed = false;
        let now = Instant::now();
        for s in &mut self.sessions {
            let cwd = s.tab().focused_cwd();
            let before = (s.status.cwd.clone(), s.status.branch.clone());
            s.status.set_cwd(cwd.as_ref().and_then(|c| c.to_str()), false);
            changed |= (s.status.cwd.clone(), s.status.branch.clone()) != before;
            changed |= s.status.tick(now);
        }
        if changed {
            self.invalidate();
        }
    }

    fn drop_clients(&mut self, mut dead: Vec<usize>) {
        if dead.is_empty() {
            return;
        }
        dead.sort_unstable();
        dead.dedup();
        for i in dead.into_iter().rev() {
            self.clients.remove(i);
            self.esc_deadline.remove(i);
        }
        self.resize_sessions();
    }
}
