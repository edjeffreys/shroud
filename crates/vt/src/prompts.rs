//! A command has output only once a later prompt bounds it, so the prompt being typed at never
//! claims the blank rows below it.

use crate::Terminal;
use crate::ffi::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CommandRows {
    pub prompt: u32,
    /// The cells the shell marked as typed input (OSC 133 B), both ends included.
    pub input: Option<((u16, u32), (u16, u32))>,
    pub output: Option<(u32, u32)>,
}

impl Terminal {
    pub fn prompt_rows(&self) -> Vec<u32> {
        (0..self.viewport().total).filter(|&y| self.row_semantic(y) == GHOSTTY_ROW_SEMANTIC_PROMPT).collect()
    }

    pub fn command_at(&self, prompt: u32) -> CommandRows {
        let total = self.viewport().total;
        let mut last = prompt;
        while last + 1 < total && self.row_semantic(last + 1) == GHOSTTY_ROW_SEMANTIC_PROMPT_CONTINUATION {
            last += 1;
        }
        let cols = self.cols().unwrap_or(0);
        let mut cells = (prompt..=last)
            .flat_map(|r| (0..cols).map(move |x| (x, r)))
            .filter(|&(x, r)| self.cell_semantic(x, r) == GHOSTTY_CELL_SEMANTIC_INPUT);
        let input = cells.next().map(|first| (first, cells.next_back().unwrap_or(first)));
        let next = (last + 1..total).find(|&y| self.row_semantic(y) == GHOSTTY_ROW_SEMANTIC_PROMPT);
        let output = next.filter(|&n| n > last + 1).map(|n| (last + 1, n - 1));
        CommandRows { prompt, input, output }
    }

    pub fn command_text(&self, c: &CommandRows) -> Option<String> {
        let (from, to) = c.input?;
        let text = self.text_between(from, to, false)?;
        Some(text.trim().to_string()).filter(|t| !t.is_empty())
    }

    pub fn output_text(&self, c: &CommandRows) -> Option<String> {
        let (from, to) = c.output?;
        self.text_between((0, from), (self.cols()?.saturating_sub(1), to), false).filter(|t| !t.is_empty())
    }

    /// Until the next prompt is drawn, the command still running counts as the last.
    pub fn last_finished_prompt(&self) -> Option<u32> {
        let cursor = self.cursor_screen_row()?;
        let above = |from: u32| (0..=from).rev().find(|&y| self.row_semantic(y) == GHOSTTY_ROW_SEMANTIC_PROMPT);
        let here = above(cursor)?;
        let at_prompt = (here..=cursor).all(|y| self.row_semantic(y) != GHOSTTY_ROW_SEMANTIC_NONE);
        if at_prompt { above(here.checked_sub(1)?) } else { Some(here) }
    }
}

#[cfg(test)]
mod tests {
    use crate::Terminal;

    fn command(t: &mut Terminal, cmd: &str, output: &str) {
        t.write(format!("\x1b]133;A\x07$ \x1b]133;B\x07{cmd}\r\n\x1b]133;C\x07{output}\x1b]133;D;0\x07").as_bytes());
    }

    #[test]
    fn each_prompt_has_its_command_and_output() {
        let mut t = Terminal::new(40, 10).unwrap();
        command(&mut t, "echo one", "one\r\n");
        command(&mut t, "true", "");
        command(&mut t, "printf", "a\r\nb\r\n");
        t.write(b"\x1b]133;A\x07$ \x1b]133;B\x07");
        let rows = t.prompt_rows();
        assert_eq!(rows.len(), 4);
        let cmds: Vec<_> = rows.iter().map(|&p| t.command_at(p)).collect();
        assert_eq!(
            cmds.iter().map(|c| t.command_text(c)).collect::<Vec<_>>(),
            [Some("echo one".into()), Some("true".into()), Some("printf".into()), None]
        );
        assert_eq!(
            cmds.iter().map(|c| t.output_text(c)).collect::<Vec<_>>(),
            [Some("one".into()), None, Some("a\nb".into()), None]
        );
        assert_eq!(t.last_finished_prompt(), Some(rows[2]));
    }

    #[test]
    fn before_the_next_prompt_the_running_command_is_the_last() {
        let mut t = Terminal::new(40, 10).unwrap();
        command(&mut t, "echo one", "one\r\n");
        t.write(b"\x1b]133;A\x07$ \x1b]133;B\x07sleep 1\r\n\x1b]133;C\x07");
        assert_eq!(t.last_finished_prompt(), Some(t.prompt_rows()[1]));
    }

    #[test]
    fn a_prompt_without_marked_input_has_no_command() {
        let mut t = Terminal::new(40, 10).unwrap();
        t.write(b"\x1b]133;A\x07$ ls\r\n\x1b]133;C\x07file\r\n\x1b]133;A\x07$ ");
        let c = t.command_at(t.prompt_rows()[0]);
        assert_eq!((t.command_text(&c), t.output_text(&c).as_deref()), (None, Some("file")));
    }
}
