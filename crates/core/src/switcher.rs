//! Filtering, editing and drawing only; the server owns sessions and acts on the outcome.

use crate::input::{KeyAction, KeyEvent, Mods, key};
use crate::layout::rect::Rect;
use crate::overlay::{Overlay, Span};
use crate::render::frame::Cursor;
use crate::render::grid::Grid;
use crate::render::text::{char_width, str_width};
use crate::theme::Theme;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    pub name: String,
    pub current: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    Open,
    Close,
    Switch(String),
    Create(String),
    Rename {
        from: String,
        to: String,
    },
    /// Empty clears the name.
    RenameTab(String),
    Kill(String),
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum Mode {
    Pick,
    Create,
    /// `back` returns to the list on Escape when renaming started there.
    Rename {
        from: String,
        back: bool,
    },
    RenameTab,
    ConfirmKill(String),
}

#[derive(Debug)]
pub struct Switcher {
    mode: Mode,
    query: String,
    entries: Vec<Entry>,
    shown: Vec<usize>,
    selected: usize,
}

enum Row<'a> {
    Session(&'a Entry),
    New(&'a str),
}

const MAX_WIDTH: u16 = 64;
const HINT: &str = "enter switch · ^r rename · ^x close · esc";

impl Switcher {
    /// Expects entries most recent first.
    pub fn pick(entries: Vec<Entry>) -> Switcher {
        let mut s = Switcher { mode: Mode::Pick, query: String::new(), entries, shown: Vec::new(), selected: 0 };
        s.refilter();
        s.selected = s.rows().iter().position(|r| matches!(r, Row::Session(e) if !e.current)).unwrap_or(0);
        s
    }

    pub fn create() -> Switcher {
        Switcher { mode: Mode::Create, query: String::new(), entries: Vec::new(), shown: Vec::new(), selected: 0 }
    }

    pub fn rename(name: &str) -> Switcher {
        Switcher {
            mode: Mode::Rename { from: name.into(), back: false },
            query: name.into(),
            entries: Vec::new(),
            shown: Vec::new(),
            selected: 0,
        }
    }

    pub fn rename_tab(name: Option<&str>) -> Switcher {
        Switcher {
            mode: Mode::RenameTab,
            query: name.unwrap_or_default().into(),
            entries: Vec::new(),
            shown: Vec::new(),
            selected: 0,
        }
    }

    pub fn key(&mut self, ev: &KeyEvent) -> Outcome {
        if ev.action == KeyAction::Release {
            return Outcome::Open;
        }
        let ctrl = ev.mods.without_locks() == Mods::CTRL;
        let letter = char::from_u32(ev.base.unwrap_or(ev.code)).filter(|_| ctrl);
        if let Mode::ConfirmKill(name) = &self.mode {
            let name = name.clone();
            self.mode = Mode::Pick;
            let yes = ev.code == key::ENTER || ev.text.eq_ignore_ascii_case("y");
            return if yes { Outcome::Kill(name) } else { Outcome::Open };
        }
        match (ev.code, letter) {
            (key::ESCAPE, _) | (_, Some('c')) => return self.back(),
            (key::ENTER | key::KP_ENTER, _) => return self.enter(),
            (key::DOWN, _) | (_, Some('n')) => self.step(1),
            (key::UP, _) | (_, Some('p')) => self.step(-1),
            (key::BACKSPACE, _) => {
                self.query.pop();
                self.refilter();
            }
            (_, Some('u')) => {
                self.query.clear();
                self.refilter();
            }
            (_, Some('r')) => {
                if let Some(Row::Session(e)) = self.rows().get(self.selected) {
                    let from = e.name.clone();
                    self.query.clone_from(&from);
                    self.mode = Mode::Rename { from, back: true };
                }
            }
            (_, Some('x')) => {
                if let Some(Row::Session(e)) = self.rows().get(self.selected) {
                    self.mode = Mode::ConfirmKill(e.name.clone());
                }
            }
            _ if !ctrl && !ev.text.is_empty() && ev.text.chars().all(|c| !c.is_control()) => {
                self.query.push_str(&ev.text);
                self.refilter();
            }
            _ => {}
        }
        Outcome::Open
    }

    pub fn draw(&self, grid: &mut Grid, area: Rect, theme: &Theme) -> Cursor {
        let hidden = Cursor { visible: false, ..Cursor::default() };
        let Some((overlay, cursor)) = self.view(area) else { return hidden };
        overlay.draw(grid, area, (0, 0), theme);
        let r = overlay.rect(area, (0, 0));
        cursor.map_or(hidden, |(x, y)| Cursor { x: r.x + 1 + x, y: r.y + 1 + y, visible: true, ..Cursor::default() })
    }

    /// The switcher as an overlay, plus where the cursor goes inside it.
    pub fn view(&self, area: Rect) -> Option<(Overlay, Option<(u16, u16)>)> {
        let rows = self.rows();
        let listing = matches!(self.mode, Mode::Pick | Mode::ConfirmKill(_));
        let w = area.w.saturating_sub(4).min(MAX_WIDTH);
        let room = area.h.saturating_sub(2 + 4) as usize;
        if w < 16 || area.h < 5 || (listing && room == 0) {
            return None;
        }
        let text_w = w - 4;
        let dim = Some("switcher.border");
        let prompt = match &self.mode {
            Mode::Pick => String::new(),
            Mode::Create => "new session ".into(),
            Mode::Rename { from, .. } => format!("rename {from} to "),
            Mode::RenameTab => "name this tab ".into(),
            Mode::ConfirmKill(name) => format!("close session {name} and its shells? y/n"),
        };
        let mut first = vec![span(" ", None, false), span(&prompt, dim, false)];
        let mut cursor = None;
        if !matches!(self.mode, Mode::ConfirmKill(_)) {
            first.push(span("› ", Some("switcher.match"), false));
            let used = width(&first);
            let query = tail(&self.query, (text_w + 1).saturating_sub(used));
            first.push(span(&query, None, false));
            cursor = Some((width(&first), 0));
        }
        let mut lines = vec![first];
        if listing {
            let start = self.selected.saturating_sub(room.saturating_sub(1));
            if rows.is_empty() {
                lines.push(vec![span(" no sessions match", dim, false)]);
            }
            for (n, row) in rows.iter().enumerate().skip(start).take(room) {
                lines.push(self.row_line(row, n == self.selected, text_w));
            }
            lines.push(vec![span(" ", None, false), span(HINT, dim, false)]);
        }
        let overlay =
            Overlay { lines, width: Some(w - 2), border: dim.map(Into::into), modal: true, ..Overlay::default() };
        Some((overlay, cursor))
    }

    fn row_line(&self, row: &Row, selected: bool, text_w: u16) -> Vec<Span> {
        let (look, bold) = if selected { (Some("switcher.selected"), true) } else { (None, false) };
        let mut line = vec![span(" ", None, false), span(if selected { "▸ " } else { "  " }, look, bold)];
        match row {
            Row::Session(e) => {
                let hits = matches(&self.query, &e.name).unwrap_or_default();
                for (ci, ch) in e.name.chars().enumerate() {
                    let hit = hits.contains(&ci);
                    line.push(span(
                        ch.encode_utf8(&mut [0; 4]),
                        if hit { Some("switcher.match") } else { look },
                        bold || hit,
                    ));
                }
                let used = width(&line);
                if e.current && text_w + 1 > used + 7 {
                    line.push(span(&" ".repeat((text_w + 1 - used - 7) as usize), None, false));
                    line.push(span("current", Some("switcher.border"), false));
                }
            }
            Row::New(name) => {
                line.push(span("+ new session ", look, bold));
                line.push(span(name, look, true));
            }
        }
        line
    }

    fn rows(&self) -> Vec<Row<'_>> {
        let mut rows: Vec<Row> = self.shown.iter().map(|&i| Row::Session(&self.entries[i])).collect();
        let name = self.query.trim();
        if !name.is_empty() && !self.entries.iter().any(|e| e.name == name) {
            rows.push(Row::New(name));
        }
        rows
    }

    fn enter(&mut self) -> Outcome {
        let name = self.query.trim().to_string();
        match &self.mode {
            Mode::Create if !name.is_empty() => Outcome::Create(name),
            Mode::Rename { from, .. } if !name.is_empty() => Outcome::Rename { from: from.clone(), to: name },
            Mode::RenameTab => Outcome::RenameTab(name),
            Mode::Pick => match self.rows().get(self.selected) {
                Some(Row::Session(e)) => Outcome::Switch(e.name.clone()),
                Some(Row::New(n)) => Outcome::Create(n.to_string()),
                None => Outcome::Open,
            },
            _ => Outcome::Open,
        }
    }

    fn back(&mut self) -> Outcome {
        if let Mode::Rename { back: true, .. } = self.mode {
            // The list wasn't refiltered while renaming, so the selection still points at the same row.
            self.mode = Mode::Pick;
            self.query.clear();
            return Outcome::Open;
        }
        Outcome::Close
    }

    fn step(&mut self, by: isize) {
        let n = self.rows().len();
        if n > 0 && self.mode == Mode::Pick {
            self.selected = (self.selected as isize + by).rem_euclid(n as isize) as usize;
        }
    }

    fn refilter(&mut self) {
        if self.mode != Mode::Pick {
            return;
        }
        let mut scored: Vec<(i32, usize)> =
            self.entries.iter().enumerate().filter_map(|(i, e)| Some((score(&self.query, &e.name)?, i))).collect();
        // Stable on index, so an empty query keeps the most-recent-first order.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.shown = scored.into_iter().map(|(_, i)| i).collect();
        self.selected = 0;
    }
}

