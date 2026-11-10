//! Visual mode: nvim motions over a pane's history, then copy. Knows nothing of terminals; the
//! server reads text through `Text` and scrolls the pane to `top`.

mod keys;
mod motion;
mod object;

use crate::render::{Highlight, Shape};

/// A pane's history as visual mode sees it. Rows count from the oldest line kept.
pub trait Text {
    fn rows(&self) -> u32;
    fn cols(&self) -> u16;
    /// One char per column; a wide glyph's second column is `'\0'`.
    fn line(&self, y: u32) -> Vec<char>;
    fn is_prompt(&self, y: u32) -> bool;
    /// Every row at once, for searching; override it where reading row by row is slow.
    fn all_lines(&self) -> Vec<Vec<char>> {
        (0..self.rows()).map(|y| self.line(y)).collect()
    }
}

/// A row's text as one char per column, as `Text::line` gives it: a wide glyph's second column
/// is `'\0'`.
pub fn columns(row: &str) -> Vec<char> {
    let mut out = Vec::with_capacity(row.len());
    for c in row.chars() {
        let w = crate::render::text::char_width(c);
        if w == 0 && !out.is_empty() {
            continue;
        }
        out.push(c);
        if w == 2 {
            out.push('\0');
        }
    }
    out
}

/// Ordered by row, then column, which is reading order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct Pos {
    pub y: u32,
    pub x: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Selection {
    pub start: Pos,
    pub end: Pos,
    pub shape: Shape,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    Stay,
    Copy(Selection),
    Exit,
    Say(String),
}

#[derive(Debug)]
pub struct Visual {
    pub cursor: Pos,
    /// The server scrolls the pane here after every key.
    pub top: u32,
    height: u16,
    anchor: Option<(Pos, Shape)>,
    count: Option<u32>,
    pending: Option<Pending>,
    /// `y` typed with no selection, waiting for a motion or text object; holds its count.
    operator: Option<u32>,
    typing: Option<(bool, String)>,
    /// Where the cursor and view were when the search began: incremental search moves from
    /// there, and Escape goes back.
    origin: (Pos, u32),
    last_search: Option<(String, bool)>,
    last_find: Option<(char, bool, bool)>,
}

/// Keys that need another key to mean anything.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pending {
    G,
    Find { forward: bool, till: bool },
    Object { around: bool },
}

/// How much a motion covers when yanked, as in nvim: `w` stops short of where it lands, `e`
/// includes it, and `j` takes whole lines.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Reach {
    Exclusive,
    Inclusive,
    Lines,
}

impl Visual {
    pub fn new(cursor: Pos, top: u32, height: u16) -> Visual {
        Visual {
            cursor,
            top,
            height: height.max(1),
            anchor: None,
            count: None,
            pending: None,
            operator: None,
            typing: None,
            origin: (cursor, top),
            last_search: None,
            last_find: None,
        }
    }

    pub fn resize(&mut self, height: u16) {
        self.height = height.max(1);
    }

    pub fn prompt(&self) -> Option<String> {
        self.typing.as_ref().map(|(forward, q)| format!("{}{q}", if *forward { '/' } else { '?' }))
    }

    pub fn selection(&self) -> Option<Selection> {
        let (anchor, shape) = self.anchor?;
        let (start, end) = (anchor.min(self.cursor), anchor.max(self.cursor));
        Some(match shape {
            Shape::Block => Selection {
                start: Pos { y: start.y, x: anchor.x.min(self.cursor.x) },
                end: Pos { y: end.y, x: anchor.x.max(self.cursor.x) },
                shape,
            },
            _ => Selection { start, end, shape },
        })
    }

    /// Output may have moved the pane since the last key, so `top` is passed in.
    pub fn highlight(&self, top: u32) -> Option<Highlight> {
        let s = self.selection()?;
        let local = |p: Pos| (p.x, p.y as i64 - top as i64);
        Some(Highlight { from: local(s.start), to: local(s.end), shape: s.shape })
    }

    /// What the search being typed, else the last one, matches in the rows on screen.
    pub fn matches(&self, text: &dyn Text, top: u32) -> Vec<Highlight> {
        let query = match &self.typing {
            Some((_, q)) if !q.is_empty() => q.as_str(),
            _ => match &self.last_search {
                Some((q, _)) => q.as_str(),
                None => return Vec::new(),
            },
        };
        let Some(needle) = motion::Needle::new(query) else { return Vec::new() };
        let end = (top + self.height as u32).min(text.rows());
        (top..end)
            .flat_map(|y| {
                let line = text.line(y);
                needle.find_in(&line).into_iter().map(move |(a, b)| Highlight {
                    from: (a, (y - top) as i64),
                    to: (b, (y - top) as i64),
                    shape: Shape::Chars,
                })
            })
            .collect()
    }

