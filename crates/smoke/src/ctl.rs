//! `control.sock` and `shroud events`. Lines read while waiting for another are kept for later checks.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use serde_json::Value;
use shroud_core::protocol::socket_path;

use crate::harness::Env;

pub struct Ctl {
    stream: UnixStream,
    buf: Vec<u8>,
    queue: VecDeque<Value>,
}

impl Ctl {
    pub fn connect(env: &Env) -> Ctl {
        let stream = UnixStream::connect(socket_path::control_socket(&env.runtime())).expect("control socket");
        stream.set_read_timeout(Some(Duration::from_millis(50))).expect("timeout");
        Ctl { stream, buf: Vec::new(), queue: VecDeque::new() }
    }

    pub fn send(&mut self, v: Value) {
        let mut line = v.to_string();
        line.push('\n');
        let _ = self.stream.write_all(line.as_bytes());
    }

    fn read_some(&mut self) {
        let mut chunk = [0u8; 65536];
        match self.stream.read(&mut chunk) {
            Ok(n) if n > 0 => self.buf.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() != ErrorKind::WouldBlock && e.kind() != ErrorKind::TimedOut => {}
            _ => {}
        }
        while let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            if let Ok(v) = serde_json::from_slice(&line) {
                self.queue.push_back(v);
            }
        }
    }

    pub fn wait(&mut self, timeout: Duration, cond: impl Fn(&Value) -> bool) -> Option<Value> {
        let end = Instant::now() + timeout;
        loop {
            if let Some(i) = self.queue.iter().position(&cond) {
                return self.queue.remove(i);
            }
            if Instant::now() >= end {
                return None;
            }
            self.read_some();
        }
    }

    pub fn reply(&mut self, v: Value) -> Value {
        let id = v["id"].clone();
        self.send(v);
        self.wait(Duration::from_secs(4), |l| l["id"] == id).unwrap_or(Value::Null)
    }

    pub fn event(&mut self, timeout: Duration, cond: impl Fn(&Value) -> bool) -> Option<Value> {
        self.wait(timeout, |l| l.get("event").is_some() && cond(l))
    }
}

pub struct Events {
    child: Child,
    rx: Receiver<Value>,
    pub seen: Vec<Value>,
}

impl Events {
    pub fn start(env: &Env) -> Events {
        let mut child =
            env.command(&["events"]).stdout(Stdio::piped()).stdin(Stdio::null()).spawn().expect("shroud events");
        let out = child.stdout.take().expect("events stdout");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str(&line)
                    && tx.send(v).is_err()
                {
                    break;
                }
            }
        });
        Events { child, rx, seen: Vec::new() }
    }

    pub fn wait(&mut self, timeout: Duration, cond: impl Fn(&Value) -> bool) -> bool {
        let end = Instant::now() + timeout;
        loop {
            if self.seen.iter().any(&cond) {
                return true;
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            if let Ok(v) = self.rx.recv_timeout(left.min(Duration::from_millis(50))) {
                self.seen.push(v);
            }
        }
    }

    pub fn clear(&mut self) {
        while let Ok(v) = self.rx.try_recv() {
            drop(v);
        }
        self.seen.clear();
    }

    pub fn names(&self) -> Vec<String> {
        self.seen.iter().filter_map(|e| e["event"].as_str().map(str::to_string)).collect()
    }
}

impl Drop for Events {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn is(e: &Value, name: &str) -> bool {
    e["event"] == name
}
