//! A caller's terminal: buffered output, key input, prompts and line editing.

use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

use openbbs_core::text::pipe_to_ansi;
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;

use crate::keys::{Key, KeyDecoder};

/// Why a session ended early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hangup {
    /// The caller disconnected.
    Disconnected,
    /// The caller sat idle too long.
    Idle,
}

impl fmt::Display for Hangup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Hangup::Disconnected => f.write_str("caller disconnected"),
            Hangup::Idle => f.write_str("idle timeout"),
        }
    }
}

impl std::error::Error for Hangup {}

pub type TermResult<T> = Result<T, Hangup>;

/// How long to wait for the rest of an escape sequence before treating ESC as a key.
const ESC_TIMEOUT: Duration = Duration::from_millis(60);

pub struct Term {
    output: mpsc::UnboundedSender<Vec<u8>>,
    input: mpsc::Receiver<Vec<u8>>,
    size: watch::Receiver<(u16, u16)>,
    decoder: KeyDecoder,
    pending: VecDeque<Key>,
    idle: Duration,
}

/// Options for [`Term::read_line`].
#[derive(Debug, Clone, Copy)]
pub struct LineOpts {
    pub max_len: usize,
    pub mask: bool,
}

impl LineOpts {
    pub fn text(max_len: usize) -> Self {
        Self {
            max_len,
            mask: false,
        }
    }

    pub fn password() -> Self {
        Self {
            max_len: 64,
            mask: true,
        }
    }
}

impl Term {
    pub fn new(
        output: mpsc::UnboundedSender<Vec<u8>>,
        input: mpsc::Receiver<Vec<u8>>,
        size: watch::Receiver<(u16, u16)>,
        idle: Duration,
    ) -> Self {
        Self {
            output,
            input,
            size,
            decoder: KeyDecoder::default(),
            pending: VecDeque::new(),
            idle,
        }
    }

    /// Terminal size as (columns, rows).
    pub fn size(&self) -> (u16, u16) {
        *self.size.borrow()
    }

    /// Send raw text. Bare `\n` becomes `\r\n`.
    pub fn write(&self, text: &str) -> TermResult<()> {
        let mut out = String::with_capacity(text.len() + 8);
        let mut prev = '\0';
        for c in text.chars() {
            if c == '\n' && prev != '\r' {
                out.push('\r');
            }
            out.push(c);
            prev = c;
        }
        self.output
            .send(out.into_bytes())
            .map_err(|_| Hangup::Disconnected)
    }

    /// Send text containing pipe color codes.
    pub fn print(&self, text: &str) -> TermResult<()> {
        self.write(&pipe_to_ansi(text))
    }

    /// Send text with pipe codes, then a new line.
    pub fn println(&self, text: &str) -> TermResult<()> {
        self.print(text)?;
        self.write("\x1b[0m\r\n")
    }

    pub fn clear(&self) -> TermResult<()> {
        self.write("\x1b[0m\x1b[2J\x1b[H")
    }

    /// Move the cursor (1-based row and column).
    pub fn goto(&self, row: u16, col: u16) -> TermResult<()> {
        self.write(&format!("\x1b[{row};{col}H"))
    }

    pub fn hide_cursor(&self, hide: bool) -> TermResult<()> {
        self.write(if hide { "\x1b[?25l" } else { "\x1b[?25h" })
    }

    /// Wait for the next key press.
    pub async fn read_key(&mut self) -> TermResult<Key> {
        loop {
            if let Some(key) = self.pending.pop_front() {
                return Ok(key);
            }
            let wait = if self.decoder.has_pending() {
                ESC_TIMEOUT
            } else {
                self.idle
            };
            match timeout(wait, self.input.recv()).await {
                Ok(Some(bytes)) => self.pending.extend(self.decoder.feed(&bytes)),
                Ok(None) => return Err(Hangup::Disconnected),
                Err(_) if self.decoder.has_pending() => {
                    self.pending.extend(self.decoder.flush());
                }
                Err(_) => return Err(Hangup::Idle),
            }
        }
    }

    /// Send bytes exactly as given (no newline conversion).
    pub fn write_raw(&self, bytes: Vec<u8>) -> TermResult<()> {
        self.output.send(bytes).map_err(|_| Hangup::Disconnected)
    }

