//! Restore must run on every exit path, or the user's shell is left in raw mode with
//! mouse reporting on. `Tty` restores on drop, and that covers panics too.

use std::io::{self, Write};
use std::os::fd::{AsFd, BorrowedFd};

use rustix::termios::{OptionalActions, Termios, tcgetattr, tcgetwinsize, tcsetattr};
use shroud_core::protocol::Size;

// Kitty flags 31 make every key unambiguous, so it can be re-encoded for any pane's mode.
const SETUP: &[u8] = b"\x1b[?1049h\x1b[>31u\x1b[?1003h\x1b[?1006h\x1b[?2004h\x1b[?1004h\x1b[?2031h\x1b[?996n";
const TEARDOWN: &[u8] =
    b"\x1b[<u\x1b[?2031l\x1b[?1004l\x1b[?2004l\x1b[?1006l\x1b[?1003l\x1b[0m\x1b[0 q\x1b[?25h\x1b[?1049l";

#[derive(Debug)]
pub struct Tty {
    saved: Termios,
}

impl Tty {
    pub fn enter() -> io::Result<Tty> {
        let stdin = io::stdin();
        let saved = tcgetattr(stdin.as_fd())?;
        let mut raw = saved.clone();
        raw.make_raw();
        tcsetattr(stdin.as_fd(), OptionalActions::Flush, &raw)?;
        let mut out = io::stdout().lock();
        out.write_all(SETUP)?;
        out.flush()?;
        Ok(Tty { saved })
    }

    pub fn size() -> io::Result<Size> {
        let ws = tcgetwinsize(io::stdout().as_fd())?;
        Ok(Size { cols: ws.ws_col.max(1), rows: ws.ws_row.max(1), px_w: ws.ws_xpixel, px_h: ws.ws_ypixel })
    }

    pub fn stdin_fd() -> BorrowedFd<'static> {
        // SAFETY: fd 0 stays open for the life of the process.
        unsafe { BorrowedFd::borrow_raw(0) }
    }
}

impl Drop for Tty {
    fn drop(&mut self) {
        let mut out = io::stdout().lock();
        let _ = out.write_all(TEARDOWN);
        let _ = out.flush();
        let _ = tcsetattr(io::stdin().as_fd(), OptionalActions::Flush, &self.saved);
    }
}
