//! `config.toml`. Spans come from toml's parse-only tables, so every problem has a line.

use toml::Spanned;
use toml::de::{DeTable, DeValue};

use super::{Config, PluginSpec, Problem, Source, effective};
use crate::input::bindings::parse_sequence;
use crate::input::chord::Chord;
use crate::input::commands::{Command, command};
use crate::input::shortcut::Shortcut;
use crate::theme::Theme;

/// `Err` means a syntax error, so the caller keeps its current config.
pub fn parse(text: &str) -> Result<(Config, Vec<Problem>), Problem> {
    let doc = DeTable::parse(text).map_err(|e| match e.span() {
        Some(span) => Problem::at(text, span.start, e.message()),
        None => Problem { line: None, message: e.message().to_string() },
    })?;
    let mut p = Parser { text, config: Config::default(), problems: Vec::new(), plugins: Vec::new() };
    for (key, value) in doc.get_ref() {
        match key.get_ref().as_ref() {
            "shell" => {
                if let Some(s) = p.string(value, "shell") {
                    p.config.shell = Some(s.to_string());
                }
            }
            "pane_frames" => match value.get_ref().as_bool() {
                Some(on) => p.config.pane_frames = on,
                None => p.problem(value, "`pane_frames` is true or false"),
            },
            "scrollback" => {
                match value.get_ref().as_integer().and_then(|i| u32::from_str_radix(i.as_str(), i.radix()).ok()) {
                    Some(mb) if mb <= 100_000 => p.config.scrollback_mb = Some(mb),
                    _ => p.problem(value, "`scrollback` is megabytes per pane, a whole number from 0 to 100000"),
                }
            }
            "leader" => {
                if let Some(s) = p.string(value, "leader") {
                    match Chord::parse(s) {
                        Ok(c) => p.config.bindings.leader = c,
                        Err(e) => p.problem(value, e),
                    }
                }
            }
            "theme" => {
                if let Some(t) = p.table(value, "theme") {
                    p.theme(t, "");
                }
            }
            "keys" => {
                if let Some(t) = p.table(value, "keys") {
                    p.keys(t);
                }
            }
            "plugins" => match value.get_ref().as_array() {
                Some(list) => list.iter().for_each(|v| p.plugin(v)),
                None => p.problem(value, "`plugins` is a list: write each as [[plugins]]"),
            },
            other => p.problem(key, format!("unknown setting `{other}`")),
        }
    }
    p.check_bindings();
    p.config.plugins = effective(std::mem::take(&mut p.plugins));
    // toml's map is sorted by key, not by position in the file.
    p.problems.sort_by_key(|pr| pr.line.unwrap_or(usize::MAX));
    Ok((p.config, p.problems))
}

struct Parser<'t> {
    text: &'t str,
    config: Config,
    problems: Vec<Problem>,
    /// As written, with `enabled`; built-in defaults are added at the end.
    plugins: Vec<(PluginSpec, bool)>,
}

