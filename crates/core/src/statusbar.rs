//! Must not allocate: drawn into every client's frame on every render.

use crate::render::cell::{Cell, Style, Width};
use crate::render::grid::Grid;
use crate::render::highlight::Shape;
use crate::render::text::{Writer, str_width};
use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Normal,
    Leader,
    Jump,
    /// With the shape of the selection being made, if any.
    Visual(Option<Shape>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Badge {
    Failed,
    Done,
}

/// Only panes with news appear, so a quiet session shows no pane list at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PaneBadge {
    /// 1-based, matching `Ctrl+Space 03w`, so the badge says how to get there.
    pub pane: u16,
    pub badge: Badge,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TabEntry<'a> {
    pub active: bool,
    /// A badged pane anywhere in the tab, so news in a hidden tab still shows.
    pub badge: Option<Badge>,
    /// Drawn after the number, which stays because `Ctrl+Space 3` needs it.
    pub name: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Status<'a> {
    pub mode: Mode,
    pub session: Option<&'a str>,
    /// Drawn only when there's more than one.
    pub tabs: &'a [TabEntry<'a>],
    /// Shown where the mode goes when no mode is active.
    pub zoomed: bool,
    /// Typed keys reach every pane drawn in the tab. Shown where the mode goes, after zoom,
    /// since a zoomed tab draws, and so types into, only one pane.
    pub broadcast: bool,
    pub cwd: Option<&'a str>,
    pub branch: Option<&'a str>,
    /// Replaces cwd and branch while set, e.g. the result of a copy.
    pub message: Option<&'a str>,
    pub badges: &'a [PaneBadge],
    /// Server-wide trouble such as a broken config; it outlasts every other piece.
    pub alert: Option<&'a str>,
    /// The previous render's cost, while the frame-time overlay is on.
    pub frame_time: Option<&'a str>,
    pub clock: &'a str,
}

const MARGIN: u16 = 1;
const GAP: u16 = 1;

pub fn draw(grid: &mut Grid, y: u16, s: &Status, theme: &Theme) {
    if y >= grid.rows() {
        return;
    }
    for cell in grid.row_mut(y) {
        *cell = Cell::new("", Width::Narrow, theme.status());
    }
    let end = grid.cols().saturating_sub(MARGIN);
    let mode = match s.mode {
        Mode::Normal if s.zoomed => Some(" ZOOM "),
        Mode::Normal if s.broadcast => Some(" BROADCAST "),
        Mode::Normal => None,
        Mode::Leader => Some(" LEADER "),
        Mode::Jump => Some(" JUMP "),
        Mode::Visual(None | Some(Shape::Chars)) => Some(" VISUAL "),
        Mode::Visual(Some(Shape::Lines)) => Some(" V-LINE "),
        Mode::Visual(Some(Shape::Block)) => Some(" V-BLOCK "),
    };
    // The reversed tag is its own padding, so it sits flush with the edge.
    let start = if mode.is_some() { 0 } else { MARGIN };
    let (main, main_style) = match s.message {
        Some(m) => (Some(m), theme.status_with(theme.status_message)),
        None => (s.cwd, theme.status()),
    };
    let mut p = Pieces {
        mode: mode.map_or(0, str_width),
        session: s.session,
        tabs: if s.tabs.len() > 1 { s.tabs } else { &[] },
        branch: s.branch.filter(|_| s.message.is_none()),
        badges: s.badges,
        alert: s.alert,
        frame_time: s.frame_time,
        clock: Some(s.clock).filter(|c| !c.is_empty()),
    };
    let room = end.saturating_sub(start);
    let main_full = main.map_or(0, str_width);
    let needs = |p: &Pieces| p.others() + main_full + if main_full > 0 && p.others() > 0 { GAP } else { 0 };
    while needs(&p) > room && p.drop_lowest() {}
    let others = p.others();
    let main_w = main_full.min(room.saturating_sub(others).saturating_sub(if others > 0 { GAP } else { 0 }));

    let mut w = Writer::new(grid, y, start, end);
    if let Some(m) = mode {
        w.text(m, theme.status_mode());
        w.x += GAP;
    }
    if let Some(name) = p.session {
        w.text(name, Style { bold: true, ..theme.status_with(theme.status_session) });
        w.x += GAP;
    }
    for (i, t) in p.tabs.iter().enumerate() {
        let style = if t.active {
            Style { bold: true, ..theme.status_with(theme.status_tab_active) }
        } else {
            theme.status_with(theme.status_tab)
        };
        w.number(i as u16 + 1, style);
        if let Some(n) = t.name {
            w.text(":", style);
            w.text(n, style);
        }
        if let Some(b) = t.badge {
            badge(&mut w, b, theme);
        }
        w.x += GAP;
    }
    if let Some(m) = main.filter(|_| main_w > 0) {
        w.text_tail(m, main_w, main_style);
        w.x += GAP;
    }
    if let Some(b) = p.branch {
        w.text(b, theme.status());
    }

    w.x = end.saturating_sub(right_width(&p));
    if let Some(a) = p.alert {
        w.text(a, theme.status_with(theme.badge_error));
        w.x += GAP;
    }
    for b in p.badges {
        w.number(b.pane, theme.status());
        badge(&mut w, b.badge, theme);
        w.x += GAP;
    }
    if let Some(f) = p.frame_time {
        w.text(f, theme.status());
        w.x += GAP;
    }
    if let Some(c) = p.clock {
        w.text(c, theme.status());
    }
}

