//! Spawns `shroud __plugins` and carries out the watchdog's verdicts. Nothing here waits on
//! the host, so a stuck plugin can't stall typing or rendering.

use std::hash::{BuildHasher, RandomState};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use shroud_core::config::{PluginSpec, Source};
use shroud_core::watchdog::{AfterExit, Watchdog};

use super::{CONFIG_MESSAGE_FOR, Server};
use crate::config_file;

pub const TOKEN_ENV: &str = "SHROUD_PLUGIN_TOKEN";
pub const SOCKET_ENV: &str = "SHROUD_CONTROL_SOCKET";

#[derive(Debug)]
pub struct PluginHost {
    child: Option<Child>,
    token: String,
    conn: Option<u64>,
    watchdog: Watchdog,
    plugins: Vec<PluginSpec>,
    /// init.lua is read by the host, not here; the server only notices it change.
    init: config_file::Watch,
    has_init: bool,
    gave_up: bool,
    /// Kept until the config changes, so the bar's `plugin` tag outlasts the message.
    pub alert: Option<String>,
}

impl Default for PluginHost {
    fn default() -> PluginHost {
        PluginHost {
            child: None,
            token: String::new(),
            conn: None,
            watchdog: Watchdog::default(),
            plugins: Vec::new(),
            init: config_file::Watch::new(config_file::init_path()),
            has_init: false,
            gave_up: false,
            alert: None,
        }
    }
}

impl PluginHost {
    pub fn deadline(&self) -> Option<Instant> {
        Some(self.watchdog.deadline().map_or(self.init.next_check, |d| d.min(self.init.next_check)))
    }

    pub fn is_host(&self, conn: u64) -> bool {
        self.conn == Some(conn)
    }

    fn kill(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.conn = None;
    }
}

impl Drop for PluginHost {
    fn drop(&mut self) {
        self.kill();
    }
}

impl Server {
    pub(super) fn sync_plugins(&mut self) {
        if self.settings.plugins == self.plugin_host.plugins {
            return;
        }
        let host = &mut self.plugin_host;
        host.plugins = self.settings.plugins.clone();
        host.watchdog.forgive();
        host.gave_up = false;
        host.alert = None;
        host.kill();
        self.spawn_host();
    }

    pub(super) fn watch_plugins(&mut self) {
        let now = Instant::now();
        if let Some(text) = self.plugin_host.init.poll(now) {
            self.plugin_host.has_init = text.is_some();
            self.reload_plugins();
        }
        if let Some(plugin) = self.plugin_host.watchdog.overdue(now) {
            if let Some(c) = &mut self.plugin_host.child {
                let _ = c.kill();
            }
            self.plugin_alert(format!("plugin {plugin} stopped: busy for 2 s; off until the config changes"));
        }
        let exited = self.plugin_host.child.as_mut().is_some_and(|c| !matches!(c.try_wait(), Ok(None)));
        if !exited {
            return;
        }
        self.plugin_host.child = None;
        self.plugin_host.conn = None;
        match self.plugin_host.watchdog.exited(now) {
            AfterExit::Restart { blamed } => {
                if let Some(p) = blamed {
                    self.plugin_alert(format!("plugin {p} crashed the plugin host; off until the config changes"));
                }
                self.spawn_host();
            }
            AfterExit::GiveUp => {
                self.plugin_host.gave_up = true;
                self.plugin_alert("the plugin host keeps crashing; plugins are off until the config changes".into());
            }
        }
    }

