//! `shroud pane …` and `shroud events`: thin, blocking clients of `control.sock`.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde_json::Value;
use shroud_core::protocol::api::{Dir, FloatSpec, Input, Request, Target, encode_request};

pub const USAGE: &str = "  shroud pane ls                     list sessions, tabs and panes as JSON
  shroud pane split [opts]           open a pane next to the target; prints its id
  shroud pane run [opts] <cmd>...    open a pane and type a command into its shell
      opts: -p <pane>  --down  --float  --cwd <dir>  --focus
            run only: --wait (exit with the command's code, print its output)  --close (close the pane when it exits)  --keep (hold it when the command ends: Enter reruns, q closes)
  shroud pane send [-p <pane>] (<text> | -k <key>)...   type text and keys, e.g. send 'make' -k enter
  shroud pane output [-p <pane>]     print the pane's last command output
  shroud pane capture [-p <pane>] [--history]   print the screen's text, or everything kept
  shroud command <name> [count]      run a command as if its key were pressed, e.g. zoom, goto-tab 3
  shroud pane close [-p <pane>]
  shroud pane focus [-p <pane>]
  shroud events                      stream events as JSON lines
  The target pane is -p, else $SHROUD_PANE (the pane you're in), else the focused pane.";

pub fn pane(socket: &Path, args: &[&str]) -> io::Result<i32> {
    let Some((&sub, rest)) = args.split_first() else { return usage() };
    let mut opts = Opts::parse(rest)?;
    let request = match sub {
        "ls" => Request::List,
        "split" | "run" => {
            let command = (sub == "run").then(|| opts.words.join(" "));
            if command.as_ref().is_some_and(String::is_empty) || (sub == "split" && !opts.words.is_empty()) {
                return usage();
            }
            Request::Split {
                pane: opts.pane,
                dir: opts.dir,
                cwd: opts.cwd.take(),
                command,
                focus: opts.focus,
                wait: opts.wait,
                float: opts.float.then(FloatSpec::default),
                close_on_exit: opts.close,
                keep: opts.keep,
            }
        }
        "send" if !opts.input.is_empty() => Request::Send { pane: opts.pane, input: std::mem::take(&mut opts.input) },
        "output" => Request::Output { pane: opts.pane },
        "capture" => Request::Capture { pane: opts.pane, history: opts.history },
        "close" => Request::Close { pane: opts.pane },
        "focus" => Request::Focus { pane: opts.pane },
        _ => return usage(),
    };
    let Some(mut conn) = Conn::open(socket) else { return Ok(1) };
    let reply = conn.call(&request)?;
    if reply["ok"] != true {
        eprintln!("shroud: {}", reply["error"].as_str().unwrap_or("request failed"));
        return Ok(1);
    }
    match sub {
        "ls" => println!("{}", serde_json::to_string_pretty(&reply["sessions"]).unwrap_or_default()),
        "split" | "run" if opts.wait => {
            if let Some(text) = reply["output"].as_str() {
                println!("{text}");
            }
            return Ok(reply["exit"].as_i64().map_or(0, |e| e as i32));
        }
        "split" | "run" => println!("{}", reply["pane"]),
        "output" | "capture" => println!("{}", reply["text"].as_str().unwrap_or_default()),
        _ => {}
    }
    Ok(0)
}

pub fn command(socket: &Path, words: &[&str]) -> io::Result<i32> {
    if words.is_empty() {
        return usage();
    }
    let Some(mut conn) = Conn::open(socket) else { return Ok(1) };
    let reply = conn.call(&Request::Run { command: Target::Name(words.join(" ")), client: None, session: None })?;
    if reply["ok"] != true {
        eprintln!("shroud: {}", reply["error"].as_str().unwrap_or("request failed"));
        return Ok(1);
    }
    Ok(0)
}

pub fn events(socket: &Path) -> io::Result<i32> {
    let Some(mut conn) = Conn::open(socket) else { return Ok(1) };
    conn.call(&Request::Subscribe)?;
    let mut out = io::stdout().lock();
    loop {
        let mut line = String::new();
        if conn.reader.read_line(&mut line)? == 0 {
            return Ok(0);
        }
        if line.contains("\"event\"") {
            out.write_all(line.as_bytes())?;
            out.flush()?;
        }
    }
}

/// One request, or `None` when no server is running, which callers treat as nothing to tell.
pub fn ask(socket: &Path, request: &Request) -> Option<Value> {
    let stream = UnixStream::connect(socket).ok()?;
    let reader = BufReader::new(stream.try_clone().ok()?);
    Conn { stream, reader }.call(request).ok()
}

fn usage() -> io::Result<i32> {
    eprintln!("usage:\n{USAGE}");
    Ok(2)
}

#[derive(Default)]
struct Opts {
    pane: Option<u32>,
    dir: Dir,
    cwd: Option<String>,
    focus: bool,
    wait: bool,
    float: bool,
    close: bool,
    keep: bool,
    history: bool,
    input: Vec<Input>,
    words: Vec<String>,
}

impl Opts {
    fn parse(args: &[&str]) -> io::Result<Opts> {
        let mut o = Opts { pane: std::env::var("SHROUD_PANE").ok().and_then(|p| p.parse().ok()), ..Opts::default() };
        let mut it = args.iter();
        let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidInput, m.to_string());
        while let Some(&a) = it.next() {
            match a {
                "-p" | "--pane" => {
                    o.pane = Some(it.next().and_then(|p| p.parse().ok()).ok_or_else(|| bad("-p needs a pane id"))?)
                }
                "--down" => o.dir = Dir::Down,
                "--focus" => o.focus = true,
                "--wait" => o.wait = true,
                "--float" => o.float = true,
                "--close" => o.close = true,
                "--keep" => o.keep = true,
                "--history" => o.history = true,
                "--cwd" => {
                    let dir = it.next().ok_or_else(|| bad("--cwd needs a directory"))?;
                    o.cwd = Some(absolute(dir)?.to_string_lossy().into_owned());
                }
                "-k" | "--key" => o.input.push(Input::Key(it.next().ok_or_else(|| bad("-k needs a key"))?.to_string())),
                "--" => it.by_ref().for_each(|t| o.text(t)),
                text => o.text(text),
            }
        }
        Ok(o)
    }

    fn text(&mut self, t: &str) {
        self.input.push(Input::Text(t.to_string()));
        self.words.push(t.to_string());
    }
}

fn absolute(dir: &str) -> io::Result<PathBuf> {
    let p = PathBuf::from(dir);
    Ok(if p.is_absolute() { p } else { std::env::current_dir()?.join(p) })
}

struct Conn {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Conn {
    fn open(socket: &Path) -> Option<Conn> {
        let Ok(stream) = UnixStream::connect(socket) else {
            eprintln!("shroud: no server running");
            return None;
        };
        let reader = BufReader::new(stream.try_clone().ok()?);
        Some(Conn { stream, reader })
    }

    fn call(&mut self, request: &Request) -> io::Result<Value> {
        self.stream.write_all(encode_request(Some(1), request).as_bytes())?;
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                return Err(io::Error::other("the server closed the connection"));
            }
            let v: Value = serde_json::from_str(&line).map_err(io::Error::other)?;
            if v["id"] == 1 {
                return Ok(v);
            }
        }
    }
}
