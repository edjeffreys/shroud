//! `shroud plugin …`. git runs here, in the CLI, never in the server, so a slow network
//! can't stall a session.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use shroud_core::config::{Lock, Locked, PluginSpec, Source, Want, parse, want};
use shroud_core::protocol::api::Request;

use crate::client::api::ask;
use crate::{config_file, layout_child};

pub const USAGE: &str = "  shroud plugin sync                 install missing plugins, at their locked commits
  shroud plugin update [<name>...]   move plugins to the newest commit of their version and relock
  shroud plugin ls                   list configured plugins and their state
  shroud plugin clean                delete installed plugins config no longer lists
  shroud plugin types                print the Lua API's definitions for lua-language-server
";

pub fn run(args: &[&str], control: &Path) -> io::Result<i32> {
    match args {
        ["types"] => {
            print!("{}", shroud_plugins::TYPES);
            Ok(0)
        }
        ["sync"] => sync(control, None),
        ["update", names @ ..] => sync(control, Some(names)),
        ["ls"] => ls(control),
        ["clean"] => clean(),
        _ => {
            eprintln!("usage:\n{USAGE}");
            Ok(2)
        }
    }
}

struct Setup {
    plugins: Vec<PluginSpec>,
    root: PathBuf,
    lock_path: PathBuf,
}

fn setup() -> Result<Setup, String> {
    let path = config_file::settings_path().ok_or("no HOME or XDG_CONFIG_HOME, so there is no config")?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let (config, problems) = parse(&text).map_err(|p| format!("{}: {p}", path.display()))?;
    for p in problems {
        eprintln!("shroud: {}: {p}", path.display());
    }
    let root = config_file::data_dir().ok_or("no HOME or XDG_DATA_HOME for installed plugins")?.join("plugins");
    let lock_path = config_file::lock_path().ok_or("no config directory for plugins.lock")?;
    let mut plugins = config.plugins;
    // As in the plugin host: a spec in init.lua replaces config.toml's entry for its source.
    for (src, version, enabled) in lua_specs()? {
        plugins.retain(|p| p.src != src);
        if enabled {
            plugins.push(PluginSpec { src, version, opts: serde_json::json!({}) });
        }
    }
    Ok(Setup { plugins, root, lock_path })
}

/// Runs init.lua in a child, which is killed if it takes too long just to name its plugins.
fn lua_specs() -> Result<Vec<(Source, Option<String>, bool)>, String> {
    let Some(init) = config_file::init_path().filter(|p| p.is_file()) else { return Ok(Vec::new()) };
    let fail = |e: io::Error| format!("{}: {e}", init.display());
    let mut child = Command::new(std::env::current_exe().map_err(fail)?)
        .args(["__plugins", "--specs"])
        .arg(&init)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(fail)?;
    let reply = layout_child::talk(&mut child, "");
    let _ = child.kill();
    let _ = child.wait();
    let json = reply
        .map_err(fail)?
        .ok_or_else(|| format!("{} took over a second just to name its plugins", init.display()))?;
    let v: serde_json::Value = serde_json::from_slice(&json).map_err(|e| format!("{}: {e}", init.display()))?;
    if let Some(e) = v["error"].as_str() {
        return Err(format!("{}: {e}", init.display()));
    }
    let mut out = Vec::new();
    for s in v.as_array().into_iter().flatten() {
        let src =
            Source::parse(s["src"].as_str().unwrap_or_default()).map_err(|e| format!("{}: {e}", init.display()))?;
        out.push((src, s["version"].as_str().map(str::to_string), s["enabled"].as_bool().unwrap_or(true)));
    }
    Ok(out)
}

