//! The keys of visual mode, as nvim reads them: counts, pending keys, and `y` as an operator
//! that copies whatever motion or text object follows it.

use super::{Outcome, Pending, Pos, Reach, Selection, Text, Visual, motion, object};
use crate::input::chord::{Chord, Key};
use crate::input::{KeyAction, KeyEvent, Mods, key};
use crate::render::Shape;

impl Visual {
    pub fn key(&mut self, ev: &KeyEvent, text: &dyn Text) -> Outcome {
        if ev.action == KeyAction::Release {
            return Outcome::Stay;
        }
        if self.typing.is_some() {
            return self.typing_key(ev, text);
        }
        let Some(chord) = Chord::from_event(ev) else { return Outcome::Stay };
        let plain = match chord.key {
            Key::Char(ch) if chord.mods == Mods::NONE => Some(ch),
            _ => None,
        };
        let pending = self.pending.take();
        // These take the next key as data, so a digit there is a target, not a count.
        match (pending, plain) {
            (Some(Pending::Find { forward, till }), Some(ch)) => {
                self.last_find = Some((ch, forward, till));
                let n = self.count.take().unwrap_or(1);
                return self.find(text, ch, forward, till, n, false);
            }
            (Some(Pending::Object { around }), Some(ch)) => return self.object(text, around, ch),
            (Some(Pending::Find { .. } | Pending::Object { .. }), None) => return self.cancel(),
            _ => {}
        }
        let g = matches!(pending, Some(Pending::G));
        if let Some(d) = chord.digit()
            && (d != 0 || self.count.is_some())
        {
            self.count = Some(self.count.unwrap_or(0).saturating_mul(10).saturating_add(d).min(99_999));
            return Outcome::Stay;
        }
        let n = self.count.take().unwrap_or(1);
        let half = (self.height / 2).max(1) as u32;
        let ctrl = chord.mods == Mods::CTRL;
        let ch = match chord.key {
            Key::Char(ch) if chord.mods == Mods::NONE || ctrl => ch,
            Key::Named(k) => match k {
                key::LEFT => 'h',
                key::RIGHT => 'l',
                key::UP => 'k',
                key::DOWN => 'j',
                key::HOME => '0',
                key::END => '$',
                key::PAGE_UP => return self.page(text, -(self.height as i64)),
                key::PAGE_DOWN => return self.page(text, self.height as i64),
                key::ENTER => return self.copy(text),
                key::ESCAPE => return self.escape(),
                _ => return Outcome::Stay,
            },
            _ => return Outcome::Stay,
        };
        if ctrl {
            self.operator = None;
            return match ch {
                'd' => self.page(text, half as i64 * n as i64),
                'u' => self.page(text, -(half as i64) * n as i64),
                'f' => self.page(text, self.height as i64 * n as i64),
                'b' => self.page(text, -(self.height as i64) * n as i64),
                'v' => self.toggle(Shape::Block),
                'c' => Outcome::Exit,
                _ => Outcome::Stay,
            };
        }
        match ch {
            'g' if !g => {
                self.pending = Some(Pending::G);
                Outcome::Stay
            }
            'f' | 'F' | 't' | 'T' => {
                self.pending = Some(Pending::Find { forward: ch.is_lowercase(), till: ch.eq_ignore_ascii_case(&'t') });
                // The count belongs to the find, which reads it once its target arrives.
                self.count = Some(n);
                Outcome::Stay
            }
            ';' | ',' => match self.last_find {
                Some((target, forward, till)) => self.find(text, target, forward == (ch == ';'), till, n, true),
                None => Outcome::Stay,
            },
            'i' | 'a' if self.operator.is_some() || self.anchor.is_some() => {
                self.pending = Some(Pending::Object { around: ch == 'a' });
                Outcome::Stay
            }
            'y' if self.anchor.is_some() => self.copy(text),
            'y' if self.operator.is_some() => self.yank_lines(text, n),
            'y' => {
                self.operator = Some(n);
                Outcome::Stay
            }
            'Y' => match self.selection() {
                Some(s) => {
                    self.anchor = Some((s.start, Shape::Lines));
                    self.copy(text)
                }
                None => self.yank_lines(text, n),
            },
            'o' => {
                if let Some((a, shape)) = self.anchor {
                    self.anchor = Some((self.cursor, shape));
                    self.go(a);
                }
                Outcome::Stay
            }
            '/' | '?' => {
                self.operator = None;
                self.typing = Some((ch == '/', String::new()));
                self.origin = (self.cursor, self.top);
                Outcome::Stay
            }
            'n' | 'N' => self.search_again(text, ch == 'n'),
            'v' => self.toggle(Shape::Chars),
            'V' => self.toggle(Shape::Lines),
            'q' => Outcome::Exit,
            _ => match self.motion(text, ch, g, n) {
                Ok(Some((to, reach))) => self.arrive(text, to, reach),
                Ok(None) => self.cancel(),
                Err(say) => {
                    self.operator = None;
                    Outcome::Say(say)
                }
            },
        }
    }