    /// Wait for the next chunk of raw input bytes, for passing straight
    /// through to a door program. Keys already decoded are discarded.
    pub async fn read_raw(&mut self) -> TermResult<Vec<u8>> {
        self.pending.clear();
        self.decoder = KeyDecoder::default();
        match timeout(self.idle, self.input.recv()).await {
            Ok(Some(bytes)) => Ok(bytes),
            Ok(None) => Err(Hangup::Disconnected),
            Err(_) => Err(Hangup::Idle),
        }
    }

    /// Show a prompt and wait for any key.
    pub async fn pause(&mut self) -> TermResult<()> {
        let prompt = "|08[|07Press any key|08]";
        self.print(prompt)?;
        self.read_key().await?;
        // Erase the prompt.
        self.write("\r\x1b[0m\x1b[K")
    }

    /// Ask a yes/no question. Enter picks `default`.
    pub async fn yes_no(&mut self, question: &str, default: bool) -> TermResult<bool> {
        let hint = if default { "|15Y|07/n" } else { "|07y/|15N" };
        self.print(&format!("{question} |08[{hint}|08]|07 "))?;
        let answer = loop {
            match self.read_key().await? {
                Key::Enter => break default,
                Key::Char('y' | 'Y') => break true,
                Key::Char('n' | 'N') => break false,
                _ => {}
            }
        };
        self.println(if answer { "|15Yes" } else { "|15No" })?;
        Ok(answer)
    }

    /// Read a line of text with echo and backspace. Returns it trimmed.
    pub async fn read_line(&mut self, opts: LineOpts) -> TermResult<String> {
        let mut line = String::new();
        self.write("\x1b[1;37m")?;
        loop {
            match self.read_key().await? {
                Key::Enter => break,
                Key::Backspace | Key::Delete | Key::Left => {
                    if line.pop().is_some() {
                        self.write("\x08 \x08")?;
                    }
                }
                Key::Char(c) if !c.is_control() && line.chars().count() < opts.max_len => {
                    line.push(c);
                    if opts.mask {
                        self.write("*")?;
                    } else {
                        self.write(c.encode_utf8(&mut [0; 4]))?;
                    }
                }
                _ => {}
            }
        }
        self.write("\x1b[0m\r\n")?;
        Ok(line.trim().to_string())
    }

    /// Print a prompt (pipe codes allowed) and read a line.
    pub async fn ask(&mut self, prompt: &str, opts: LineOpts) -> TermResult<String> {
        self.print(prompt)?;
        self.read_line(opts).await
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// A terminal wired to channels, for tests.
    pub struct Harness {
        pub term: Term,
        pub keys: mpsc::Sender<Vec<u8>>,
        pub screen: mpsc::UnboundedReceiver<Vec<u8>>,
    }

    impl Harness {
        pub fn new() -> Self {
            let (out_tx, out_rx) = mpsc::unbounded_channel();
            let (in_tx, in_rx) = mpsc::channel(64);
            let (_size_tx, size_rx) = watch::channel((80, 24));
            Self {
                term: Term::new(out_tx, in_rx, size_rx, Duration::from_secs(5)),
                keys: in_tx,
                screen: out_rx,
            }
        }

        pub fn output(&mut self) -> String {
            let mut all = Vec::new();
            while let Ok(chunk) = self.screen.try_recv() {
                all.extend(chunk);
            }
            String::from_utf8_lossy(&all).into_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Harness;
    use super::*;

    #[tokio::test]
    async fn read_line_edits_and_echoes() {
        let mut h = Harness::new();
        h.keys.send(b"helx\x7flo\r".to_vec()).await.unwrap();
        let line = h.term.read_line(LineOpts::text(20)).await.unwrap();
        assert_eq!(line, "hello");
        let out = h.output();
        assert!(out.contains("hel"));
        assert!(out.contains("\x08 \x08"));
    }

    #[tokio::test]
    async fn masked_and_length_limited() {
        let mut h = Harness::new();
        h.keys.send(b"secret\r".to_vec()).await.unwrap();
        let line = h
            .term
            .read_line(LineOpts {
                max_len: 3,
                mask: true,
            })
            .await
            .unwrap();
        assert_eq!(line, "sec");
        let out = h.output();
        assert!(out.contains("***"));
        assert!(!out.contains("sec"));
    }

    #[tokio::test]
    async fn disconnect_is_reported() {
        let mut h = Harness::new();
        drop(h.keys);
        assert_eq!(h.term.read_key().await, Err(Hangup::Disconnected));
    }

    #[tokio::test]
    async fn writes_crlf() {
        let h = Harness::new();
        let mut h = h;
        h.term.write("a\nb\r\nc").unwrap();
        assert_eq!(h.output(), "a\r\nb\r\nc");
    }
}