/// `update` is `None` for sync, else the names to update (none means all).
fn sync(control: &Path, update: Option<&[&str]>) -> io::Result<i32> {
    let s = match setup() {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    let old = read_lock(&s.lock_path);
    let mut lock = Lock::default();
    let mut bad = false;
    let clashes = clashing_dirs(&s.plugins);
    for spec in &s.plugins {
        let src = spec.src.spec();
        let Some(url) = spec.src.git_url() else { continue };
        let Some(name) = spec.src.dir_name().filter(|n| !clashes.contains(*n)) else {
            println!("{src}: can't pick an install directory; another plugin has the same repository name");
            bad = true;
            continue;
        };
        let renew = update.is_some_and(|names| names.is_empty() || names.contains(&name));
        match install(spec, &url, &s.root.join(name), old.get(&src), renew) {
            Ok((commit, what)) => {
                println!("{name}: {what} at {}", short(&commit));
                lock.plugins.push(Locked { src, version: spec.version.clone(), commit });
            }
            Err(e) => {
                println!("{name}: {e}");
                bad = true;
                lock.plugins.extend(old.get(&src).cloned());
            }
        }
    }
    if update.is_some_and(|names| names.iter().any(|n| !s.plugins.iter().any(|p| p.src.dir_name() == Some(n)))) {
        println!("some names given aren't git plugins in config.toml");
        bad = true;
    }
    if lock != old {
        if let Some(dir) = s.lock_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&s.lock_path, lock.write())?;
    }
    if ask(control, &Request::PluginsReload).is_some() {
        println!("restarted the plugin host");
    }
    Ok(i32::from(bad))
}

fn install(
    spec: &PluginSpec,
    url: &str,
    dir: &Path,
    locked: Option<&Locked>,
    update: bool,
) -> Result<(String, &'static str), String> {
    let fresh = !dir.join(".git").exists();
    if fresh {
        if dir.exists() {
            return Err(format!("{} exists but isn't a git checkout; remove it and sync again", dir.display()));
        }
        let parent = dir.parent().ok_or("no parent for the install directory")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        run_git(Command::new("git").args(["clone", "--quiet", "--end-of-options", url]).arg(dir))?;
    }
    let before = git(dir, &["rev-parse", "HEAD"])?;
    let commit = match want(spec, locked, update) {
        Want::Commit(c) => {
            if commit_of(dir, &c).is_err() {
                git(dir, &["fetch", "--quiet", "--tags", "origin"])?;
            }
            commit_of(dir, &c).map_err(|_| format!("the locked commit {} isn't in {url}", short(&c)))?
        }
        Want::Resolve(version) => {
            if !fresh {
                git(dir, &["fetch", "--quiet", "--tags", "--force", "origin"])?;
            }
            resolve(dir, version.as_deref())?
        }
    };
    if commit != before {
        git(dir, &["-c", "advice.detachedHead=false", "checkout", "--quiet", "--detach", &commit])?;
    }
    if !dir.join("plugin.toml").is_file() {
        return Err(format!("{url} has no plugin.toml at {}", short(&commit)));
    }
    let what = match (fresh, commit == before) {
        (true, _) => "installed",
        (false, true) => "up to date",
        (false, false) if update => "updated",
        (false, false) => "checked out",
    };
    Ok((commit, what))
}

/// A branch means its newest commit upstream, so `origin/` is tried before tags and hashes.
fn resolve(dir: &Path, version: Option<&str>) -> Result<String, String> {
    let Some(v) = version else {
        return commit_of(dir, "refs/remotes/origin/HEAD").map_err(|_| "the repository has no default branch".into());
    };
    [format!("refs/remotes/origin/{v}"), format!("refs/tags/{v}"), v.to_string()]
        .iter()
        .find_map(|r| commit_of(dir, r).ok())
        .ok_or_else(|| format!("no branch, tag or commit called {v}"))
}

fn commit_of(dir: &Path, rev: &str) -> Result<String, String> {
    git(dir, &["rev-parse", "--verify", "--quiet", "--end-of-options", &format!("{rev}^{{commit}}")])
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    run_git(Command::new("git").arg("-C").arg(dir).args(args))
}