    pub fn local_cursor(&self, top: u32) -> Option<(u16, u16)> {
        let y = self.cursor.y.checked_sub(top)?;
        (y < self.height as u32).then_some((self.cursor.x, y as u16))
    }

    pub fn select(&mut self, shape: Shape) {
        self.anchor = Some((self.cursor, shape));
    }

    /// A mouse drag selects characters from where it started to `to`.
    pub fn drag(&mut self, from: Pos, to: Pos) {
        self.cancel();
        self.anchor = Some((from, Shape::Chars));
        self.go(to);
    }

    /// The wheel moves the view, taking the cursor with it as `Ctrl+E`-style paging does.
    pub fn scroll(&mut self, text: &dyn Text, by: i64) {
        self.page(text, by);
    }

    fn toggle(&mut self, shape: Shape) -> Outcome {
        self.anchor = match self.anchor {
            Some((_, s)) if s == shape => None,
            Some((a, _)) => Some((a, shape)),
            None => Some((self.cursor, shape)),
        };
        Outcome::Stay
    }

    fn copy(&mut self, text: &dyn Text) -> Outcome {
        let last = text.cols().saturating_sub(1);
        let s = self.selection().unwrap_or(Selection {
            start: Pos { x: 0, ..self.cursor },
            end: Pos { x: last, ..self.cursor },
            shape: Shape::Lines,
        });
        Outcome::Copy(match s.shape {
            Shape::Lines => Selection { start: Pos { x: 0, ..s.start }, end: Pos { x: last, ..s.end }, ..s },
            _ => s,
        })
    }

    fn escape(&mut self) -> Outcome {
        if self.operator.is_some() || self.pending.is_some() {
            return self.cancel();
        }
        if self.anchor.take().is_some() { Outcome::Stay } else { Outcome::Exit }
    }

    fn page(&mut self, text: &dyn Text, by: i64) -> Outcome {
        let rows = text.rows().max(1) as i64;
        let max_top = (rows - self.height as i64).max(0);
        self.top = (self.top as i64 + by).clamp(0, max_top) as u32;
        self.go(Pos { y: (self.cursor.y as i64 + by).clamp(0, rows - 1) as u32, ..self.cursor });
        Outcome::Stay
    }

