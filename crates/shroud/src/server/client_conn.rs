//! No new frame while bytes are unsent: that backpressure stops floods queuing stale frames.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;

use shroud_core::input::{Decoder, Event, Keybinds};
use shroud_core::layout::PaneId;
use shroud_core::overlay::Span;
use shroud_core::protocol::{Attach, ClientMsg, Control, DecodeError, ExitReason, ServerMsg, Size};
use shroud_core::render::Frame;
use shroud_core::statusbar::Mode;
use shroud_core::switcher::Switcher;
use shroud_core::visual::Visual;

use super::session::SessionId;

#[derive(Debug)]
pub struct Bar {
    pub left: Vec<Span>,
    pub right: Vec<Span>,
    pub fill: Option<String>,
}

#[derive(Debug)]
pub struct ClientConn {
    /// Stable across the server's list changing; events name clients by it.
    pub id: u64,
    pub stream: UnixStream,
    pub size: Option<Size>,
    pub session: Option<SessionId>,
    pub switcher: Option<Switcher>,
    pub visual: Option<(PaneId, Visual)>,
    pub drag: Option<super::mouse_select::Drag>,
    pub jump: Option<super::jump::Jump>,
    pub last_click: Option<super::mouse_select::Click>,
    pub bar: Option<Bar>,
    /// Last state sent to the bar's plugin, so it hears only of changes.
    pub bar_state: Option<serde_json::Value>,
    pub keybinds: Keybinds,
    /// When the current key sequence last changed, until `key_pending` is reported for it.
    pub pending_since: Option<std::time::Instant>,
    pub pending_reported: bool,
    pub decoder: Decoder,
    pub needs_frame: bool,
    pub images: super::images::ClientImages,
    pub closing: bool,
    pub prev: Option<Frame>,
    pub next: Frame,
    rbuf: Vec<u8>,
    wbuf: Vec<u8>,
}

/// `Attach`, owned, since decoded messages borrow the receive buffer.
#[derive(Debug)]
pub enum Want {
    Recent,
    Named(String),
    New { name: String, cwd: String },
}

#[derive(Debug)]
pub enum Request {
    List,
    Kill(String),
    Rename { from: String, to: String },
    StopServer { restart: bool },
}

#[derive(Debug)]
pub enum Incoming {
    Hello { size: Size, version: u16, want: Want },
    Control { version: u16, request: Request },
    Events(Vec<Event>),
    Resize(Size),
    Detach,
}

impl ClientConn {
    pub fn mode(&self) -> Mode {
        if self.keybinds.is_pending() {
            Mode::Leader
        } else if let Some((_, v)) = &self.visual {
            Mode::Visual(v.selection().map(|s| s.shape))
        } else if self.keybinds.is_jumping() {
            Mode::Jump
        } else {
            Mode::Normal
        }
    }

    pub fn new(stream: UnixStream) -> io::Result<ClientConn> {
        stream.set_nonblocking(true)?;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Ok(ClientConn {
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            stream,
            size: None,
            session: None,
            switcher: None,
            visual: None,
            drag: None,
            jump: None,
            last_click: None,
            bar: None,
            bar_state: None,
            keybinds: Keybinds::new(),
            pending_since: None,
            pending_reported: false,
            decoder: Decoder::new(),
            needs_frame: true,
            images: Default::default(),
            closing: false,
            prev: None,
            next: Frame::new(1, 1),
            rbuf: Vec::new(),
            wbuf: Vec::new(),
        })
    }

    pub fn wants_write(&self) -> bool {
        !self.wbuf.is_empty()
    }

    pub fn read(&mut self, out: &mut Vec<Incoming>) -> io::Result<()> {
        let mut buf = [0u8; 16 * 1024];
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => self.rbuf.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        let mut pos = 0;
        loop {
            match ClientMsg::decode(&self.rbuf[pos..]) {
                Ok(Some((msg, n))) => {
                    pos += n;
                    out.push(match msg {
                        ClientMsg::Hello { version, size, attach } => Incoming::Hello {
                            size,
                            version,
                            want: match attach {
                                Attach::Recent => Want::Recent,
                                Attach::Named(n) => Want::Named(n.into()),
                                Attach::New { name, cwd } => Want::New { name: name.into(), cwd: cwd.into() },
                            },
                        },
                        ClientMsg::Control { version, control } => Incoming::Control {
                            version,
                            request: match control {
                                Control::List => Request::List,
                                Control::Kill(n) => Request::Kill(n.into()),
                                Control::Rename { from, to } => Request::Rename { from: from.into(), to: to.into() },
                                Control::StopServer { restart } => Request::StopServer { restart },
                            },
                        },
                        ClientMsg::Input(bytes) => {
                            let mut events = Vec::new();
                            self.decoder.feed(bytes, &mut events);
                            Incoming::Events(events)
                        }
                        ClientMsg::Resize(size) => Incoming::Resize(size),
                        ClientMsg::Detach => Incoming::Detach,
                    });
                }
                Ok(None) => break,
                Err(e @ (DecodeError::UnknownTag(_) | DecodeError::TooLarge(_) | DecodeError::Malformed)) => {
                    return Err(io::Error::other(format!("bad client message: {e:?}")));
                }
            }
        }
        self.rbuf.drain(..pos);
        Ok(())
    }

    pub fn send(&mut self, msg: ServerMsg) {
        msg.encode(&mut self.wbuf);
    }

    pub fn exit(&mut self, reason: ExitReason) {
        if !self.images.is_empty() {
            self.send(ServerMsg::Output(shroud_core::render::images::REMOVE_ALL));
        }
        self.send(ServerMsg::Exit(reason));
        self.closing = true;
    }

    pub fn flush(&mut self) -> io::Result<()> {
        while !self.wbuf.is_empty() {
            match self.stream.write(&self.wbuf) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => {
                    self.wbuf.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        if self.closing {
            return Err(io::ErrorKind::ConnectionAborted.into());
        }
        Ok(())
    }
}
