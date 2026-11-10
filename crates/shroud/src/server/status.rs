//! The status bar's inputs that need I/O or the clock. Refreshed on events, never per frame.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use shroud_core::statusbar::display_path;

const MESSAGE_FOR: Duration = Duration::from_secs(3);

#[derive(Debug, Default)]
pub struct StatusInputs {
    pub cwd: Option<String>,
    pub branch: Option<String>,
    pub clock: String,
    message: Option<(String, Instant)>,
    for_cwd: Option<String>,
    next_tick: Option<Instant>,
}

impl StatusInputs {
    pub fn message(&self) -> Option<&str> {
        self.message.as_ref().map(|(m, _)| m.as_str())
    }

    pub fn say(&mut self, text: impl Into<String>) {
        self.say_for(text, MESSAGE_FOR);
    }

    pub fn say_for(&mut self, text: impl Into<String>, how_long: Duration) {
        self.message = Some((text.into(), Instant::now() + how_long));
    }

    /// `force` re-reads the branch even if the cwd is unchanged, since a command may have switched it.
    pub fn set_cwd(&mut self, cwd: Option<&str>, force: bool) {
        if !force && self.for_cwd.as_deref() == cwd {
            return;
        }
        self.for_cwd = cwd.map(str::to_string);
        let home = std::env::var("HOME").ok();
        self.cwd = cwd.map(|c| display_path(c, home.as_deref()));
        self.branch = cwd.and_then(|c| git_branch(Path::new(c)));
    }

    /// When the bar next changes without an event.
    pub fn deadline(&self) -> Option<Instant> {
        [self.next_tick, self.message.as_ref().map(|(_, until)| *until)].into_iter().flatten().min()
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if self.message.as_ref().is_some_and(|(_, until)| *until <= now) {
            self.message = None;
            changed = true;
        }
        if self.next_tick.is_none_or(|t| t <= now) {
            let (clock, until_next_minute) = local_clock();
            changed |= clock != self.clock;
            self.clock = clock;
            self.next_tick = Some(now + until_next_minute);
        }
        changed
    }
}

fn git_branch(cwd: &Path) -> Option<String> {
    let head = fs::read_to_string(git_dir(cwd)?.join("HEAD")).ok()?;
    let head = head.trim();
    Some(match head.strip_prefix("ref: ") {
        Some(r) => r.strip_prefix("refs/heads/").unwrap_or(r).to_string(),
        None => head.get(..7)?.to_string(),
    })
}

/// Worktrees and submodules point `.git` at the real directory.
fn git_dir(cwd: &Path) -> Option<PathBuf> {
    cwd.ancestors().find_map(|d| {
        let dot = d.join(".git");
        if dot.is_dir() {
            return Some(dot);
        }
        let link = fs::read_to_string(&dot).ok()?;
        let target = PathBuf::from(link.strip_prefix("gitdir:")?.trim());
        Some(if target.is_absolute() { target } else { d.join(target) })
    })
}

fn local_clock() -> (String, Duration) {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t = secs as libc::time_t;
    // SAFETY: localtime_r only writes the `tm` we own; the reentrant variant needs no global state.
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return (String::new(), Duration::from_secs(60));
    }
    let until = Duration::from_secs(60 - tm.tm_sec.clamp(0, 59) as u64);
    (format!("{:02}:{:02}", tm.tm_hour, tm.tm_min), until)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shroud-status-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub/deeper")).unwrap();
        dir
    }

    #[test]
    fn branch_from_head_in_a_parent_directory() {
        let dir = repo("plain");
        fs::create_dir(dir.join(".git")).unwrap();
        fs::write(dir.join(".git/HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        assert_eq!(git_branch(&dir.join("sub/deeper")).as_deref(), Some("feature/x"));
        fs::write(dir.join(".git/HEAD"), "0123456789abcdef\n").unwrap();
        assert_eq!(git_branch(&dir).as_deref(), Some("0123456"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn worktree_gitdir_files_are_followed() {
        let dir = repo("worktree");
        fs::create_dir_all(dir.join("real")).unwrap();
        fs::write(dir.join("real/HEAD"), "ref: refs/heads/wt\n").unwrap();
        fs::write(dir.join("sub/.git"), "gitdir: ../real\n").unwrap();
        assert_eq!(git_branch(&dir.join("sub/deeper")).as_deref(), Some("wt"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clock_is_hh_mm_and_ticks_within_a_minute() {
        let (clock, until) = local_clock();
        assert_eq!(clock.len(), 5, "{clock}");
        assert!(until <= Duration::from_secs(60) && until > Duration::ZERO);
    }
}
