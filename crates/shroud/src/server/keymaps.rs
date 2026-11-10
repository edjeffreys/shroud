//! Runtime binds vanish with their connection, so the table is rebuilt from config rather than
//! edited in place.

use serde_json::{Value, json};
use shroud_core::input::Chord;
use shroud_core::input::Shortcut;
use shroud_core::input::bindings::parse_sequence;
use shroud_core::input::commands::{Command, command};
use shroud_core::protocol::api::Target;

use super::Server;

#[derive(Debug)]
pub struct Keymap {
    owner: u64,
    keys: Vec<Chord>,
    direct: bool,
    command: Option<Command>,
}

/// `Ok(None)` is `"none"`, which unbinds. A table is a shortcut, read as config.toml reads one.
pub(super) fn target_command(target: Target) -> Result<Option<Command>, String> {
    match target {
        Target::Name(spec) if spec == "none" => Ok(None),
        Target::Name(spec) => command(&spec).map(Some).ok_or_else(|| format!("unknown command `{spec}`")),
        Target::Shortcut(table) => Shortcut::from_json(&Value::Object(table)).map(|s| Some(Command::Shortcut(s))),
    }
}

impl Server {
    pub(super) fn bind(
        &mut self,
        owner: u64,
        keys: &str,
        target: Option<Target>,
        direct: bool,
    ) -> Result<Value, String> {
        let keys = parse_sequence(keys)?;
        if direct && keys.len() != 1 {
            return Err("direct keys are single keys, not sequences".into());
        }
        let command = match target {
            None => None,
            Some(t) => target_command(t)?,
        };
        self.keymaps.retain(|k| !(k.keys == keys && k.direct == direct));
        self.keymaps.push(Keymap { owner, keys, direct, command });
        self.rebuild_bindings();
        Ok(json!({}))
    }

    pub(super) fn rebuild_bindings(&mut self) {
        self.settings.bindings = self.config_bindings.clone();
        for c in &self.plugin_commands {
            if let Some(k) = &c.key {
                let _ = self.settings.bindings.bind_plugin_default(k.clone(), &c.name);
            }
        }
        for k in &self.keymaps {
            if k.direct {
                self.settings.bindings.bind_direct(k.keys[0], k.command.clone());
            } else {
                self.settings.bindings.bind_leader(k.keys.clone(), k.command.clone());
            }
        }
    }

    pub(super) fn drop_keymaps_of(&mut self, owner: u64) {
        let before = self.keymaps.len();
        self.keymaps.retain(|k| k.owner != owner);
        if self.keymaps.len() != before {
            self.rebuild_bindings();
        }
    }
}
