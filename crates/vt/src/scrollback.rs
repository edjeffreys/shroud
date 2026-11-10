//! Visual mode's view of the whole history. Rows count from the oldest line kept, the same space
//! the scrollbar and absolute viewport scrolling use.

use std::ffi::c_void;

use crate::Terminal;
use crate::ffi::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Viewport {
    pub total: u32,
    pub top: u32,
    pub rows: u16,
    pub cols: u16,
}

impl Terminal {
    pub fn viewport(&self) -> Viewport {
        let mut sb = GhosttyTerminalScrollbar { total: 0, offset: 0, len: 0 };
        let _ = self.get(GHOSTTY_TERMINAL_DATA_SCROLLBAR, &mut sb as *mut _ as *mut c_void);
        Viewport {
            total: sb.total.min(u32::MAX as u64) as u32,
            top: sb.offset.min(u32::MAX as u64) as u32,
            rows: sb.len.min(u16::MAX as u64) as u16,
            cols: self.cols().unwrap_or(0),
        }
    }

    pub fn scroll_to_row(&mut self, top: u32) {
        let sv = GhosttyTerminalScrollViewport {
            tag: GHOSTTY_TERMINAL_SCROLL_VIEWPORT_ROW,
            value: GhosttyTerminalScrollViewportValue { row: top as usize },
        };
        // SAFETY: raw is live and the value arm matches the tag.
        let _ = unsafe { ghostty_terminal_scroll_viewport(self.raw, sv) };
    }

    pub fn cursor_in_history(&self) -> Option<(u16, u32)> {
        let mut x = 0u16;
        self.get(GHOSTTY_TERMINAL_DATA_CURSOR_X, &mut x as *mut _ as *mut c_void).ok()?;
        Some((x, self.cursor_screen_row()?))
    }

    /// One char per column, so a column index is a position. A wide glyph's second column is
    /// `'\0'` and a blank cell is `' '`.
    pub fn history_line(&self, y: u32) -> Vec<char> {
        let cols = self.cols().unwrap_or(0);
        (0..cols)
            .map(|x| {
                let Some(r) = self.grid_ref(x, y) else { return ' ' };
                let (mut cell, mut cp, mut wide): (GhosttyCell, u32, GhosttyCellWide) =
                    (0, 0, GHOSTTY_CELL_WIDE_NARROW);
                // SAFETY: `r` is fresh; CODEPOINT writes a u32 and WIDE a GhosttyCellWide.
                unsafe {
                    if ghostty_grid_ref_cell(&r, &mut cell) != GHOSTTY_SUCCESS {
                        return ' ';
                    }
                    ghostty_cell_get(cell, GHOSTTY_CELL_DATA_CODEPOINT, &mut cp as *mut _ as *mut c_void);
                    ghostty_cell_get(cell, GHOSTTY_CELL_DATA_WIDE, &mut wide as *mut _ as *mut c_void);
                }
                match wide {
                    GHOSTTY_CELL_WIDE_SPACER_TAIL | GHOSTTY_CELL_WIDE_SPACER_HEAD => '\0',
                    _ => char::from_u32(cp).filter(|&c| c != '\0').unwrap_or(' '),
                }
            })
            .collect()
    }

    pub fn is_prompt_row(&self, y: u32) -> bool {
        self.row_semantic(y) == GHOSTTY_ROW_SEMANTIC_PROMPT
    }

    /// What a double click copies, by Ghostty's own word boundaries.
    pub fn word_at(&self, x: u16, y: u32) -> Option<String> {
        let opts = GhosttyTerminalSelectWordOptions {
            size: std::mem::size_of::<GhosttyTerminalSelectWordOptions>(),
            ref_: self.grid_ref(x, y)?,
            boundary_codepoints: std::ptr::null(),
            boundary_codepoints_len: 0,
        };
        let mut sel: GhosttySelection = unsafe { std::mem::zeroed() };
        // SAFETY: the ref is fresh and a null boundary list means Ghostty's defaults.
        (unsafe { ghostty_terminal_select_word(self.raw, &opts, &mut sel) } == GHOSTTY_SUCCESS).then_some(())?;
        self.format_plain(&sel).filter(|s| !s.is_empty())
    }

