//! A named group of tabs. Names are unique; the user picks them.

use std::io;
use std::path::Path;
use std::time::Instant;

use shroud_core::layout::PaneId;
use shroud_core::protocol::Size;
use shroud_core::statusbar::{Badge, TabEntry};

use shroud_core::config::PaneSpec;
use shroud_core::layout::Shape;

use super::launch::Launch;
use super::pane::Pane;
use super::status::StatusInputs;
use super::tab::Tab;

pub type SessionId = u32;

#[derive(Debug)]
pub struct Session {
    pub id: SessionId,
    pub name: String,
    pub last_used: Instant,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub status: StatusInputs,
    size: Size,
    launch: Launch,
}

impl Session {
    pub fn new(id: SessionId, name: String, cwd: &Path, size: Size, launch: Launch) -> io::Result<Session> {
        let tab = Tab::new(size, launch.clone(), Some(cwd))?;
        Ok(Session {
            id,
            name,
            last_used: Instant::now(),
            tabs: vec![tab],
            active: 0,
            status: StatusInputs::default(),
            size,
            launch,
        })
    }

    pub fn with_layout(
        id: SessionId,
        name: String,
        cwd: &Path,
        size: Size,
        launch: Launch,
        layout: &[Shape<PaneSpec>],
    ) -> io::Result<Session> {
        let tabs = layout
            .iter()
            .map(|shape| Tab::from_shape(size, launch.clone(), shape, cwd))
            .collect::<io::Result<Vec<_>>>()?;
        if tabs.is_empty() {
            return Session::new(id, name, cwd, size, launch);
        }
        Ok(Session {
            id,
            name,
            last_used: Instant::now(),
            tabs,
            active: 0,
            status: StatusInputs::default(),
            size,
            launch,
        })
    }

    pub fn set_launch(&mut self, launch: &Launch) {
        self.launch = launch.clone();
        self.tabs.iter_mut().for_each(|t| t.set_launch(launch.clone()));
    }

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    pub fn panes(&self) -> impl Iterator<Item = (&PaneId, &Pane)> {
        self.tabs.iter().flat_map(|t| t.panes.iter())
    }

    pub fn panes_mut(&mut self) -> impl Iterator<Item = (&PaneId, &mut Pane)> {
        self.tabs.iter_mut().flat_map(|t| t.panes.iter_mut())
    }

    /// The bool is whether a client can see the pane, which decides badges.
    pub fn pane_mut(&mut self, id: PaneId) -> Option<(&mut Pane, bool)> {
        let active = self.active;
        self.tabs.iter_mut().enumerate().find_map(|(i, t)| {
            let watched = i == active && t.focused == id;
            t.panes.get_mut(&id).map(|p| (p, watched))
        })
    }

    pub fn tab_entries(&self) -> Vec<TabEntry<'_>> {
        let worst = |t: &Tab| {
            let mut badges = t.panes.values().filter_map(|p| p.badge);
            let first = badges.next()?;
            Some(if first == Badge::Failed || badges.any(|b| b == Badge::Failed) { Badge::Failed } else { Badge::Done })
        };
        self.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| TabEntry { active: i == self.active, badge: worst(t), name: t.name.as_deref() })
            .collect()
    }

    pub fn resize(&mut self, size: Size) {
        if size != self.size {
            self.size = size;
            self.tabs.iter_mut().for_each(|t| t.resize(size));
        }
    }

    /// After the active tab, without switching to it, so a script never moves the user.
    pub fn add_tab_from_shape(&mut self, shape: &Shape<PaneSpec>, base: &Path) -> io::Result<usize> {
        let tab = Tab::from_shape(self.size, self.launch.clone(), shape, base)?;
        self.tabs.insert(self.active + 1, tab);
        Ok(self.active + 1)
    }

    pub fn new_tab(&mut self) -> io::Result<()> {
        let cwd = self.tab().focused_cwd();
        self.new_tab_at(cwd.as_deref()).map(|_| ())
    }

    /// After the current tab, and goes to it; returns its pane.
    pub fn new_tab_at(&mut self, cwd: Option<&Path>) -> io::Result<PaneId> {
        let tab = Tab::new(self.size, self.launch.clone(), cwd)?;
        let pane = tab.focused;
        self.tabs.insert(self.active + 1, tab);
        self.goto(self.active + 1);
        Ok(pane)
    }

    /// False for a pane alone in its tab.
    pub fn move_to_new_tab(&mut self) -> bool {
        let id = self.tab().focused;
        let Some(pane) = self.tab_mut().take(id) else { return false };
        self.tabs.insert(self.active + 1, Tab::adopt(self.size, self.launch.clone(), id, pane));
        self.goto(self.active + 1);
        true
    }

    pub fn goto(&mut self, index: usize) {
        let index = index.min(self.tabs.len() - 1);
        if index != self.active
            && let Some(left) = self.tabs.get_mut(self.active)
        {
            left.left_at = Some(Instant::now());
        }
        self.active = index;
        let tab = self.tab_mut();
        let focused = tab.focused;
        if let Some(p) = tab.panes.get_mut(&focused) {
            p.badge = None;
        }
    }

    /// Times rather than an index, so inserting or closing tabs can't point it at the wrong one.
    pub fn last_tab(&mut self) -> bool {
        let last = self
            .tabs
            .iter()
            .enumerate()
            .filter(|&(i, t)| i != self.active && t.left_at.is_some())
            .max_by_key(|(_, t)| t.left_at)
            .map(|(i, _)| i);
        last.inspect(|&i| self.goto(i)).is_some()
    }

    pub fn step(&mut self, by: isize) {
        let n = self.tabs.len() as isize;
        self.goto((self.active as isize + by).rem_euclid(n) as usize);
    }

    /// False once the session is empty.
    pub fn reap(&mut self) -> bool {
        for tab in &mut self.tabs {
            let exited: Vec<PaneId> =
                tab.panes.iter_mut().filter_map(|(id, p)| (p.exited || p.try_reap()).then_some(*id)).collect();
            for id in exited {
                tab.remove(id);
            }
        }
        let before = self.tabs.len();
        let mut index = 0;
        let mut new_active = self.active;
        self.tabs.retain(|t| {
            let keep = !t.panes.is_empty();
            if !keep && index < self.active {
                new_active -= 1;
            }
            index += 1;
            keep
        });
        if self.tabs.is_empty() {
            return false;
        }
        if self.tabs.len() != before {
            // As in nvim, the tab to the right takes over. Set directly: nobody chose to leave a tab.
            self.active = new_active.min(self.tabs.len() - 1);
            self.goto(self.active);
        }
        true
    }
}
