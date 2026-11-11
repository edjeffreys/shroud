//! The host's state machine: lines in, lines out, Lua in between. No sockets here, so tests
//! drive it with strings.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use mlua::{Function, Lua, LuaOptions, LuaSerdeExt, StdLib, Thread, Value as LuaValue};
use serde_json::{Value, json};

use crate::builtin::Files;
use crate::manifest::{self, API, Manifest};
use crate::raw::{raw_api, ser_options};
use crate::state::{Plugin, State, Timer};
use crate::system::Jobs;
use crate::user::{Dirs, USER, resolve};

const PRELUDE: &str = include_str!("prelude.lua");
const HELLO_ID: u64 = 0;

pub struct Host {
    pub(crate) lua: Lua,
    pub(crate) state: Rc<RefCell<State>>,
    pub(crate) boot: Function,
    pub(crate) dirs: Dirs,
    pub(crate) disabled: HashSet<String>,
    /// Until the first load finishes, specs wait so they can be merged with config's.
    pub(crate) booting: bool,
    pub(crate) spec_only: bool,
}

impl Host {
    pub fn new(out: Box<dyn Write>, data_root: Option<PathBuf>) -> mlua::Result<Host> {
        // SAFETY: plugins are trusted code, as in nvim, and get LuaJIT's `ffi` and `debug`.
        // Running them in this process, not the server, is what contains the damage.
        let lua = unsafe { Lua::unsafe_new_with(StdLib::ALL, LuaOptions::default()) };
        let state = Rc::new(RefCell::new(State {
            out,
            next_id: HELLO_ID + 1,
            current: None,
            plugins: Vec::new(),
            handlers: BTreeMap::new(),
            commands: HashMap::new(),
            internal: HashMap::new(),
            sent: HashMap::new(),
            waiting: HashMap::new(),
            overlays: HashMap::new(),
            bar: None,
            timers: Vec::new(),
            subscribed: false,
            data_root,
            specs: Vec::new(),
            jobs: Jobs::default(),
            on_exit: HashMap::new(),
            finished: HashMap::new(),
            keymaps: 0,
        }));
        let raw = raw_api(&lua, &state)?;
        let boot = lua.load(PRELUDE).set_name("=shroud").call::<Function>((raw, format!("{}.{}.0", API.0, API.1)))?;
        Ok(Host { lua, state, boot, dirs: Dirs::default(), disabled: HashSet::new(), booting: true, spec_only: false })
    }

    pub fn hello(&mut self, token: &str) {
        self.state.borrow_mut().write(json!({"id": HELLO_ID, "cmd": "plugin_hello", "token": token}));
    }