    /// Where a motion key lands, and how a yank over it is cut. `None` for a key that isn't one.
    fn motion(&self, text: &dyn Text, ch: char, g: bool, n: u32) -> Result<Option<(Pos, Reach)>, String> {
        let n = n.saturating_mul(self.operator.unwrap_or(1));
        let (rows, cols) = (text.rows().max(1), text.cols().max(1));
        let c = self.cursor;
        let times = |f: &dyn Fn(Pos) -> Pos| (0..n).fold(c, |p, _| f(p));
        Ok(Some(match ch {
            'g' => (Pos { y: 0, x: 0 }, Reach::Lines),
            'e' | 'E' if g => (times(&|p| motion::word_end_back(text, p, ch == 'E')), Reach::Inclusive),
            _ if g => return Ok(None),
            'h' => (Pos { x: c.x.saturating_sub(n as u16), ..c }, Reach::Exclusive),
            'l' => (Pos { x: (c.x as u32 + n).min(cols as u32 - 1) as u16, ..c }, Reach::Exclusive),
            'j' => (Pos { y: (c.y + n).min(rows - 1), ..c }, Reach::Lines),
            'k' => (Pos { y: c.y.saturating_sub(n), ..c }, Reach::Lines),
            '0' => (Pos { x: 0, ..c }, Reach::Exclusive),
            '^' => (Pos { x: motion::first_non_blank(text, c.y), ..c }, Reach::Exclusive),
            '$' => (Pos { x: motion::last_non_blank(text, c.y), ..c }, Reach::Inclusive),
            'w' | 'W' => (times(&|p| motion::word_forward(text, p, ch == 'W')), Reach::Exclusive),
            'b' | 'B' => (times(&|p| motion::word_back(text, p, ch == 'B')), Reach::Exclusive),
            'e' | 'E' => (times(&|p| motion::word_end(text, p, ch == 'E')), Reach::Inclusive),
            'G' => (Pos { y: rows - 1, x: 0 }, Reach::Lines),
            'H' => (Pos { y: self.top, ..c }, Reach::Lines),
            'M' => (Pos { y: (self.top + self.height as u32 / 2).min(rows - 1), ..c }, Reach::Lines),
            'L' => (Pos { y: (self.top + self.height as u32 - 1).min(rows - 1), ..c }, Reach::Lines),
            '[' | ']' => match motion::prompt(text, c.y, if ch == '[' { -(n as i32) } else { n as i32 }) {
                Some(y) => (Pos { y, x: 0 }, Reach::Lines),
                None => return Err("no more prompts".into()),
            },
            _ => return Ok(None),
        }))
    }

    fn arrive(&mut self, text: &dyn Text, to: Pos, reach: Reach) -> Outcome {
        if self.operator.take().is_none() {
            self.go(to);
            return Outcome::Stay;
        }
        let last = text.cols().saturating_sub(1);
        let (s, e) = (self.cursor.min(to), self.cursor.max(to));
        Outcome::Copy(match reach {
            Reach::Lines => Selection { start: Pos { x: 0, ..s }, end: Pos { x: last, ..e }, shape: Shape::Lines },
            Reach::Inclusive => Selection { start: s, end: e, shape: Shape::Chars },
            Reach::Exclusive => {
                // An exclusive motion to a line start stops at the end of the line before, as `yw` on a last word does.
                let end = match (e.x, e.y > s.y) {
                    (_, _) if s == e => return Outcome::Stay,
                    (0, true) => Pos { y: e.y - 1, x: last },
                    _ => Pos { x: e.x.saturating_sub(1), ..e },
                };
                Selection { start: s, end, shape: Shape::Chars }
            }
        })
    }

    fn find(&mut self, text: &dyn Text, target: char, forward: bool, till: bool, n: u32, again: bool) -> Outcome {
        let mut p = self.cursor;
        // Repeating `t` from just before its char would find that same char again.
        if again && till {
            let x = if forward { p.x as u32 + 1 } else { (p.x as u32).saturating_sub(1) };
            p.x = x.min(text.cols().saturating_sub(1) as u32) as u16;
        }
        for i in 0..n {
            match motion::find_in_line(text, p, target, forward, till && i + 1 == n) {
                Some(to) => p = to,
                None => return self.cancel(),
            }
        }
        self.arrive(text, p, if forward { Reach::Inclusive } else { Reach::Exclusive })
    }

