//! Menu definitions (TOML files in the theme's `menus/` folder) and the menu
//! runner, which draws a menu and returns what the caller picked.
//!
//! Menus come in two styles:
//!
//! * **lightbar**: OpenBBS draws a framed menu; arrow keys move a highlight bar
//!   and Enter selects. Hotkeys work too.
//! * **hotkey**: the classic style. Shows the menu (or a custom ANSI `screen`)
//!   and waits for a hotkey at the prompt.
//!
//! A menu with a custom `screen` always runs in hotkey style, since OpenBBS
//! can't know where the art puts each item.

use std::fmt;

use anyhow::{Result, bail};
use openbbs_core::text::strip_pipes;
use serde::Deserialize;

use crate::keys::Key;
use crate::term::{Term, TermResult};
use crate::theme::Colors;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    #[default]
    Lightbar,
    Hotkey,
}

/// What a menu item does.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub enum Action {
    /// Open another menu.
    Menu(String),
    /// Return to the previous menu.
    Back,
    Logoff,
    Who,
    UserList,
    UserInfo,
    /// Show a screen file, then pause.
    Screen(String),
    /// Message area list.
    Messages,
    /// Read every new message.
    NewScan,
    /// The caller's mailbox.
    Mail,
    /// Write a new letter.
    SendMail,
    /// File areas.
    Files,
    /// Sysop: sync file areas with the folders on disk.
    RescanFiles,
    /// Placeholder for features that aren't built yet.
    ComingSoon(String),
}

impl TryFrom<String> for Action {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        let (verb, arg) = match s.split_once(':') {
            Some((v, a)) => (v.trim(), Some(a.trim().to_string())),
            None => (s.trim(), None),
        };
        let action = match (verb, arg) {
            ("menu", Some(name)) if !name.is_empty() => Action::Menu(name),
            ("screen", Some(name)) if !name.is_empty() => Action::Screen(name),
            ("comingsoon", Some(what)) => Action::ComingSoon(what),
            ("back", None) => Action::Back,
            ("logoff", None) => Action::Logoff,
            ("who", None) => Action::Who,
            ("userlist", None) => Action::UserList,
            ("userinfo", None) => Action::UserInfo,
            ("messages", None) => Action::Messages,
            ("newscan", None) => Action::NewScan,
            ("mail", None) => Action::Mail,
            ("sendmail", None) => Action::SendMail,
            ("files", None) => Action::Files,
            ("rescanfiles", None) => Action::RescanFiles,
            _ => return Err(format!("unknown menu action `{s}`")),
        };
        Ok(action)
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Menu(n) => write!(f, "menu:{n}"),
            Action::Back => f.write_str("back"),
            Action::Logoff => f.write_str("logoff"),
            Action::Who => f.write_str("who"),
            Action::UserList => f.write_str("userlist"),
            Action::UserInfo => f.write_str("userinfo"),
            Action::Messages => f.write_str("messages"),
            Action::NewScan => f.write_str("newscan"),
            Action::Mail => f.write_str("mail"),
            Action::SendMail => f.write_str("sendmail"),
            Action::Files => f.write_str("files"),
            Action::RescanFiles => f.write_str("rescanfiles"),
            Action::Screen(n) => write!(f, "screen:{n}"),
            Action::ComingSoon(w) => write!(f, "comingsoon:{w}"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub key: String,
    pub label: String,
    pub action: Action,
    /// Minimum access level to see and use this item.
    #[serde(default)]
    pub level: i64,
}

impl Item {
    pub fn hotkey(&self) -> char {
        self.key.chars().next().unwrap_or('?').to_ascii_uppercase()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Menu {
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub style: Style,
    /// Optional screen (in `screens/`) to show instead of drawing the menu.
    #[serde(default)]
    pub screen: Option<String>,
    #[serde(default)]
    pub prompt: String,
    #[serde(default, rename = "item")]
    pub items: Vec<Item>,
}

impl Menu {
    pub fn parse(source: &str) -> Result<Self> {
        let menu: Menu = toml::from_str(source)?;
        let mut seen = Vec::new();
        for item in &menu.items {
            if item.key.chars().count() != 1 || !item.key.chars().all(|c| c.is_ascii_graphic()) {
                bail!("item `{}`: key must be a single character", item.label);
            }
            let k = item.hotkey();
            if seen.contains(&k) {
                bail!("key `{k}` is used by more than one item");
            }
            seen.push(k);
        }
        Ok(menu)
    }

    /// Items the caller's access level allows.
    pub fn visible(&self, level: i64) -> Vec<&Item> {
        self.items.iter().filter(|i| level >= i.level).collect()
    }
}

/// Draws a menu and waits for a choice.
pub struct MenuRunner<'a> {
    pub menu: &'a Menu,
    pub colors: &'a Colors,
    pub level: i64,
    /// Decoded custom screen, if the menu has one.
    pub screen: Option<String>,
}

impl MenuRunner<'_> {
    pub async fn choose(&self, term: &mut Term) -> TermResult<Action> {
        let items = self.menu.visible(self.level);
        if items.is_empty() {
            term.println("|12This menu has no items you can use.")?;
            return Ok(Action::Back);
        }
        if self.screen.is_some() || self.menu.style == Style::Hotkey {
            self.run_hotkey(term, &items).await
        } else {
            self.run_lightbar(term, &items).await
        }
    }

