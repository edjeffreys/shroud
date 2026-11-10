//! When the plugin host must be killed or restarted. The server only carries it out, so the
//! rules are tested without a process.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

pub const HANDLER_LIMIT: Duration = Duration::from_secs(2);
const CRASH_WINDOW: Duration = Duration::from_secs(60);
const MAX_CRASHES: usize = 3;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AfterExit {
    Restart {
        blamed: Option<String>,
    },
    /// Crashing with no plugin running means restarting can't help.
    GiveUp,
}

#[derive(Clone, Debug, Default)]
pub struct Watchdog {
    running: Option<(String, Instant)>,
    disabled: BTreeSet<String>,
    crashes: Vec<Instant>,
    killed: bool,
}

impl Watchdog {
    pub fn running(&mut self, plugin: Option<String>, now: Instant) {
        self.running = plugin.map(|p| (p, now));
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.running.as_ref().map(|(_, since)| *since + HANDLER_LIMIT)
    }

    /// The caller kills the host when this names a plugin.
    pub fn overdue(&mut self, now: Instant) -> Option<String> {
        let (plugin, since) = self.running.as_ref()?;
        if now < *since + HANDLER_LIMIT {
            return None;
        }
        let plugin = plugin.clone();
        self.disabled.insert(plugin.clone());
        self.running = None;
        self.killed = true;
        Some(plugin)
    }

    pub fn exited(&mut self, now: Instant) -> AfterExit {
        if std::mem::take(&mut self.killed) {
            return AfterExit::Restart { blamed: None };
        }
        if let Some((plugin, _)) = self.running.take() {
            self.disabled.insert(plugin.clone());
            return AfterExit::Restart { blamed: Some(plugin) };
        }
        self.crashes.retain(|t| now.duration_since(*t) < CRASH_WINDOW);
        self.crashes.push(now);
        if self.crashes.len() >= MAX_CRASHES { AfterExit::GiveUp } else { AfterExit::Restart { blamed: None } }
    }

    pub fn disabled(&self) -> &BTreeSet<String> {
        &self.disabled
    }

    /// A config change may have fixed what went wrong, so every plugin gets another chance.
    pub fn forgive(&mut self) {
        *self = Watchdog::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slow_handler_is_blamed_and_its_kill_is_not_a_crash() {
        let t0 = Instant::now();
        let mut w = Watchdog::default();
        w.running(Some("stuck".into()), t0);
        assert_eq!(w.deadline(), Some(t0 + HANDLER_LIMIT));
        assert_eq!(w.overdue(t0 + Duration::from_millis(1999)), None);
        assert_eq!(w.overdue(t0 + HANDLER_LIMIT), Some("stuck".into()));
        assert_eq!(w.deadline(), None);
        assert_eq!(w.exited(t0 + HANDLER_LIMIT), AfterExit::Restart { blamed: None });
        assert!(w.disabled().contains("stuck"));
    }

    #[test]
    fn waiting_for_a_reply_is_not_running() {
        let t0 = Instant::now();
        let mut w = Watchdog::default();
        w.running(Some("slowpoke".into()), t0);
        w.running(None, t0);
        assert_eq!(w.overdue(t0 + Duration::from_secs(60)), None);
    }

    #[test]
    fn a_crash_blames_the_running_plugin_else_counts_towards_giving_up() {
        let t0 = Instant::now();
        let mut w = Watchdog::default();
        w.running(Some("ffi".into()), t0);
        assert_eq!(w.exited(t0), AfterExit::Restart { blamed: Some("ffi".into()) });
        assert_eq!(w.exited(t0), AfterExit::Restart { blamed: None });
        assert_eq!(w.exited(t0 + Duration::from_secs(1)), AfterExit::Restart { blamed: None });
        assert_eq!(w.exited(t0 + Duration::from_secs(2)), AfterExit::GiveUp);
        w.forgive();
        assert!(w.disabled().is_empty());
        assert_eq!(w.exited(t0 + Duration::from_secs(3)), AfterExit::Restart { blamed: None });
    }

    #[test]
    fn old_crashes_are_forgotten() {
        let t0 = Instant::now();
        let mut w = Watchdog::default();
        w.exited(t0);
        w.exited(t0 + Duration::from_secs(1));
        assert_eq!(w.exited(t0 + CRASH_WINDOW + Duration::from_secs(1)), AfterExit::Restart { blamed: None });
    }
}
