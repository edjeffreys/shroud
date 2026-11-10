//! Prompt-relative navigation and copying. Both lean on OSC 133 marks libghostty-vt already keeps.

use std::ffi::c_void;
use std::ptr;

use crate::Terminal;
use crate::ffi::*;

impl Terminal {
    /// Negative scrolls towards older prompts.
    pub fn scroll_to_prompt(&mut self, delta: isize) {
        let sv = GhosttyTerminalScrollViewport {
            tag: GHOSTTY_TERMINAL_SCROLL_VIEWPORT_DELTA_PROMPT,
            value: GhosttyTerminalScrollViewportValue { delta_prompt: delta },
        };
        // SAFETY: raw is live and the value arm matches the tag.
        let _ = unsafe { ghostty_terminal_scroll_viewport(self.raw, sv) };
    }

    /// None when the last command printed nothing or the shell sends no OSC 133.
    pub fn last_output(&self) -> Option<String> {
        let mut prompt = self.cursor_screen_row()?;
        // Multi-line prompts mark their later rows as continuations.
        while prompt > 0 && self.row_semantic(prompt) == GHOSTTY_ROW_SEMANTIC_PROMPT_CONTINUATION {
            prompt -= 1;
        }
        let end = prompt.checked_sub(1)?;
        if self.row_semantic(prompt) != GHOSTTY_ROW_SEMANTIC_PROMPT
            || self.row_semantic(end) != GHOSTTY_ROW_SEMANTIC_NONE
        {
            return None;
        }
        // Bounded by prompt rows ourselves: a glyph printed before the shell's first mark counts
        // as output, so select_output's own start can reach into the previous prompt.
        let mut start = end;
        while start > 0 && self.row_semantic(start - 1) == GHOSTTY_ROW_SEMANTIC_NONE {
            start -= 1;
        }
        if start == 0 && self.row_semantic(0) == GHOSTTY_ROW_SEMANTIC_NONE {
            return None;
        }
        let mut probe: GhosttySelection = unsafe { std::mem::zeroed() };
        // SAFETY: the ref is fresh and nothing has changed the terminal since.
        if unsafe { ghostty_terminal_select_output(self.raw, &self.grid_ref(0, end)?, &mut probe) } != GHOSTTY_SUCCESS {
            return None;
        }
        let mut sel: GhosttySelection = unsafe { std::mem::zeroed() };
        sel.start = self.grid_ref(0, start)?;
        sel.end = self.grid_ref(self.cols()?.saturating_sub(1), end)?;
        self.format_plain(&sel).filter(|s| !s.is_empty())
    }

    /// The last finished command's line as typed, without the prompt. None when the shell
    /// doesn't mark where input starts (OSC 133 B).
    pub fn last_command(&self) -> Option<String> {
        let prompt = self.last_finished_prompt()?;
        self.command_text(&self.command_at(prompt))
    }

    pub(crate) fn cell_semantic(&self, x: u16, y: u32) -> GhosttyCellSemanticContent {
        let mut kind: GhosttyCellSemanticContent = GHOSTTY_CELL_SEMANTIC_OUTPUT;
        let Some(r) = self.grid_ref(x, y) else { return kind };
        let mut cell: GhosttyCell = 0;
        // SAFETY: `r` is fresh; SEMANTIC_CONTENT writes a GhosttyCellSemanticContent.
        unsafe {
            if ghostty_grid_ref_cell(&r, &mut cell) == GHOSTTY_SUCCESS {
                ghostty_cell_get(cell, GHOSTTY_CELL_DATA_SEMANTIC_CONTENT, &mut kind as *mut _ as *mut c_void);
            }
        }
        kind
    }

    pub(crate) fn cols(&self) -> Option<u16> {
        let mut cols = 0u16;
        self.get(GHOSTTY_TERMINAL_DATA_COLS, &mut cols as *mut _ as *mut c_void).ok()?;
        Some(cols)
    }