pub fn display_path(path: &str, home: Option<&str>) -> String {
    match home.filter(|h| !h.is_empty() && *h != "/") {
        Some(h) if path == h => "~".into(),
        Some(h) if path.strip_prefix(h).is_some_and(|rest| rest.starts_with('/')) => format!("~{}", &path[h.len()..]),
        _ => path.into(),
    }
}

fn badge(w: &mut Writer, b: Badge, theme: &Theme) {
    match b {
        Badge::Failed => w.text("✗", theme.status_with(theme.badge_error)),
        Badge::Done => w.text("✓", theme.status_with(theme.badge_done)),
    }
}

/// Everything but the cwd, which truncates instead of dropping out.
struct Pieces<'a> {
    mode: u16,
    session: Option<&'a str>,
    tabs: &'a [TabEntry<'a>],
    branch: Option<&'a str>,
    badges: &'a [PaneBadge],
    alert: Option<&'a str>,
    frame_time: Option<&'a str>,
    clock: Option<&'a str>,
}

impl Pieces<'_> {
    fn others(&self) -> u16 {
        let tabs = joined_iter(self.tabs.iter().enumerate().map(|(i, t)| {
            digits(i as u16 + 1) + t.name.map_or(0, |n| 1 + str_width(n)) + u16::from(t.badge.is_some())
        }));
        let left = joined(&[self.mode, self.session.map_or(0, str_width), tabs, self.branch.map_or(0, str_width)]);
        let right = right_width(self);
        left + right + if left > 0 && right > 0 { GAP } else { 0 }
    }

    /// Frame time, clock, then branch, badges, tabs, the session name and finally the alert.
    fn drop_lowest(&mut self) -> bool {
        if self.frame_time.take().is_some() || self.clock.take().is_some() || self.branch.take().is_some() {
            return true;
        }
        if !self.badges.is_empty() {
            self.badges = &[];
            return true;
        }
        if !self.tabs.is_empty() {
            self.tabs = &[];
            return true;
        }
        self.session.take().is_some() || self.alert.take().is_some()
    }
}

fn right_width(p: &Pieces) -> u16 {
    joined(&[
        p.alert.map_or(0, str_width),
        joined_iter(p.badges.iter().map(|b| digits(b.pane) + 1)),
        p.frame_time.map_or(0, str_width),
        p.clock.map_or(0, str_width),
    ])
}

