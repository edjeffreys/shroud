//! A `control.sock` connection. A subscriber that stops reading is dropped rather than
//! buffered forever.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Instant;

use serde_json::Value;
use shroud_core::layout::PaneId;
use shroud_core::protocol::api::MAX_LINE;

const MAX_UNSENT: usize = 4 << 20;

#[derive(Debug)]
pub struct ControlConn {
    /// Stable, unlike its index in the server's list: overlays name their owner by it.
    pub id: u64,
    pub stream: UnixStream,
    pub subscribed: bool,
    pub waits: Vec<Wait>,
    rbuf: Vec<u8>,
    wbuf: Vec<u8>,
}

/// `split` with `wait`. The output is read at the next prompt, since it is bounded by one.
#[derive(Debug)]
pub struct Wait {
    pub id: Option<Value>,
    pub pane: PaneId,
    pub started: Instant,
    pub finished: Option<(Option<i32>, Instant)>,
}

impl ControlConn {
    pub fn new(stream: UnixStream) -> io::Result<ControlConn> {
        stream.set_nonblocking(true)?;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(ControlConn { id, stream, subscribed: false, waits: Vec::new(), rbuf: Vec::new(), wbuf: Vec::new() })
    }

    pub fn wants_write(&self) -> bool {
        !self.wbuf.is_empty()
    }

    /// `Ok(true)` at EOF: answer the lines that came before it, then drop the connection.
    pub fn read(&mut self, lines: &mut Vec<String>) -> io::Result<bool> {
        let mut buf = [0u8; 16 * 1024];
        let mut eof = false;
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(n) => self.rbuf.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        while let Some(end) = self.rbuf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.rbuf.drain(..=end).collect();
            let text = String::from_utf8_lossy(&line[..end]).trim().to_string();
            if !text.is_empty() {
                lines.push(text);
            }
        }
        if self.rbuf.len() > MAX_LINE {
            return Err(io::Error::other("request line too long"));
        }
        Ok(eof)
    }

    pub fn send(&mut self, line: &str) {
        self.wbuf.extend_from_slice(line.as_bytes());
    }

    pub fn flush(&mut self) -> io::Result<()> {
        while !self.wbuf.is_empty() {
            match self.stream.write(&self.wbuf) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => {
                    self.wbuf.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        if self.wbuf.len() > MAX_UNSENT {
            return Err(io::Error::other("control client stopped reading"));
        }
        Ok(())
    }
}