impl Parser<'_> {
    fn problem<T>(&mut self, at: &Spanned<T>, message: impl Into<String>) {
        self.problems.push(Problem::at(self.text, at.span().start, message));
    }

    fn string<'v>(&mut self, value: &'v Spanned<DeValue<'_>>, what: &str) -> Option<&'v str> {
        let s = value.get_ref().as_str();
        if s.is_none() {
            self.problem(value, format!("`{what}` must be a string"));
        }
        s
    }

    fn table<'v, 'i>(&mut self, value: &'v Spanned<DeValue<'i>>, what: &str) -> Option<&'v DeTable<'i>> {
        let t = value.get_ref().as_table();
        if t.is_none() {
            self.problem(value, format!("`{what}` must be a table"));
        }
        t
    }

    // TOML reads `status.bg` as nested tables.
    fn theme(&mut self, table: &DeTable<'_>, prefix: &str) {
        for (key, value) in table {
            let name = format!("{prefix}{}", key.get_ref());
            if let Some(sub) = value.get_ref().as_table() {
                self.theme(sub, &format!("{name}."));
                continue;
            }
            let colour = match value.get_ref() {
                DeValue::String(s) => Theme::parse_color(s),
                DeValue::Integer(i) => i.as_str().parse::<i64>().map_err(|e| e.to_string()).and_then(Theme::palette),
                _ => Err("a colour is \"default\", a palette index or \"#rrggbb\"".to_string()),
            };
            match (self.config.theme.token_mut(&name), colour) {
                (None, _) => self.problem(key, format!("unknown theme token `{name}`")),
                (Some(_), Err(e)) => self.problem(value, format!("{name}: {e}")),
                (Some(slot), Ok(c)) => *slot = c,
            }
        }
    }

    fn keys(&mut self, table: &DeTable<'_>) {
        for (key, value) in table {
            let Some(binds) = self.table(value, key.get_ref()) else { continue };
            match key.get_ref().as_ref() {
                "leader" => {
                    for (keys, cmd) in binds {
                        let seq = match parse_sequence(keys.get_ref()) {
                            Ok(seq) => seq,
                            Err(e) => {
                                self.problem(keys, e);
                                continue;
                            }
                        };
                        if let Some(cmd) = self.command(cmd) {
                            self.config.bindings.bind_leader(seq, cmd);
                        }
                    }
                }
                "direct" => {
                    for (k, cmd) in binds {
                        let chord = match parse_sequence(k.get_ref()) {
                            Ok(seq) if seq.len() == 1 => seq[0],
                            Ok(_) => {
                                self.problem(k, "direct keys are single keys, not sequences");
                                continue;
                            }
                            Err(e) => {
                                self.problem(k, e);
                                continue;
                            }
                        };
                        if let Some(cmd) = self.command(cmd) {
                            self.config.bindings.bind_direct(chord, cmd);
                        }
                    }
                }
                other => {
                    self.problem(key, format!("unknown key table `keys.{other}`; use `keys.leader` or `keys.direct`"))
                }
            }
        }
    }

    /// `Some(None)` is `"none"`, which unbinds. A table is a shortcut: `{ run = "nvim" }`.
    fn command(&mut self, value: &Spanned<DeValue<'_>>) -> Option<Option<Command>> {
        if let DeValue::Table(_) = value.get_ref() {
            let parsed = json(value.get_ref()).and_then(|v| Shortcut::from_json(&v));
            return match parsed {
                Ok(s) => Some(Some(Command::Shortcut(s))),
                Err(e) => {
                    self.problem(value, e);
                    None
                }
            };
        }
        let Some(name) = value.get_ref().as_str() else {
            self.problem(value, "a binding is a command name or a shortcut table like { run = \"nvim\" }");
            return None;
        };
        if name == "none" {
            return Some(None);
        }
        let found = command(name);
        if found.is_none() {
            self.problem(value, format!("unknown command `{name}`"));
        }
        found.map(Some)
    }

    fn plugin(&mut self, value: &Spanned<DeValue<'_>>) {
        let Some(table) = self.table(value, "a [[plugins]] entry") else { return };
        let (mut src, mut version, mut opts, mut enabled) =
            (None, None, serde_json::Value::Object(Default::default()), true);
        for (key, v) in table {
            match key.get_ref().as_ref() {
                "src" => src = self.string(v, "src").map(|s| (s.to_string(), v.span().start)),
                "version" => version = self.string(v, "version").map(str::to_string),
                "enabled" => match v.get_ref() {
                    DeValue::Boolean(b) => enabled = *b,
                    _ => self.problem(v, "`enabled` must be true or false"),
                },
                "opts" => match v.get_ref() {
                    DeValue::Table(_) => {
                        opts = json(v.get_ref()).unwrap_or_else(|e| {
                            self.problem(v, e);
                            opts.clone()
                        })
                    }
                    _ => self.problem(v, "`opts` must be a table"),
                },
                other => self.problem(key, format!("unknown plugin setting `{other}`")),
            }
        }
        let Some((src, at)) = src else {
            self.problem(value, "a [[plugins]] entry needs `src`");
            return;
        };
        match Source::parse(&src) {
            Ok(src) if self.plugins.iter().any(|(p, _)| p.src == src) => {
                self.problems.push(Problem::at(self.text, at, format!("`{}` is listed twice", src.spec())))
            }
            Ok(src) => self.plugins.push((PluginSpec { src, version, opts }, enabled)),
            Err(e) => self.problems.push(Problem::at(self.text, at, e)),
        }
    }

    fn check_bindings(&mut self) {
        let b = &self.config.bindings;
        let mut found: Vec<String> =
            b.shadowed().iter().map(|(short, long)| format!("`{}` hides `{}`", spec(short), spec(long))).collect();
        if b.direct(b.leader).is_some() {
            found.push("a direct key is also the leader, so it never fires".into());
        }
        self.problems.extend(found.into_iter().map(|message| Problem { line: None, message }));
    }
}