    fn object(&mut self, text: &dyn Text, around: bool, kind: char) -> Outcome {
        let Some((start, end)) = object::object(text, self.cursor, around, kind) else { return self.cancel() };
        if self.operator.take().is_some() {
            return Outcome::Copy(Selection { start, end, shape: Shape::Chars });
        }
        self.anchor = Some((start, Shape::Chars));
        self.go(end);
        Outcome::Stay
    }

    fn yank_lines(&mut self, text: &dyn Text, n: u32) -> Outcome {
        let n = n.saturating_mul(self.operator.take().unwrap_or(1));
        let end = (self.cursor.y + n - 1).min(text.rows().max(1) - 1);
        Outcome::Copy(Selection {
            start: Pos { y: self.cursor.y, x: 0 },
            end: Pos { y: end, x: text.cols().saturating_sub(1) },
            shape: Shape::Lines,
        })
    }

    pub(super) fn cancel(&mut self) -> Outcome {
        self.operator = None;
        self.pending = None;
        self.count = None;
        Outcome::Stay
    }

    fn typing_key(&mut self, ev: &KeyEvent, text: &dyn Text) -> Outcome {
        let Some((forward, query)) = &mut self.typing else { return Outcome::Stay };
        let forward = *forward;
        match ev.code {
            key::ESCAPE => return self.end_search(true),
            key::BACKSPACE if query.is_empty() => return self.end_search(true),
            key::BACKSPACE => {
                query.pop();
            }
            key::ENTER | key::KP_ENTER => {
                let query = std::mem::take(query);
                if query.is_empty() {
                    self.end_search(false);
                    return self.search_again(text, true);
                }
                let found = self.cursor != self.origin.0;
                self.last_search = Some((query.clone(), forward));
                return if found {
                    self.end_search(false)
                } else {
                    self.end_search(true);
                    Outcome::Say(format!("not found: {query}"))
                };
            }
            _ if ![Mods::CTRL, Mods::ALT, Mods::SUPER].iter().any(|&m| ev.mods.contains(m))
                && ev.text.chars().all(|c| !c.is_control()) =>
            {
                query.push_str(&ev.text)
            }
            _ => return Outcome::Stay,
        }
        // Incremental, as nvim's incsearch: each change searches again from where it began.
        let query = query.clone();
        let (from, top) = self.origin;
        self.top = top;
        match motion::search(&text.all_lines(), from, &query, forward) {
            Some(p) if !query.is_empty() => self.go(p),
            _ => self.cursor = from,
        }
        Outcome::Stay
    }

    fn end_search(&mut self, restore: bool) -> Outcome {
        self.typing = None;
        if restore {
            (self.cursor, self.top) = self.origin;
        }
        Outcome::Stay
    }

