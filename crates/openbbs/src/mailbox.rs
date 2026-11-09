//! Private mail screens: inbox, reading, sending and replying.

use anyhow::Result;
use openbbs_core::mail::{self, Mail};
use openbbs_core::messages::SUBJECT_MAX;
use openbbs_core::text::{escape_pipes, user_colors, wrap};
use openbbs_core::users;

use crate::boards::short_date;
use crate::editor::{self, Edit};
use crate::keys::Key;
use crate::picker::{Pick, Picker, fit};
use crate::session::Session;
use crate::term::{LineOpts, Term};

impl Session {
    pub(crate) async fn mailbox(&mut self, term: &mut Term) -> Result<()> {
        let mut start = 0;
        loop {
            self.node.set_activity("Reading mail");
            let (me, _) = self.me();
            let letters = self.bbs.store.call(move |c| mail::inbox(c, me)).await?;
            let unread = letters.iter().filter(|m| m.is_unread()).count();
            let rows: Vec<String> = letters
                .iter()
                .map(|m| {
                    let marker = if m.is_unread() { "|14*" } else { " " };
                    format!(
                        "{marker} |15{} |07{} |08{}",
                        fit(&escape_pipes(&m.subject), 40),
                        fit(&escape_pipes(&m.from_name), 20),
                        short_date(&m.sent_at)
                    )
                })
                .collect();
            let title = format!(
                "Private Mail  |08·  |07{} letters, {unread} unread",
                letters.len()
            );
            let theme = self.bbs.theme();
            let picker = Picker {
                title: &title,
                header: "   Subject                                  From                 Date",
                rows: &rows,
                hotkeys: &[('S', "Send"), ('D', "Delete")],
                start,
                empty: "Your mailbox is empty. Press S to send a letter.",
                colors: &theme.colors,
            };
            match picker.run(term).await? {
                Pick::Item(i) => {
                    start = i;
                    self.read_letter(term, &letters[i]).await?;
                }
                Pick::Key('S', _) => self.send_mail(term, None).await?,
                Pick::Key('D', Some(i)) => {
                    start = i.saturating_sub(1);
                    let letter = &letters[i];
                    if term
                        .yes_no(
                            &format!("|07Delete \"{}\"?", escape_pipes(&letter.subject)),
                            false,
                        )
                        .await?
                    {
                        let id = letter.id;
                        self.bbs
                            .store
                            .call(move |c| mail::delete(c, me, id))
                            .await?;
                    }
                }
                Pick::Key(..) => {}
                Pick::Quit => return Ok(()),
            }
        }
    }

    async fn read_letter(&mut self, term: &mut Term, letter: &Mail) -> Result<()> {
        let (me, _) = self.me();
        let id = letter.id;
        self.bbs
            .store
            .call(move |c| mail::mark_read(c, me, id))
            .await?;
        let (cols, _) = term.size();
        term.clear()?;
        term.println("|11 Private Mail")?;
        term.println(&format!(
            "|03 From: |15{}  |08{}",
            fit(&escape_pipes(&letter.from_name), 30),
            letter.sent_at
        ))?;
        term.println(&format!("|03 Subj: |15{}", escape_pipes(&letter.subject)))?;
        let rule = format!("|08{}", "─".repeat(usize::from(cols.saturating_sub(1))));
        term.println(&rule)?;
        for line in wrap(&letter.body, usize::from(cols.saturating_sub(2))) {
            term.println(&format!(" |07{}", user_colors(&line)))?;
        }
        term.println(&rule)?;
        let can_reply = letter.from_id.is_some();
        let prompt = if can_reply {
            "|08[|11R|08]|07eply |08[|11D|08]|07elete |08[|11Q|08]|07uit |08» "
        } else {
            "|08[|11D|08]|07elete |08[|11Q|08]|07uit |08» "
        };
        term.print(prompt)?;
        loop {
            match term.read_key().await? {
                Key::Char('r' | 'R') if can_reply => {
                    term.println("|15Reply")?;
                    return self.send_mail(term, Some(letter)).await;
                }
                Key::Char('d' | 'D') => {
                    term.println("|15Delete")?;
                    if term.yes_no("|07Delete this letter?", false).await? {
                        let id = letter.id;
                        self.bbs
                            .store
                            .call(move |c| mail::delete(c, me, id))
                            .await?;
                    }
                    return Ok(());
                }
                Key::Char('q' | 'Q' | ' ') | Key::Enter | Key::Esc => return Ok(()),
                _ => {}
            }
        }
    }

    /// Write a letter, or a reply to `original`.
    pub(crate) async fn send_mail(
        &mut self,
        term: &mut Term,
        original: Option<&Mail>,
    ) -> Result<()> {
        self.node.set_activity("Writing mail");
        term.println("")?;
        let (to, subject, quote) = match original {
            Some(m) => {
                let subject = if m.subject.to_ascii_lowercase().starts_with("re:") {
                    m.subject.clone()
                } else {
                    format!("Re: {}", m.subject)
                };
                let quote = if term.yes_no("|07Quote their letter?", true).await? {
                    editor::quote(&m.from_name, &m.body, 12)
                } else {
                    Vec::new()
                };
                term.println(&format!("|03To: |15{}", escape_pipes(&m.from_name)))?;
                term.println(&format!("|03Subject: |15{}", escape_pipes(&subject)))?;
                (m.from_name.clone(), subject, quote)
            }
            None => {
                let to = loop {
                    let to = term
                        .ask(
                            "|03To |08(handle, blank to cancel)|03: ",
                            LineOpts::text(users::HANDLE_MAX),
                        )
                        .await?;
                    if to.is_empty() {
                        return Ok(());
                    }
                    let check = to.clone();
                    let found = self
                        .bbs
                        .store
                        .call(move |c| users::find_for_login(c, &check))
                        .await?;
                    match found {
                        Some((user, _)) => break user.handle,
                        None => term.println("|12There's no user by that name.")?,
                    }
                };
                let subject = term
                    .ask("|03Subject: ", LineOpts::text(SUBJECT_MAX))
                    .await?;
                if subject.is_empty() {
                    term.println("|07No subject, so nothing was sent.")?;
                    term.pause().await?;
                    return Ok(());
                }
                (to, subject, Vec::new())
            }
        };
        let body = match editor::edit(term, quote).await? {
            Edit::Save(body) => body,
            Edit::Abort => {
                term.println("|07Letter discarded.")?;
                term.pause().await?;
                return Ok(());
            }
        };
        let (me, _) = self.me();
        let result = self
            .bbs
            .store
            .call(move |c| mail::send(c, me, &to, &subject, &body))
            .await;
        match result {
            Ok(to) => {
                tracing::info!(to, "mail sent");
                term.println(&format!("|10Sent to |15{to}|10."))?;
            }
            Err(e) => term.println(&format!("|12Couldn't send: {e}"))?,
        }
        term.pause().await?;
        Ok(())
    }
}
