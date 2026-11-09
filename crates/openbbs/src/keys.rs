//! Turns raw bytes from the caller's terminal into key presses.

/// A key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Tab,
    Esc,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
}

/// Incremental decoder for terminal input: UTF-8 text, control keys and
/// VT100/xterm escape sequences. Feed it bytes as they arrive.
#[derive(Default)]
pub struct KeyDecoder {
    buf: Vec<u8>,
    last_was_cr: bool,
}

enum Parse {
    Key(Key, usize),
    Skip(usize),
    Incomplete,
}

impl KeyDecoder {
    /// Add bytes and return every complete key press they finish.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Key> {
        self.buf.extend_from_slice(bytes);
        let mut keys = Vec::new();
        loop {
            match self.parse() {
                Parse::Key(key, used) => {
                    self.buf.drain(..used);
                    keys.push(key);
                }
                Parse::Skip(used) => {
                    self.buf.drain(..used);
                }
                Parse::Incomplete => break,
            }
        }
        keys
    }

    /// True if a partial sequence (like a lone ESC) is waiting for more bytes.
    pub fn has_pending(&self) -> bool {
        !self.buf.is_empty()
    }

    /// Called when no more bytes arrived in time: a waiting lone ESC is the Esc key.
    pub fn flush(&mut self) -> Vec<Key> {
        if self.buf.is_empty() {
            return Vec::new();
        }
        let was_esc = self.buf[0] == 0x1B;
        self.buf.clear();
        if was_esc { vec![Key::Esc] } else { Vec::new() }
    }

    fn parse(&mut self) -> Parse {
        let Some(&first) = self.buf.first() else {
            return Parse::Incomplete;
        };
        let cr = self.last_was_cr;
        self.last_was_cr = first == b'\r';
        match first {
            b'\r' => Parse::Key(Key::Enter, 1),
            // CR LF and CR NUL are one Enter.
            b'\n' | 0 if cr => Parse::Skip(1),
            b'\n' => Parse::Key(Key::Enter, 1),
            0x7F | 0x08 => Parse::Key(Key::Backspace, 1),
            b'\t' => Parse::Key(Key::Tab, 1),
            0x1B => self.parse_escape(),
            0x00..=0x1F => Parse::Skip(1),
            _ => self.parse_utf8(),
        }
    }

    fn parse_escape(&mut self) -> Parse {
        let Some(&kind) = self.buf.get(1) else {
            return Parse::Incomplete;
        };
        match kind {
            b'[' | b'O' => {
                // Find the final byte of the sequence (0x40..=0x7E after params).
                let Some(end) = self.buf[2..].iter().position(|b| (0x40..=0x7E).contains(b)) else {
                    return if self.buf.len() > 16 {
                        Parse::Skip(self.buf.len())
                    } else {
                        Parse::Incomplete
                    };
                };
                let end = end + 2;
                let params = &self.buf[2..end];
                let key = match (self.buf[end], params) {
                    (b'A', _) => Some(Key::Up),
                    (b'B', _) => Some(Key::Down),
                    (b'C', _) => Some(Key::Right),
                    (b'D', _) => Some(Key::Left),
                    (b'H', _) => Some(Key::Home),
                    (b'F', _) => Some(Key::End),
                    (b'~', b"1") | (b'~', b"7") => Some(Key::Home),
                    (b'~', b"4") | (b'~', b"8") => Some(Key::End),
                    (b'~', b"3") => Some(Key::Delete),
                    (b'~', b"5") => Some(Key::PageUp),
                    (b'~', b"6") => Some(Key::PageDown),
                    _ => None,
                };
                match key {
                    Some(k) => Parse::Key(k, end + 1),
                    None => Parse::Skip(end + 1),
                }
            }
            // ESC followed by something else: Esc, then handle the next byte normally.
            _ => Parse::Key(Key::Esc, 1),
        }
    }

    fn parse_utf8(&mut self) -> Parse {
        let first = self.buf[0];
        let len = match first {
            0x00..=0x7F => 1,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => return Parse::Skip(1),
        };
        if self.buf.len() < len {
            return Parse::Incomplete;
        }
        match std::str::from_utf8(&self.buf[..len]) {
            Ok(s) => Parse::Key(Key::Char(s.chars().next().unwrap_or('?')), len),
            Err(_) => Parse::Skip(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(bytes: &[u8]) -> Vec<Key> {
        KeyDecoder::default().feed(bytes)
    }

    #[test]
    fn text_and_enter() {
        assert_eq!(
            decode(b"hi\r\n"),
            vec![Key::Char('h'), Key::Char('i'), Key::Enter]
        );
        assert_eq!(
            decode(b"a\r\0b\n"),
            vec![Key::Char('a'), Key::Enter, Key::Char('b'), Key::Enter]
        );
        assert_eq!(decode(b"\r\r"), vec![Key::Enter, Key::Enter]);
    }

    #[test]
    fn arrows_and_specials() {
        assert_eq!(
            decode(b"\x1b[A\x1b[B\x1bOC\x1b[D\x1b[3~\x7f"),
            vec![
                Key::Up,
                Key::Down,
                Key::Right,
                Key::Left,
                Key::Delete,
                Key::Backspace
            ]
        );
    }

    #[test]
    fn split_sequences_and_utf8() {
        let mut d = KeyDecoder::default();
        assert!(d.feed(b"\x1b").is_empty());
        assert!(d.has_pending());
        assert!(d.feed(b"[").is_empty());
        assert_eq!(d.feed(b"A"), vec![Key::Up]);
        let euro = "€".as_bytes();
        assert!(d.feed(&euro[..1]).is_empty());
        assert_eq!(d.feed(&euro[1..]), vec![Key::Char('€')]);
    }

    #[test]
    fn lone_escape_after_timeout() {
        let mut d = KeyDecoder::default();
        assert!(d.feed(b"\x1b").is_empty());
        assert_eq!(d.flush(), vec![Key::Esc]);
        assert_eq!(decode(b"\x1bq"), vec![Key::Esc, Key::Char('q')]);
    }
}
