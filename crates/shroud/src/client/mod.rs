//! The attached client stays dumb: it moves bytes, so it can't drift out of sync with
//! the server, and a server crash only costs a clean exit, never the terminal.

pub mod api;
mod tty;

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, poll};
use shroud_core::protocol::{Attach, ClientMsg, Control, ExitReason, ServerMsg, VERSION};

use tty::Tty;

/// A one-shot request; never starts a server, since there would be nothing to act on.
pub fn control(socket: &Path, control: Control) -> io::Result<i32> {
    let Ok(mut stream) = UnixStream::connect(socket) else {
        eprintln!("shroud: no server running");
        return Ok(1);
    };
    let mut out = Vec::new();
    ClientMsg::Control { version: VERSION, control }.encode(&mut out);
    stream.write_all(&out)?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf)?;
    let mut pos = 0;
    while let Ok(Some((msg, used))) = ServerMsg::decode(&buf[pos..]) {
        pos += used;
        match msg {
            ServerMsg::Reply { ok, text } => {
                if ok {
                    println!("{text}");
                } else {
                    eprintln!("shroud: {text}");
                }
                return Ok(i32::from(!ok));
            }
            ServerMsg::Exit(ExitReason::VersionMismatch { server }) => {
                eprintln!("shroud: server speaks protocol {server}, this client {VERSION}; restart the server");
                return Ok(1);
            }
            _ => {}
        }
    }
    Ok(1)
}

pub fn attach(socket: &Path, log: &Path, attach: Attach) -> io::Result<i32> {
    let mut stream = connect_or_spawn(socket, log)?;
    let (sig_read, sig_write) = UnixStream::pair()?;
    sig_read.set_nonblocking(true)?;
    for sig in [signal_hook::consts::SIGWINCH, signal_hook::consts::SIGTERM, signal_hook::consts::SIGHUP] {
        signal_hook::low_level::pipe::register(sig, sig_write.try_clone()?)?;
    }

    let mut out = Vec::new();
    ClientMsg::Hello { version: VERSION, size: Tty::size()?, attach }.encode(&mut out);
    stream.write_all(&out)?;

    let tty = Tty::enter()?;
    let mut refusal = None;
    let result = forward(&mut stream, &sig_read, &mut refusal);
    drop(tty);
    match result {
        Ok(ExitReason::Refused) => {
            eprintln!("shroud: {}", refusal.unwrap_or_default());
            Ok(1)
        }
        Ok(ExitReason::Detached) => {
            println!("[detached]");
            Ok(0)
        }
        Ok(ExitReason::LastPaneClosed) | Ok(ExitReason::ServerShutdown) => Ok(0),
        Ok(ExitReason::Restart) => restart(socket),
        Ok(ExitReason::VersionMismatch { server }) => {
            eprintln!("shroud: server speaks protocol {server}, this client {VERSION}; restart the server");
            Ok(1)
        }
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
            eprintln!("shroud: server exited unexpectedly (log: {})", log.display());
            Ok(1)
        }
        Err(e) => Err(e),
    }
}

/// Runs the binary afresh, so a rebuild takes effect. The old server must have let go of the
/// socket first, or the new client would reach it and wait for a reply that never comes.
fn restart(socket: &Path) -> io::Result<i32> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while UnixStream::connect(socket).is_ok() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let exe = std::env::current_exe()?;
    Err(Command::new(exe).exec())
}

fn forward(stream: &mut UnixStream, signals: &UnixStream, refusal: &mut Option<String>) -> io::Result<ExitReason> {
    let stdin = Tty::stdin_fd();
    let mut stdout = io::stdout().lock();
    let mut rbuf: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut out = Vec::new();
    loop {
        let mut fds = [
            PollFd::new(&stdin, PollFlags::IN),
            PollFd::new(stream, PollFlags::IN),
            PollFd::new(signals, PollFlags::IN),
        ];
        match poll(&mut fds, None) {
            Ok(_) | Err(rustix::io::Errno::INTR) => {}
            Err(e) => return Err(e.into()),
        }
        let [input, server, signal] = fds.map(|f| f.revents());

        if !signal.is_empty() {
            let mut sink = [0u8; 64];
            let mut s = signals;
            while matches!(s.read(&mut sink), Ok(n) if n > 0) {}
            // Only SIGWINCH is benign; HUP/TERM mean the terminal is going away.
            if rustix::termios::tcgetattr(stdin).is_err() {
                return Ok(ExitReason::Detached);
            }
            out.clear();
            ClientMsg::Resize(Tty::size()?).encode(&mut out);
            stream.write_all(&out)?;
        }
        if input.intersects(PollFlags::IN | PollFlags::HUP) {
            let n = rustix::io::read(stdin, &mut buf).map_err(io::Error::from)?;
            if n == 0 {
                return Ok(ExitReason::Detached);
            }
            out.clear();
            ClientMsg::Input(&buf[..n]).encode(&mut out);
            stream.write_all(&out)?;
        }
        if server.intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR) {
            let n = stream.read(&mut buf)?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            rbuf.extend_from_slice(&buf[..n]);
            let mut pos = 0;
            while let Some((msg, used)) =
                ServerMsg::decode(&rbuf[pos..]).map_err(|e| io::Error::other(format!("{e:?}")))?
            {
                pos += used;
                match msg {
                    ServerMsg::Output(bytes) => stdout.write_all(bytes)?,
                    ServerMsg::Welcome { .. } => {}
                    ServerMsg::Reply { text, .. } => *refusal = Some(text.to_string()),
                    ServerMsg::Exit(reason) => {
                        stdout.flush()?;
                        return Ok(reason);
                    }
                }
            }
            stdout.flush()?;
            rbuf.drain(..pos);
        }
    }
}

fn connect_or_spawn(socket: &Path, log: &Path) -> io::Result<UnixStream> {
    if let Ok(s) = UnixStream::connect(socket) {
        return Ok(s);
    }
    let log_file = std::fs::OpenOptions::new().create(true).append(true).open(log)?;
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.arg("__server").stdin(Stdio::null()).stdout(Stdio::null()).stderr(log_file);
    // SAFETY: setsid is async-signal-safe; it detaches the server from this terminal.
    unsafe {
        cmd.pre_exec(|| {
            rustix::process::setsid()?;
            Ok(())
        });
    }
    cmd.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match UnixStream::connect(socket) {
            Ok(s) => return Ok(s),
            Err(e) if Instant::now() >= deadline => {
                return Err(io::Error::new(e.kind(), format!("server did not start (log: {}): {e}", log.display())));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}
