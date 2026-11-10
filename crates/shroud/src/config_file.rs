//! Shared by the server and `shroud config check`, so both read the same files.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

// Polling is one stat a second; a kqueue watch would also have to follow editors that
// replace the file instead of writing it.
const CHECK_EVERY: Duration = Duration::from_secs(1);
pub const SETTINGS: &str = "config.toml";
pub const LOCK: &str = "plugins.lock";
pub const INIT: &str = "init.lua";
pub const GLOBAL_LAYOUT: &str = "layout.kdl";
pub const PROJECT_LAYOUT: &str = ".shroud.kdl";

pub fn dir() -> Option<PathBuf> {
    match std::env::var_os("XDG_CONFIG_HOME").filter(|d| Path::new(d).is_absolute()) {
        Some(d) => Some(PathBuf::from(d).join("shroud")),
        None => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/shroud")),
    }
}

/// Installed plugins and their state; never inside the config directory, which people
/// keep in git.
pub fn data_dir() -> Option<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME").filter(|d| Path::new(d).is_absolute()) {
        Some(d) => Some(PathBuf::from(d).join("shroud")),
        None => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/shroud")),
    }
}

/// Plugins declared in Lua. Runs in the plugin host, never the server.
pub fn init_path() -> Option<PathBuf> {
    dir().map(|d| d.join(INIT))
}

pub fn lock_path() -> Option<PathBuf> {
    dir().map(|d| d.join(LOCK))
}

pub fn settings_path() -> Option<PathBuf> {
    dir().map(|d| d.join(SETTINGS))
}

/// The starting directory's own layout wins; parents are never searched.
pub fn layout_for(cwd: &Path) -> Option<(PathBuf, String)> {
    [Some(cwd.join(PROJECT_LAYOUT)), dir().map(|d| d.join(GLOBAL_LAYOUT))]
        .into_iter()
        .flatten()
        .find_map(|p| std::fs::read_to_string(&p).ok().map(|text| (p, text)))
}

#[derive(Debug)]
pub struct Watch {
    path: Option<PathBuf>,
    stamp: Option<(SystemTime, u64)>,
    pub next_check: Instant,
}

impl Watch {
    pub fn new(path: Option<PathBuf>) -> Watch {
        Watch { path, stamp: None, next_check: Instant::now() }
    }

    pub fn poll(&mut self, now: Instant) -> Option<Option<String>> {
        if now < self.next_check {
            return None;
        }
        self.next_check = now + CHECK_EVERY;
        let path = self.path.as_ref()?;
        let stamp = std::fs::metadata(path).ok().map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()));
        if stamp == self.stamp {
            return None;
        }
        self.stamp = stamp;
        Some(std::fs::read_to_string(path).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_creation_change_and_removal_once_each() {
        let dir = std::env::temp_dir().join(format!("shroud-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(SETTINGS);
        let _ = std::fs::remove_file(&path);
        let mut w = Watch::new(Some(path.clone()));
        let mut later = Instant::now();
        let mut tick = || {
            later += CHECK_EVERY;
            later
        };
        assert_eq!(w.poll(tick()), None);
        std::fs::write(&path, "a = 1").unwrap();
        assert_eq!(w.poll(tick()), Some(Some("a = 1".into())));
        assert_eq!(w.poll(tick()), None);
        std::fs::write(&path, "a = 22").unwrap();
        assert_eq!(w.poll(tick()), Some(Some("a = 22".into())));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(w.poll(tick()), Some(None));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