    fn go(&mut self, to: Pos) {
        self.cursor = to;
        let h = self.height as u32;
        if to.y < self.top {
            self.top = to.y;
        } else if to.y >= self.top + h {
            self.top = to.y + 1 - h;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::input::{KeyEvent, Mods, key};

    pub struct Lines {
        cells: Vec<Vec<char>>,
        cols: u16,
        pub prompts: Vec<u32>,
    }

    impl Lines {
        pub fn new(lines: &[&str]) -> Lines {
            Lines::from_cells(lines.iter().map(|l| l.chars().collect()).collect())
        }

        pub fn from_cells(cells: Vec<Vec<char>>) -> Lines {
            let cols = cells.iter().map(Vec::len).max().unwrap_or(0).max(1) as u16;
            Lines { cells, cols, prompts: Vec::new() }
        }
    }

    impl Text for Lines {
        fn rows(&self) -> u32 {
            self.cells.len() as u32
        }

        fn cols(&self) -> u16 {
            self.cols
        }

        fn line(&self, y: u32) -> Vec<char> {
            let mut l = self.cells.get(y as usize).cloned().unwrap_or_default();
            l.resize(self.cols as usize, ' ');
            l
        }

        fn is_prompt(&self, y: u32) -> bool {
            self.prompts.contains(&y)
        }
    }

    pub fn press(v: &mut Visual, t: &Lines, keys: &str) -> Outcome {
        keys.chars()
            .map(|c| v.key(&KeyEvent { text: c.to_string(), ..KeyEvent::press(c as u32, Mods::NONE) }, t))
            .last()
            .unwrap()
    }

    pub fn ctrl(v: &mut Visual, t: &Lines, c: char) -> Outcome {
        v.key(&KeyEvent::press(c as u32, Mods::CTRL), t)
    }

    fn text(n: usize) -> Lines {
        let owned: Vec<String> = (0..n).map(|i| format!("line {i}")).collect();
        Lines::new(&owned.iter().map(String::as_str).collect::<Vec<_>>())
    }

    #[test]
    fn moving_off_screen_scrolls_the_view() {
        let t = text(20);
        let mut v = Visual::new(Pos { y: 19, x: 0 }, 16, 4);
        press(&mut v, &t, "k");
        assert_eq!((v.cursor.y, v.top), (18, 16));
        press(&mut v, &t, "gg");
        assert_eq!((v.cursor, v.top), (Pos { y: 0, x: 0 }, 0));
        press(&mut v, &t, "G");
        assert_eq!((v.cursor.y, v.top), (19, 16));
        press(&mut v, &t, "5k");
        assert_eq!((v.cursor.y, v.top), (14, 14));
        ctrl(&mut v, &t, 'u');
        assert_eq!((v.cursor.y, v.top), (12, 12));
    }

    #[test]
    fn v_selects_and_y_copies_in_reading_order() {
        let t = Lines::new(&["hello world", "second"]);
        let mut v = Visual::new(Pos { y: 1, x: 3 }, 0, 4);
        press(&mut v, &t, "v");
        press(&mut v, &t, "k");
        press(&mut v, &t, "0");
        assert_eq!(v.highlight(0), Some(Highlight { from: (0, 0), to: (3, 1), shape: Shape::Chars }));
        assert_eq!(
            press(&mut v, &t, "y"),
            Outcome::Copy(Selection { start: Pos { y: 0, x: 0 }, end: Pos { y: 1, x: 3 }, shape: Shape::Chars })
        );
    }

    #[test]
    fn line_and_block_selections_and_y_without_one() {
        let t = Lines::new(&["abcd", "efgh"]);
        let mut v = Visual::new(Pos { y: 0, x: 2 }, 0, 4);
        assert_eq!(press(&mut v, &t, "y"), Outcome::Stay, "y waits for a motion");
        assert_eq!(
            press(&mut v, &t, "y"),
            Outcome::Copy(Selection { start: Pos { y: 0, x: 0 }, end: Pos { y: 0, x: 3 }, shape: Shape::Lines })
        );
        ctrl(&mut v, &t, 'v');
        press(&mut v, &t, "jh");
        assert_eq!(
            v.selection(),
            Some(Selection { start: Pos { y: 0, x: 1 }, end: Pos { y: 1, x: 2 }, shape: Shape::Block })
        );
        press(&mut v, &t, "V");
        assert_eq!(
            press(&mut v, &t, "y"),
            Outcome::Copy(Selection { start: Pos { y: 0, x: 0 }, end: Pos { y: 1, x: 3 }, shape: Shape::Lines })
        );
    }

    #[test]
    fn a_drag_selects_and_the_wheel_scrolls() {
        let t = text(20);
        let mut v = Visual::new(Pos { y: 19, x: 0 }, 16, 4);
        v.drag(Pos { y: 17, x: 2 }, Pos { y: 18, x: 1 });
        assert_eq!(
            v.selection(),
            Some(Selection { start: Pos { y: 17, x: 2 }, end: Pos { y: 18, x: 1 }, shape: Shape::Chars })
        );
        v.scroll(&t, -3);
        assert_eq!((v.top, v.cursor.y), (13, 15));
    }

    #[test]
    fn escape_clears_the_selection_first_then_leaves() {
        let t = text(3);
        let mut v = Visual::new(Pos::default(), 0, 4);
        press(&mut v, &t, "v");
        let esc = KeyEvent::press(key::ESCAPE, Mods::NONE);
        assert_eq!(v.key(&esc, &t), Outcome::Stay);
        assert!(v.selection().is_none());
        assert_eq!(v.key(&esc, &t), Outcome::Exit);
    }

    #[test]
    fn search_is_typed_then_repeated() {
        let t = Lines::new(&["a err", "b", "c err"]);
        let mut v = Visual::new(Pos::default(), 0, 4);
        press(&mut v, &t, "/er");
        assert_eq!(v.prompt().as_deref(), Some("/er"));
        v.key(&KeyEvent::press(key::ENTER, Mods::NONE), &t);
        assert_eq!((v.cursor, v.prompt()), (Pos { y: 0, x: 2 }, None));
        press(&mut v, &t, "n");
        assert_eq!(v.cursor, Pos { y: 2, x: 2 });
        press(&mut v, &t, "N");
        assert_eq!(v.cursor, Pos { y: 0, x: 2 });
        press(&mut v, &t, "/zz");
        assert_eq!(v.key(&KeyEvent::press(key::ENTER, Mods::NONE), &t), Outcome::Say("not found: zz".into()));
    }

    #[test]
    fn counts_and_prompt_jumps() {
        let mut t = Lines::new(&["$ a", "1", "2", "$ b", "3"]);
        t.prompts = vec![0, 3];
        let mut v = Visual::new(Pos { y: 4, x: 0 }, 0, 5);
        press(&mut v, &t, "[");
        assert_eq!(v.cursor.y, 3);
        press(&mut v, &t, "[");
        assert_eq!(v.cursor.y, 0);
        assert_eq!(press(&mut v, &t, "["), Outcome::Say("no more prompts".into()));
        press(&mut v, &t, "3j");
        assert_eq!(v.cursor.y, 3);
        press(&mut v, &t, "0");
        assert_eq!(v.cursor.x, 0, "0 without a count is the line start");
    }
}
