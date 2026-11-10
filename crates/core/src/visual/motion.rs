//! Where nvim's motions land in a pane's history. Reads text only through `Text`.

use super::{Pos, Text};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Class {
    Blank,
    Word,
    Punct,
}

pub(super) fn class(c: char) -> Class {
    match c {
        ' ' | '\0' | '\t' => Class::Blank,
        c if c.is_alphanumeric() || c == '_' => Class::Word,
        _ => Class::Punct,
    }
}

/// A WORD (`W`, `iW`) is anything between blanks.
pub(super) fn class_of(c: char, big: bool) -> Class {
    match class(c) {
        Class::Punct if big => Class::Word,
        k => k,
    }
}

/// Caches the line, since motions revisit it cell by cell.
pub(super) struct Cursor<'a> {
    text: &'a dyn Text,
    y: u32,
    line: Vec<char>,
    big: bool,
}

impl<'a> Cursor<'a> {
    pub(super) fn new(text: &'a dyn Text, y: u32, big: bool) -> Cursor<'a> {
        Cursor { text, y, line: text.line(y), big }
    }

    fn at(&mut self, p: Pos) -> Class {
        class_of(self.char_at(p), self.big)
    }

    pub(super) fn char_at(&mut self, p: Pos) -> char {
        if p.y != self.y {
            self.y = p.y;
            self.line = self.text.line(p.y);
        }
        self.line.get(p.x as usize).copied().unwrap_or(' ')
    }

    /// True when a line end was crossed: that always ends a word.
    pub(super) fn next(&self, p: Pos) -> Option<(Pos, bool)> {
        if p.x + 1 < self.text.cols() {
            Some((Pos { x: p.x + 1, ..p }, false))
        } else if p.y + 1 < self.text.rows() {
            Some((Pos { y: p.y + 1, x: 0 }, true))
        } else {
            None
        }
    }

    pub(super) fn prev(&self, p: Pos) -> Option<(Pos, bool)> {
        if p.x > 0 {
            Some((Pos { x: p.x - 1, ..p }, false))
        } else if p.y > 0 {
            Some((Pos { y: p.y - 1, x: self.text.cols().saturating_sub(1) }, true))
        } else {
            None
        }
    }
}

pub fn word_forward(text: &dyn Text, from: Pos, big: bool) -> Pos {
    let mut c = Cursor::new(text, from.y, big);
    let start = c.at(from);
    let mut p = from;
    loop {
        let Some((n, crossed)) = c.next(p) else { return p };
        p = n;
        if crossed || c.at(p) != start {
            break;
        }
    }
    while c.at(p) == Class::Blank {
        let Some((n, _)) = c.next(p) else { return p };
        p = n;
    }
    p
}

pub fn word_back(text: &dyn Text, from: Pos, big: bool) -> Pos {
    let mut c = Cursor::new(text, from.y, big);
    let Some((mut p, _)) = c.prev(from) else { return from };
    while c.at(p) == Class::Blank {
        let Some((n, _)) = c.prev(p) else { return p };
        p = n;
    }
    let kind = c.at(p);
    while let Some((n, false)) = c.prev(p) {
        if c.at(n) != kind {
            break;
        }
        p = n;
    }
    p
}

pub fn word_end(text: &dyn Text, from: Pos, big: bool) -> Pos {
    let mut c = Cursor::new(text, from.y, big);
    let Some((mut p, _)) = c.next(from) else { return from };
    while c.at(p) == Class::Blank {
        let Some((n, _)) = c.next(p) else { return p };
        p = n;
    }
    let kind = c.at(p);
    while let Some((n, false)) = c.next(p) {
        if c.at(n) != kind {
            break;
        }
        p = n;
    }
    p
}

/// `ge`: the end of the previous word.
pub fn word_end_back(text: &dyn Text, from: Pos, big: bool) -> Pos {
    let mut c = Cursor::new(text, from.y, big);
    let start = c.at(from);
    let mut p = from;
    while start != Class::Blank && c.at(p) == start {
        match c.prev(p) {
            Some((n, false)) => p = n,
            Some((n, true)) => {
                p = n;
                break;
            }
            None => return from,
        }
    }
    while c.at(p) == Class::Blank {
        let Some((n, _)) = c.prev(p) else { return p };
        p = n;
    }
    p
}

/// `f`, `t`, `F` and `T`: the line only, as in nvim. `till` stops one short of the char.
pub fn find_in_line(text: &dyn Text, from: Pos, target: char, forward: bool, till: bool) -> Option<Pos> {
    let line = text.line(from.y);
    let x = from.x as usize;
    let hit = if forward {
        (x + 1..line.len()).find(|&i| line[i] == target)?
    } else {
        (0..x).rev().find(|&i| line[i] == target)?
    };
    let x = match (till, forward) {
        (false, _) => hit,
        (true, true) => hit - 1,
        (true, false) => hit + 1,
    };
    Some(Pos { x: x as u16, ..from })
}

pub fn first_non_blank(text: &dyn Text, y: u32) -> u16 {
    text.line(y).iter().position(|&ch| class(ch) != Class::Blank).unwrap_or(0) as u16
}

pub fn last_non_blank(text: &dyn Text, y: u32) -> u16 {
    text.line(y).iter().rposition(|&ch| class(ch) != Class::Blank).unwrap_or(0) as u16
}

/// Smartcase, as most nvim configs set it: an uppercase letter makes the search exact.
pub struct Needle {
    chars: Vec<char>,
    exact: bool,
}

impl Needle {
    pub fn new(text: &str) -> Option<Needle> {
        let exact = text.chars().any(char::is_uppercase);
        let mut n = Needle { chars: Vec::new(), exact };
        n.chars = text.chars().map(|c| n.fold(c)).collect();
        (!n.chars.is_empty()).then_some(n)
    }

