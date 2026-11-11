//! One PTY, its libghostty-vt terminal, and the grid mirror compose reads.

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use shroud_core::input::OuterColors;
use shroud_core::input::{KeyEvent, MouseEvent};
use shroud_core::layout::PaneId;
use shroud_core::protocol::tags::Tags;
use shroud_core::render::Links;
use shroud_core::render::images::Placement;
use shroud_core::render::{Cursor, Grid, passthrough};
use shroud_core::shell::{Mark, Tracker};
use shroud_core::statusbar::Badge;
use shroud_vt::{Compression, Effect, KeyEncoder, MouseEncoder, Snapshot, Terminal, Viewport};

use super::launch::Launch;
use super::visual::History;
use crate::pty::{Pty, Spawn};

// Caps a mode-2026 hold so a crashed program can't freeze its pane.
const MAX_HOLD: Duration = Duration::from_secs(1);
// Shorter commands in a background pane are noise, not news.
const BADGE_MIN_RUNTIME: Duration = Duration::from_secs(1);
const READ_CHUNK: usize = 64 * 1024;
const MAX_EXITS: usize = 500;

#[derive(Debug)]
pub struct Pane {
    pub grid: Grid,
    pub cursor: Cursor,
    pub title: String,
    /// Set by a plugin with `pane_title`; shown in place of the program's title.
    pub label: Option<String>,
    pub shell: Tracker,
    pub badge: Option<Badge>,
    pub exited: bool,
    pub images: Vec<Placement>,
    pub tags: Tags,
    /// Set when a shortcut or `split{keep}` typed a command to watch.
    pub command_pane: Option<super::shortcuts::CommandPane>,
    // Images are placed in pixels, so a new cell size matters even at the same grid size.
    cell: (u32, u32),
    pub outer_gen: u64,
    command_started: Option<Instant>,
    /// Prompt row, command text and exit code of recent commands, for jump mode.
    exits: Vec<(u32, Option<String>, Option<i32>)>,
    dirty: bool,
    echo_due: bool,
    held_since: Option<Instant>,
    last_output: Instant,
    scrollback: Option<usize>,
    /// The compression activity token when compression last finished.
    compressed: Option<u64>,
    pty: Pty,
    term: Terminal,
    snap: Snapshot,
    keys: KeyEncoder,
    mouse: MouseEncoder,
    to_pty: Vec<u8>,
    effects: Vec<Effect>,
}

