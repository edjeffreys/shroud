//! A plain poll(2) set: shroud has a few dozen fds at most, and one thread means no locks.

use std::io;
use std::os::fd::BorrowedFd;
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use shroud_core::layout::PaneId;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Token {
    Listener,
    ControlListener,
    Signal,
    Client(usize),
    Control(usize),
    Pane(PaneId),
}

#[derive(Clone, Copy, Debug)]
pub struct Ready {
    pub token: Token,
    pub readable: bool,
    pub writable: bool,
    pub hangup: bool,
}

pub struct Interest<'a> {
    pub fd: BorrowedFd<'a>,
    pub token: Token,
    pub write: bool,
}

pub fn wait(interests: &[Interest], timeout: Option<Duration>) -> io::Result<Vec<Ready>> {
    let mut fds: Vec<PollFd> = interests
        .iter()
        .map(|i| {
            let flags = if i.write { PollFlags::IN | PollFlags::OUT } else { PollFlags::IN };
            PollFd::from_borrowed_fd(i.fd, flags)
        })
        .collect();
    let ts = timeout.map(|d| Timespec { tv_sec: d.as_secs() as _, tv_nsec: d.subsec_nanos() as _ });
    match poll(&mut fds, ts.as_ref()) {
        Ok(_) => {}
        Err(rustix::io::Errno::INTR) => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    }
    Ok(fds
        .iter()
        .zip(interests)
        .filter_map(|(fd, i)| {
            let r = fd.revents();
            (!r.is_empty()).then_some(Ready {
                token: i.token,
                readable: r.intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR),
                writable: r.contains(PollFlags::OUT),
                hangup: r.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL),
            })
        })
        .collect())
}
