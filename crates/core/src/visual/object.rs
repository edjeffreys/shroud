//! Text objects (`iw`, `a"`, `i(`…). Words and quotes stay on one line, as in nvim; brackets
//! may span lines, so their search is bounded to keep a key cheap in a long history.

use super::motion::{Class, Cursor, class_of};
use super::{Pos, Text};

const BRACKET_ROWS: u32 = 1000;

/// Both ends inclusive. `kind` is the key typed after `i` or `a`.
pub fn object(text: &dyn Text, at: Pos, around: bool, kind: char) -> Option<(Pos, Pos)> {
    match kind {
        'w' | 'W' => Some(word(text, at, around, kind == 'W')),
        '"' | '\'' | '`' => quote(text, at, around, kind),
        '(' | ')' | 'b' => pair(text, at, around, '(', ')'),
        '[' | ']' => pair(text, at, around, '[', ']'),
        '{' | '}' | 'B' => pair(text, at, around, '{', '}'),
        '<' | '>' => pair(text, at, around, '<', '>'),
        _ => None,
    }
}

fn word(text: &dyn Text, at: Pos, around: bool, big: bool) -> (Pos, Pos) {
    // A wide glyph's second column belongs to its first.
    let mut line = text.line(at.y);
    for i in 1..line.len() {
        if line[i] == '\0' {
            line[i] = line[i - 1];
        }
    }
    let n = line.len();
    let cls = |i: usize| class_of(line.get(i).copied().unwrap_or(' '), big);
    let x = (at.x as usize).min(n.saturating_sub(1));
    let run = |from: usize| {
        let k = cls(from);
        let (mut s, mut e) = (from, from);
        while s > 0 && cls(s - 1) == k {
            s -= 1;
        }
        while e + 1 < n && cls(e + 1) == k {
            e += 1;
        }
        (s, e)
    };
    let (mut s, mut e) = run(x);
    if around {
        if cls(x) == Class::Blank {
            if e + 1 < n {
                e = run(e + 1).1;
            }
        } else if e + 1 < n && cls(e + 1) == Class::Blank {
            e = run(e + 1).1;
        } else if s > 0 && cls(s - 1) == Class::Blank {
            s = run(s - 1).0;
        }
    }
    (Pos { x: s as u16, y: at.y }, Pos { x: e as u16, y: at.y })
}

/// Quotes pair up from the line start, so the cursor between two pairs picks the next one.
fn quote(text: &dyn Text, at: Pos, around: bool, q: char) -> Option<(Pos, Pos)> {
    let line = text.line(at.y);
    let marks: Vec<usize> = line.iter().enumerate().filter(|&(_, &c)| c == q).map(|(i, _)| i).collect();
    let x = at.x as usize;
    let pairs = marks.as_chunks::<2>().0;
    let &[a, b] = pairs.iter().find(|&&[a, b]| a <= x && x <= b).or_else(|| pairs.iter().find(|&&[a, _]| a > x))?;
    let (s, mut e) = if around { (a, b) } else { (a + 1, b.checked_sub(1)?) };
    if s > e {
        return None;
    }
    if around {
        while e + 1 < line.len() && line[e + 1] == ' ' {
            e += 1;
        }
    }
    Some((Pos { x: s as u16, y: at.y }, Pos { x: e as u16, y: at.y }))
}

fn pair(text: &dyn Text, at: Pos, around: bool, open: char, close: char) -> Option<(Pos, Pos)> {
    let mut c = Cursor::new(text, at.y, false);
    let start = scan(&mut c, at, open, close, false)?;
    let end = scan(&mut c, at, close, open, true)?;
    if around {
        return Some((start, end));
    }
    let (s, _) = c.next(start)?;
    let (e, _) = c.prev(end)?;
    (s <= e).then_some((s, e))
}

