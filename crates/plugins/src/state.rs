//! What the host and the Lua-facing calls share. Borrowed briefly, never across a resume,
//! since Lua calls back in.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use mlua::{Function, Thread};

use crate::system::Jobs;
use serde_json::{Value, json};

pub struct Plugin {
    pub name: String,
    pub commands: HashSet<String>,
}

pub struct Timer {
    pub at: Instant,
    pub plugin: usize,
    pub func: Function,
}

pub struct State {
    pub out: Box<dyn Write>,
    pub next_id: u64,
    pub current: Option<usize>,
    pub plugins: Vec<Plugin>,
    pub handlers: BTreeMap<String, Vec<(usize, Function)>>,
    pub commands: HashMap<String, (usize, Function)>,
    /// Requests the host made for itself: nobody is waiting on the reply.
    pub internal: HashMap<u64, Option<usize>>,
    /// A plugin's request, until its handler yields to wait for the reply.
    pub sent: HashMap<u64, String>,
    pub waiting: HashMap<u64, (usize, Thread, String)>,
    pub overlays: HashMap<u64, usize>,
    /// The plugin that took the status bar, and so alone hears `status`.
    pub bar: Option<usize>,
    pub timers: Vec<Timer>,
    pub subscribed: bool,
    pub data_root: Option<PathBuf>,
    /// From `shroud.plugins{…}`, loaded once the call that made them returns, since the
    /// host can't start another plugin's Lua while one is running.
    pub specs: Vec<mlua::Table>,
    pub jobs: Jobs,
    /// `shroud.system` jobs given an `on_exit`, which runs as their plugin.
    pub on_exit: HashMap<u64, (usize, Function)>,
    /// Results nobody has waited for yet.
    pub finished: HashMap<u64, Value>,
    /// Numbers the commands `shroud.keymap` makes for functions without a `name`.
    pub keymaps: u32,
}

impl State {
    pub fn write(&mut self, v: Value) {
        let mut line = v.to_string();
        line.push('\n');
        // A write error means the server is gone; the read loop sees EOF and exits.
        let _ = self.out.write_all(line.as_bytes()).and_then(|()| self.out.flush());
    }

    pub fn fresh_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id - 1
    }

    pub fn send(&mut self, mut v: Value) -> u64 {
        let id = self.fresh_id();
        v["id"] = id.into();
        self.write(v);
        id
    }

    pub fn request(&mut self, v: Value, for_plugin: Option<usize>) {
        let id = self.send(v);
        self.internal.insert(id, for_plugin);
    }

    pub fn report(&mut self, plugin: usize, message: &str) {
        let name = self.plugins[plugin].name.clone();
        eprintln!("shroud: plugin {name}: {message}");
        let first = message.lines().next().unwrap_or_default();
        self.write(json!({"cmd": "plugin_error", "plugin": name, "message": first}));
    }

    pub fn current(&self) -> mlua::Result<usize> {
        self.current.ok_or_else(|| mlua::Error::runtime("shroud's functions only work inside a plugin's code"))
    }
}