    pub(crate) fn cursor_screen_row(&self) -> Option<u32> {
        let (mut rows, mut cursor_y, mut total) = (0u16, 0u16, 0usize);
        self.get(GHOSTTY_TERMINAL_DATA_ROWS, &mut rows as *mut _ as *mut c_void).ok()?;
        self.get(GHOSTTY_TERMINAL_DATA_CURSOR_Y, &mut cursor_y as *mut _ as *mut c_void).ok()?;
        self.get(GHOSTTY_TERMINAL_DATA_TOTAL_ROWS, &mut total as *mut _ as *mut c_void).ok()?;
        u32::try_from(total.saturating_sub(rows as usize) + cursor_y as usize).ok()
    }

    pub(crate) fn grid_ref(&self, x: u16, y: u32) -> Option<GhosttyGridRef> {
        let point = GhosttyPoint {
            tag: GHOSTTY_POINT_TAG_SCREEN,
            value: GhosttyPointValue { coordinate: GhosttyPointCoordinate { x, y } },
        };
        let mut r: GhosttyGridRef = unsafe { std::mem::zeroed() };
        // SAFETY: out-pointer is valid; a zeroed ref is the documented starting value.
        (unsafe { ghostty_terminal_grid_ref(self.raw, point, &mut r) } == GHOSTTY_SUCCESS).then_some(r)
    }

    pub(crate) fn row_semantic(&self, y: u32) -> GhosttyRowSemanticPrompt {
        let mut kind: GhosttyRowSemanticPrompt = GHOSTTY_ROW_SEMANTIC_NONE;
        let Some(r) = self.grid_ref(0, y) else { return kind };
        let mut row: GhosttyRow = 0;
        // SAFETY: `r` is fresh; SEMANTIC_PROMPT writes a GhosttyRowSemanticPrompt.
        unsafe {
            if ghostty_grid_ref_row(&r, &mut row) == GHOSTTY_SUCCESS {
                ghostty_row_get(row, GHOSTTY_ROW_DATA_SEMANTIC_PROMPT, &mut kind as *mut _ as *mut c_void);
            }
        }
        kind
    }

    // Unwrapped and trimmed plain text is exactly what Ghostty puts on the clipboard.
    pub(crate) fn format_plain(&self, sel: &GhosttySelection) -> Option<String> {
        self.format(sel, true)
    }