    /// What a triple click copies: the whole line, wrapped rows and all.
    pub fn line_at(&self, x: u16, y: u32) -> Option<String> {
        let opts = GhosttyTerminalSelectLineOptions {
            size: std::mem::size_of::<GhosttyTerminalSelectLineOptions>(),
            ref_: self.grid_ref(x, y)?,
            whitespace: std::ptr::null(),
            whitespace_len: 0,
            semantic_prompt_boundary: false,
        };
        let mut sel: GhosttySelection = unsafe { std::mem::zeroed() };
        // SAFETY: as in `word_at`.
        (unsafe { ghostty_terminal_select_line(self.raw, &opts, &mut sel) } == GHOSTTY_SUCCESS).then_some(())?;
        self.format_plain(&sel).filter(|s| !s.is_empty())
    }

    /// One string per history row, wrapped rows kept apart so an index is a row. Through the
    /// formatter, which is about 70 times faster than reading cells one by one.
    pub fn history_rows(&self) -> Vec<String> {
        let total = self.viewport().total;
        let (Some(cols), Some(last)) = (self.cols(), total.checked_sub(1)) else { return Vec::new() };
        let mut sel: GhosttySelection = unsafe { std::mem::zeroed() };
        let (Some(start), Some(end)) = (self.grid_ref(0, 0), self.grid_ref(cols.saturating_sub(1), last)) else {
            return Vec::new();
        };
        sel.start = start;
        sel.end = end;
        let mut rows: Vec<String> =
            self.format(&sel, false).unwrap_or_default().split('\n').map(str::to_string).collect();
        rows.resize(total as usize, String::new());
        rows
    }

    /// Unwrapped and trimmed, as Ghostty would copy it. Both ends are included.
    pub fn text_between(&self, start: (u16, u32), end: (u16, u32), rectangle: bool) -> Option<String> {
        let mut sel: GhosttySelection = unsafe { std::mem::zeroed() };
        sel.start = self.grid_ref(start.0, start.1)?;
        sel.end = self.grid_ref(end.0, end.1)?;
        sel.rectangle = rectangle;
        self.format_plain(&sel)
    }
}

#[cfg(test)]
mod tests {
    use crate::Terminal;

    fn filled(lines: usize) -> Terminal {
        let mut t = Terminal::new(10, 4).unwrap();
        for i in 0..lines {
            t.write(format!("line {i}\r\n").as_bytes());
        }
        t
    }

    #[test]
    fn rows_count_from_the_oldest_line_and_scrolling_is_absolute() {
        let mut t = filled(10);
        let v = t.viewport();
        assert_eq!((v.total, v.rows, v.cols), (11, 4, 10));
        assert_eq!(v.top, 7, "pinned to the bottom");
        assert_eq!(t.history_line(0).iter().collect::<String>().trim_end(), "line 0");
        assert_eq!(t.cursor_in_history(), Some((0, 10)));
        t.scroll_to_row(2);
        assert_eq!(t.viewport().top, 2);
    }

    #[test]
    fn wide_glyphs_keep_columns_and_text_spans_rows() {
        let mut t = Terminal::new(10, 4).unwrap();
        t.write("a中b\r\nnext".as_bytes());
        assert_eq!(&t.history_line(0)[..4], &['a', '中', '\0', 'b']);
        assert_eq!(t.text_between((0, 0), (3, 1), false).as_deref(), Some("a中b\nnext"));
        assert_eq!(t.text_between((0, 0), (0, 1), true).as_deref(), Some("a\nn"));
    }

    #[test]
    fn clicks_copy_a_word_or_a_whole_wrapped_line() {
        let mut t = Terminal::new(10, 4).unwrap();
        t.write(b"see src/main.rs ok");
        assert_eq!(t.word_at(5, 0).as_deref(), Some("src/main.rs"));
        assert_eq!(t.line_at(2, 1).as_deref(), Some("see src/main.rs ok"));
        assert_eq!(t.word_at(3, 0), None, "a blank has no word");
    }

    #[test]
    fn history_rows_keep_wrapped_rows_apart() {
        let mut t = Terminal::new(10, 4).unwrap();
        t.write("abcdefghijklm\r\n中b\r\n".as_bytes());
        let rows = t.history_rows();
        assert_eq!(rows.len(), t.viewport().total as usize);
        assert_eq!(&rows[..3], ["abcdefghij", "klm", "中b"]);
    }

    #[test]
    fn prompt_rows_come_from_osc_133() {
        let mut t = Terminal::new(10, 4).unwrap();
        t.write(b"out\r\n\x1b]133;A\x07$ ");
        assert!(!t.is_prompt_row(0));
        assert!(t.is_prompt_row(1));
    }
}