/// The unmatched `want` nearest `from`, counting `other`s to skip nested pairs.
fn scan(c: &mut Cursor, from: Pos, want: char, other: char, forward: bool) -> Option<Pos> {
    let mut depth = 0u32;
    let mut p = from;
    loop {
        let ch = c.char_at(p);
        if ch == want {
            if depth == 0 {
                return Some(p);
            }
            depth -= 1;
        } else if ch == other && p != from {
            depth += 1;
        }
        let (n, _) = if forward { c.next(p)? } else { c.prev(p)? };
        if n.y.abs_diff(from.y) > BRACKET_ROWS {
            return None;
        }
        p = n;
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::Lines;
    use super::*;

    fn p(x: u16, y: u32) -> Pos {
        Pos { x, y }
    }

    fn text_of(t: &Lines, r: Option<(Pos, Pos)>) -> Option<String> {
        let (s, e) = r?;
        let mut out = String::new();
        for y in s.y..=e.y {
            let line = t.line(y);
            let (a, b) =
                (if y == s.y { s.x as usize } else { 0 }, if y == e.y { e.x as usize } else { line.len() - 1 });
            out.extend(line[a..=b].iter().filter(|&&c| c != '\0'));
            if y != e.y {
                out.push('\n');
            }
        }
        Some(out.trim_end().to_string())
    }

    #[test]
    fn words_and_their_blanks() {
        let t = Lines::new(&["foo.bar  baz"]);
        assert_eq!(text_of(&t, object(&t, p(1, 0), false, 'w')).as_deref(), Some("foo"));
        assert_eq!(object(&t, p(5, 0), true, 'w'), Some((p(4, 0), p(8, 0))), "trailing blanks");
        assert_eq!(object(&t, p(10, 0), true, 'w'), Some((p(7, 0), p(11, 0))), "leading blanks at the line end");
        assert_eq!(text_of(&t, object(&t, p(1, 0), false, 'W')).as_deref(), Some("foo.bar"));
        assert_eq!(
            object(&t, p(7, 0), true, 'w'),
            Some((p(7, 0), p(11, 0))),
            "from a blank, the blank and the next word"
        );
        let wide = Lines::from_cells(vec![vec!['中', '\0', '文', '\0', ' ', 'x']]);
        assert_eq!(object(&wide, p(1, 0), false, 'w'), Some((p(0, 0), p(3, 0))));
    }

    #[test]
    fn quotes_pair_from_the_line_start() {
        let t = Lines::new(&[r#"say "hi" and "bye" now"#]);
        assert_eq!(text_of(&t, object(&t, p(5, 0), false, '"')).as_deref(), Some("hi"));
        assert_eq!(text_of(&t, object(&t, p(5, 0), true, '"')).as_deref(), Some(r#""hi""#));
        assert_eq!(object(&t, p(5, 0), true, '"'), Some((p(4, 0), p(8, 0))), "with the blank after");
        assert_eq!(text_of(&t, object(&t, p(0, 0), false, '"')).as_deref(), Some("hi"), "before any, the next pair");
        assert_eq!(
            text_of(&t, object(&t, p(10, 0), false, '"')).as_deref(),
            Some("bye"),
            "between pairs, the next one"
        );
        let empty = Lines::new(&[r#"x "" y"#]);
        assert_eq!(object(&empty, p(2, 0), false, '"'), None);
    }

    #[test]
    fn brackets_nest_and_span_lines() {
        let t = Lines::new(&["f(a, (b), c)"]);
        assert_eq!(text_of(&t, object(&t, p(3, 0), false, '(')).as_deref(), Some("a, (b), c"));
        assert_eq!(text_of(&t, object(&t, p(6, 0), false, 'b')).as_deref(), Some("b"));
        assert_eq!(text_of(&t, object(&t, p(5, 0), true, ')')).as_deref(), Some("(b)"), "on the open bracket");
        assert_eq!(
            text_of(&t, object(&t, p(11, 0), true, '(')).as_deref(),
            Some("(a, (b), c)"),
            "on the close bracket"
        );
        let multi = Lines::new(&["fn x() {", "  body", "}"]);
        assert_eq!(
            text_of(&multi, object(&multi, p(2, 1), false, '{')).as_deref(),
            Some("  body"),
            "the line break after the brace is left out"
        );
        assert_eq!(object(&t, p(0, 0), false, '('), None, "outside any pair");
        assert_eq!(object(&t, p(0, 0), false, 'q'), None);
    }
}