    pub(crate) fn format(&self, sel: &GhosttySelection, unwrap: bool) -> Option<String> {
        let mut f: GhosttyFormatter = ptr::null_mut();
        // SAFETY: the formatter borrows raw and is freed before returning.
        unsafe {
            if ghostty_formatter_terminal_new(ptr::null(), &mut f, self.raw) != GHOSTTY_SUCCESS {
                return None;
            }
            let on = true;
            let ok = ghostty_formatter_set(f, GHOSTTY_FORMATTER_OPT_UNWRAP, &unwrap as *const _ as *const c_void)
                == GHOSTTY_SUCCESS
                && ghostty_formatter_set(f, GHOSTTY_FORMATTER_OPT_TRIM, &on as *const _ as *const c_void)
                    == GHOSTTY_SUCCESS
                && ghostty_formatter_set(f, GHOSTTY_FORMATTER_OPT_SELECTION, sel as *const _ as *const c_void)
                    == GHOSTTY_SUCCESS;
            let (mut buf, mut len) = (ptr::null_mut::<u8>(), 0usize);
            let text = (ok && ghostty_formatter_format_alloc(f, ptr::null(), &mut buf, &mut len) == GHOSTTY_SUCCESS)
                .then(|| {
                    let s = if buf.is_null() {
                        String::new()
                    } else {
                        String::from_utf8_lossy(std::slice::from_raw_parts(buf, len)).into_owned()
                    };
                    ghostty_free(ptr::null(), buf, len);
                    s
                });
            ghostty_formatter_free(f);
            text
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::c_void;

    use crate::Terminal;
    use crate::ffi::*;

    fn viewport_offset(t: &Terminal) -> u64 {
        let mut sb = GhosttyTerminalScrollbar { total: 0, offset: 0, len: 0 };
        let _ = t.get(GHOSTTY_TERMINAL_DATA_SCROLLBAR, &mut sb as *mut _ as *mut c_void);
        sb.offset
    }

    fn command(t: &mut Terminal, cmd: &str, output: &str, exit: u8) {
        t.write(
            format!("\x1b]133;A\x07$ \x1b]133;B\x07{cmd}\r\n\x1b]133;C\x07{output}\x1b]133;D;{exit}\x07").as_bytes(),
        );
    }

    fn prompt(t: &mut Terminal) {
        t.write(b"\x1b]133;A\x07$ \x1b]133;B\x07");
    }

    #[test]
    fn copies_the_most_recent_output_only() {
        let mut t = Terminal::new(40, 10).unwrap();
        command(&mut t, "echo one", "one\r\n", 0);
        command(&mut t, "printf", "two\r\nthree\r\n", 0);
        prompt(&mut t);
        assert_eq!(t.last_output().as_deref(), Some("two\nthree"));
    }

    #[test]
    fn a_command_without_output_copies_nothing() {
        let mut t = Terminal::new(40, 10).unwrap();
        command(&mut t, "echo one", "one\r\n", 0);
        command(&mut t, "true", "", 0);
        prompt(&mut t);
        assert_eq!(t.last_output(), None);
    }

    #[test]
    fn long_lines_are_unwrapped() {
        let mut t = Terminal::new(10, 10).unwrap();
        command(&mut t, "x", "abcdefghijklmno\r\n", 0);
        prompt(&mut t);
        assert_eq!(t.last_output().as_deref(), Some("abcdefghijklmno"));
    }

    #[test]
    fn multi_line_prompts_are_skipped() {
        let mut t = Terminal::new(40, 10).unwrap();
        command(&mut t, "ls", "file\r\n", 0);
        t.write(b"\x1b]133;A\x07~/src\r\n> \x1b]133;B\x07");
        assert_eq!(t.last_output().as_deref(), Some("file"));
    }

    #[test]
    fn first_command_after_shell_start_excludes_the_unmarked_prompt() {
        let mut t = Terminal::new(40, 10).unwrap();
        t.write(
            "~/src\r\n\u{e602} \x1b]133;P;k=i\x07\x1b]133;B\x07echo one\r\n\x1b]133;C\x07one\r\n\x1b]133;D;0\x07"
                .as_bytes(),
        );
        prompt(&mut t);
        assert_eq!(t.last_output().as_deref(), Some("one"));
    }

    #[test]
    fn prompt_jumps_move_the_viewport_only_when_there_is_scrollback() {
        let mut t = Terminal::new(40, 5).unwrap();
        command(&mut t, "a", "1\r\n", 0);
        prompt(&mut t);
        let before = viewport_offset(&t);
        t.scroll_to_prompt(-1);
        assert_eq!(viewport_offset(&t), before, "everything fits, so there is nowhere to scroll");
        command(&mut t, "seq", &"x\r\n".repeat(20), 0);
        prompt(&mut t);
        let bottom = viewport_offset(&t);
        t.scroll_to_prompt(-1);
        assert!(viewport_offset(&t) < bottom);
    }

    #[test]
    fn the_command_line_comes_without_its_prompt() {
        let mut t = Terminal::new(40, 10).unwrap();
        command(&mut t, "echo one", "one\r\n", 0);
        command(&mut t, "printf 'a b'", "a b\r\n", 0);
        prompt(&mut t);
        assert_eq!(t.last_command().as_deref(), Some("printf 'a b'"));
        command(&mut t, "true", "", 0);
        prompt(&mut t);
        assert_eq!(t.last_command().as_deref(), Some("true"), "a command without output still has its line");
    }

    #[test]
    fn long_and_multi_line_prompts_still_give_the_command() {
        let mut t = Terminal::new(10, 10).unwrap();
        t.write(b"\x1b]133;A\x07~/src\r\n> \x1b]133;B\x07echo abcdefghij\r\n\x1b]133;C\x07out\r\n\x1b]133;D;0\x07");
        prompt(&mut t);
        assert_eq!(t.last_command().as_deref(), Some("echo abcdefghij"));
    }

    #[test]
    fn no_shell_integration_copies_nothing() {
        let mut t = Terminal::new(40, 10).unwrap();
        t.write(b"$ ls\r\nfile\r\n$ ");
        assert_eq!(t.last_output(), None);
        assert_eq!(t.last_command(), None);
    }
}
