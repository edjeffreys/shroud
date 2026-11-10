//! Rebuilt when config changes `shell`: the integration env depends on the shell.

use std::path::{Path, PathBuf};

use shroud_core::shell::integration_env;

#[derive(Clone, Debug, PartialEq)]
pub struct Launch {
    pub shell: String,
    pub term: String,
    pub env: Vec<(&'static str, String)>,
    /// `pane_frames`: tiles sit inside a frame, so every pane has a border above it for its title.
    pub frames: bool,
}

impl Launch {
    pub fn detect(shell: Option<&str>) -> Launch {
        let shell =
            shell.map(str::to_string).or_else(|| std::env::var("SHELL").ok()).unwrap_or_else(|| "/bin/zsh".into());
        // An unresolvable TERM makes curses programs like htop refuse to start.
        let term = match std::env::var("TERM") {
            Ok(t) if t == "xterm-ghostty" && terminfo_exists(&t) => t,
            _ => "xterm-256color".into(),
        };
        let env = match std::env::var("GHOSTTY_RESOURCES_DIR") {
            Ok(res) if Path::new(&res).is_dir() => integration_env(&shell, &res, |k| std::env::var(k).ok()),
            _ => Vec::new(),
        };
        Launch { shell, term, env, frames: false }
    }
}

fn terminfo_exists(name: &str) -> bool {
    let mut dirs: Vec<PathBuf> = Vec::new();
    dirs.extend(std::env::var_os("TERMINFO").map(PathBuf::from));
    dirs.extend(std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".terminfo")));
    if let Some(list) = std::env::var_os("TERMINFO_DIRS") {
        dirs.extend(std::env::split_paths(&list));
    }
    dirs.extend(["/usr/share/terminfo", "/opt/homebrew/share/terminfo"].map(PathBuf::from));
    let first = name.as_bytes()[0];
    // ncurses uses a letter directory on Linux and a hex one on macOS.
    let subdirs = [(first as char).to_string(), format!("{first:02x}")];
    dirs.iter().any(|d| subdirs.iter().any(|s| d.join(s).join(name).exists()))
}
