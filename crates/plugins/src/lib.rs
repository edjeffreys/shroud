//! Runs only in the `shroud __plugins` process, never in the server: a JIT-compiled loop can't be interrupted.

mod builtin;
mod host;
pub mod manifest;
mod raw;
mod state;
mod system;
mod user;

use std::io::{self, Read};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};

pub use host::Host;
pub use manifest::API;
pub use user::USER;

pub const TYPES: &str = include_str!("../lua/shroud.lua");

/// Returns when the server closes the socket.
pub fn run(socket: &Path, token: &str, data_root: Option<PathBuf>) -> io::Result<()> {
    let mut stream = UnixStream::connect(socket)?;
    let mut host = Host::new(Box::new(stream.try_clone()?), data_root).map_err(|e| io::Error::other(e.to_string()))?;
    let (mut wake, wake_tx) = UnixStream::pair()?;
    wake.set_nonblocking(true)?;
    host.set_wake(wake_tx);
    host.hello(token);
    // Kept across reads: one can stop partway through a line.
    let mut pending = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let wait = host.next_timer().map(|t| t.saturating_duration_since(Instant::now()).max(Duration::from_millis(1)));
        let ts = wait.map(|d| Timespec { tv_sec: d.as_secs() as _, tv_nsec: d.subsec_nanos() as _ });
        let mut fds = [PollFd::new(&stream, PollFlags::IN), PollFd::new(&wake, PollFlags::IN)];
        match poll(&mut fds, ts.as_ref()) {
            Ok(_) | Err(rustix::io::Errno::INTR) => {}
            Err(e) => return Err(e.into()),
        }
        let readable = !fds[0].revents().is_empty();
        while matches!(wake.read(&mut buf), Ok(n) if n > 0) {}
        // Blocking, because the writer shares this socket; poll said a read won't wait.
        if readable {
            match stream.read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => pending.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        let mut start = 0;
        while let Some(end) = pending[start..].iter().position(|&b| b == b'\n') {
            host.line(&String::from_utf8_lossy(&pending[start..start + end + 1]));
            start += end + 1;
        }
        pending.drain(..start);
        host.tick(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::TYPES;

    #[test]
    fn every_function_in_the_prelude_has_a_definition() {
        let prelude = include_str!("prelude.lua");
        let wrapped = prelude.lines().filter_map(|l| l.strip_prefix("wrap(\"")).map(|l| l.split('"').next().unwrap());
        let raw = prelude
            .lines()
            .filter_map(|l| l.strip_prefix("shroud."))
            .filter_map(|l| l.split_once(" = raw."))
            .map(|(name, _)| name);
        let names: Vec<&str> = wrapped.chain(raw).chain(["request"]).collect();
        assert!(names.len() > 15, "{names:?}");
        for name in names {
            let defined =
                TYPES.contains(&format!("function shroud.{name}(")) || TYPES.contains(&format!("shroud.{name} = {{}}"));
            assert!(defined, "shroud.{name} has no definition");
        }
    }
}