    async fn run_hotkey(&self, term: &mut Term, items: &[&Item]) -> TermResult<Action> {
        let c = self.colors;
        term.clear()?;
        match &self.screen {
            Some(screen) => term.print(screen)?,
            None => {
                term.println("")?;
                term.println(&format!("  {}{}", c.title, self.menu.title))?;
                if !self.menu.subtitle.is_empty() {
                    term.println(&format!("  {}{}", c.subtitle, self.menu.subtitle))?;
                }
                term.println("")?;
                for item in items {
                    term.println(&format!(
                        "  {}[{}{}{}] {}{}",
                        c.bracket,
                        c.key,
                        item.hotkey(),
                        c.bracket,
                        c.label,
                        item.label
                    ))?;
                }
            }
        }
        term.println("")?;
        term.print(&self.prompt())?;
        loop {
            if let Key::Char(ch) = term.read_key().await?
                && let Some(item) = find(items, ch)
            {
                term.println(&format!("|15{}", item.label))?;
                return Ok(item.action.clone());
            }
        }
    }

    async fn run_lightbar(&self, term: &mut Term, items: &[&Item]) -> TermResult<Action> {
        let c = self.colors;
        let (cols, rows) = term.size();
        let cols = cols.max(40);
        let label_width = items
            .iter()
            .map(|i| i.label.chars().count())
            .max()
            .unwrap_or(0);
        let bar_width = (label_width + 10).max(32);
        let title = strip_pipes(&self.menu.title);
        let subtitle = strip_pipes(&self.menu.subtitle);
        let frame_width = bar_width
            .max(title.chars().count() + 8)
            .max(subtitle.chars().count() + 8);
        let frame_left = centered(cols, frame_width);
        let bar_left = centered(cols, bar_width);

        // Fit short terminals: drop the spacing lines first.
        let compact = usize::from(rows) < items.len() + 12;
        term.hide_cursor(true)?;
        term.clear()?;
        let mut row: u16 = if compact { 1 } else { 2 };
        let inner = frame_width - 2;
        term.goto(row, frame_left)?;
        term.print(&format!("{}╭{}╮", c.frame, "─".repeat(inner)))?;
        row += 1;
        term.goto(row, frame_left)?;
        term.print(&format!(
            "{}│{}{}{}│",
            c.frame,
            c.title,
            center_text(&title, inner),
            c.frame
        ))?;
        if !subtitle.is_empty() {
            row += 1;
            term.goto(row, frame_left)?;
            term.print(&format!(
                "{}│{}{}{}│",
                c.frame,
                c.subtitle,
                center_text(&subtitle, inner),
                c.frame
            ))?;
        }
        row += 1;
        term.goto(row, frame_left)?;
        term.print(&format!("{}╰{}╯", c.frame, "─".repeat(inner)))?;
        row += if compact { 1 } else { 2 };

        let first_item_row = row;
        let draw = |term: &Term, index: usize, selected: bool| -> TermResult<()> {
            let item = items[index];
            let text = format!(" {}  {}", item.hotkey(), item.label);
            let pad = bar_width.saturating_sub(text.chars().count() + 2);
            term.goto(first_item_row + index as u16, bar_left)?;
            if selected {
                term.print(&format!(
                    "{}{}{}{}|RS",
                    c.highlight,
                    "▌",
                    text,
                    " ".repeat(pad + 1)
                ))
            } else {
                term.print(&format!(
                    "|RS {}[{}{}{}] {}{}{}|RS",
                    c.bracket,
                    c.key,
                    item.hotkey(),
                    c.bracket,
                    c.label,
                    item.label,
                    " ".repeat(pad + 1)
                ))
            }
        };
        let mut selected = 0usize;
        for i in 0..items.len() {
            draw(term, i, i == selected)?;
        }
        row = first_item_row + items.len() as u16 + if compact { 0 } else { 1 };
        let hint = "↑↓ move   Enter select   or press a hotkey";
        term.goto(row, centered(cols, hint.chars().count()))?;
        term.print(&format!("{}{hint}", c.hint))?;
        let prompt_row = row + if compact { 1 } else { 2 };
        let prompt = self.prompt();
        let prompt_col = bar_left + strip_pipes(&prompt).chars().count() as u16;
        term.goto(prompt_row, bar_left)?;
        term.print(&prompt)?;

        let action = loop {
            let key = term.read_key().await?;
            let previous = selected;
            match key {
                Key::Up | Key::Left => selected = (selected + items.len() - 1) % items.len(),
                Key::Down | Key::Right | Key::Tab => selected = (selected + 1) % items.len(),
                Key::Home | Key::PageUp => selected = 0,
                Key::End | Key::PageDown => selected = items.len() - 1,
                Key::Enter => break items[selected].action.clone(),
                Key::Char(ch) => {
                    if let Some(index) = items
                        .iter()
                        .position(|i| i.hotkey() == ch.to_ascii_uppercase())
                    {
                        if index != selected {
                            draw(term, selected, false)?;
                            draw(term, index, true)?;
                        }
                        selected = index;
                        break items[index].action.clone();
                    }
                }
                _ => {}
            }
            if previous != selected {
                draw(term, previous, false)?;
                draw(term, selected, true)?;
                term.goto(prompt_row, prompt_col)?;
            }
        };
        term.goto(prompt_row, prompt_col)?;
        term.println(&format!("|15{}", items[selected].label))?;
        term.hide_cursor(false)?;
        Ok(action)
    }