    fn fold(&self, c: char) -> char {
        if self.exact { c } else { c.to_lowercase().next().unwrap_or(c) }
    }

    /// First and last column of each match. Wide glyphs leave a '\0' column, which is skipped
    /// so a match can span them.
    pub fn find_in(&self, line: &[char]) -> Vec<(u16, u16)> {
        let cells: Vec<(u16, char)> =
            line.iter().enumerate().filter(|&(_, &c)| c != '\0').map(|(x, &c)| (x as u16, self.fold(c))).collect();
        let tail = |x: u16| if line.get(x as usize + 1) == Some(&'\0') { x + 1 } else { x };
        cells
            .windows(self.chars.len())
            .filter(|w| w.iter().map(|&(_, c)| c).eq(self.chars.iter().copied()))
            .map(|w| (w[0].0, tail(w[w.len() - 1].0)))
            .collect()
    }
}

/// Over `lines`, the whole history read at once, since reading it row by row is slow.
pub fn search(lines: &[Vec<char>], from: Pos, needle: &str, forward: bool) -> Option<Pos> {
    let needle = Needle::new(needle)?;
    let rows = lines.len() as u32;
    if rows == 0 {
        return None;
    }
    for step in 0..=rows {
        let y = if forward { (from.y + step) % rows } else { (from.y + rows - step % rows) % rows };
        let starts = needle.find_in(&lines[y as usize]).into_iter().map(|(x, _)| x);
        let found = if forward {
            starts.filter(|&x| step > 0 || x > from.x).min()
        } else {
            starts.filter(|&x| step > 0 || x < from.x).max()
        };
        if let Some(x) = found {
            return Some(Pos { y, x });
        }
    }
    None
}

pub fn prompt(text: &dyn Text, from: u32, n: i32) -> Option<u32> {
    let mut y = from;
    let mut found = None;
    for _ in 0..n.unsigned_abs() {
        let next = if n < 0 {
            (0..y).rev().find(|&r| text.is_prompt(r))
        } else {
            (y + 1..text.rows()).find(|&r| text.is_prompt(r))
        };
        y = next?;
        found = Some(y);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::super::tests::Lines;
    use super::*;

    fn p(x: u16, y: u32) -> Pos {
        Pos { x, y }
    }

    #[test]
    fn words_split_on_class_and_line_ends() {
        let t = Lines::new(&["foo.bar  baz", "qux"]);
        assert_eq!(word_forward(&t, p(0, 0), false), p(3, 0));
        assert_eq!(word_forward(&t, p(3, 0), false), p(4, 0));
        assert_eq!(word_forward(&t, p(4, 0), false), p(9, 0));
        assert_eq!(word_forward(&t, p(9, 0), false), p(0, 1), "past the trailing blanks");
        assert_eq!(word_back(&t, p(0, 1), false), p(9, 0));
        assert_eq!(word_back(&t, p(9, 0), false), p(4, 0));
        assert_eq!(word_end(&t, p(0, 0), false), p(2, 0));
        assert_eq!(word_end(&t, p(4, 0), false), p(6, 0));
        assert_eq!(word_end_back(&t, p(9, 0), false), p(6, 0));
        assert_eq!(word_end_back(&t, p(5, 0), false), p(3, 0));
        assert_eq!(word_end_back(&t, p(0, 1), false), p(11, 0));
    }

    #[test]
    fn words_ignore_punctuation() {
        let t = Lines::new(&["foo.bar  baz", "qux"]);
        assert_eq!(word_forward(&t, p(0, 0), true), p(9, 0));
        assert_eq!(word_end(&t, p(0, 0), true), p(6, 0));
        assert_eq!(word_back(&t, p(9, 0), true), p(0, 0));
        assert_eq!(word_end_back(&t, p(9, 0), true), p(6, 0));
    }

    #[test]
    fn finds_stay_on_the_line() {
        let t = Lines::new(&["a(b, c)", "x,"]);
        assert_eq!(find_in_line(&t, p(0, 0), ',', true, false), Some(p(3, 0)));
        assert_eq!(find_in_line(&t, p(0, 0), ',', true, true), Some(p(2, 0)));
        assert_eq!(find_in_line(&t, p(6, 0), '(', false, false), Some(p(1, 0)));
        assert_eq!(find_in_line(&t, p(6, 0), '(', false, true), Some(p(2, 0)));
        assert_eq!(find_in_line(&t, p(2, 0), ',', true, true), Some(p(2, 0)), "already next to it");
        assert_eq!(find_in_line(&t, p(0, 0), 'x', true, false), None);
    }

    #[test]
    fn line_ends_skip_blanks() {
        let t = Lines::new(&["  hi  "]);
        assert_eq!((first_non_blank(&t, 0), last_non_blank(&t, 0)), (2, 3));
    }

    #[test]
    fn search_wraps_is_smartcase_and_spans_wide_glyphs() {
        let t = Lines::new(&["Error one", "ok", "error two"]);
        let t = t.all_lines();
        assert_eq!(search(&t, p(0, 0), "error", true), Some(p(0, 2)));
        assert_eq!(search(&t, p(0, 2), "error", true), Some(p(0, 0)), "wraps");
        assert_eq!(search(&t, p(0, 2), "Error", true), Some(p(0, 0)));
        assert_eq!(search(&t, p(0, 0), "Error", true), Some(p(0, 0)), "only match, found again after wrapping");
        assert_eq!(search(&t, p(0, 0), "error", false), Some(p(0, 2)));
        assert_eq!(search(&t, p(0, 0), "nope", true), None);
        let wide = vec![vec!['a', '中', '\0', 'b']];
        assert_eq!(search(&wide, p(0, 0), "中b", true), Some(p(1, 0)));
        assert_eq!(
            Needle::new("中b").unwrap().find_in(&wide[0]),
            [(1, 3)],
            "a match ends past a wide glyph's second column"
        );
    }

    #[test]
    fn prompts_count_in_either_direction() {
        let mut t = Lines::new(&["$ a", "out", "$ b", "out", "$ c"]);
        t.prompts = vec![0, 2, 4];
        assert_eq!(prompt(&t, 4, -1), Some(2));
        assert_eq!(prompt(&t, 4, -2), Some(0));
        assert_eq!(prompt(&t, 0, 1), Some(2));
        assert_eq!(prompt(&t, 0, -1), None);
    }
}