fn json(v: &DeValue<'_>) -> Result<serde_json::Value, String> {
    use serde_json::Value;
    Ok(match v {
        DeValue::String(s) => Value::String(s.to_string()),
        DeValue::Integer(i) => {
            i64::from_str_radix(&i.as_str().replace('_', ""), i.radix()).map_err(|e| e.to_string())?.into()
        }
        DeValue::Float(f) => {
            let n: f64 = f
                .as_str()
                .replace('_', "")
                .parse()
                .map_err(|_| format!("`{}` can't be passed to a plugin", f.as_str()))?;
            serde_json::Number::from_f64(n).map(Value::Number).ok_or("inf and nan can't be passed to a plugin")?
        }
        DeValue::Boolean(b) => Value::Bool(*b),
        DeValue::Datetime(d) => Value::String(d.to_string()),
        DeValue::Array(a) => Value::Array(a.iter().map(|x| json(x.get_ref())).collect::<Result<_, _>>()?),
        DeValue::Table(t) => Value::Object(
            t.iter().map(|(k, x)| Ok((k.get_ref().to_string(), json(x.get_ref())?))).collect::<Result<_, String>>()?,
        ),
    })
}

fn spec(keys: &[Chord]) -> String {
    keys.iter().map(|c| format!("{c}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::bindings::Lookup;
    use crate::input::keybinds::Action;
    use crate::layout::{Axis, Direction};
    use crate::render::color::Color;

    fn ok(text: &str) -> (Config, Vec<Problem>) {
        parse(text).expect("parses")
    }

    fn leader_action(c: &Config, keys: &str) -> Option<Action> {
        match c.bindings.after_leader(&parse_sequence(keys).unwrap()) {
            Lookup::Found(Command::Builtin { build, .. }) => Some(build(None)),
            _ => None,
        }
    }

    #[test]
    fn an_empty_file_is_the_defaults() {
        let (c, problems) = ok("");
        assert!(problems.is_empty());
        assert_eq!(c.theme, Theme::default());
        assert_eq!(leader_action(&c, "v"), Some(Action::Split(Axis::Horizontal)));
    }

    #[test]
    fn scrollback_is_megabytes_and_defaults_to_ghosttys() {
        assert_eq!(ok("").0.scrollback_bytes(), 10_000_000);
        let (c, problems) = ok("scrollback = 0x40\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(c.scrollback_bytes(), 64_000_000);
        for bad in ["-1", "\"10MB\"", "200000"] {
            let (c, problems) = ok(&format!("scrollback = {bad}\n"));
            assert_eq!((c.scrollback_mb, problems.len()), (None, 1), "{bad}");
        }
    }

    #[test]
    fn theme_tokens_take_both_spellings() {
        let (c, problems) =
            ok("[theme]\n\"border.active\" = \"#010203\"\nstatus.bg = 0\nbadge = { error = \"default\" }\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(c.theme.border_active, Color::Rgb(1, 2, 3));
        assert_eq!(c.theme.status_bg, Color::Palette(0));
        assert_eq!(c.theme.badge_error, Color::Default);
    }

    #[test]
    fn a_bad_value_keeps_that_default_and_says_where() {
        let (c, problems) = ok("[theme]\nstatus.fg = 4\nstatus.bg = 300\nborder.nope = 1\n");
        assert_eq!(c.theme.status_fg, Color::Palette(4));
        assert_eq!(c.theme.status_bg, Theme::default().status_bg);
        let lines: Vec<_> = problems.iter().map(|p| p.line).collect();
        assert_eq!(lines, vec![Some(3), Some(4)]);
        assert!(problems[1].message.contains("border.nope"));
    }

    #[test]
    fn keys_merge_over_the_defaults() {
        let text = "leader = \"ctrl+a\"\n[keys.leader]\n\"g t\" = \"close\"\nv = \"none\"\n[keys.direct]\n\"ctrl+h\" = \"focus-left\"\n";
        let (c, problems) = ok(text);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(c.bindings.leader, Chord::parse("ctrl+a").unwrap());
        assert_eq!(leader_action(&c, "g t"), Some(Action::Close));
        assert_eq!(leader_action(&c, "v"), None);
        assert_eq!(leader_action(&c, "s"), Some(Action::Split(Axis::Vertical)));
        assert_eq!(
            c.bindings.direct(Chord::parse("ctrl+h").unwrap()).and_then(|b| match b {
                Command::Builtin { build, .. } => Some(build(None)),
                _ => None,
            }),
            Some(Action::Focus(Direction::Left))
        );
    }

    #[test]
    fn binding_problems_are_reported() {
        let text = "[keys.leader]\nz = \"fly\"\n\"ctrl+nope\" = \"close\"\ng = \"close\"\n[keys.direct]\n\"a b\" = \"close\"\n[keys.other]\n";
        let (_, problems) = ok(text);
        let msgs: Vec<_> = problems.iter().map(|p| p.to_string()).collect();
        assert!(msgs.iter().any(|m| m.starts_with("line 2:") && m.contains("fly")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.starts_with("line 3:") && m.contains("nope")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.starts_with("line 6:") && m.contains("single keys")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("keys.other")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("`g` hides `g t`")), "{msgs:?}");
    }

    #[test]
    fn a_table_binds_a_shortcut() {
        use crate::input::shortcut::{Open, Program};
        let text = "[keys.leader]\n\"g e\" = { run = \"nvim\" }\n\"g P\" = { exec = [\"git\", \"pull\"] }\n[keys.direct]\n\"alt+k\" = { run = \"k9s\", open = \"tab\" }\n";
        let (c, problems) = ok(text);
        assert!(problems.is_empty(), "{problems:?}");
        let at = |keys: &str| match c.bindings.after_leader(&parse_sequence(keys).unwrap()) {
            Lookup::Found(Command::Shortcut(s)) => s,
            other => panic!("{other:?}"),
        };
        assert_eq!(at("g e"), Shortcut::Run { command: "nvim".into(), open: Open::Float, keep: false, cwd: None });
        assert!(matches!(at("g P"), Shortcut::Exec { program: Program::Argv(a), .. } if a == ["git", "pull"]));
        assert!(matches!(
            c.bindings.direct(Chord::parse("alt+k").unwrap()),
            Some(Command::Shortcut(Shortcut::Run { open: Open::Tab, .. }))
        ));
    }

    #[test]
    fn a_bad_shortcut_costs_only_its_key() {
        let text = "[keys.leader]\n\"g e\" = { run = \"nvim\", open = \"left\" }\n\"g k\" = 3\nv = \"vsplit\"\n";
        let (c, problems) = ok(text);
        let msgs: Vec<_> = problems.iter().map(|p| p.to_string()).collect();
        assert!(msgs.iter().any(|m| m.starts_with("line 2:") && m.contains("\"left\"")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.starts_with("line 3:") && m.contains("shortcut table")), "{msgs:?}");
        assert!(matches!(c.bindings.after_leader(&parse_sequence("g e").unwrap()), Lookup::Unbound));
        assert_eq!(leader_action(&c, "v"), Some(Action::Split(Axis::Horizontal)));
    }

    #[test]
    fn syntax_errors_are_err_with_a_line() {
        let e = parse("shell = \"zsh\"\n[theme\n").unwrap_err();
        assert_eq!(e.line, Some(2));
    }

    #[test]
    fn plugins_keep_their_source_version_and_opts() {
        let text = "[[plugins]]\nsrc = \"path:~/src/harpoon\"\nversion = \"v1.2.0\"\n[plugins.opts]\nmarks = 4\nnames = [\"a\", \"b\"]\nnested = { on = true, ratio = 0.5 }\n[[plugins]]\nsrc = \"github:someone/hints\"\n";
        let (c, problems) = ok(text);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(c.plugins[0].src, Source::Path("~/src/harpoon".into()));
        assert_eq!(c.plugins[0].version.as_deref(), Some("v1.2.0"));
        assert_eq!(
            c.plugins[0].opts,
            serde_json::json!({"marks": 4, "names": ["a", "b"], "nested": {"on": true, "ratio": 0.5}})
        );
        assert_eq!(c.plugins[1].src, Source::Github("someone/hints".into()));
        assert_eq!(c.plugins[1].opts, serde_json::json!({}));
    }

    #[test]
    fn bad_plugin_entries_are_skipped_with_a_line() {
        let text = "[[plugins]]\nversion = \"1\"\n[[plugins]]\nsrc = \"ftp:x\"\n[[plugins]]\nsrc = \"github:nope\"\ncolour = 1\n";
        let (c, problems) = ok(text);
        assert_eq!(c.plugins.len(), 1, "only the default hints");
        let msgs: Vec<_> = problems.iter().map(|p| p.to_string()).collect();
        assert!(msgs.iter().any(|m| m.contains("needs `src`")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.starts_with("line 4:") && m.contains("ftp:")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.starts_with("line 6:") && m.contains("owner/repo")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.starts_with("line 7:") && m.contains("colour")), "{msgs:?}");
        assert!(ok("plugins = 3\n").1[0].message.contains("[[plugins]]"));
        assert!(
            ok("[[plugins]]\nsrc = \"path:x\"\n[[plugins]]\nsrc = \"path:x\"\n").1[0].message.contains("listed twice")
        );
        assert!(ok("[[plugins]]\nsrc = \"path:x\"\nenabled = 1\n").1[0].message.contains("true or false"));
    }

    #[test]
    fn unknown_settings_and_wrong_types() {
        let (c, problems) = ok("shel = \"zsh\"\nshell = 3\n");
        assert_eq!(c.shell, None);
        assert_eq!(problems.len(), 2);
    }

    #[test]
    fn pane_frames_is_a_switch() {
        assert!(ok("pane_frames = true\n").0.pane_frames);
        assert!(!ok("").0.pane_frames);
        assert!(ok("pane_frames = 1\n").1[0].message.contains("true or false"));
    }
}
