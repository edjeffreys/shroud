//! The switcher's outcomes go through lifecycle, so it can't do anything the CLI couldn't.

use shroud_core::input::KeyEvent;
use shroud_core::switcher::{Entry, Outcome, Switcher};

use super::Server;

/// Short, since every tab's name shares the status bar.
const MAX_TAB_NAME: usize = 24;

impl Server {
    pub(super) fn open_switcher(&mut self, i: usize) {
        let current = self.clients[i].session;
        let mut sessions: Vec<_> = self.sessions.iter().collect();
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_used));
        let entries = sessions.iter().map(|s| Entry { name: s.name.clone(), current: Some(s.id) == current }).collect();
        self.clients[i].switcher = Some(Switcher::pick(entries));
    }

    fn rename_tab(&mut self, i: usize, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.chars().count() > MAX_TAB_NAME || name.chars().any(char::is_control) {
            return Err(format!("tab names are up to {MAX_TAB_NAME} printable characters"));
        }
        if let Some(s) = self.session_of(i) {
            s.tab_mut().name = (!name.is_empty()).then(|| name.to_string());
        }
        Ok(())
    }

    pub(super) fn switcher_key(&mut self, i: usize, k: &KeyEvent) {
        let Some(sw) = &mut self.clients[i].switcher else { return };
        let result = match sw.key(k) {
            Outcome::Open => return,
            Outcome::Close => Ok(()),
            Outcome::Switch(name) => self.id_of(&name).map(|id| self.attach(i, id)),
            Outcome::Create(name) => {
                let cwd = self.session_of(i).and_then(|s| s.tab().focused_cwd()).unwrap_or_default();
                let size = self.clients[i].size.unwrap_or(super::DEFAULT_SIZE);
                self.create(&name, &cwd, size).map(|id| self.attach(i, id))
            }
            Outcome::Rename { from, to } => self.id_of(&from).and_then(|id| self.rename(id, &to)),
            Outcome::Kill(name) => self.id_of(&name).map(|id| self.kill(id)),
            Outcome::RenameTab(name) => self.rename_tab(i, &name),
        };
        self.clients[i].switcher = None;
        if let Err(why) = result
            && let Some(s) = self.session_of(i)
        {
            s.status.say(why);
        }
    }
}
