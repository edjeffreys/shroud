//! `~/.config/shroud/init.lua`: the user's own plugin, which can also declare others with
//! `shroud.plugins{…}`, nvim-style.

use std::cell::RefCell;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mlua::{Function, Value as LuaValue};
use serde_json::{Value, json};
use shroud_core::config::Source;

use crate::builtin::Files;
use crate::host::Host;
use crate::state::Plugin;

pub const USER: &str = "user";

#[derive(Default)]
pub struct Dirs {
    pub config: Option<PathBuf>,
    pub install: Option<PathBuf>,
}

/// One entry of `shroud.plugins{…}`.
pub struct Spec {
    pub src: String,
    version: Option<String>,
    opts: LuaValue,
    config: Option<Function>,
    enabled: bool,
}

impl Spec {
    fn from_table(t: &mlua::Table) -> Result<Spec, String> {
        let first: Option<String> = t.raw_get(1).map_err(|e| format!("a plugin's source must be a string: {e}"))?;
        let named: Option<String> = t.raw_get("src").map_err(|e| format!("a plugin's source must be a string: {e}"))?;
        let src = first.or(named).ok_or("a plugin spec needs its source first, as in { \"github:owner/repo\" }")?;
        Source::parse(&src)?;
        Ok(Spec {
            version: t.raw_get("version").map_err(|e| format!("{src}: version: {e}"))?,
            opts: t.raw_get("opts").map_err(|e| e.to_string())?,
            config: t.raw_get("config").map_err(|e| format!("{src}: config must be a function: {e}"))?,
            enabled: t.raw_get::<Option<bool>>("enabled").map_err(|e| format!("{src}: enabled: {e}"))?.unwrap_or(true),
            src,
        })
    }
}

/// Where a spec's plugin lives, by the same rules the server uses for config.toml's.
pub fn resolve(src: &str, dirs: &Dirs) -> Result<Files, String> {
    let dir = match Source::parse(src)? {
        Source::Builtin(name) => return Files::builtin(&name),
        Source::Path(p) => match p.strip_prefix("~/") {
            Some(rest) => {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)).ok_or("no HOME for a ~ path")?
            }
            None if Path::new(&p).is_absolute() => PathBuf::from(p),
            None => dirs.config.as_ref().ok_or("no config directory for a relative path")?.join(p),
        },
        other => {
            let dir = other
                .dir_name()
                .zip(dirs.install.as_ref())
                .map(|(n, d)| d.join(n))
                .ok_or("no install directory for it")?;
            if !dir.join("plugin.toml").is_file() {
                return Err("not installed: run `shroud plugin sync`".into());
            }
            dir
        }
    };
    Files::read(&dir)
}

impl Host {
    /// `~/.config/shroud/init.lua`, run as a plugin called `user` with no manifest.
    pub(crate) fn load_user(&mut self, init: &Path) {
        if self.disabled.contains(USER) {
            eprintln!("shroud: {} stays off until the config changes", init.display());
            return;
        }
        let code = match std::fs::read_to_string(init) {
            Ok(c) => c,
            Err(e) => {
                return self.state.borrow_mut().write(
                    json!({"cmd": "plugin_error", "plugin": USER, "message": format!("{}: {e}", init.display())}),
                );
            }
        };
        let plugin = {
            let mut st = self.state.borrow_mut();
            st.plugins.push(Plugin { name: USER.into(), commands: HashSet::new() });
            st.plugins.len() - 1
        };
        match self.lua.load(code).set_name(format!("@{}", init.display())).into_function() {
            Ok(chunk) => match self.lua.create_thread(self.boot.clone()) {
                Ok(thread) => self.resume(plugin, thread, mlua::MultiValue::from_vec(vec![LuaValue::Function(chunk)])),
                Err(e) => self.state.borrow_mut().report(plugin, &e.to_string()),
            },
            Err(e) => self.state.borrow_mut().report(plugin, &e.to_string()),
        }
    }

    pub(crate) fn take_specs(&mut self) -> Vec<Spec> {
        let tables = std::mem::take(&mut self.state.borrow_mut().specs);
        let user = self.state.borrow().plugins.iter().position(|p| p.name == USER);
        let mut out = Vec::new();
        for t in tables {
            match Spec::from_table(&t) {
                Ok(spec) => out.push(spec),
                Err(e) => {
                    if let Some(u) = user {
                        self.state.borrow_mut().report(u, &e);
                    }
                }
            }
        }
        out
    }

