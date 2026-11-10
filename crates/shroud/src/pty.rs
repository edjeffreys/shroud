//! `pre_exec` must make the child a session leader with the PTY as its controlling
//! terminal, or job control breaks in the shell.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use rustix::fs::{OFlags, fcntl_setfl};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use rustix::termios::{Winsize, tcsetwinsize};

// A server started from inside one would otherwise make every pane think it's still there.
const OUTER_MULTIPLEXER: &[&str] = &["ZELLIJ", "TMUX"];

#[derive(Debug)]
pub struct Pty {
    master: File,
    child: Child,
}

#[derive(Clone, Debug)]
pub struct Spawn<'a> {
    pub program: &'a str,
    pub cwd: Option<&'a Path>,
    pub cols: u16,
    pub rows: u16,
    pub term: &'a str,
    pub env: &'a [(&'static str, String)],
    pub pane: u32,
}

impl Pty {
    pub fn spawn(opts: &Spawn) -> io::Result<Pty> {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
        // macOS's posix_openpt has no O_CLOEXEC; without it every child inherits every master.
        rustix::io::fcntl_setfd(&master, rustix::io::FdFlags::CLOEXEC)?;
        grantpt(&master)?;
        unlockpt(&master)?;
        let name = ptsname(&master, Vec::new())?;
        // NOCTTY: the server is a session leader and would otherwise steal the child's terminal.
        let slave: OwnedFd = File::options()
            .read(true)
            .write(true)
            .custom_flags(rustix::fs::OFlags::NOCTTY.bits() as i32)
            .open(name.to_str().map_err(io::Error::other)?)?
            .into();
        tcsetwinsize(&master, winsize(opts.cols, opts.rows))?;

        let mut cmd = Command::new(opts.program);
        cmd.arg("-l")
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave))
            .env("TERM", opts.term)
            .env("COLORTERM", "truecolor")
            .env("SHROUD", "1")
            // The CLI acts on this pane by default, so a script inside shroud acts on its own pane.
            .env("SHROUD_PANE", opts.pane.to_string())
            // Panes are parsed by libghostty-vt whatever the outer terminal is, and programs gate
            // features (synchronized output, kitty keys) on this name. Dotfiles detect shroud by $SHROUD.
            .env("TERM_PROGRAM", "ghostty")
            .env_remove("TERM_PROGRAM_VERSION")
            .env_remove("SHROUD_LIBVT_DIR")
            .envs(opts.env.iter().map(|(k, v)| (k, v)));
        for (k, _) in std::env::vars_os() {
            if OUTER_MULTIPLEXER.iter().any(|p| k.to_string_lossy().starts_with(p)) {
                cmd.env_remove(k);
            }
        }
        if let Some(cwd) = opts.cwd.filter(|c| c.is_dir()) {
            cmd.current_dir(cwd);
        }
        // SAFETY: setsid and TIOCSCTTY are async-signal-safe syscalls, fine between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(std::io::stdin().as_fd())?;
                Ok(())
            });
        }
        let child = cmd.spawn()?;
        fcntl_setfl(&master, OFlags::NONBLOCK)?;
        Ok(Pty { master: master.into(), child })
    }

    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        tcsetwinsize(&self.master, winsize(cols, rows))?;
        Ok(())
    }

    /// Ok(0) means the child side is gone.
    pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.master.read(buf) {
            // macOS reports a closed slave as EIO rather than EOF.
            Err(e) if e.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) => Ok(0),
            r => r,
        }
    }

    pub fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.master.write(buf)
    }

    pub fn fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.master.as_fd()
    }

    pub fn foreground(&self) -> Option<String> {
        crate::process::foreground(&self.master, self.child.id())
    }

    pub fn try_reap(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    pub fn hang_up(&mut self) {
        let pid = rustix::process::Pid::from_child(&self.child);
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::HUP);
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.hang_up();
        let _ = self.child.try_wait();
    }
}

fn winsize(cols: u16, rows: u16) -> Winsize {
    Winsize { ws_col: cols, ws_row: rows, ws_xpixel: 0, ws_ypixel: 0 }
}
