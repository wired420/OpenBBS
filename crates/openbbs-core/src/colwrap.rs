//! Wraps door output at a fixed width, the way an 80-column terminal would.
//!
//! Classic doors draw for an 80x25 screen and count on the terminal moving
//! to the next line after column 80. On a wider window that never happens,
//! so their art spills to the right. [`ColumnWrap`] follows the cursor
//! column through the output (text, CR, backspace, tabs and the ANSI cursor
//! commands doors use) and inserts a line break wherever an 80-column
//! terminal would have wrapped.

/// Follows the cursor column of a stream and wraps it at `width`.
#[derive(Debug, Clone)]
pub struct ColumnWrap {
    width: usize,
    col: usize,
    saved_col: usize,
    state: State,
    params: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Text,
    Escape,
    Csi,
}

impl ColumnWrap {
    pub fn new(width: usize) -> Self {
        Self {
            width: width.max(1),
            col: 0,
            saved_col: 0,
            state: State::Text,
            params: String::new(),
        }
    }

    /// Returns `text` with line breaks added where the terminal would wrap.
    /// Escape sequences may be split across calls.
    pub fn process(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len() + 8);
        for c in text.chars() {
            match self.state {
                State::Text => self.text_char(c, &mut out),
                State::Escape => {
                    out.push(c);
                    self.state = if c == '[' {
                        self.params.clear();
                        State::Csi
                    } else {
                        State::Text
                    };
                }
                State::Csi => {
                    out.push(c);
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        self.csi(c);
                        self.state = State::Text;
                    } else {
                        self.params.push(c);
                    }
                }
            }
        }
        out
    }

    fn text_char(&mut self, c: char, out: &mut String) {
        match c {
            '\x1b' => self.state = State::Escape,
            '\r' => self.col = 0,
            '\x08' => self.col = self.col.saturating_sub(1),
            '\t' => self.col = ((self.col / 8 + 1) * 8).min(self.width - 1),
            c if (c as u32) < 0x20 || c == '\x7f' => {}
            _ => {
                // The terminal wraps when a character arrives with the
                // cursor already past the last column, not when the last
                // column is filled, so a full line followed by CR LF gives
                // one line break, not two.
                if self.col >= self.width {
                    out.push_str("\r\n");
                    self.col = 0;
                }
                self.col += 1;
            }
        }
        out.push(c);
    }

    fn csi(&mut self, command: char) {
        let mut nums = self
            .params
            .trim_start_matches(['?', '=', '>'])
            .split(';')
            .map(|p| p.parse::<usize>().ok());
        let first = nums.next().flatten();
        let second = nums.next().flatten();
        let n = first.unwrap_or(1).max(1);
        let last = self.width - 1;
        match command {
            'C' => self.col = (self.col + n).min(last),
            'D' => self.col = self.col.min(last).saturating_sub(n),
            'G' => self.col = (n - 1).min(last),
            'H' | 'f' => self.col = (second.unwrap_or(1).max(1) - 1).min(last),
            'E' | 'F' => self.col = 0,
            'J' if first == Some(2) => self.col = 0,
            's' => self.saved_col = self.col,
            'u' => self.col = self.saved_col,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_lines_pass_through() {
        let mut w = ColumnWrap::new(80);
        assert_eq!(w.process("hello\r\nworld"), "hello\r\nworld");
    }

    #[test]
    fn wraps_long_lines_at_the_width() {
        let mut w = ColumnWrap::new(4);
        assert_eq!(w.process("abcdefghij"), "abcd\r\nefgh\r\nij");
    }

    #[test]
    fn a_full_line_then_crlf_breaks_once() {
        let mut w = ColumnWrap::new(4);
        assert_eq!(w.process("abcd\r\nef"), "abcd\r\nef");
    }

    #[test]
    fn colors_take_no_room() {
        let mut w = ColumnWrap::new(4);
        assert_eq!(
            w.process("ab\x1b[1;33mcd\x1b[0me"),
            "ab\x1b[1;33mcd\x1b[0m\r\ne"
        );
    }

    #[test]
    fn escape_split_across_calls() {
        let mut w = ColumnWrap::new(4);
        let mut out = w.process("ab\x1b[1;");
        out += &w.process("33mcde");
        assert_eq!(out, "ab\x1b[1;33mcd\r\ne");
    }

    #[test]
    fn follows_cursor_commands() {
        let mut w = ColumnWrap::new(10);
        // Jump to column 9, then two characters: the second one wraps.
        assert_eq!(w.process("\x1b[5;9Hxyz"), "\x1b[5;9Hxy\r\nz");
        // Cursor forward is clamped to the last column.
        let mut w = ColumnWrap::new(10);
        assert_eq!(w.process("\x1b[50Cab"), "\x1b[50Ca\r\nb");
        // Save and restore.
        let mut w = ColumnWrap::new(10);
        assert_eq!(
            w.process("abc\x1b[s\r\x1b[u1234567x"),
            "abc\x1b[s\r\x1b[u1234567\r\nx"
        );
    }
}
