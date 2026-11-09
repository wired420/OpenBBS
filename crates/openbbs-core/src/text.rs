//! Pipe color codes and `@TOKEN@` substitution for menu and prompt text.
//!
//! Pipe codes are the common BBS shorthand for ANSI colors:
//!
//! | Code        | Meaning                                  |
//! |-------------|------------------------------------------|
//! | `|00`-`|15` | foreground color (DOS palette order)     |
//! | `|16`-`|23` | background color                         |
//! | `|CL`       | clear the screen                         |
//! | `|CR`       | new line                                 |
//! | `|RS`       | reset colors                             |
//! | `||`        | a literal `|`                            |

/// DOS color index to ANSI SGR parameters (foreground).
const FG: [&str; 16] = [
    "0;30", "0;34", "0;32", "0;36", "0;31", "0;35", "0;33", "0;37", "1;30", "1;34", "1;32", "1;36",
    "1;31", "1;35", "1;33", "1;37",
];
/// DOS color index to ANSI background parameters.
const BG: [&str; 8] = ["40", "44", "42", "46", "41", "45", "43", "47"];

/// Expand pipe codes into ANSI escape sequences. Unknown codes are left as-is.
pub fn pipe_to_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    let mut rest = input;
    while let Some(pos) = rest.find('|') {
        out.push_str(&rest[..pos]);
        let code = &rest[pos + 1..];
        if let Some(after) = code.strip_prefix('|') {
            out.push('|');
            rest = after;
            continue;
        }
        let two: String = code.chars().take(2).collect();
        let replacement = match two.as_str() {
            "CL" => Some("\x1b[0m\x1b[2J\x1b[H".to_string()),
            "CR" => Some("\r\n".to_string()),
            "RS" => Some("\x1b[0m".to_string()),
            _ if two.len() == 2 && two.bytes().all(|b| b.is_ascii_digit()) => {
                match two.parse::<usize>().unwrap_or(99) {
                    n @ 0..=15 => Some(format!("\x1b[{}m", FG[n])),
                    n @ 16..=23 => Some(format!("\x1b[{}m", BG[n - 16])),
                    _ => None,
                }
            }
            _ => None,
        };
        match replacement {
            Some(r) => {
                out.push_str(&r);
                rest = &code[two.len()..];
            }
            None => {
                out.push('|');
                rest = code;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Remove pipe codes, leaving plain text (used to measure display width).
pub fn strip_pipes(input: &str) -> String {
    let ansi = pipe_to_ansi(input);
    strip_ansi(&ansi)
}

/// Remove ANSI escape sequences.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Replace `@NAME@` tokens using `lookup`. Unknown tokens are left untouched.
pub fn expand_tokens(input: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('@') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('@') {
            Some(end)
                if end > 0
                    && after[..end]
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b == b'_') =>
            {
                match lookup(&after[..end]) {
                    Some(value) => out.push_str(&value),
                    None => out.push_str(&rest[start..start + end + 2]),
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('@');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(pipe_to_ansi("|15Hi"), "\x1b[1;37mHi");
        assert_eq!(pipe_to_ansi("|01|17x"), "\x1b[0;34m\x1b[44mx");
        assert_eq!(pipe_to_ansi("a||b"), "a|b");
        assert_eq!(pipe_to_ansi("|99 |X"), "|99 |X");
        assert_eq!(pipe_to_ansi("end|"), "end|");
    }

    #[test]
    fn strips() {
        assert_eq!(strip_pipes("|12Red |07text"), "Red text");
    }

    #[test]
    fn tokens() {
        let out = expand_tokens("Hi @USER@, mail@example.com @NOPE@", |name| {
            (name == "USER").then(|| "Clayton".to_string())
        });
        assert_eq!(out, "Hi Clayton, mail@example.com @NOPE@");
    }
}
