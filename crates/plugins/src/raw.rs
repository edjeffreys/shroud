//! The calls behind the `shroud` table. Each finds its plugin from whoever is running, so
//! modules a plugin requires need no handle of their own.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use mlua::{Function, Lua, LuaSerdeExt, Value as LuaValue};
use serde_json::{Value, json};

use crate::manifest;
use crate::state::{State, Timer};
use crate::system::Spec;
use rustix::process::Signal;

// Events every connection gets; the rest need a subscription.
const UNSUBSCRIBED: &[&str] = &["command", "overlay_key", "overlay_closed", "status"];

pub fn ser_options() -> mlua::serde::SerializeOptions {
    mlua::serde::SerializeOptions::new().serialize_none_to_null(false).serialize_unit_to_null(false)
}

pub fn raw_api(lua: &Lua, state: &Rc<RefCell<State>>) -> mlua::Result<mlua::Table> {
    let raw = lua.create_table()?;

    let st = state.clone();
    raw.set(
        "send",
        lua.create_function(move |lua, req: mlua::Table| {
            let mut s = st.borrow_mut();
            s.current()?;
            let v: Value = lua.from_value(LuaValue::Table(req))?;
            let cmd = v.get("cmd").and_then(Value::as_str).unwrap_or_default().to_string();
            let id = s.send(v);
            s.sent.insert(id, cmd);
            Ok(id)
        })?,
    )?;

    let st = state.clone();
    raw.set(
        "on",
        lua.create_function(move |_, (event, func): (String, Function)| {
            let mut s = st.borrow_mut();
            let plugin = s.current()?;
            if !UNSUBSCRIBED.contains(&event.as_str()) && !s.subscribed {
                s.subscribed = true;
                s.request(json!({"cmd": "subscribe"}), None);
            }
            s.handlers.entry(event).or_default().push((plugin, func));
            Ok(())
        })?,
    )?;

    let st = state.clone();
    raw.set(
        "command",
        lua.create_function(move |_, (name, func): (String, Function)| {
            let mut s = st.borrow_mut();
            let plugin = s.current()?;
            if !manifest::is_name(&name) {
                return Err(mlua::Error::runtime(format!(
                    "`{name}` isn't a command name: use lowercase letters, digits, - and _"
                )));
            }
            let full = format!("{}.{name}", s.plugins[plugin].name);
            if s.plugins[plugin].commands.insert(name) {
                s.request(json!({"cmd": "register_command", "name": full}), Some(plugin));
            }
            s.commands.insert(full, (plugin, func));
            Ok(())
        })?,
    )?;

    // A function becomes a command of the plugin's own, so keys only ever name commands.
    let st = state.clone();
    raw.set(
        "keymap",
        lua.create_function(move |lua, (keys, target, opts): (String, LuaValue, Option<mlua::Table>)| {
            let mut s = st.borrow_mut();
            let plugin = s.current()?;
            let direct = opts.as_ref().map(|o| o.get::<Option<bool>>("direct")).transpose()?.flatten().unwrap_or(false);
            let command = match target {
                LuaValue::Nil => None,
                LuaValue::String(spec) => Some(Value::String(spec.to_str()?.to_string())),
                // A shortcut, `{ run = "nvim" }`: the server checks it, as it does config.toml's.
                LuaValue::Table(t) => Some(lua.from_value::<Value>(LuaValue::Table(t))?),
                LuaValue::Function(func) => {
                    let name = match opts.as_ref().map(|o| o.get::<Option<String>>("name")).transpose()?.flatten() {
                        Some(n) => n,
                        None => {
                            s.keymaps += 1;
                            format!("keymap-{}", s.keymaps)
                        }
                    };
                    if !manifest::is_name(&name) {
                        return Err(mlua::Error::runtime(format!(
                            "`{name}` isn't a command name: use lowercase letters, digits, - and _"
                        )));
                    }
                    let full = format!("{}.{name}", s.plugins[plugin].name);
                    if s.plugins[plugin].commands.insert(name) {
                        s.request(json!({"cmd": "register_command", "name": full}), Some(plugin));
                    }
                    s.commands.insert(full.clone(), (plugin, func));
                    Some(Value::String(full))
                }
                _ => {
                    return Err(mlua::Error::runtime(
                        "a keymap's target is a command name, a shortcut table, a function, or nil to unbind",
                    ));
                }
            };
            s.request(json!({"cmd": "bind", "keys": keys, "command": command, "direct": direct}), Some(plugin));
            Ok(())
        })?,
    )?;

    let st = state.clone();
    raw.set(
        "after",
        lua.create_function(move |_, (ms, func): (u64, Function)| {
            let mut s = st.borrow_mut();
            let plugin = s.current()?;
            s.timers.push(Timer { at: Instant::now() + Duration::from_millis(ms), plugin, func });
            Ok(())
        })?,
    )?;

    let st = state.clone();
    raw.set(
        "log",
        lua.create_function(move |_, msg: String| {
            let s = st.borrow();
            let name = s.current.map_or("?", |p| s.plugins[p].name.as_str());
            eprintln!("shroud: plugin {name}: {msg}");
            Ok(())
        })?,
    )?;

    let st = state.clone();
    raw.set(
        "data_dir",
        lua.create_function(move |_, ()| {
            let s = st.borrow();
            let plugin = s.current()?;
            let root = s
                .data_root
                .as_ref()
                .ok_or_else(|| mlua::Error::runtime("no data directory: set HOME or XDG_DATA_HOME"))?;
            let dir = root.join(&s.plugins[plugin].name);
            std::fs::create_dir_all(&dir).map_err(mlua::Error::external)?;
            Ok(dir.to_string_lossy().to_string())
        })?,
    )?;

    let st = state.clone();
    raw.set(
        "plugins",
        lua.create_function(move |lua, list: mlua::Table| {
            let mut s = st.borrow_mut();
            s.current()?;
            for spec in list.sequence_values::<LuaValue>() {
                match spec? {
                    LuaValue::String(src) => {
                        let t = lua.create_table()?;
                        t.set(1, src)?;
                        s.specs.push(t);
                    }
                    LuaValue::Table(t) => s.specs.push(t),
                    _ => return Err(mlua::Error::runtime("each plugin is a source string or a table")),
                }
            }
            Ok(())
        })?,
    )?;

    let st = state.clone();
    raw.set(
        "system",
        lua.create_function(move |_, (cmd, opts, on_exit): (Vec<String>, Option<mlua::Table>, Option<Function>)| {
            let mut spec = Spec { argv: cmd, ..Spec::default() };
            if let Some(o) = opts {
                spec.cwd = o.get("cwd")?;
                spec.env = o.get::<Option<_>>("env")?.unwrap_or_default();
                spec.stdin = o.get("stdin")?;
                spec.timeout = o.get::<Option<u64>>("timeout")?.map(Duration::from_millis);
            }
            let mut s = st.borrow_mut();
            let plugin = s.current()?;
            let id = s.fresh_id();
            let pid =
                s.jobs.start(id, spec).map_err(|e| mlua::Error::runtime(format!("couldn't start the command: {e}")))?;
            if let Some(f) = on_exit {
                s.on_exit.insert(id, (plugin, f));
            }
            Ok((id, pid))
        })?,
    )?;

    let st = state.clone();
    raw.set(
        "system_result",
        lua.create_function(move |lua, id: u64| {
            let mut s = st.borrow_mut();
            if s.on_exit.contains_key(&id) {
                return Err(mlua::Error::runtime(
                    "this job has an on_exit, which gets its result; don't also wait for it",
                ));
            }
            match s.finished.remove(&id) {
                Some(v) => lua.to_value_with(&v, ser_options()),
                None => Ok(LuaValue::Nil),
            }
        })?,
    )?;

    // Waiting is a request whose reply is the job's result.
    let st = state.clone();
    raw.set(
        "system_wait",
        lua.create_function(move |_, id: u64| {
            let mut s = st.borrow_mut();
            s.current()?;
            s.sent.insert(id, "system".into());
            Ok(id)
        })?,
    )?;

    let st = state.clone();
    raw.set(
        "system_kill",
        lua.create_function(move |_, (id, signal): (u64, Option<String>)| {
            let signal =
                match signal.as_deref().unwrap_or("term").trim_start_matches("sig").to_ascii_lowercase().as_str() {
                    "term" => Signal::TERM,
                    "kill" => Signal::KILL,
                    "int" => Signal::INT,
                    "hup" => Signal::HUP,
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "unknown signal `{other}`: use term, kill, int or hup"
                        )));
                    }
                };
            Ok(st.borrow().jobs.kill(id, signal))
        })?,
    )?;

    let json = lua.create_table()?;
    json.set(
        "encode",
        lua.create_function(|lua, v: LuaValue| {
            let v: Value = lua.from_value(v)?;
            Ok(v.to_string())
        })?,
    )?;
    json.set(
        "decode",
        lua.create_function(|lua, text: String| {
            let v: Value = serde_json::from_str(&text).map_err(mlua::Error::external)?;
            lua.to_value_with(&v, ser_options())
        })?,
    )?;
    raw.set("json", json)?;

    Ok(raw)
}
