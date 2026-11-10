//! A program on a PTY of its own, as a terminal runs it, read without blocking.

use std::fs::File;
use std::io::{self, ErrorKind, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::{OFlags, fcntl_setfl};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use rustix::termios::{Winsize, tcsetwinsize};

pub struct Pty {
    master: File,
    pub child: Child,
    pub closed: bool,
}

impl Pty {
    /// Replaces `cmd`'s stdio with the PTY.
    pub fn spawn(mut cmd: Command, cols: u16, rows: u16) -> io::Result<Pty> {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
        rustix::io::fcntl_setfd(&master, rustix::io::FdFlags::CLOEXEC)?;
        grantpt(&master)?;
        unlockpt(&master)?;
        let name = ptsname(&master, Vec::new())?;
        let slave: OwnedFd = File::options()
            .read(true)
            .write(true)
            .custom_flags(OFlags::NOCTTY.bits() as i32)
            .open(name.to_str().map_err(io::Error::other)?)?
            .into();
        // Sized before the program starts, as a terminal does; set afterwards, it races the program.
        tcsetwinsize(&master, Winsize { ws_row: rows, ws_col: cols, ws_xpixel: cols * 10, ws_ypixel: rows * 20 })?;
        cmd.stdin(Stdio::from(slave.try_clone()?)).stdout(Stdio::from(slave.try_clone()?)).stderr(Stdio::from(slave));
        // SAFETY: setsid and TIOCSCTTY are async-signal-safe, fine between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(std::io::stdin().as_fd())?;
                Ok(())
            });
        }
        let child = cmd.spawn()?;
        fcntl_setfl(&master, OFlags::NONBLOCK)?;
        Ok(Pty { master: master.into(), child, closed: false })
    }

    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.master.write_all(bytes)
    }

    pub fn read(&mut self, wait: Duration, out: &mut Vec<u8>) -> usize {
        if self.closed {
            std::thread::sleep(wait);
            return 0;
        }
        let ts = Timespec { tv_sec: wait.as_secs() as _, tv_nsec: wait.subsec_nanos() as _ };
        let mut fds = [PollFd::new(&self.master, PollFlags::IN)];
        if !matches!(poll(&mut fds, Some(&ts)), Ok(n) if n > 0) {
            return 0;
        }
        let start = out.len();
        let mut buf = [0u8; 65536];
        loop {
            match self.master.read(&mut buf) {
                Ok(0) => {
                    self.closed = true;
                    break;
                }
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                // macOS reports a closed PTY as EIO.
                Err(_) => {
                    self.closed = true;
                    break;
                }
            }
        }
        out.len() - start
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
