//! The message editor: a classic line editor with automatic word wrap.
//!
//! Callers type their message line by line. Long lines wrap on their own. A
//! command typed alone on a line controls the editor:
//! `/S` save, `/A` abort, `/L` list, `/D` delete the last line, `/?` help.

use openbbs_core::messages::BODY_MAX_LINES;

use crate::keys::Key;
use crate::term::{Term, TermResult};

/// Width of a message line. Fits an 80-column screen with the line numbers.
pub const LINE_WIDTH: usize = 72;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    Save(String),
    Abort,
}

const HELP: &str = "|08  Commands on a line by themselves: |15/S|07 save  |15/A|07 abort  \
|15/L|07 list  |15/D|07 delete last line  |15/?|07 help";

/// Run the editor. `initial` lines (like a quote) start the message.
pub async fn edit(term: &mut Term, initial: Vec<String>) -> TermResult<Edit> {
    let mut lines = initial;
    term.println("")?;
    term.println("|07  Type your message. Lines wrap on their own.")?;
    term.println(HELP)?;
    term.println("|08  ────────────────────────────────────────────────────────────────────────")?;
    for (i, line) in lines.iter().enumerate() {
        term.println(&format!("|08{:>3}: |07{}", i + 1, line.replace('|', "||")))?;
    }
    let mut carry = String::new();
    loop {
        if lines.len() >= BODY_MAX_LINES {
            term.println("|12  The message is as long as it can be. Saving.")?;
            return Ok(Edit::Save(lines.join("\n")));
        }
        let line = read_wrapped_line(term, lines.len() + 1, &mut carry).await?;
        match line.trim().to_ascii_uppercase().as_str() {
            "/S" => {
                while lines.last().is_some_and(|l| l.trim().is_empty()) {
                    lines.pop();
                }
                if lines.is_empty() {
                    term.println("|12  The message is empty. Type something, or /A to abort.")?;
                    continue;
                }
                return Ok(Edit::Save(lines.join("\n")));
            }
            "/A" => {
                if term.yes_no("|07  Throw this message away?", false).await? {
                    return Ok(Edit::Abort);
                }
            }
            "/L" => {
                for (i, line) in lines.iter().enumerate() {
                    term.println(&format!("|08{:>3}: |07{}", i + 1, line.replace('|', "||")))?;
                }
            }
            "/D" => {
                if let Some(gone) = lines.pop() {
                    term.println(&format!("|08  Deleted: {}", gone.replace('|', "||")))?;
                }
            }
            "/?" | "/H" => term.println(HELP)?,
            _ => lines.push(line),
        }
    }
}

/// Read one line, starting with `carry` (a word wrapped from the line before).
/// When the line overflows, the last word moves into `carry` for the next one.
async fn read_wrapped_line(
    term: &mut Term,
    number: usize,
    carry: &mut String,
) -> TermResult<String> {
    let mut line = std::mem::take(carry);
    term.print(&format!("|08{number:>3}: |15"))?;
    term.write(&line)?;
    loop {
        match term.read_key().await? {
            Key::Enter => break,
            Key::Backspace | Key::Delete | Key::Left => {
                if line.pop().is_some() {
                    term.write("\x08 \x08")?;
                }
            }
            Key::Tab => {
                let spaces = 4 - line.chars().count() % 4;
                for _ in 0..spaces {
                    if line.chars().count() < LINE_WIDTH {
                        line.push(' ');
                        term.write(" ")?;
                    }
                }
            }
            Key::Char(c) if !c.is_control() => {
                if line.chars().count() < LINE_WIDTH {
                    line.push(c);
                    term.write(c.encode_utf8(&mut [0; 4]))?;
                    continue;
                }
                // Line is full: wrap the last word to the next line.
                let split = line.rfind(' ').filter(|&i| i > 0);
                if let Some(i) = split {
                    let tail: String = line[i + 1..].to_string();
                    for _ in 0..tail.chars().count() {
                        term.write("\x08 \x08")?;
                    }
                    line.truncate(i);
                    *carry = tail;
                }
                if c != ' ' {
                    carry.push(c);
                }
                break;
            }
            _ => {}
        }
    }
    term.write("\x1b[0m\r\n")?;
    Ok(line.trim_end().to_string())
}

/// Quote a message for a reply: wrapped, with the author's initials.
pub fn quote(author: &str, body: &str, max_lines: usize) -> Vec<String> {
    let initials: String = author
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(3)
        .collect::<String>()
        .to_ascii_uppercase();
    let prefix = format!(" {initials}> ");
    let mut out: Vec<String> = openbbs_core::text::wrap(body, LINE_WIDTH - prefix.len())
        .into_iter()
        // Don't quote quotes of quotes.
        .filter(|l| !is_quote(l))
        .take(max_lines)
        .map(|l| {
            if l.is_empty() {
                l
            } else {
                format!("{prefix}{l}")
            }
        })
        .collect();
    out.push(String::new());
    out
}

/// True for a line that is already a quote, like ` DA> text`.
fn is_quote(line: &str) -> bool {
    line.char_indices()
        .take(6)
        .any(|(i, c)| c == '>' && line[..i].trim().chars().all(|c| c.is_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::testing::Harness;

    #[tokio::test]
    async fn types_wraps_and_saves() {
        let mut h = Harness::new();
        let long = "word ".repeat(20); // 100 chars, must wrap
        let input = format!("Hello\r{long}\r/l\r/s\r");
        h.keys.send(input.into_bytes()).await.unwrap();
        let Edit::Save(body) = edit(&mut h.term, vec![]).await.unwrap() else {
            panic!("expected save");
        };
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines[0], "Hello");
        assert!(lines[1].len() <= LINE_WIDTH);
        assert!(lines.len() == 3, "long line wrapped onto two: {lines:?}");
        assert_eq!(body.split_whitespace().filter(|w| *w == "word").count(), 20);
    }

    #[tokio::test]
    async fn delete_and_abort() {
        let mut h = Harness::new();
        h.keys.send(b"one\rtwo\r/d\r/a\ry".to_vec()).await.unwrap();
        assert_eq!(edit(&mut h.term, vec![]).await.unwrap(), Edit::Abort);
        assert!(h.output().contains("Deleted: two"));
    }

    #[tokio::test]
    async fn empty_message_is_not_saved() {
        let mut h = Harness::new();
        h.keys.send(b"/s\rhi\r/s\r".to_vec()).await.unwrap();
        assert_eq!(
            edit(&mut h.term, vec![]).await.unwrap(),
            Edit::Save("hi".into())
        );
    }

    #[test]
    fn quotes_with_initials() {
        let q = quote("Dark Avenger", "Hello world", 10);
        assert_eq!(q[0], " DA> Hello world");
        assert_eq!(q.last().unwrap(), "");
        let again = quote("Bob", &q.join("\n"), 10);
        assert_eq!(
            again,
            vec![String::new(), String::new()],
            "quotes aren't re-quoted"
        );
    }
}
