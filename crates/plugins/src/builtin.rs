//! Plugins shipped inside the binary. They load through the same path as any other, so
//! config can turn them off or replace them.

use std::path::{Path, PathBuf};

pub struct Files {
    pub manifest: String,
    pub init: String,
    pub chunk_name: String,
    pub lua_dir: Option<PathBuf>,
}

impl Files {
    pub fn read(dir: &Path) -> Result<Files, String> {
        let init = dir.join("init.lua");
        Ok(Files {
            manifest: std::fs::read_to_string(dir.join("plugin.toml")).map_err(|e| format!("plugin.toml: {e}"))?,
            init: std::fs::read_to_string(&init).map_err(|e| format!("init.lua: {e}"))?,
            chunk_name: format!("@{}", init.display()),
            lua_dir: Some(dir.join("lua")),
        })
    }

    pub fn builtin(name: &str) -> Result<Files, String> {
        let (_, manifest, init) =
            BUILTIN.iter().find(|(n, _, _)| *n == name).ok_or_else(|| format!("no built-in plugin {name}"))?;
        Ok(Files {
            manifest: manifest.to_string(),
            init: init.to_string(),
            chunk_name: format!("@builtin/{name}/init.lua"),
            lua_dir: None,
        })
    }
}

pub const BUILTIN: &[(&str, &str, &str)] = &[
    ("hints", include_str!("../builtin/hints/plugin.toml"), include_str!("../builtin/hints/init.lua")),
    ("sessions", include_str!("../builtin/sessions/plugin.toml"), include_str!("../builtin/sessions/init.lua")),
];

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::host::Host;
    use crate::host::tests::{Shared, TestDir, answer, bindings, cmds, find, start_with};

    #[test]
    fn hints_show_the_next_keys_and_close_on_key_done() {
        let (mut h, out) = start_with(vec![json!({"builtin": "hints", "opts": {}})], &[], None);
        assert_eq!(cmds(&out.take()), vec!["subscribe"]);
        h.line(r#"{"event":"key_pending","client":1,"session":"s","keys":["ctrl+space"]}"#);
        answer(&mut h, &out, "bindings", bindings());
        let lines = out.take();
        let open = find(&lines, "overlay_open");
        assert_eq!((open["session"].as_str(), open["title"].as_str()), (Some("s"), Some("ctrl+space")));
        let text = open["lines"].to_string();
        for want in ["vsplit", "+more", "1-9", "goto-tab"] {
            assert!(text.contains(want), "{want} missing from {text}");
        }
        assert!(!text.contains("goto-tab 3"), "digits fold into one line");
        let id = open["id"].as_u64().unwrap();
        h.line(&json!({"id": id, "ok": true, "overlay": 5}).to_string());
        h.line(r#"{"event":"key_pending","client":1,"session":"s","keys":["ctrl+space","g"]}"#);
        answer(&mut h, &out, "bindings", bindings());
        let update = out.take();
        let update = find(&update, "overlay_update");
        assert!(update["lines"].to_string().contains("next-tab") && update["overlay"] == 5);
        h.line(&json!({"id": update["id"], "ok": true}).to_string());
        h.line(r#"{"event":"key_done","client":2}"#);
        assert!(cmds(&out.take()).is_empty(), "another client's key_done leaves it alone");
        h.line(r#"{"event":"key_done","client":1}"#);
        assert_eq!(find(&out.take(), "overlay_close")["overlay"], 5);
    }

    #[test]
    fn hints_never_open_for_a_sequence_that_already_ended() {
        let (mut h, out) = start_with(vec![json!({"builtin": "hints", "opts": {}})], &[], None);
        out.take();
        h.line(r#"{"event":"key_pending","client":1,"session":"s","keys":["ctrl+space"]}"#);
        let lines = out.take();
        h.line(r#"{"event":"key_done","client":1}"#);
        let id = find(&lines, "bindings")["id"].as_u64().unwrap();
        h.line(&json!({"id": id, "ok": true, "leader": "ctrl+space", "after_leader": [{"keys": "v", "command": "vsplit"}], "direct": []}).to_string());
        assert!(!cmds(&out.take()).contains(&"overlay_open"));
    }

    #[test]
    fn sessions_save_to_its_data_dir_and_restore_what_isnt_running() {
        let data = TestDir::new("sessions-data");
        let (mut h, out) = start_with(vec![json!({"builtin": "sessions", "opts": {}})], &[], Some(data.to_path_buf()));
        let lines = out.take();
        let keys: Vec<_> = lines
            .iter()
            .filter(|l| l["cmd"] == "register_command")
            .map(|l| (l["name"].clone(), l["key"].clone()))
            .collect();
        assert!(
            keys.contains(&(json!("sessions.save"), json!("g s")))
                && keys.contains(&(json!("sessions.restore"), json!("g r")))
        );

        let layout = json!({"root": {"split": "right", "parts": [{"cwd": "/tmp", "size": 50}, {"cwd": "/", "size": 50, "process": "nvim notes.md"}]}});
        h.line(r#"{"event":"command","name":"sessions.save","client":1,"session":"work","count":null}"#);
        answer(&mut h, &out, "list", json!({"sessions": [{"name": "work", "tabs": [{"active": true, "panes": []}]}]}));
        answer(&mut h, &out, "layout", json!({"layout": layout}));
        assert_eq!(find(&out.take(), "message")["text"], "saved 1 session");
        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(data.join("sessions/sessions.json")).unwrap()).unwrap();
        assert_eq!(saved["sessions"][0]["tabs"][0], layout);

        h.line(r#"{"event":"command","name":"sessions.restore","client":1,"session":"default","count":null}"#);
        answer(&mut h, &out, "list", json!({"sessions": [{"name": "default", "tabs": []}]}));
        let new = answer(&mut h, &out, "session_new", json!({"session": "work"}));
        assert_eq!(find(&new, "session_new")["cwd"], "/tmp");
        let applied = answer(&mut h, &out, "apply_layout", json!({"tab": 2}));
        assert_eq!(find(&applied, "apply_layout")["layout"], layout);
        let built = answer(
            &mut h,
            &out,
            "layout",
            json!({"layout": {"root": {"split": "right", "parts": [{"pane": 8}, {"pane": 9}]}}}),
        );
        assert_eq!(find(&built, "layout")["tab"], 2);
        let typed = answer(&mut h, &out, "send", json!({}));
        assert_eq!(find(&typed, "send")["pane"], 9);
        assert_eq!(find(&typed, "send")["input"], json!([{"text": "nvim notes.md"}]), "typed without Enter");
        answer(
            &mut h,
            &out,
            "list",
            json!({"sessions": [{"name": "work", "tabs": [{"active": true, "panes": [{"id": 7}]}, {"active": false, "panes": [{"id": 8}]}]}]}),
        );
        let closed = answer(&mut h, &out, "close", json!({}));
        assert_eq!(find(&closed, "close")["pane"], 7, "the tab session_new made goes");
        assert_eq!(find(&out.take(), "message")["text"], "restored 1 session");
    }

    #[test]
    fn auto_restore_replaces_the_first_clients_lone_shell_once_per_server() {
        let data = TestDir::new("auto-data");
        std::fs::create_dir_all(data.join("sessions")).unwrap();
        let layout = json!({"root": {"cwd": "/tmp"}});
        std::fs::write(
            data.join("sessions/sessions.json"),
            json!({"version": 1, "sessions": [{"name": "default", "tabs": [layout]}]}).to_string(),
        )
        .unwrap();
        let boot = |data: &TestDir| {
            let out = Shared::default();
            let mut h = Host::new(Box::new(out.clone()), Some(data.to_path_buf())).unwrap();
            h.hello("tok");
            out.take();
            let plugins = json!([{"builtin": "sessions", "opts": {"auto_restore": true}}]);
            h.line(
                &json!({"id": 0, "ok": true, "plugins": plugins, "server": {"pid": 42, "started": 1000}}).to_string(),
            );
            (h, out)
        };
        let (mut h, out) = boot(&data);
        answer(
            &mut h,
            &out,
            "list",
            json!({"sessions": [{"name": "default", "tabs": [{"active": true, "panes": [{"id": 1, "running": false}]}]}]}),
        );
        let applied = answer(&mut h, &out, "apply_layout", json!({"tab": 2}));
        assert_eq!(find(&applied, "apply_layout")["session"], "default");
        answer(&mut h, &out, "layout", json!({"layout": {"root": {"pane": 2}}}));
        assert!(!cmds(&applied).contains(&"session_new"), "the running session is reused");
        answer(
            &mut h,
            &out,
            "list",
            json!({"sessions": [{"name": "default", "tabs": [{"active": true, "panes": [{"id": 1}]}, {"active": false, "panes": [{"id": 2}]}]}]}),
        );
        assert_eq!(find(&out.take(), "close")["pane"], 1);
        drop(h);
        let (_h, out) = boot(&data);
        assert!(!cmds(&out.take()).contains(&"list"), "a restarted host doesn't restore again");
    }
}