    pub(crate) fn load_specs(&mut self, specs: Vec<Spec>) {
        for spec in specs.into_iter().filter(|s| s.enabled) {
            let files = resolve(&spec.src, &self.dirs);
            self.load_reporting(&spec.src, files, spec.opts, spec.config, Some(USER));
        }
    }

    /// For `plugin sync` and `ls`: run init.lua only to learn which plugins it names. Its
    /// requests go nowhere, and the caller kills the process if it never returns.
    pub fn specs_of(init: &Path) -> Result<Vec<Value>, String> {
        let out = Buffer::default();
        let mut h = Host::new(Box::new(out.clone()), None).map_err(|e| e.to_string())?;
        h.spec_only = true;
        h.load_user(init);
        let specs = h.take_specs();
        let written = String::from_utf8_lossy(&out.0.borrow()).to_string();
        let errors: Vec<String> = written
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|v| v["cmd"] == "plugin_error")
            .filter_map(|v| v["message"].as_str().map(str::to_string))
            .collect();
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
        Ok(specs.into_iter().map(|s| json!({"src": s.src, "version": s.version, "enabled": s.enabled})).collect())
    }
}

#[derive(Clone, Default)]
struct Buffer(Rc<RefCell<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::host::Host;
    use crate::host::tests::{Shared, TestDir, find, plugin};

    fn init_lua(test: &str, code: &str) -> TestDir {
        TestDir::with_file(&format!("{test}-init"), "init.lua", code)
    }

    #[test]
    fn init_lua_declares_plugins_with_lua_opts_and_config_functions() {
        let hello = plugin(
            "initlua",
            "hello",
            "",
            "return { setup = function(o) shroud.request{cmd = 'send', input = {{text = o.greet('setup')}}} end }",
        );
        let other = plugin("initlua", "other", "", "return { name = 'other' }");
        let init = init_lua(
            "initlua",
            &format!(
                "shroud.plugins {{\n  {{ 'path:{}', opts = {{ greet = function(w) return w .. '!' end }} }},\n  {{ 'path:{}', config = function(m, o) shroud.request{{cmd = 'send', input = {{{{text = 'config ' .. m.name}}}}}} end }},\n}}\nshroud.command('mine', function() end)\n",
                hello.display(),
                other.display()
            ),
        );
        let out = Shared::default();
        let mut h = Host::new(Box::new(out.clone()), None).unwrap();
        h.hello("tok");
        out.take();
        h.line(&json!({"id": 0, "ok": true, "plugins": [], "init": init}).to_string());
        let lines = out.take();
        let sent: Vec<_> = lines.iter().filter(|l| l["cmd"] == "send").map(|l| l["input"][0]["text"].clone()).collect();
        assert_eq!(sent, vec![json!("setup!"), json!("config other")], "opts carry functions; config replaces setup");
        assert_eq!(find(&lines, "register_command")["name"], "user.mine");
        assert_eq!(h.plugin_names(), vec!["user", "hello", "other"]);
    }

    #[test]
    fn a_spec_replaces_configs_entry_for_the_same_source() {
        let init = init_lua("replace", "shroud.plugins { { 'builtin:hints', enabled = false } }\n");
        let out = Shared::default();
        let mut h = Host::new(Box::new(out.clone()), None).unwrap();
        h.hello("tok");
        h.line(&json!({"id": 0, "ok": true, "plugins": [{"src": "builtin:hints", "builtin": "hints", "opts": {}}], "init": init}).to_string());
        assert_eq!(h.plugin_names(), vec!["user"]);
    }

    #[test]
    fn specs_can_be_read_without_running_plugins() {
        let init = init_lua(
            "specs",
            "shroud.plugins { 'github:a/b', { 'git:/tmp/x', version = 'v2' }, { 'builtin:hints', enabled = false } }\nshroud.split{}\n",
        );
        let specs = Host::specs_of(&init).unwrap();
        assert_eq!(
            specs,
            vec![
                json!({"src": "github:a/b", "version": null, "enabled": true}),
                json!({"src": "git:/tmp/x", "version": "v2", "enabled": true}),
                json!({"src": "builtin:hints", "version": null, "enabled": false}),
            ]
        );
        let bad = init_lua("specs-bad", "shroud.plugins { { 'ftp:nope' } }\n");
        assert!(Host::specs_of(&bad).unwrap_err().contains("ftp"));
        let broken = init_lua("specs-broken", "this is not lua\n");
        assert!(Host::specs_of(&broken).is_err());
    }
}