    pub(super) fn plugin_hello(&mut self, conn: u64, token: &str) -> Result<Value, String> {
        let host = &mut self.plugin_host;
        if host.child.is_none() || host.conn.is_some() || token != host.token {
            return Err("only the plugin host shroud started may say hello".into());
        }
        host.conn = Some(conn);
        let mut plugins = Vec::new();
        let mut trouble = Vec::new();
        for spec in &host.plugins {
            let src = spec.src.spec();
            if let Source::Builtin(name) = &spec.src {
                plugins.push(json!({"src": src, "builtin": name, "opts": spec.opts}));
                continue;
            }
            match resolve(&spec.src) {
                Ok(dir) => plugins.push(json!({"src": src, "dir": dir, "opts": spec.opts})),
                // init.lua may name the same source and supply it; the host decides.
                Err(_) if host.has_init => plugins.push(json!({"src": src, "missing": true})),
                Err(e) => trouble.push(e),
            }
        }
        let disabled: Vec<&String> = host.watchdog.disabled().iter().collect();
        let init = config_file::init_path().filter(|_| host.has_init);
        let install = config_file::data_dir().map(|d| d.join("plugins"));
        let started =
            (SystemTime::now() - self.start.elapsed()).duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        let server = json!({"pid": std::process::id(), "started": started});
        let reply = json!({"plugins": plugins, "disabled": disabled, "init": init, "config_dir": config_file::dir(), "install_dir": install, "server": server});
        for t in trouble {
            self.plugin_alert(t);
        }
        Ok(reply)
    }

    /// `plugin sync` changed what's on disk without touching the config.
    pub(super) fn reload_plugins(&mut self) {
        let host = &mut self.plugin_host;
        host.watchdog.forgive();
        host.gave_up = false;
        host.alert = None;
        host.kill();
        self.spawn_host();
        self.invalidate();
    }

    pub(super) fn plugin_status(&self) -> Value {
        let host = &self.plugin_host;
        json!({"running": host.child.is_some(), "disabled": host.watchdog.disabled(), "alert": host.alert})
    }

    pub(super) fn plugin_running(&mut self, conn: u64, plugin: Option<String>) {
        if self.plugin_host.is_host(conn) {
            self.plugin_host.watchdog.running(plugin, Instant::now());
        }
    }

    pub(super) fn plugin_alert(&mut self, message: String) {
        eprintln!("shroud: {message}");
        self.sessions.iter_mut().for_each(|s| s.status.say_for(message.clone(), CONFIG_MESSAGE_FOR));
        self.plugin_host.alert = Some(message);
        self.invalidate();
    }

    fn spawn_host(&mut self) {
        let host = &mut self.plugin_host;
        if (host.plugins.is_empty() && !host.has_init) || host.gave_up {
            return;
        }
        host.token = token();
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => return self.plugin_alert(format!("can't start the plugin host: {e}")),
        };
        let spawned = Command::new(exe)
            .arg("__plugins")
            .env(TOKEN_ENV, &host.token)
            .env(SOCKET_ENV, &self.cfg.control)
            .stdin(Stdio::null())
            .spawn();
        match spawned {
            Ok(c) => host.child = Some(c),
            Err(e) => self.plugin_alert(format!("can't start the plugin host: {e}")),
        }
    }
}

fn resolve(src: &Source) -> Result<PathBuf, String> {
    let p = match src {
        Source::Path(p) => p,
        Source::Builtin(_) => return Err("built-in plugins have no directory".into()),
        other => {
            let dir = other
                .dir_name()
                .zip(config_file::data_dir())
                .map(|(n, d)| d.join("plugins").join(n))
                .ok_or("no install directory for it")?;
            return if dir.join("plugin.toml").is_file() {
                Ok(dir)
            } else {
                Err(format!("{} isn't installed: run `shroud plugin sync`", other.spec()))
            };
        }
    };
    let dir = match p.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)).ok_or("no HOME for a ~ path")?,
        None if Path::new(p).is_absolute() => PathBuf::from(p),
        None => config_file::dir().map(|d| d.join(p)).ok_or("no config directory for a relative path")?,
    };
    if dir.join("plugin.toml").is_file() { Ok(dir) } else { Err(format!("no plugin.toml in {}", dir.display())) }
}

/// Keeps other processes from posing as the host. They run as the same user and could do
/// worse anyway, so std's random hash keys are enough.
fn token() -> String {
    let s = RandomState::new();
    format!("{:016x}{:016x}", s.hash_one(1u8), s.hash_one(2u8))
}
