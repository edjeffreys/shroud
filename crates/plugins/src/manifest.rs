//! `plugin.toml`. Read before any of the plugin's Lua runs, so a broken plugin is refused cheaply.

use std::collections::BTreeMap;

use serde::Deserialize;

/// While the major is 0, a minor bump may break plugins.
pub const API: (u32, u32) = (0, 1);

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub api: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub commands: BTreeMap<String, CommandDecl>,
}

/// Declared here rather than in Lua so a key is known before the plugin runs.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandDecl {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

pub fn parse(text: &str) -> Result<Manifest, String> {
    let m: Manifest = toml::from_str(text).map_err(|e| e.message().to_string())?;
    if !is_name(&m.name) {
        return Err(format!("`{}` isn't a plugin name: use lowercase letters, digits, - and _", m.name));
    }
    if let Some(bad) = m.commands.keys().find(|c| !is_name(c)) {
        return Err(format!("`{bad}` isn't a command name: use lowercase letters, digits, - and _"));
    }
    compatible(&m.api)?;
    Ok(m)
}

pub fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn compatible(wanted: &str) -> Result<(), String> {
    let bad = || format!("api `{wanted}` should look like \"{}.{}\"", API.0, API.1);
    let mut parts = wanted.split('.').map(|p| p.parse::<u32>().map_err(|_| bad()));
    let (major, minor) = (parts.next().ok_or_else(bad)??, parts.next().transpose()?.unwrap_or(0));
    let breaks = major != API.0 || (major == 0 && minor != API.1) || minor > API.1;
    if breaks {
        return Err(format!("needs plugin API {wanted}, but this shroud has {}.{}", API.0, API.1));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_declares_commands_and_their_keys() {
        let m = parse(
            "name = \"hello\"\nversion = \"1.0.0\"\napi = \"0.1\"\n[commands]\ngreet = { key = \"g y\" }\nquiet = {}\n",
        )
        .unwrap();
        assert_eq!(m.name, "hello");
        assert_eq!(m.commands["greet"].key.as_deref(), Some("g y"));
        assert_eq!(m.commands["quiet"].key, None);
    }

    #[test]
    fn bad_manifests_say_why() {
        let base = |extra: &str| format!("version = \"1\"\n{extra}");
        assert!(parse(&base("name = \"Hello\"\napi = \"0.1\"")).unwrap_err().contains("plugin name"));
        assert!(parse(&base("name = \"h\"\napi = \"0.2\"")).unwrap_err().contains("needs plugin API 0.2"));
        assert!(parse(&base("name = \"h\"\napi = \"1.0\"")).unwrap_err().contains("needs plugin API"));
        assert!(parse(&base("name = \"h\"\napi = \"zero\"")).unwrap_err().contains("should look like"));
        assert!(parse(&base("name = \"h\"\napi = \"0.1\"\ncolour = 1")).unwrap_err().contains("colour"));
        assert!(
            parse(&base("name = \"h\"\napi = \"0.1\"\n[commands]\n\"a.b\" = {}")).unwrap_err().contains("command name")
        );
    }
}