fn joined(parts: &[u16]) -> u16 {
    joined_iter(parts.iter().copied())
}

fn joined_iter(parts: impl Iterator<Item = u16>) -> u16 {
    let (sum, n) = parts.filter(|&w| w > 0).fold((0u16, 0u16), |(s, n), w| (s.saturating_add(w), n + 1));
    sum + GAP * n.saturating_sub(1)
}

fn digits(n: u16) -> u16 {
    n.checked_ilog10().map_or(1, |d| d as u16 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::color::Color;

    fn row(grid: &Grid, y: u16) -> String {
        grid.row(y)
            .iter()
            .filter(|c| c.width != Width::Spacer)
            .map(|c| if c.text().is_empty() { " " } else { c.text() })
            .collect()
    }

    fn render(width: u16, s: &Status) -> Grid {
        let mut g = Grid::new(width, 1);
        draw(&mut g, 0, s, &Theme::default());
        g
    }

    const BADGES: &[PaneBadge] =
        &[PaneBadge { pane: 1, badge: Badge::Failed }, PaneBadge { pane: 3, badge: Badge::Done }];

    #[test]
    fn directory_left_clock_right() {
        let s = Status { cwd: Some("~/src/shroud"), branch: Some("main"), clock: "09:41", ..Status::default() };
        assert_eq!(row(&render(30, &s), 0), " ~/src/shroud main      09:41 ");
    }

    #[test]
    fn mode_sits_flush_left_and_badges_sit_by_the_clock() {
        let s = Status {
            mode: Mode::Leader,
            cwd: Some("~"),
            branch: Some("main"),
            badges: BADGES,
            clock: "09:41",
            ..Status::default()
        };
        assert_eq!(row(&render(34, &s), 0), " LEADER  ~ main       1✗ 3✓ 09:41 ");
    }

    #[test]
    fn narrow_bars_drop_clock_then_branch_then_badges_then_truncate_cwd() {
        let s = Status {
            cwd: Some("~/src/shroud"),
            branch: Some("main"),
            badges: &BADGES[..1],
            clock: "09:41",
            ..Status::default()
        };
        assert_eq!(row(&render(26, &s), 0), " ~/src/shroud main     1✗ ");
        assert_eq!(row(&render(19, &s), 0), " ~/src/shroud   1✗ ");
        assert_eq!(row(&render(14, &s), 0), " ~/src/shroud ");
        assert_eq!(row(&render(9, &s), 0), " …shroud ");
    }

    #[test]
    fn session_and_tabs_lead_and_a_single_tab_is_hidden() {
        let tabs = [
            TabEntry { active: false, badge: None, name: None },
            TabEntry { active: true, badge: None, name: None },
            TabEntry { active: false, badge: Some(Badge::Failed), name: None },
        ];
        let s =
            Status { session: Some("shroud"), tabs: &tabs, cwd: Some("~/src"), clock: "09:41", ..Status::default() };
        let g = render(32, &s);
        assert_eq!(row(&g, 0), " shroud 1 2 3✗ ~/src      09:41 ");
        assert!(g.get(10, 0).style.bold, "the active tab stands out");
        let one = Status { session: Some("shroud"), tabs: &tabs[..1], cwd: Some("~/src"), ..Status::default() };
        assert_eq!(row(&render(16, &one), 0), " shroud ~/src   ");
    }

    #[test]
    fn named_tabs_keep_their_number_and_zoom_takes_the_mode_slot() {
        let tabs = [
            TabEntry { active: true, badge: None, name: Some("api") },
            TabEntry { active: false, badge: Some(Badge::Done), name: None },
        ];
        let s = Status { session: Some("s"), tabs: &tabs, zoomed: true, clock: "09:41", ..Status::default() };
        assert_eq!(row(&render(28, &s), 0), " ZOOM  s 1:api 2✓     09:41 ");
        let leader = Status { mode: Mode::Leader, ..s };
        assert!(row(&render(28, &leader), 0).starts_with(" LEADER "), "a mode outranks zoom");
        let both = Status { broadcast: true, ..s };
        assert!(row(&render(28, &both), 0).starts_with(" ZOOM "), "a zoomed tab types into one pane");
        let broadcast = Status { zoomed: false, ..both };
        assert_eq!(row(&render(32, &broadcast), 0), " BROADCAST  s 1:api 2✓    09:41 ");
    }

    #[test]
    fn tabs_and_session_drop_after_badges() {
        let tabs =
            [TabEntry { active: true, badge: None, name: None }, TabEntry { active: false, badge: None, name: None }];
        let s = Status {
            session: Some("shroud"),
            tabs: &tabs,
            cwd: Some("~/src"),
            badges: &BADGES[..1],
            ..Status::default()
        };
        assert_eq!(row(&render(22, &s), 0), " shroud 1 2 ~/src  1✗ ");
        assert_eq!(row(&render(19, &s), 0), " shroud 1 2 ~/src  ");
        assert_eq!(row(&render(15, &s), 0), " shroud ~/src  ");
        assert_eq!(row(&render(8, &s), 0), " ~/src  ");
    }

    #[test]
    fn message_replaces_cwd_and_branch() {
        let s = Status { cwd: Some("~/x"), branch: Some("main"), message: Some("copied 3 lines"), ..Status::default() };
        let g = render(20, &s);
        assert_eq!(row(&g, 0), " copied 3 lines     ");
        assert_eq!(g.get(1, 0).style.fg, Theme::default().status_message);
    }

    #[test]
    fn wide_characters_take_two_cells() {
        let s = Status { cwd: Some("~/日本"), clock: "09:41", ..Status::default() };
        let g = render(15, &s);
        assert_eq!(row(&g, 0), " ~/日本  09:41 ");
        assert_eq!(g.get(3, 0).width, Width::Wide);
    }

    #[test]
    fn badges_use_their_tokens() {
        let s = Status { badges: BADGES, ..Status::default() };
        let theme = Theme { badge_error: Color::Rgb(9, 9, 9), ..Theme::default() };
        let mut g = Grid::new(10, 1);
        draw(&mut g, 0, &s, &theme);
        assert_eq!(row(&g, 0), "    1✗ 3✓ ");
        assert_eq!(g.get(5, 0).style.fg, Color::Rgb(9, 9, 9));
    }

    #[test]
    fn the_alert_sits_before_badges_and_drops_last() {
        let s = Status {
            session: Some("work"),
            cwd: Some("~"),
            badges: &BADGES[..1],
            alert: Some("config"),
            clock: "09:41",
            ..Status::default()
        };
        let g = render(30, &s);
        assert_eq!(row(&g, 0), " work ~       config 1✗ 09:41 ");
        assert_eq!(g.get(14, 0).style.fg, Theme::default().badge_error);
        assert_eq!(row(&render(10, &s), 0), " ~ config ");
    }

    #[test]
    fn frame_time_sits_by_the_clock_and_drops_first() {
        let s = Status { cwd: Some("~"), frame_time: Some("0.3ms"), clock: "09:41", ..Status::default() };
        assert!(row(&render(30, &s), 0).ends_with("0.3ms 09:41 "));
        assert!(!row(&render(12, &s), 0).contains("0.3ms"));
    }

    #[test]
    fn home_becomes_tilde() {
        assert_eq!(display_path("/Users/ed/src", Some("/Users/ed")), "~/src");
        assert_eq!(display_path("/Users/ed", Some("/Users/ed")), "~");
        assert_eq!(display_path("/Users/eddie", Some("/Users/ed")), "/Users/eddie");
        assert_eq!(display_path("/tmp", Some("/")), "/tmp");
    }
}
