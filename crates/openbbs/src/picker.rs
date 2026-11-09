//! A scrolling lightbar list: arrow keys move, Enter opens, hotkeys run
//! commands. Used for message areas, message lists, mailboxes and more.

use openbbs_core::text::strip_pipes;

use crate::keys::Key;
use crate::term::{Term, TermResult};
use crate::theme::Colors;

/// What the caller did in a [`Picker`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// Enter on a row.
    Item(usize),
    /// A hotkey (upper-cased), with the row that was selected (if any).
    Key(char, Option<usize>),
    /// Q or Esc.
    Quit,
}

pub struct Picker<'a> {
    pub title: &'a str,
    /// Column headings, drawn above the rows (pipe codes allowed).
    pub header: &'a str,
    /// One pre-formatted line per row (pipe codes allowed).
    pub rows: &'a [String],
    /// Extra commands shown in the footer, like `('P', "Post")`.
    pub hotkeys: &'a [(char, &'a str)],
    /// Row selected at the start.
    pub start: usize,
    /// Shown when there are no rows.
    pub empty: &'a str,
    pub colors: &'a Colors,
}

impl Picker<'_> {
    pub async fn run(&self, term: &mut Term) -> TermResult<Pick> {
        let (cols, rows) = term.size();
        let first_row: u16 = 4;
        let page = usize::from(rows.saturating_sub(first_row + 2).max(3));
        let width = usize::from(cols.saturating_sub(1));
        let count = self.rows.len();
        let mut selected = self.start.min(count.saturating_sub(1));
        let mut top = (selected / page) * page;

        term.hide_cursor(true)?;
        self.draw_frame(term, rows)?;
        self.draw_page(term, top, selected, page, width, first_row)?;
        let result = loop {
            let key = term.read_key().await?;
            let previous = (top, selected);
            match key {
                Key::Up if selected > 0 => selected -= 1,
                Key::Down if selected + 1 < count => selected += 1,
                Key::PageUp => selected = selected.saturating_sub(page),
                Key::PageDown => selected = (selected + page).min(count.saturating_sub(1)),
                Key::Home => selected = 0,
                Key::End => selected = count.saturating_sub(1),
                Key::Enter if count > 0 => break Pick::Item(selected),
                Key::Esc => break Pick::Quit,
                Key::Char(c) => {
                    let c = c.to_ascii_uppercase();
                    if self.hotkeys.iter().any(|(k, _)| *k == c) {
                        break Pick::Key(c, (count > 0).then_some(selected));
                    }
                    if c == 'Q' {
                        break Pick::Quit;
                    }
                }
                _ => {}
            }
            if count == 0 || previous == (top, selected) {
                continue;
            }
            if selected < top || selected >= top + page {
                top = (selected / page) * page;
                self.draw_page(term, top, selected, page, width, first_row)?;
            } else {
                self.draw_row(term, previous.1, false, top, width, first_row)?;
                self.draw_row(term, selected, true, top, width, first_row)?;
            }
        };
        term.hide_cursor(false)?;
        term.clear()?;
        Ok(result)
    }

    fn draw_frame(&self, term: &Term, rows: u16) -> TermResult<()> {
        let c = self.colors;
        term.clear()?;
        term.goto(1, 2)?;
        term.print(&format!("{}{}", c.title, self.title))?;
        term.goto(3, 1)?;
        term.print(&format!("{}{}", c.subtitle, self.header))?;
        let mut footer = format!("{}↑↓ move  Enter open", c.hint);
        for (key, label) in self.hotkeys {
            footer.push_str(&format!(
                "  {}[{}{}{}]{}{}",
                c.bracket, c.key, key, c.bracket, c.label, label
            ));
        }
        footer.push_str(&format!(
            "  {}[{}Q{}]{}Quit",
            c.bracket, c.key, c.bracket, c.label
        ));
        term.goto(rows, 2)?;
        term.print(&footer)
    }

    fn draw_page(
        &self,
        term: &Term,
        top: usize,
        selected: usize,
        page: usize,
        width: usize,
        first_row: u16,
    ) -> TermResult<()> {
        if self.rows.is_empty() {
            term.goto(first_row, 3)?;
            return term.print(&format!("{}{}", self.colors.hint, self.empty));
        }
        for slot in 0..page {
            let index = top + slot;
            term.goto(first_row + slot as u16, 1)?;
            term.write("\x1b[0m\x1b[K")?;
            if index < self.rows.len() {
                self.draw_row(term, index, index == selected, top, width, first_row)?;
            }
        }
        Ok(())
    }

    fn draw_row(
        &self,
        term: &Term,
        index: usize,
        selected: bool,
        top: usize,
        width: usize,
        first_row: u16,
    ) -> TermResult<()> {
        term.goto(first_row + (index - top) as u16, 1)?;
        let row = &self.rows[index];
        if selected {
            let plain: String = strip_pipes(row).chars().take(width).collect();
            let pad = width.saturating_sub(plain.chars().count());
            term.print(&format!(
                "{}{}{}|RS",
                self.colors.highlight,
                plain.replace('|', "||"),
                " ".repeat(pad)
            ))
        } else {
            term.print(&format!("|RS{row}|RS\x1b[K"))
        }
    }
}

/// Fit text into exactly `width` columns, cutting it with `…` if needed.
pub fn fit(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len <= width {
        format!("{text}{}", " ".repeat(width - len))
    } else if width == 0 {
        String::new()
    } else {
        let mut cut: String = text.chars().take(width - 1).collect();
        cut.push('…');
        cut
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::testing::Harness;

    fn rows(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("Row {i}")).collect()
    }

    #[tokio::test]
    async fn moves_and_selects() {
        let colors = Colors::default();
        let rows = rows(50);
        let picker = Picker {
            title: "T",
            header: "H",
            rows: &rows,
            hotkeys: &[('P', "Post")],
            start: 0,
            empty: "none",
            colors: &colors,
        };
        let mut h = Harness::new();
        // Page down past the first page, up one, then Enter.
        h.keys.send(b"\x1b[6~\x1b[A\r".to_vec()).await.unwrap();
        assert_eq!(picker.run(&mut h.term).await.unwrap(), Pick::Item(17));
        h.keys.send(b"p".to_vec()).await.unwrap();
        assert_eq!(
            picker.run(&mut h.term).await.unwrap(),
            Pick::Key('P', Some(0))
        );
        h.keys.send(b"q".to_vec()).await.unwrap();
        assert_eq!(picker.run(&mut h.term).await.unwrap(), Pick::Quit);
    }

    #[tokio::test]
    async fn empty_list_still_takes_hotkeys() {
        let colors = Colors::default();
        let picker = Picker {
            title: "T",
            header: "H",
            rows: &[],
            hotkeys: &[('S', "Send")],
            start: 0,
            empty: "No mail.",
            colors: &colors,
        };
        let mut h = Harness::new();
        h.keys.send(b"\rs".to_vec()).await.unwrap();
        assert_eq!(picker.run(&mut h.term).await.unwrap(), Pick::Key('S', None));
        assert!(h.output().contains("No mail."));
    }

    #[test]
    fn fits() {
        assert_eq!(fit("abc", 5), "abc  ");
        assert_eq!(fit("abcdef", 4), "abc…");
    }
}
