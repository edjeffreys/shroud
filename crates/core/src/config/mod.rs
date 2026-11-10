//! Must not read files. A problem costs only the setting it names; nothing is fatal.

mod layout_file;
mod lockfile;
mod plugins;
mod settings;

pub use layout_file::{PaneSpec, layout_bounds, parse_layout};
pub use lockfile::{Lock, Locked};
pub use plugins::{BUILTIN, PluginSpec, Source, Want, effective, want};
pub use settings::parse;

impl Config {
    pub fn scrollback_bytes(&self) -> usize {
        self.scrollback_mb.unwrap_or(DEFAULT_SCROLLBACK_MB) as usize * 1_000_000
    }
}

use crate::input::Bindings;
use crate::theme::Theme;

#[derive(Clone, Debug)]
pub struct Config {
    pub theme: Theme,
    pub bindings: Bindings,
    pub shell: Option<String>,
    pub plugins: Vec<PluginSpec>,
    /// Megabytes of history per pane; `None` is the default.
    pub scrollback_mb: Option<u32>,
    pub pane_frames: bool,
}

/// Built so that having no config file at all still turns the default built-in plugins on.
impl Default for Config {
    fn default() -> Config {
        Config {
            theme: Theme::default(),
            bindings: Bindings::default(),
            shell: None,
            plugins: effective(Vec::new()),
            scrollback_mb: None,
            pane_frames: false,
        }
    }
}

/// Ghostty's default.
pub const DEFAULT_SCROLLBACK_MB: u32 = 10;

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Problem {
    pub line: Option<usize>,
    pub message: String,
}

impl Problem {
    pub(crate) fn at(text: &str, offset: usize, message: impl Into<String>) -> Problem {
        let line = text.get(..offset).map(|before| before.matches('\n').count() + 1);
        Problem { line, message: message.into() }
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(n) => write!(f, "line {n}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_config_file_still_runs_the_default_builtins() {
        assert_eq!(Config::default().plugins.iter().map(|p| p.src.spec()).collect::<Vec<_>>(), ["builtin:hints"]);
    }
}