/// Consecutive letters and letters starting a word score higher, so `api` prefers `api-gateway`
/// over `a-pi-thing`.
fn score(query: &str, name: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let hits = matches(query, name)?;
    let chars: Vec<char> = name.chars().collect();
    let mut total = 0;
    for (n, &i) in hits.iter().enumerate() {
        total += 1;
        if n > 0 && hits[n - 1] + 1 == i {
            total += 4;
        }
        if i == 0 || matches!(chars[i - 1], '/' | '-' | '_' | ' ' | '.') {
            total += 3;
        }
    }
    let span = hits.last().zip(hits.first()).map_or(0, |(l, f)| l - f) as i32;
    Some(total * 8 - span - chars.len() as i32 / 8)
}

fn matches(query: &str, name: &str) -> Option<Vec<usize>> {
    let chars: Vec<char> = name.chars().collect();
    let mut hits = Vec::new();
    let mut from = 0;
    for q in query.chars().flat_map(char::to_lowercase) {
        let at = from + chars[from..].iter().position(|c| c.to_lowercase().any(|l| l == q))?;
        hits.push(at);
        from = at + 1;
    }
    Some(hits)
}

fn span(text: &str, style: Option<&str>, bold: bool) -> Span {
    Span { text: text.into(), style: style.map(Into::into), bold, ..Span::default() }
}

