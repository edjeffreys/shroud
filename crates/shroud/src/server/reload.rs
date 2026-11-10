//! Applies a changed `config.toml`. A file with a syntax error keeps the running config.

use std::time::Instant;

use shroud_core::config::{self, Problem};

use super::launch::Launch;
use super::{CONFIG_MESSAGE_FOR, Server};

impl Server {
    pub(super) fn poll_config(&mut self, startup: bool) {
        let Some(text) = self.watch.poll(Instant::now()) else { return };
        let (settings, problems) = match text.as_deref().map(config::parse) {
            None => (config::Config::default(), Vec::new()),
            Some(Ok(parsed)) => parsed,
            Some(Err(syntax)) if startup => (config::Config::default(), vec![syntax]),
            Some(Err(syntax)) => (
                self.settings.clone(),
                vec![Problem { message: format!("{} (kept the running config)", syntax.message), ..syntax }],
            ),
        };
        if settings.shell != self.settings.shell || settings.pane_frames != self.launch.frames {
            if settings.shell != self.settings.shell {
                self.launch = Launch::detect(settings.shell.as_deref());
            }
            self.launch.frames = settings.pane_frames;
            self.sessions.iter_mut().for_each(|s| s.set_launch(&self.launch));
        }
        self.config_bindings = settings.bindings.clone();
        self.settings = settings;
        self.rebuild_bindings();
        self.sync_plugins();
        self.problems = problems;
        if !startup {
            let note = self.config_note().unwrap_or_else(|| "config reloaded".into());
            self.sessions.iter_mut().for_each(|s| s.status.say_for(note.clone(), CONFIG_MESSAGE_FOR));
        }
        self.invalidate();
    }

    pub(super) fn config_note(&self) -> Option<String> {
        let first = self.problems.first()?;
        let more = match self.problems.len() - 1 {
            0 => String::new(),
            n => format!(" (+{n} more: shroud config check)"),
        };
        Some(format!("config {first}{more}"))
    }
}