    /// `N` goes against the direction the search was typed in.
    fn search_again(&mut self, text: &dyn Text, same_way: bool) -> Outcome {
        let Some((query, forward)) = self.last_search.clone() else { return Outcome::Say("no previous search".into()) };
        match motion::search(&text.all_lines(), self.cursor, &query, forward == same_way) {
            Some(p) => {
                self.go(p);
                Outcome::Stay
            }
            None => Outcome::Say(format!("not found: {query}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{Lines, press};
    use super::*;
    use crate::render::Highlight;

    fn sel(a: (u16, u32), b: (u16, u32)) -> Outcome {
        Outcome::Copy(Selection { start: Pos { x: a.0, y: a.1 }, end: Pos { x: b.0, y: b.1 }, shape: Shape::Chars })
    }

    #[test]
    fn y_takes_a_motion_cut_as_nvim_cuts_it() {
        let t = Lines::new(&["foo.bar baz", "next line"]);
        let at = |x| Visual::new(Pos { y: 0, x }, 0, 4);
        assert_eq!(press(&mut at(0), &t, "yw"), sel((0, 0), (2, 0)), "w is exclusive");
        assert_eq!(press(&mut at(0), &t, "yW"), sel((0, 0), (7, 0)));
        assert_eq!(press(&mut at(0), &t, "ye"), sel((0, 0), (2, 0)), "e is inclusive");
        assert_eq!(press(&mut at(0), &t, "y$"), sel((0, 0), (10, 0)));
        assert_eq!(press(&mut at(8), &t, "yw"), sel((8, 0), (10, 0)), "the last word stops at the line end");
        assert_eq!(press(&mut at(4), &t, "y0"), sel((0, 0), (3, 0)));
        assert_eq!(press(&mut at(0), &t, "2yw"), press(&mut at(0), &t, "y2w"));
        assert_eq!(press(&mut at(0), &t, "yt."), sel((0, 0), (2, 0)));
        assert_eq!(press(&mut at(0), &t, "yf."), sel((0, 0), (3, 0)));
        assert_eq!(press(&mut at(5), &t, "yiw"), sel((4, 0), (6, 0)));
        assert_eq!(press(&mut at(5), &t, "yaW"), sel((0, 0), (7, 0)));
        assert_eq!(
            press(&mut at(3), &t, "yj"),
            Outcome::Copy(Selection { start: Pos { y: 0, x: 0 }, end: Pos { y: 1, x: 10 }, shape: Shape::Lines })
        );
        assert_eq!(press(&mut at(0), &t, "Y"), press(&mut at(0), &t, "yy"));
        let mut v = at(0);
        press(&mut v, &t, "y");
        assert_eq!(v.key(&KeyEvent::press(key::ESCAPE, Mods::NONE), &t), Outcome::Stay);
        assert_eq!(press(&mut v, &t, "w"), Outcome::Stay, "escape dropped the y");
        assert_eq!(v.cursor.x, 3);
    }

    #[test]
    fn finds_repeat_and_objects_select() {
        let t = Lines::new(&["a,b,c,d", r#"say "hi there" ok"#]);
        let mut v = Visual::new(Pos::default(), 0, 4);
        press(&mut v, &t, "f,");
        assert_eq!(v.cursor.x, 1);
        press(&mut v, &t, ";");
        assert_eq!(v.cursor.x, 3);
        press(&mut v, &t, ",");
        assert_eq!(v.cursor.x, 1);
        press(&mut v, &t, "0t,;");
        assert_eq!(v.cursor.x, 2, "; after t moves on to the next one");
        press(&mut v, &t, "02f,");
        assert_eq!(v.cursor.x, 3);
        let mut v = Visual::new(Pos { y: 1, x: 6 }, 0, 4);
        press(&mut v, &t, "vi\"");
        assert_eq!(
            v.selection(),
            Some(Selection { start: Pos { y: 1, x: 5 }, end: Pos { y: 1, x: 12 }, shape: Shape::Chars })
        );
        press(&mut v, &t, "o");
        assert_eq!(v.cursor.x, 5, "o swaps the ends");
        assert_eq!(press(&mut v, &t, "y"), sel((5, 1), (12, 1)));
    }

    #[test]
    fn search_moves_as_you_type_and_escape_goes_back() {
        let t = Lines::new(&["start", "alpha", "beta", "alphabet"]);
        let mut v = Visual::new(Pos::default(), 0, 4);
        press(&mut v, &t, "/al");
        assert_eq!(v.cursor, Pos { y: 1, x: 0 });
        press(&mut v, &t, "phab");
        assert_eq!(v.cursor, Pos { y: 3, x: 0 }, "the longer query only matches further on");
        assert_eq!(v.matches(&t, 0), [Highlight { from: (0, 3), to: (5, 3), shape: Shape::Chars }]);
        v.key(&KeyEvent::press(key::ESCAPE, Mods::NONE), &t);
        assert_eq!((v.cursor, v.prompt()), (Pos::default(), None));
        assert!(v.matches(&t, 0).is_empty(), "an abandoned search highlights nothing");
        press(&mut v, &t, "/bet");
        v.key(&KeyEvent::press(key::ENTER, Mods::NONE), &t);
        assert_eq!(v.cursor, Pos { y: 2, x: 0 });
        assert_eq!(v.matches(&t, 0).len(), 2, "after Enter the matches stay lit");
        press(&mut v, &t, "/zz");
        assert_eq!(v.key(&KeyEvent::press(key::ENTER, Mods::NONE), &t), Outcome::Say("not found: zz".into()));
        assert_eq!(v.cursor, Pos { y: 2, x: 0 }, "a failed search goes back");
    }

    #[test]
    fn ge_and_word_motions() {
        let t = Lines::new(&["one two.three"]);
        let mut v = Visual::new(Pos { y: 0, x: 12 }, 0, 4);
        press(&mut v, &t, "ge");
        assert_eq!(v.cursor.x, 7);
        press(&mut v, &t, "gE");
        assert_eq!(v.cursor.x, 2);
        press(&mut v, &t, "W");
        assert_eq!(v.cursor.x, 4);
        press(&mut v, &t, "E");
        assert_eq!(v.cursor.x, 12);
        press(&mut v, &t, "B");
        assert_eq!(v.cursor.x, 4);
    }
}