fn width(line: &[Span]) -> u16 {
    line.iter().map(|s| str_width(&s.text)).fold(0, u16::saturating_add)
}

/// The end of `s` that fits in `room` cells, so the cursor end of a long query stays visible.
fn tail(s: &str, room: u16) -> String {
    if str_width(s) <= room {
        return s.into();
    }
    let mut out: Vec<char> = Vec::new();
    let mut used = 1;
    for ch in s.chars().rev() {
        used += char_width(ch).max(1);
        if used > room {
            break;
        }
        out.push(ch);
    }
    std::iter::once('…').chain(out.into_iter().rev()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<Entry> {
        ["default", "api-gateway", "notes", "a-pi-thing"]
            .iter()
            .map(|n| Entry { name: n.to_string(), current: *n == "default" })
            .collect()
    }

    fn typed(s: &mut Switcher, text: &str) -> Outcome {
        let mut last = Outcome::Open;
        for ch in text.chars() {
            last = s.key(&KeyEvent { text: ch.to_string(), ..KeyEvent::press(ch as u32, Mods::NONE) });
        }
        last
    }

    fn press(s: &mut Switcher, code: u32, mods: Mods) -> Outcome {
        s.key(&KeyEvent::press(code, mods))
    }

    fn names(s: &Switcher) -> Vec<String> {
        s.rows()
            .iter()
            .map(|r| match r {
                Row::Session(e) => e.name.clone(),
                Row::New(n) => format!("+{n}"),
            })
            .collect()
    }

    #[test]
    fn opens_on_the_most_recent_other_session() {
        let mut s = Switcher::pick(entries());
        assert_eq!(press(&mut s, key::ENTER, Mods::NONE), Outcome::Switch("api-gateway".into()));
    }

    #[test]
    fn filtering_prefers_word_starts_and_offers_to_create() {
        let mut s = Switcher::pick(entries());
        typed(&mut s, "api");
        assert_eq!(names(&s), ["api-gateway", "a-pi-thing", "+api"]);
        typed(&mut s, "x");
        assert_eq!(names(&s), ["+apix"]);
        assert_eq!(press(&mut s, key::ENTER, Mods::NONE), Outcome::Create("apix".into()));
    }

    #[test]
    fn an_exact_name_is_not_offered_for_creation() {
        let mut s = Switcher::pick(entries());
        typed(&mut s, "notes");
        assert_eq!(names(&s), ["notes"]);
    }

    #[test]
    fn ctrl_r_renames_the_selected_session_and_escape_goes_back() {
        let mut s = Switcher::pick(entries());
        press(&mut s, 'r' as u32, Mods::CTRL);
        assert_eq!(s.query, "api-gateway");
        press(&mut s, key::ESCAPE, Mods::NONE);
        assert_eq!(s.mode, Mode::Pick);
        press(&mut s, 'r' as u32, Mods::CTRL);
        press(&mut s, 'u' as u32, Mods::CTRL);
        typed(&mut s, "gw");
        assert_eq!(
            press(&mut s, key::ENTER, Mods::NONE),
            Outcome::Rename { from: "api-gateway".into(), to: "gw".into() }
        );
    }

    #[test]
    fn ctrl_x_asks_before_closing() {
        let mut s = Switcher::pick(entries());
        press(&mut s, 'x' as u32, Mods::CTRL);
        assert_eq!(typed(&mut s, "n"), Outcome::Open);
        assert_eq!(s.mode, Mode::Pick);
        press(&mut s, 'x' as u32, Mods::CTRL);
        assert_eq!(typed(&mut s, "y"), Outcome::Kill("api-gateway".into()));
    }

    #[test]
    fn create_and_rename_prompts_need_a_name() {
        let mut s = Switcher::create();
        assert_eq!(press(&mut s, key::ENTER, Mods::NONE), Outcome::Open);
        typed(&mut s, " work ");
        assert_eq!(press(&mut s, key::ENTER, Mods::NONE), Outcome::Create("work".into()));
        let mut r = Switcher::rename("default");
        assert_eq!(press(&mut r, key::ESCAPE, Mods::NONE), Outcome::Close);
    }

    #[test]
    fn tab_names_may_be_cleared() {
        let mut s = Switcher::rename_tab(Some("api"));
        press(&mut s, 'u' as u32, Mods::CTRL);
        assert_eq!(press(&mut s, key::ENTER, Mods::NONE), Outcome::RenameTab(String::new()));
        let mut s = Switcher::rename_tab(None);
        typed(&mut s, "logs");
        assert_eq!(press(&mut s, key::ENTER, Mods::NONE), Outcome::RenameTab("logs".into()));
    }

    #[test]
    fn draws_list_tags_and_hint() {
        let s = Switcher::pick(entries());
        let mut g = Grid::new(60, 14);
        let cur = s.draw(&mut g, Rect::new(0, 0, 60, 14), &Theme::default());
        let text =
            |y: u16| g.row(y).iter().map(|c| if c.text().is_empty() { " " } else { c.text() }).collect::<String>();
        assert!(text(cur.y + 1).contains("default") && text(cur.y + 1).contains("current"));
        assert!(text(cur.y + 2).contains("▸ api-gateway"));
        assert!((0..14).any(|y| text(y).contains("^x close")));
    }

    #[test]
    fn drawn_as_an_overlay_keeping_the_query_tail_by_the_cursor() {
        let mut s = Switcher::create();
        typed(&mut s, &"x".repeat(40));
        typed(&mut s, "end");
        let mut g = Grid::new(30, 8);
        let cur = s.draw(&mut g, Rect::new(0, 0, 30, 8), &Theme::default());
        let text = g.row(cur.y).iter().map(|c| if c.text().is_empty() { " " } else { c.text() }).collect::<String>();
        assert!(text.contains("…") && text.contains("xend"), "{text}");
        assert_eq!(g.get(cur.x - 1, cur.y).text(), "d");
        assert_eq!(g.get(cur.x + 1, cur.y).text(), "│");
        assert_eq!(g.get(cur.x + 1, cur.y).style.fg, Theme::default().switcher_border);
    }
}