fn run_git(cmd: &mut Command) -> Result<String, String> {
    // A private repository must fail, not wait for a password nobody will type.
    let out =
        cmd.env("GIT_TERMINAL_PROMPT", "0").stdin(Stdio::null()).output().map_err(|e| format!("can't run git: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(err.lines().find(|l| !l.trim().is_empty()).unwrap_or("git failed").trim().to_string())
}

fn ls(control: &Path) -> io::Result<i32> {
    let s = match setup() {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    if s.plugins.is_empty() {
        println!("no plugins in config.toml");
        return Ok(0);
    }
    let lock = read_lock(&s.lock_path);
    let status = ask(control, &Request::PluginStatus);
    let off: BTreeSet<String> = status
        .as_ref()
        .and_then(|v| v["disabled"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|n| n.as_str().map(str::to_string))
        .collect();
    let mut rows = Vec::new();
    for spec in &s.plugins {
        let src = spec.src.spec();
        let dir = match &spec.src {
            Source::Path(_) | Source::Builtin(_) => None,
            other => other.dir_name().map(|n| s.root.join(n)),
        };
        let (dir, state) = match (&spec.src, dir) {
            (Source::Builtin(n), _) => {
                rows.push((
                    n.clone(),
                    src.clone(),
                    if off.contains(n) {
                        "built in; off after it hung or crashed shroud's plugin host, until config.toml changes".into()
                    } else {
                        "built in".into()
                    },
                ));
                continue;
            }
            (Source::Path(p), _) => (Some(PathBuf::from(expand(p))), "local".to_string()),
            (_, None) => (None, "no install directory".to_string()),
            (_, Some(d)) if !d.join(".git").exists() => {
                (Some(d), "not installed: run `shroud plugin sync`".to_string())
            }
            (_, Some(d)) => {
                let head = git(&d, &["rev-parse", "HEAD"]).unwrap_or_default();
                let state = match lock.get(&src) {
                    Some(l) if l.commit == head => format!("at {}", short(&head)),
                    Some(l) => format!("at {}, locked at {}: run `shroud plugin sync`", short(&head), short(&l.commit)),
                    None => format!("at {}, not locked: run `shroud plugin sync`", short(&head)),
                };
                (Some(d), state)
            }
        };
        let name = dir
            .as_ref()
            .and_then(|d| std::fs::read_to_string(d.join("plugin.toml")).ok())
            .and_then(|t| shroud_plugins::manifest::parse(&t).ok())
            .map(|m| m.name);
        let state = match &name {
            Some(n) if off.contains(n) => {
                format!("{state}; off after it hung or crashed shroud's plugin host, until config.toml changes")
            }
            _ => state,
        };
        rows.push((name.unwrap_or_else(|| "?".into()), src, state));
    }
    let w = rows.iter().map(|r| r.0.len()).max().unwrap_or(0);
    let ws = rows.iter().map(|r| r.1.len()).max().unwrap_or(0);
    for (name, src, state) in rows {
        println!("{name:w$}  {src:ws$}  {state}");
    }
    if let Some(alert) = status.as_ref().and_then(|v| v["alert"].as_str()) {
        println!("\nlast problem: {alert}");
    }
    Ok(0)
}

fn clean() -> io::Result<i32> {
    let s = match setup() {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    let keep: BTreeSet<&str> = s.plugins.iter().filter_map(|p| p.src.dir_name()).collect();
    let Ok(entries) = std::fs::read_dir(&s.root) else {
        println!("nothing installed");
        return Ok(0);
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if keep.contains(name.as_str()) || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        std::fs::remove_dir_all(entry.path())?;
        println!("removed {name}");
        removed += 1;
    }
    if removed == 0 {
        println!("nothing to remove");
    }
    Ok(0)
}

fn clashing_dirs(plugins: &[PluginSpec]) -> BTreeSet<String> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for name in plugins.iter().filter_map(|p| p.src.dir_name()) {
        *seen.entry(name).or_default() += 1;
    }
    seen.into_iter().filter(|(_, n)| *n > 1).map(|(k, _)| k.to_string()).collect()
}

/// An unreadable lockfile only loses its pins; sync writes a good one.
fn read_lock(path: &Path) -> Lock {
    let Ok(text) = std::fs::read_to_string(path) else { return Lock::default() };
    Lock::parse(&text).unwrap_or_else(|e| {
        eprintln!("shroud: {}: {e}; ignoring its pins", path.display());
        Lock::default()
    })
}

fn expand(p: &str) -> String {
    match (p.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => p.to_string(),
    }
}

fn short(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

fn fail(message: &str) -> io::Result<i32> {
    eprintln!("shroud: {message}");
    Ok(1)
}
