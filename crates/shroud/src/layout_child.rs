//! `kdl` can take seconds on a malformed file, and a thread can't be stopped; a child can.

use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use shroud_core::config::{PaneSpec, Problem, layout_bounds, parse_layout};
use shroud_core::layout::Shape;

pub const DEADLINE: Duration = Duration::from_secs(1);

// Far more than a 16 KiB layout's JSON needs.
const MAX_REPLY: usize = 1 << 20;

pub type Tabs = Vec<Shape<PaneSpec>>;

/// Blocks for at most `DEADLINE`.
pub fn parse(text: &str) -> Result<Tabs, Problem> {
    layout_bounds(text)?;
    run(text).unwrap_or_else(|e| Err(problem(format!("couldn't parse it: {e}"))))
}

pub fn serve() -> io::Result<i32> {
    let mut text = String::new();
    io::stdin().read_to_string(&mut text)?;
    io::stdout().write_all(&serde_json::to_vec(&parse_layout(&text)).map_err(io::Error::other)?)?;
    Ok(0)
}

fn run(text: &str) -> io::Result<Result<Tabs, Problem>> {
    let mut child = Command::new(std::env::current_exe()?)
        .arg("__layout")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let reply = talk(&mut child, text);
    let _ = child.kill();
    child.wait()?;
    match reply? {
        Some(json) => serde_json::from_slice(&json).map_err(io::Error::other),
        None => Ok(Err(problem(format!("parsing took over {} s, so it's probably malformed", DEADLINE.as_secs())))),
    }
}

/// `None` when the deadline passes first.
pub fn talk(child: &mut Child, text: &str) -> io::Result<Option<Vec<u8>>> {
    // The child reads all of stdin before parsing, so this write can't wait on `kdl`.
    child.stdin.take().ok_or_else(|| io::Error::other("no stdin"))?.write_all(text.as_bytes())?;
    let mut out = child.stdout.take().ok_or_else(|| io::Error::other("no stdout"))?;
    rustix::io::ioctl_fionbio(&out, true)?;
    let end = Instant::now() + DEADLINE;
    let (mut reply, mut buf) = (Vec::new(), [0; 8192]);
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(None);
        }
        let ts = Timespec { tv_sec: left.as_secs() as _, tv_nsec: left.subsec_nanos() as _ };
        match poll(&mut [PollFd::new(&out, PollFlags::IN)], Some(&ts)) {
            Ok(_) | Err(rustix::io::Errno::INTR) => {}
            Err(e) => return Err(e.into()),
        }
        match out.read(&mut buf) {
            Ok(0) => return Ok(Some(reply)),
            Ok(n) if reply.len() + n <= MAX_REPLY => reply.extend_from_slice(&buf[..n]),
            Ok(_) => return Err(io::Error::other("the parser's reply was too long")),
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => {}
            Err(e) => return Err(e),
        }
    }
}

fn problem(message: String) -> Problem {
    Problem { line: None, message }
}
