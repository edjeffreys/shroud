//! Where a `[[plugins]]` entry comes from and what `plugin sync` should do about it. The
//! git work happens in the binary; this only decides.

use super::lockfile::Locked;

/// Plugins inside the binary, and whether each is on when config doesn't mention it.
/// Key hints make the keys discoverable; restoring sessions is a matter of taste.
pub const BUILTIN: &[(&str, bool)] = &[("hints", true), ("sessions", false)];

#[derive(Clone, Debug, PartialEq)]
pub struct PluginSpec {
    pub src: Source,
    pub version: Option<String>,
    /// Handed to the plugin's `setup` unread.
    pub opts: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Path(String),
    Github(String),
    Git(String),
    Builtin(String),
}

impl Source {
    pub fn parse(src: &str) -> Result<Source, String> {
        let (kind, rest) = src
            .split_once(':')
            .filter(|(_, r)| !r.is_empty())
            .ok_or_else(|| format!("`{src}` needs a source kind: path:, github:, git: or builtin:"))?;
        Ok(match kind {
            "path" => Source::Path(rest.to_string()),
            "github" if rest.split('/').count() == 2 && !rest.starts_with('/') && !rest.ends_with('/') => {
                Source::Github(rest.to_string())
            }
            "github" => return Err(format!("`{src}`: github sources are owner/repo")),
            "git" => Source::Git(rest.to_string()),
            "builtin" if BUILTIN.iter().any(|(n, _)| *n == rest) => Source::Builtin(rest.to_string()),
            "builtin" => {
                return Err(format!(
                    "no built-in plugin `{rest}`; there are {}",
                    BUILTIN.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(" and ")
                ));
            }
            other => return Err(format!("unknown plugin source `{other}:`; use path:, github:, git: or builtin:")),
        })
    }
}

impl Source {
    /// The install directory's name, so two sources ending in the same repo name clash
    /// loudly at sync rather than overwrite each other.
    pub fn dir_name(&self) -> Option<&str> {
        let tail = match self {
            Source::Path(_) | Source::Builtin(_) => return None,
            Source::Github(r) => r,
            Source::Git(url) => url.trim_end_matches('/'),
        };
        let name = tail.rsplit(['/', ':']).next()?.trim_end_matches(".git");
        let ok = !name.is_empty() && name != "." && name != ".." && !name.starts_with('.');
        ok.then_some(name)
    }

    pub fn git_url(&self) -> Option<String> {
        match self {
            Source::Path(_) | Source::Builtin(_) => None,
            Source::Github(r) => Some(format!("https://github.com/{r}.git")),
            Source::Git(url) => Some(url.clone()),
        }
    }

    /// As config writes it, which is also the lockfile's key.
    pub fn spec(&self) -> String {
        match self {
            Source::Path(p) => format!("path:{p}"),
            Source::Github(r) => format!("github:{r}"),
            Source::Git(u) => format!("git:{u}"),
            Source::Builtin(n) => format!("builtin:{n}"),
        }
    }
}

/// What actually runs: config's entries, minus `enabled = false`, plus each default-on
/// built-in config doesn't mention. Listing a built-in is how to configure or turn it off.
pub fn effective(written: Vec<(PluginSpec, bool)>) -> Vec<PluginSpec> {
    let mentioned = |name: &str| written.iter().any(|(p, _)| p.src == Source::Builtin(name.to_string()));
    let defaults: Vec<PluginSpec> = BUILTIN
        .iter()
        .filter(|(name, on)| *on && !mentioned(name))
        .map(|(name, _)| PluginSpec {
            src: Source::Builtin(name.to_string()),
            version: None,
            opts: serde_json::json!({}),
        })
        .collect();
    written.into_iter().filter(|(_, enabled)| *enabled).map(|(p, _)| p).chain(defaults).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Want {
    /// The lockfile's commit, so every machine runs the same code.
    Commit(String),
    /// `version` (a tag, branch or commit), or the default branch for `None`.
    Resolve(Option<String>),
}

/// A pin counts only while config still asks for the version it was made from; `update`
/// ignores pins and always resolves.
pub fn want(spec: &PluginSpec, locked: Option<&Locked>, update: bool) -> Want {
    match locked {
        Some(l) if !update && l.version == spec.version => Want::Commit(l.commit.clone()),
        _ => Want::Resolve(spec.version.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(src: &str, version: Option<&str>) -> PluginSpec {
        PluginSpec {
            src: Source::parse(src).unwrap(),
            version: version.map(str::to_string),
            opts: serde_json::json!({}),
        }
    }

    #[test]
    fn install_dirs_and_urls_come_from_the_source() {
        assert_eq!(Source::parse("github:folke/hints").unwrap().dir_name(), Some("hints"));
        assert_eq!(
            Source::parse("github:folke/hints").unwrap().git_url().as_deref(),
            Some("https://github.com/folke/hints.git")
        );
        assert_eq!(Source::parse("git:https://example.com/x/sessions.git").unwrap().dir_name(), Some("sessions"));
        assert_eq!(Source::parse("git:git@example.com:me/marks.git/").unwrap().dir_name(), Some("marks"));
        assert_eq!(Source::parse("git:/tmp/repos/local").unwrap().dir_name(), Some("local"));
        assert_eq!(Source::parse("path:~/src/x").unwrap().dir_name(), None);
        for bad in ["git:https://example.com/..", "git:/tmp/.hidden"] {
            assert_eq!(Source::parse(bad).unwrap().dir_name(), None, "{bad}");
        }
        assert_eq!(Source::parse("github:a/b").unwrap().spec(), "github:a/b");
    }

    #[test]
    fn built_ins_are_on_by_default_unless_config_says_otherwise() {
        let names = |v: Vec<PluginSpec>| v.into_iter().map(|p| p.src.spec()).collect::<Vec<_>>();
        assert_eq!(names(effective(vec![])), vec!["builtin:hints"]);
        assert_eq!(names(effective(vec![(spec("builtin:hints", None), false)])), Vec::<String>::new());
        assert_eq!(
            names(effective(vec![(spec("builtin:sessions", None), true), (spec("path:x", None), false)])),
            vec!["builtin:sessions", "builtin:hints"]
        );
        assert!(Source::parse("builtin:nope").unwrap_err().contains("hints and sessions"));
        assert_eq!(Source::parse("builtin:hints").unwrap().dir_name(), None);
    }

    #[test]
    fn a_pin_holds_until_the_version_changes_or_update_runs() {
        let locked = Locked { src: "github:a/b".into(), version: Some("v1".into()), commit: "abc".into() };
        assert_eq!(want(&spec("github:a/b", Some("v1")), Some(&locked), false), Want::Commit("abc".into()));
        assert_eq!(want(&spec("github:a/b", Some("v2")), Some(&locked), false), Want::Resolve(Some("v2".into())));
        assert_eq!(want(&spec("github:a/b", Some("v1")), Some(&locked), true), Want::Resolve(Some("v1".into())));
        assert_eq!(want(&spec("github:a/b", None), None, false), Want::Resolve(None));
    }
}