/// Unique across the whole server: poll tokens and badges name panes by id, whatever tab they move to.
pub fn next_id() -> PaneId {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Effects that leave the pane, already encoded for the outer terminal.
#[derive(Debug, Default)]
pub struct Outward {
    pub bytes: Vec<u8>,
    /// Commands long enough to earn a badge if nobody is watching.
    pub finished: Vec<Badge>,
    pub exits: Vec<Option<i32>>,
    pub prompted: bool,
    /// The first output after a key, which is most likely its echo.
    pub echo: bool,
    pub bell: bool,
    pub title: bool,
    pub cwd: bool,
    pub started: bool,
}

pub const QUIET: Duration = Duration::from_secs(2);

impl Pane {
    pub fn spawn(id: PaneId, cols: u16, rows: u16, cwd: Option<&Path>, launch: &Launch) -> io::Result<Pane> {
        let term = Terminal::new(cols, rows).map_err(io::Error::other)?;
        let pty = Pty::spawn(&Spawn {
            program: &launch.shell,
            cwd,
            cols,
            rows,
            term: &launch.term,
            env: &launch.env,
            pane: id,
        })?;
        Ok(Pane {
            grid: Grid::new(cols, rows),
            cursor: Cursor::default(),
            title: String::new(),
            shell: Tracker::new(),
            badge: None,
            exited: false,
            images: Vec::new(),
            tags: Tags::new(),
            command_pane: None,
            cell: (0, 0),
            outer_gen: 0,
            command_started: None,
            label: None,
            exits: Vec::new(),
            dirty: true,
            echo_due: false,
            held_since: None,
            last_output: Instant::now(),
            scrollback: None,
            compressed: None,
            pty,
            term,
            snap: Snapshot::new().map_err(io::Error::other)?,
            keys: KeyEncoder::new().map_err(io::Error::other)?,
            mouse: MouseEncoder::new().map_err(io::Error::other)?,
            to_pty: Vec::new(),
            effects: Vec::new(),
        })
    }

    pub fn fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.pty.fd()
    }

    pub fn wants_write(&self) -> bool {
        !self.to_pty.is_empty()
    }

    /// One bounded read per wake, so a flooding pane can't starve the others.
    pub fn read(&mut self, out: &mut Outward) {
        let mut buf = vec![0u8; READ_CHUNK];
        match self.pty.read(&mut buf) {
            Ok(0) => self.exited = true,
            Ok(n) => {
                self.term.write(&buf[..n]);
                self.dirty = true;
                out.echo = std::mem::take(&mut self.echo_due);
                out.started = !out.echo && self.last_output.elapsed() >= QUIET;
                self.last_output = Instant::now();
                self.apply_effects(out);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => self.exited = true,
        }
    }

    pub fn flush(&mut self) {
        while !self.to_pty.is_empty() {
            match self.pty.write(&self.to_pty) {
                Ok(n) => {
                    self.to_pty.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return,
            }
        }
    }

    fn apply_effects(&mut self, out: &mut Outward) {
        self.term.drain_effects(&mut self.effects);
        for fx in self.effects.drain(..) {
            match fx {
                Effect::WritePty(b) => self.to_pty.extend_from_slice(&b),
                Effect::Title(t) => {
                    out.title |= t != self.title;
                    self.title = t;
                }
                Effect::Pwd(p) => {
                    let before = self.shell.cwd.clone();
                    self.shell.set_cwd(&p);
                    out.cwd |= self.shell.cwd != before;
                }
                Effect::Shell(mark) => {
                    out.prompted |= mark == Mark::PromptStart;
                    if mark == Mark::OutputStart {
                        self.command_started = Some(Instant::now());
                    }
                    if let Mark::CommandEnd { exit } = mark
                        && let Some(prompt) = self.term.last_finished_prompt()
                    {
                        let command = self.term.command_text(&self.term.command_at(prompt));
                        self.exits.retain(|(p, _, _)| *p != prompt);
                        if self.exits.len() >= MAX_EXITS {
                            self.exits.remove(0);
                        }
                        self.exits.push((prompt, command, exit));
                    }
                    let started = self.command_started.take_if(|_| matches!(mark, Mark::CommandEnd { .. }));
                    if let Some(done) = self.shell.mark(mark) {
                        out.exits.push(done.exit);
                        if started.is_some_and(|s| s.elapsed() >= BADGE_MIN_RUNTIME) {
                            out.finished.push(if done.exit.is_some_and(|e| e != 0) {
                                Badge::Failed
                            } else {
                                Badge::Done
                            });
                        }
                    }
                }
                Effect::RenderHold(true) => self.held_since = Some(Instant::now()),
                Effect::RenderHold(false) => self.held_since = None,
                Effect::Bell => {
                    out.bell = true;
                    passthrough::bell(&mut out.bytes);
                }
                Effect::Clipboard(data) => passthrough::clipboard(&mut out.bytes, &data),
                Effect::Notification { title, body } => passthrough::notification(&mut out.bytes, &title, &body),
            }
        }
    }

    pub fn snapshot(&mut self, links: &mut Links) {
        if !self.dirty {
            return;
        }
        self.expire_hold();
        if self.held_since.is_some() {
            return;
        }
        if self.snap.update(&self.term, &mut self.grid, links).is_ok()
            && let Ok(c) = self.snap.cursor()
        {
            self.cursor = c;
        }
        if let Ok(Some(list)) = self.snap.placements() {
            self.images = list;
        }
        self.dirty = false;
    }

    pub fn resize(&mut self, cols: u16, rows: u16, cell_w: u32, cell_h: u32) {
        if (self.grid.cols(), self.grid.rows(), self.cell) == (cols, rows, (cell_w, cell_h)) {
            return;
        }
        self.cell = (cell_w, cell_h);
        let grid_changed = (self.grid.cols(), self.grid.rows()) != (cols, rows);
        let _ = self.term.resize(cols, rows, cell_w, cell_h);
        // Resizing ends a hold, but its effect waits for the next output; don't stay frozen till then.
        self.held_since = None;
        // A SIGWINCH for a size the shell already has interrupts reads in progress.
        if grid_changed {
            let _ = self.pty.resize(cols, rows);
        }
        self.dirty = true;
    }

    pub fn set_scrollback(&mut self, bytes: usize) {
        if self.scrollback != Some(bytes) && self.term.set_scrollback_bytes(Some(bytes)).is_ok() {
            self.scrollback = Some(bytes);
        }
    }

    /// When a hold over undrawn output runs out. The loop wakes then: a program that died
    /// mid-update sends nothing more, so no other event is due to draw what it held back.
    pub fn hold_due(&self) -> Option<Instant> {
        self.held_since.filter(|_| self.dirty).map(|since| since + MAX_HOLD)
    }

    /// Ends a hold that has run past `MAX_HOLD`; true if it was holding back output.
    pub fn expire_hold(&mut self) -> bool {
        if self.held_since.is_some_and(|since| since.elapsed() >= MAX_HOLD) {
            self.held_since = None;
            return self.dirty;
        }
        false
    }

    pub fn compress_due(&self, idle: Duration) -> Option<Instant> {
        (self.compressed != Some(self.term.compression_activity())).then(|| self.last_output + idle)
    }

    /// True while there's more to compress.
    pub fn compress_step(&mut self) -> bool {
        if matches!(self.term.compress_step(), Ok(Compression::Pending)) {
            return true;
        }
        self.compressed = Some(self.term.compression_activity());
        false
    }

    /// Typed rather than exec'd, so the shell is still there when the command ends.
    pub fn type_line(&mut self, line: &str) {
        self.type_text(line);
        self.to_pty.push(b'\r');
    }

    pub fn type_text(&mut self, text: &str) {
        self.to_pty.extend_from_slice(text.as_bytes());
    }

    /// True when it moved the viewport: otherwise nothing changes on screen until the echo.
    pub fn key(&mut self, ev: &KeyEvent) -> bool {
        let top = self.term.viewport().top;
        self.term.scroll_to_bottom();
        self.keys.encode(&self.term, ev, &mut self.to_pty);
        self.echo_due = true;
        let moved = self.term.viewport().top != top;
        self.dirty |= moved;
        moved
    }

    pub fn tracks_mouse(&self) -> bool {
        self.term.mouse_tracking()
    }

    /// False when the program doesn't track the mouse, so the caller can scroll instead.
    pub fn mouse(&mut self, ev: &MouseEvent) -> bool {
        if !self.term.mouse_tracking() {
            return false;
        }
        let (cols, rows) = (self.grid.cols(), self.grid.rows());
        self.mouse.encode(&self.term, ev, cols, rows, &mut self.to_pty);
        true
    }

    pub fn scroll(&mut self, lines: isize) {
        self.term.scroll_viewport(lines);
        self.dirty = true;
    }

    /// Matched on the command text as well as the row, so rows shifted by trimmed history
    /// never pair a command with another's exit code.
    pub fn exit_of(&self, prompt: u32, command: &Option<String>) -> Option<i32> {
        self.exits.iter().find(|(p, c, _)| *p == prompt && c == command).and_then(|(_, _, e)| *e)
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_to_bottom();
        self.dirty = true;
    }

    pub fn scroll_to_row(&mut self, top: u32) {
        self.term.scroll_to_row(top);
        self.dirty = true;
    }

    pub fn viewport(&self) -> Viewport {
        self.term.viewport()
    }

    pub fn history(&self) -> History<'_> {
        History(&self.term)
    }

    pub fn image(&self, id: u32) -> Option<shroud_vt::Image> {
        self.term.image(id)
    }

    /// The program in the foreground, when it isn't the shell.
    pub fn process(&self) -> Option<String> {
        self.pty.foreground()
    }

    pub fn capture(&self, history: bool) -> Option<String> {
        let v = self.term.viewport();
        let last = v.total.checked_sub(1)?;
        let first = if history { 0 } else { v.total.saturating_sub(v.rows as u32) };
        self.term.text_between((0, first), (v.cols.saturating_sub(1), last), false)
    }

    pub fn last_output(&self) -> Option<String> {
        self.term.last_output()
    }

    pub fn last_command(&self) -> Option<String> {
        self.term.last_command()
    }

    pub fn paste(&mut self, data: &[u8]) {
        self.term.scroll_to_bottom();
        self.term.encode_paste(data, &mut self.to_pty);
    }

    pub fn focus(&mut self, gained: bool) {
        self.term.encode_focus(gained, &mut self.to_pty);
    }

    pub fn set_outer(&mut self, colors: &OuterColors, dark: Option<bool>, graphemes: bool) {
        self.term.set_default_colors(colors.foreground, colors.background, colors.palette().as_ref());
        self.term.set_color_scheme(dark);
        self.term.set_mode_default(2027, false, graphemes);
    }

    // Unsolicited, so only to programs that asked with mode 2031.
    pub fn report_color_scheme(&mut self, dark: bool) {
        if self.term.mode(2031, false) {
            self.to_pty.extend_from_slice(if dark { b"\x1b[?997;1n" } else { b"\x1b[?997;2n" });
        }
    }

    pub fn try_reap(&mut self) -> bool {
        self.pty.try_reap()
    }
}