    pub fn line(&mut self, line: &str) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { return };
        if let Some(id) = v.get("id").and_then(Value::as_u64) {
            self.reply(id, v);
        } else if let Some(name) = v.get("event").and_then(Value::as_str) {
            let name = name.to_string();
            self.event(&name, v);
        }
    }

    pub fn next_timer(&self) -> Option<Instant> {
        let st = self.state.borrow();
        st.timers.iter().map(|t| t.at).chain(st.jobs.next_deadline()).min()
    }

    pub fn set_wake(&mut self, wake: std::os::unix::net::UnixStream) {
        self.state.borrow_mut().jobs.set_wake(wake);
    }

    /// Hands finished `shroud.system` jobs to whoever waits: a handler blocked in `wait()`, the
    /// job's `on_exit`, or a later `wait()`.
    pub fn jobs_done(&mut self, now: Instant) {
        let done = self.state.borrow_mut().jobs.finished(now);
        for d in done {
            let v = json!({"code": d.code, "signal": d.signal, "stdout": d.stdout, "stderr": d.stderr});
            let (waiter, on_exit) = {
                let mut st = self.state.borrow_mut();
                (st.waiting.remove(&d.id), st.on_exit.remove(&d.id))
            };
            let result = match self.lua.to_value_with(&v, ser_options()) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("shroud: a job's result didn't convert: {e}");
                    continue;
                }
            };
            match (waiter, on_exit) {
                (Some((plugin, thread, _)), _) => self.resume(plugin, thread, result),
                (None, Some((plugin, func))) => self.spawn(plugin, func, result),
                (None, None) => {
                    self.state.borrow_mut().finished.insert(d.id, v);
                }
            }
        }
    }

    pub fn tick(&mut self, now: Instant) {
        self.jobs_done(now);
        let due: Vec<Timer> = {
            let mut st = self.state.borrow_mut();
            let (due, later) = std::mem::take(&mut st.timers).into_iter().partition(|t| t.at <= now);
            st.timers = later;
            due
        };
        for t in due {
            self.spawn(t.plugin, t.func, LuaValue::Nil);
        }
    }

    pub fn plugin_names(&self) -> Vec<String> {
        self.state.borrow().plugins.iter().map(|p| p.name.clone()).collect()
    }

    fn reply(&mut self, id: u64, v: Value) {
        if id == HELLO_ID {
            return self.load_all(&v);
        }
        let waiting = self.state.borrow_mut().waiting.remove(&id);
        if let Some((plugin, thread, cmd)) = waiting {
            if cmd == "overlay_open"
                && let Some(o) = v.get("overlay").and_then(Value::as_u64)
            {
                self.state.borrow_mut().overlays.insert(o, plugin);
            }
            if v.get("ok") == Some(&Value::Bool(true)) {
                match cmd.as_str() {
                    "status_take" => self.state.borrow_mut().bar = Some(plugin),
                    "status_release" => self.state.borrow_mut().bar = None,
                    _ => {}
                }
            }
            match self.lua.to_value_with(&v, ser_options()) {
                Ok(reply) => self.resume(plugin, thread, reply),
                Err(e) => self.state.borrow_mut().report(plugin, &e.to_string()),
            }
            return;
        }
        let internal = self.state.borrow_mut().internal.remove(&id);
        if let Some(Some(plugin)) = internal {
            let problem = v.get("error").or_else(|| v.get("key_conflict")).and_then(Value::as_str);
            if let Some(p) = problem {
                self.state.borrow_mut().report(plugin, p);
            }
        }
    }

    fn event(&mut self, name: &str, v: Value) {
        let targets: Vec<(usize, Function)> = {
            let st = self.state.borrow();
            match name {
                "command" => v
                    .get("name")
                    .and_then(Value::as_str)
                    .and_then(|n| st.commands.get(n))
                    .cloned()
                    .into_iter()
                    .collect(),
                "overlay_key" | "overlay_closed" => {
                    let owner = v.get("overlay").and_then(Value::as_u64).and_then(|o| st.overlays.get(&o)).copied();
                    st.handlers.get(name).into_iter().flatten().filter(|(p, _)| Some(*p) == owner).cloned().collect()
                }
                "status" => {
                    st.handlers.get(name).into_iter().flatten().filter(|(p, _)| Some(*p) == st.bar).cloned().collect()
                }
                _ => st.handlers.get(name).cloned().unwrap_or_default(),
            }
        };
        if name == "overlay_closed"
            && let Some(o) = v.get("overlay").and_then(Value::as_u64)
        {
            self.state.borrow_mut().overlays.remove(&o);
        }
        for (plugin, func) in targets {
            match self.lua.to_value_with(&v, ser_options()) {
                Ok(arg) => self.spawn(plugin, func, arg),
                Err(e) => self.state.borrow_mut().report(plugin, &e.to_string()),
            }
        }
    }

    fn load_all(&mut self, hello: &Value) {
        if let Some(e) = hello.get("error").and_then(Value::as_str) {
            eprintln!("shroud: plugin host refused: {e}");
            return;
        }
        let dir = |k: &str| hello.get(k).and_then(Value::as_str).map(PathBuf::from);
        self.dirs = Dirs { config: dir("config_dir"), install: dir("install_dir") };
        self.disabled = hello
            .get("disabled")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        if let Some(server) = hello.get("server")
            && let Ok(v) = self.lua.to_value(server)
            && let Ok(shroud) = self.lua.globals().get::<mlua::Table>("shroud")
        {
            let _ = shroud.set("server", v);
        }
        if let Some(init) = dir("init") {
            self.load_user(&init);
        }
        let ours = self.take_specs();
        for p in hello.get("plugins").and_then(Value::as_array).into_iter().flatten() {
            let src = p.get("src").and_then(Value::as_str).unwrap_or_default();
            if ours.iter().any(|s| s.src == src) {
                continue;
            }
            let opts =
                self.lua.to_value_with(p.get("opts").unwrap_or(&json!({})), ser_options()).unwrap_or(LuaValue::Nil);
            let files = match (p.get("dir").and_then(Value::as_str), p.get("builtin").and_then(Value::as_str)) {
                (Some(dir), _) => Files::read(Path::new(dir)),
                (None, Some(name)) => Files::builtin(name),
                (None, None) => resolve(src, &self.dirs),
            };
            self.load_reporting(src, files, opts, None, None);
        }
        self.load_specs(ours);
        self.booting = false;
    }

    /// Problems before a plugin has a name are blamed on its source, or on `user` for a spec.
    pub(crate) fn load_reporting(
        &mut self,
        src: &str,
        files: Result<Files, String>,
        opts: LuaValue,
        config: Option<Function>,
        blame: Option<&str>,
    ) {
        if let Err(e) = files.and_then(|f| self.load(f, opts, config)) {
            eprintln!("shroud: plugin {src}: {e}");
            let name =
                blame.map(str::to_string).unwrap_or_else(|| src.rsplit(['/', ':']).next().unwrap_or(src).to_string());
            self.state
                .borrow_mut()
                .write(json!({"cmd": "plugin_error", "plugin": name, "message": format!("{src}: {e}")}));
        }
    }

    fn load(&mut self, files: Files, opts: LuaValue, config: Option<Function>) -> Result<(), String> {
        let m: Manifest = manifest::parse(&files.manifest).map_err(|e| format!("plugin.toml: {e}"))?;
        if self.disabled.contains(&m.name) {
            eprintln!("shroud: plugin {} stays off until the config changes", m.name);
            return Ok(());
        }
        if m.name == USER || self.state.borrow().plugins.iter().any(|p| p.name == m.name) {
            return Err(format!("another plugin is already called {}", m.name));
        }
        if let Some(lua_dir) = &files.lua_dir {
            let package: mlua::Table = self.lua.globals().get("package").map_err(|e| e.to_string())?;
            let path: String = package.get("path").map_err(|e| e.to_string())?;
            let ours = format!("{0}/?.lua;{0}/?/init.lua;", lua_dir.display());
            package.set("path", format!("{ours}{path}")).map_err(|e| e.to_string())?;
        }

        let plugin = {
            let mut st = self.state.borrow_mut();
            st.plugins.push(Plugin { name: m.name.clone(), commands: m.commands.keys().cloned().collect() });
            let plugin = st.plugins.len() - 1;
            for (cmd, decl) in &m.commands {
                st.request(
                    json!({"cmd": "register_command", "name": format!("{}.{cmd}", m.name), "key": decl.key}),
                    Some(plugin),
                );
            }
            plugin
        };
        let chunk = self.lua.load(files.init).set_name(files.chunk_name).into_function().map_err(|e| e.to_string())?;
        let thread = self.lua.create_thread(self.boot.clone()).map_err(|e| e.to_string())?;
        let config = config.map_or(LuaValue::Nil, LuaValue::Function);
        self.resume(plugin, thread, mlua::MultiValue::from_vec(vec![LuaValue::Function(chunk), opts, config]));
        Ok(())
    }

    fn spawn(&mut self, plugin: usize, func: Function, arg: impl mlua::IntoLuaMulti) {
        match self.lua.create_thread(func) {
            Ok(thread) => self.resume(plugin, thread, arg),
            Err(e) => self.state.borrow_mut().report(plugin, &e.to_string()),
        }
    }

    /// The running marks bracket every slice of Lua, and the first is flushed before Lua
    /// starts, so a plugin that never returns is still named.
    pub(crate) fn resume(&mut self, plugin: usize, thread: Thread, args: impl mlua::IntoLuaMulti) {
        {
            let mut st = self.state.borrow_mut();
            let name = st.plugins[plugin].name.clone();
            st.write(json!({"cmd": "plugin_running", "plugin": name}));
            st.current = Some(plugin);
        }
        let result = thread.resume::<LuaValue>(args);
        let mut st = self.state.borrow_mut();
        st.current = None;
        st.write(json!({"cmd": "plugin_running", "plugin": null}));
        match result {
            Err(e) => st.report(plugin, &e.to_string()),
            Ok(_) if !thread.is_resumable() => {}
            Ok(LuaValue::Integer(id)) if st.sent.contains_key(&(id as u64)) => {
                let cmd = st.sent.remove(&(id as u64)).unwrap_or_default();
                st.waiting.insert(id as u64, (plugin, thread, cmd));
            }
            Ok(_) => st.report(plugin, "a handler yielded without making a request; use your own coroutines inside it, not around shroud calls"),
        }
        let more = !self.booting && !self.spec_only && !st.specs.is_empty();
        drop(st);
        if more {
            let specs = self.take_specs();
            self.load_specs(specs);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::time::Duration;

    #[derive(Clone, Default)]
    pub(crate) struct Shared(Rc<RefCell<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Shared {
        pub(crate) fn take(&self) -> Vec<Value> {
            let bytes = std::mem::take(&mut *self.0.borrow_mut());
            String::from_utf8(bytes).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
        }
    }

    /// Removes its directory when dropped, so test runs leave nothing in the temp dir.
    pub(crate) struct TestDir {
        root: PathBuf,
        path: PathBuf,
    }

    impl TestDir {
        pub(crate) fn new(name: &str) -> TestDir {
            let root = std::env::temp_dir().join(format!("shroud-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            TestDir { path: root.clone(), root }
        }

        /// The guard then stands for `file` inside the directory.
        pub(crate) fn with_file(name: &str, file: &str, text: &str) -> TestDir {
            let mut d = TestDir::new(name);
            d.path = d.root.join(file);
            std::fs::write(&d.path, text).unwrap();
            d
        }
    }

    impl std::ops::Deref for TestDir {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.path
        }
    }

    impl serde::Serialize for TestDir {
        fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            self.path.serialize(s)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    pub(crate) fn plugin(test: &str, name: &str, toml_extra: &str, init: &str) -> TestDir {
        let dir = TestDir::new(&format!("{test}-{name}"));
        std::fs::write(
            dir.join("plugin.toml"),
            format!("name = \"{name}\"\nversion = \"0.1.0\"\napi = \"0.1\"\n{toml_extra}"),
        )
        .unwrap();
        std::fs::write(dir.join("init.lua"), init).unwrap();
        dir
    }

    // The guards drop when the call returns; plugins have read their files by then.
    fn start(dirs: &[(TestDir, Value)], disabled: &[&str]) -> (Host, Shared) {
        let plugins: Vec<Value> = dirs.iter().map(|(d, o)| json!({"dir": d, "opts": o})).collect();
        start_with(plugins, disabled, None)
    }

    pub(crate) fn start_with(plugins: Vec<Value>, disabled: &[&str], data: Option<PathBuf>) -> (Host, Shared) {
        let out = Shared::default();
        let mut h = Host::new(Box::new(out.clone()), data).unwrap();
        h.hello("tok");
        assert_eq!(out.take(), vec![json!({"id": 0, "cmd": "plugin_hello", "token": "tok"})]);
        h.line(&json!({"id": 0, "ok": true, "plugins": plugins, "disabled": disabled}).to_string());
        (h, out)
    }

    pub(crate) fn cmds(lines: &[Value]) -> Vec<&str> {
        lines.iter().filter_map(|l| l["cmd"].as_str()).filter(|c| *c != "plugin_running").collect()
    }

    pub(crate) fn find<'a>(lines: &'a [Value], cmd: &str) -> &'a Value {
        lines.iter().find(|l| l["cmd"] == cmd).unwrap_or_else(|| panic!("no {cmd} in {lines:?}"))
    }

    #[test]
    fn setup_gets_opts_and_declared_commands_register_with_their_keys() {
        let d = plugin(
            "setup",
            "hello",
            "[commands]\ngreet = { key = \"g y\" }\n",
            "return { setup = function(o) shroud.request{cmd = 'send', input = {{text = o.word}}} end }",
        );
        let (_, out) = start(&[(d, json!({"word": "hi"}))], &[]);
        let lines = out.take();
        assert_eq!(find(&lines, "register_command")["name"], "hello.greet");
        assert_eq!(find(&lines, "register_command")["key"], "g y");
        assert_eq!(find(&lines, "send")["input"], json!([{"text": "hi"}]));
        let marked = lines.iter().position(|l| *l == json!({"cmd": "plugin_running", "plugin": "hello"})).unwrap();
        assert!(marked < lines.iter().position(|l| l["cmd"] == "send").unwrap(), "marked before any Lua runs");
        assert_eq!(lines.last().unwrap(), &json!({"cmd": "plugin_running", "plugin": null}), "unmarked while it waits");
    }

    #[test]
    fn a_request_yields_until_its_reply_and_the_handler_carries_on() {
        let init = "shroud.command('greet', function(ev)\n local p = shroud.split{dir = 'down'}\n shroud.overlay_open{lines = {'pane ' .. p .. ' x' .. ev.count}}\nend)\nreturn {}";
        let d = plugin("yield", "hello", "[commands]\ngreet = {}\n", init);
        let (mut h, out) = start(&[(d, json!({}))], &[]);
        out.take();
        h.line(r#"{"event":"command","name":"hello.greet","client":1,"session":"s","count":3}"#);
        let lines = out.take();
        let split = find(&lines, "split");
        assert_eq!(split["dir"], "down");
        let id = split["id"].as_u64().unwrap();
        h.line(&json!({"id": id, "ok": true, "pane": 7}).to_string());
        assert_eq!(find(&out.take(), "overlay_open")["lines"], json!(["pane 7 x3"]));
    }

    #[test]
    fn failed_requests_return_nil_and_the_error() {
        let init = "shroud.command('go', function()\n local p, err = shroud.focus{pane = 99}\n shroud.request{cmd = 'send', input = {{text = tostring(p) .. ':' .. err}}}\nend)\nreturn {}";
        let d = plugin("fail", "hello", "", init);
        let (mut h, out) = start(&[(d, json!({}))], &[]);
        out.take();
        h.line(r#"{"event":"command","name":"hello.go","client":1,"session":"s","count":null}"#);
        let id = find(&out.take(), "focus")["id"].as_u64().unwrap();
        h.line(&json!({"id": id, "ok": false, "error": "no pane 99"}).to_string());
        assert_eq!(find(&out.take(), "send")["input"], json!([{"text": "nil:no pane 99"}]));
    }

    #[test]
    fn overlay_events_reach_only_the_owner() {
        let tell = |who: &str| {
            format!(
                "shroud.on('overlay_key', function(ev) shroud.request{{cmd = 'send', input = {{{{text = '{who}:' .. ev.key}}}}}} end)\n"
            )
        };
        let a = plugin(
            "overlay",
            "a",
            "",
            &format!(
                "{}shroud.command('open', function() shroud.overlay_open{{lines = {{'x'}}}} end)\nreturn {{}}",
                tell("a")
            ),
        );
        let b = plugin("overlay", "b", "", &format!("{}return {{}}", tell("b")));
        let (mut h, out) = start(&[(a, json!({})), (b, json!({}))], &[]);
        assert!(cmds(&out.take()).iter().all(|c| *c != "subscribe"), "overlay events need no subscription");
        h.line(r#"{"event":"command","name":"a.open","client":1,"session":"s","count":null}"#);
        let id = find(&out.take(), "overlay_open")["id"].as_u64().unwrap();
        h.line(&json!({"id": id, "ok": true, "overlay": 4}).to_string());
        out.take();
        h.line(r#"{"event":"overlay_key","overlay":4,"key":"q"}"#);
        let sends: Vec<_> =
            out.take().into_iter().filter(|l| l["cmd"] == "send").map(|l| l["input"][0]["text"].clone()).collect();
        assert_eq!(sends, vec![json!("a:q")]);
    }

    #[test]
    fn state_events_subscribe_once_and_reach_every_listener() {
        let listen = "shroud.on('focus_changed', function(ev) shroud.request{cmd = 'send', input = {{text = tostring(ev.pane)}}} end)\nreturn {}";
        let (mut h, out) =
            start(&[(plugin("sub", "a", "", listen), json!({})), (plugin("sub", "b", "", listen), json!({}))], &[]);
        assert_eq!(cmds(&out.take()), vec!["subscribe"]);
        h.line(r#"{"event":"focus_changed","session":"s","pane":5}"#);
        assert_eq!(cmds(&out.take()), vec!["send", "send"]);
    }

    #[test]
    fn errors_name_the_plugin_and_a_bad_plugin_spares_the_rest() {
        let bad = plugin("errors", "bad", "", "return { setup = function() error('boom') end }");
        let old = plugin("errors", "old", "", "return {}");
        std::fs::write(old.join("plugin.toml"), "name = \"old\"\nversion = \"1\"\napi = \"0.9\"\n").unwrap();
        let good = plugin("errors", "good", "", "shroud.command('x', function() end)\nreturn {}");
        let (h, out) = start(&[(bad, json!({})), (old, json!({})), (good, json!({}))], &[]);
        let lines = out.take();
        let errors: Vec<_> = lines.iter().filter(|l| l["cmd"] == "plugin_error").collect();
        assert_eq!(errors[0]["plugin"], "bad");
        assert!(errors[0]["message"].as_str().unwrap().contains("boom"));
        assert!(errors[1]["message"].as_str().unwrap().contains("needs plugin API 0.9"));
        assert_eq!(h.plugin_names(), vec!["bad", "good"]);
        assert_eq!(find(&lines, "register_command")["name"], "good.x");
    }

    #[test]
    fn disabled_plugins_are_skipped() {
        let (h, out) = start(&[(plugin("disabled", "stuck", "", "while true do end"), json!({}))], &["stuck"]);
        assert!(h.plugin_names().is_empty());
        assert!(out.take().is_empty());
    }

    #[test]
    fn keymap_binds_commands_and_turns_functions_into_commands() {
        let init = "shroud.keymap('g y', 'zoom')\nshroud.keymap('ctrl+h', 'focus-left', {direct = true})\nshroud.keymap('g u', function() shroud.message{text = 'hi'} end, {name = 'hi'})\nshroud.keymap('g i', function() end)\nshroud.keymap('x', nil)\nreturn {}";
        let (mut h, out) = start(&[(plugin("keymap", "k", "", init), json!({}))], &[]);
        let lines = out.take();
        let binds: Vec<&Value> = lines.iter().filter(|l| l["cmd"] == "bind").collect();
        let shape: Vec<(&str, &Value, bool)> = binds
            .iter()
            .map(|b| (b["keys"].as_str().unwrap(), &b["command"], b["direct"].as_bool().unwrap()))
            .collect();
        assert_eq!(
            shape,
            [
                ("g y", &json!("zoom"), false),
                ("ctrl+h", &json!("focus-left"), true),
                ("g u", &json!("k.hi"), false),
                ("g i", &json!("k.keymap-1"), false),
                ("x", &Value::Null, false)
            ]
        );
        let registered: Vec<&Value> =
            lines.iter().filter(|l| l["cmd"] == "register_command").map(|l| &l["name"]).collect();
        assert_eq!(registered, [&json!("k.hi"), &json!("k.keymap-1")]);
        h.line(r#"{"event":"command","name":"k.hi","client":1,"session":"s"}"#);
        assert_eq!(find(&out.take(), "message")["text"], "hi");
    }

    #[test]
    fn keymap_and_run_pass_shortcut_tables_through() {
        let init = "shroud.keymap('g e', {run = 'nvim', open = 'right'})\nshroud.keymap('g P', {exec = {'git', 'pull'}})\nreturn {}";
        let (_, out) = start(&[(plugin("shortcut-keymap", "k", "", init), json!({}))], &[]);
        let lines = out.take();
        let binds: Vec<&Value> = lines.iter().filter(|l| l["cmd"] == "bind").map(|b| &b["command"]).collect();
        assert_eq!(binds, [&json!({"run": "nvim", "open": "right"}), &json!({"exec": ["git", "pull"]})]);
    }

    #[test]
    fn timers_run_their_function_as_the_plugin() {
        let init =
            "shroud.after(0, function() shroud.request{cmd = 'send', input = {{text = 'later'}}} end)\nreturn {}";
        let (mut h, out) = start(&[(plugin("timer", "t", "", init), json!({}))], &[]);
        out.take();
        assert!(h.next_timer().is_some());
        h.tick(Instant::now() + Duration::from_millis(1));
        let lines = out.take();
        assert_eq!(lines[0], json!({"cmd": "plugin_running", "plugin": "t"}));
        assert_eq!(find(&lines, "send")["input"], json!([{"text": "later"}]));
        assert!(h.next_timer().is_none());
    }

    #[test]
    fn a_stray_yield_is_reported_not_hung() {
        let (_, out) = start(&[(plugin("stray", "y", "", "coroutine.yield('nope')\nreturn {}"), json!({}))], &[]);
        assert!(
            find(&out.take(), "plugin_error")["message"].as_str().unwrap().contains("yielded without making a request")
        );
    }

    /// Answers the one request named `cmd` in what the host just wrote.
    pub(crate) fn answer(h: &mut Host, out: &Shared, cmd: &str, reply: Value) -> Vec<Value> {
        let lines = out.take();
        let id = find(&lines, cmd)["id"].as_u64().unwrap();
        let mut reply = reply;
        reply["id"] = id.into();
        reply["ok"] = true.into();
        h.line(&reply.to_string());
        lines
    }

    pub(crate) fn bindings() -> Value {
        let mut after: Vec<Value> = vec![
            json!({"keys": "v", "command": "vsplit"}),
            json!({"keys": "g t", "command": "next-tab"}),
            json!({"keys": "g T", "command": "prev-tab"}),
        ];
        after.extend((1..=9).map(|d| json!({"keys": d.to_string(), "command": format!("goto-tab {d}")})));
        json!({"leader": "ctrl+space", "after_leader": after, "direct": []})
    }

    #[test]
    fn status_events_reach_only_the_plugin_that_took_the_bar() {
        let draw = |who: &str| {
            format!(
                "shroud.on('status', function(ev) shroud.status_set{{client = ev.client, left = {{{{text = '{who} ' .. ev.mode}}}}}} end)\n"
            )
        };
        let a =
            plugin("bar", "a", "", &format!("{}return {{ setup = function() shroud.status_take() end }}", draw("a")));
        let b = plugin("bar", "b", "", &format!("{}return {{}}", draw("b")));
        let (mut h, out) = start(&[(a, json!({})), (b, json!({}))], &[]);
        let lines = out.take();
        assert!(!cmds(&lines).contains(&"subscribe"), "status needs no subscription");
        let take = find(&lines, "status_take").clone();
        h.line(&json!({"id": take["id"], "ok": true}).to_string());
        h.line(r#"{"event":"status","client":4,"mode":"leader","session":"s"}"#);
        let sets: Vec<Value> = out.take().into_iter().filter(|l| l["cmd"] == "status_set").collect();
        assert_eq!(sets.len(), 1);
        assert_eq!((sets[0]["client"].clone(), sets[0]["left"][0]["text"].clone()), (json!(4), json!("a leader")));
    }

    #[test]
    fn shroud_calls_outside_plugin_code_fail_cleanly() {
        let out = Shared::default();
        let h = Host::new(Box::new(out), None).unwrap();
        let err = h.lua.load("shroud.on('focus_changed', function() end)").exec().unwrap_err();
        assert!(err.to_string().contains("only work inside a plugin"));
    }
}