    fn prompt(&self) -> String {
        if self.menu.prompt.is_empty() {
            "|07Command: ".to_string()
        } else {
            self.menu.prompt.clone()
        }
    }
}

fn find<'a>(items: &[&'a Item], ch: char) -> Option<&'a Item> {
    let ch = ch.to_ascii_uppercase();
    items.iter().copied().find(|i| i.hotkey() == ch)
}

/// 1-based column that centers `width` characters on a `cols`-wide screen.
fn centered(cols: u16, width: usize) -> u16 {
    let width = u16::try_from(width).unwrap_or(cols);
    cols.saturating_sub(width) / 2 + 1
}

fn center_text(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        return text.chars().take(width).collect();
    }
    let left = (width - len) / 2;
    format!(
        "{}{}{}",
        " ".repeat(left),
        text,
        " ".repeat(width - len - left)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::testing::Harness;

    const SAMPLE: &str = r#"
title = "Main"
[[item]]
key = "m"
label = "Messages"
action = "menu:messages"
[[item]]
key = "S"
label = "Sysop"
action = "menu:sysop"
level = 90
[[item]]
key = "G"
label = "Goodbye"
action = "logoff"
"#;

    #[test]
    fn parses_and_filters_by_level() {
        let menu = Menu::parse(SAMPLE).unwrap();
        assert_eq!(menu.items[0].action, Action::Menu("messages".into()));
        assert_eq!(menu.items[0].hotkey(), 'M');
        assert_eq!(menu.visible(10).len(), 2);
        assert_eq!(menu.visible(99).len(), 3);
    }

    #[test]
    fn rejects_bad_menus() {
        let dup = "title='x'\n[[item]]\nkey='a'\nlabel='1'\naction='who'\n[[item]]\nkey='A'\nlabel='2'\naction='who'\n";
        assert!(Menu::parse(dup).is_err());
        let bad_action = "title='x'\n[[item]]\nkey='a'\nlabel='1'\naction='explode'\n";
        assert!(Menu::parse(bad_action).is_err());
        let long_key = "title='x'\n[[item]]\nkey='ab'\nlabel='1'\naction='who'\n";
        assert!(Menu::parse(long_key).is_err());
    }

    #[test]
    fn built_in_menus_parse() {
        let theme = crate::theme::Theme::load(std::path::Path::new("/nonexistent")).unwrap();
        for name in ["main", "sysop"] {
            Menu::parse(&theme.menu_source(name).unwrap().unwrap()).unwrap();
        }
    }

    #[tokio::test]
    async fn lightbar_arrows_then_enter() {
        let menu = Menu::parse(SAMPLE).unwrap();
        let colors = Colors::default();
        let runner = MenuRunner {
            menu: &menu,
            colors: &colors,
            level: 10,
            screen: None,
        };
        let mut h = Harness::new();
        // Down moves to Goodbye (Sysop is hidden at level 10), Enter picks it.
        h.keys.send(b"\x1b[B\r".to_vec()).await.unwrap();
        assert_eq!(runner.choose(&mut h.term).await.unwrap(), Action::Logoff);
        let screen = h.output();
        assert!(screen.contains("Messages"));
        assert!(!screen.contains("Sysop"));
    }

    #[tokio::test]
    async fn hotkeys_work_in_both_styles() {
        let mut menu = Menu::parse(SAMPLE).unwrap();
        let colors = Colors::default();
        for style in [Style::Lightbar, Style::Hotkey] {
            menu.style = style;
            let runner = MenuRunner {
                menu: &menu,
                colors: &colors,
                level: 99,
                screen: None,
            };
            let mut h = Harness::new();
            h.keys.send(b"xs".to_vec()).await.unwrap();
            assert_eq!(
                runner.choose(&mut h.term).await.unwrap(),
                Action::Menu("sysop".into())
            );
        }
    }
}
