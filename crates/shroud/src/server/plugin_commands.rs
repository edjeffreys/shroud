//! Commands plugins register at runtime. Config can bind their names before any plugin
//! loads; a key whose plugin isn't there says so rather than doing nothing.

use serde_json::{Value, json};
use shroud_core::input::Chord;
use shroud_core::input::bindings::parse_sequence;
use shroud_core::input::commands::is_plugin_name;
use shroud_core::protocol::api::{self, Event};

use super::Server;

#[derive(Debug)]
pub struct PluginCommand {
    pub name: String,
    pub owner: u64,
    pub key: Option<Vec<Chord>>,
}

impl Server {
    pub(super) fn register_command(&mut self, owner: u64, name: String, key: Option<String>) -> Result<Value, String> {
        if !is_plugin_name(&name) {
            return Err(format!("`{name}` isn't a plugin command name: use plugin.command, lowercase"));
        }
        if self.plugin_commands.iter().any(|c| c.name == name && c.owner != owner) {
            return Err(format!("`{name}` is registered by another connection"));
        }
        let key = key.map(|k| parse_sequence(&k)).transpose()?;
        self.plugin_commands.retain(|c| c.name != name);
        self.plugin_commands.push(PluginCommand { name: name.clone(), owner, key: key.clone() });
        let bound = match key {
            Some(k) => self.settings.bindings.bind_plugin_default(k, &name),
            None => Ok(()),
        };
        Ok(match bound {
            Ok(()) => json!({}),
            Err(why) => json!({"key_conflict": why}),
        })
    }

    pub(super) fn drop_commands_of(&mut self, owner: u64) {
        self.plugin_commands.retain(|c| c.owner != owner);
    }

    pub(super) fn run_plugin_command(&mut self, client: usize, name: String, count: Option<u16>) {
        let id = self.clients[client].id;
        let owner = self.plugin_commands.iter().find(|c| c.name == name).map(|c| c.owner);
        let Some(session) = self.session_of(client) else { return };
        let (Some(owner), session_name) = (owner, session.name.clone()) else {
            session.status.say(format!("no plugin provides {name}"));
            return;
        };
        let line = api::event(&Event::Command { name, client: id, session: session_name, count });
        if let Some(c) = self.controls.iter_mut().find(|c| c.id == owner) {
            c.send(&line);
        }
    }

    pub(super) fn bindings_json(&self) -> Value {
        let b = &self.settings.bindings;
        let seq = |keys: &[Chord]| keys.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(" ");
        let leader: Vec<Value> =
            b.leader_entries().map(|(k, c)| json!({"keys": seq(k), "command": c.label()})).collect();
        let direct: Vec<Value> =
            b.direct_entries().map(|(k, c)| json!({"key": k.to_string(), "command": c.label()})).collect();
        json!({"leader": b.leader.to_string(), "after_leader": leader, "direct": direct})
    }
}
