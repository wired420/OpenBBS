//! Message board screens: area list, message list, reader, posting, new scan.

use anyhow::Result;
use openbbs_core::messages::{self, Area, Message, NewMessage, SUBJECT_MAX};
use openbbs_core::text::{escape_pipes, user_colors, wrap};

use crate::editor::{self, Edit};
use crate::keys::Key;
use crate::picker::{Pick, Picker, fit};
use crate::session::Session;
use crate::term::{LineOpts, Term};

/// How a pass through the reader ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadEnd {
    /// Read past the last message.
    Finished,
    /// The caller pressed Q.
    Quit,
}

impl Session {
    /// The message areas list.
    pub(crate) async fn message_areas(&mut self, term: &mut Term) -> Result<()> {
        let mut start = 0;
        loop {
            self.node.set_activity("Browsing message areas");
            let areas = self.load_areas().await?;
            let rows: Vec<String> = areas
                .iter()
                .map(|a| {
                    let new = if a.new > 0 {
                        format!("|14{:>5}", a.new)
                    } else {
                        format!("|08{:>5}", 0)
                    };
                    format!(
                        " |03{} |15{} |07{} |03{:>6} {new}",
                        fit(&escape_pipes(&a.conference), 12),
                        fit(&escape_pipes(&a.name), 24),
                        fit(&escape_pipes(&a.description), 24),
                        a.total
                    )
                })
                .collect();
            let theme = self.bbs.theme();
            let picker = Picker {
                title: "Message Areas",
                header: " Conference   Area                     Description               Total   New",
                rows: &rows,
                hotkeys: &[('N', "New scan"), ('C', "Catch up")],
                start,
                empty: "There are no message areas you can read.",
                colors: &theme.colors,
            };
            match picker.run(term).await? {
                Pick::Item(i) => {
                    start = i;
                    self.area_messages(term, areas[i].id).await?;
                }
                Pick::Key('N', _) => self.new_scan(term).await?,
                Pick::Key('C', _) => {
                    if term
                        .yes_no("|07Mark every message in every area as read?", false)
                        .await?
                    {
                        let (id, _) = self.me();
                        let level = self.level();
                        self.bbs
                            .store
                            .call(move |c| messages::catch_up(c, id, level))
                            .await?;
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    async fn load_areas(&self) -> Result<Vec<Area>> {
        let (id, _) = self.me();
        let level = self.level();
        self.bbs
            .store
            .call(move |c| messages::areas(c, id, level, None))
            .await
    }

    async fn load_area(&self, area_id: i64) -> Result<Option<Area>> {
        let (id, _) = self.me();
        let level = self.level();
        self.bbs
            .store
            .call(move |c| messages::area(c, id, level, area_id))
            .await
    }

    /// One area's message list.
    async fn area_messages(&mut self, term: &mut Term, area_id: i64) -> Result<()> {
        let mut start: Option<usize> = None;
        loop {
            let Some(area) = self.load_area(area_id).await? else {
                return Ok(());
            };
            self.node.set_activity(&format!("Reading {}", area.name));
            let (me, _) = self.me();
            let (list, last_read) = self
                .bbs
                .store
                .call(move |c| {
                    Ok((
                        messages::list(c, area_id)?,
                        messages::last_read(c, me, area_id)?,
                    ))
                })
                .await?;
            let first_new = list.iter().position(|m| m.id > last_read);
            let start_at =
                start.unwrap_or_else(|| first_new.unwrap_or(list.len().saturating_sub(1)));
            let rows: Vec<String> = list
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let marker = if m.id > last_read { "|14*" } else { " " };
                    format!(
                        "{marker}|08{:>4} |15{} |07{} |03{} |08{}",
                        i + 1,
                        fit(&escape_pipes(&m.subject), 30),
                        fit(&escape_pipes(&m.from_name), 16),
                        fit(&escape_pipes(&m.to_name), 12),
                        short_date(&m.posted_at)
                    )
                })
                .collect();
            let title = format!(
                "{}  |08·  |07{} messages, {} new",
                area.name, area.total, area.new
            );
            let theme = self.bbs.theme();
            let can_post = self.level() >= area.post_level;
            let hotkeys: &[(char, &str)] = if can_post {
                &[('N', "Read new"), ('P', "Post")]
            } else {
                &[('N', "Read new")]
            };
            let picker = Picker {
                title: &title,
                header: "     #  Subject                        From             To           Date",
                rows: &rows,
                hotkeys,
                start: start_at,
                empty: "No messages yet. Press P to write the first one.",
                colors: &theme.colors,
            };
            let ids: Vec<i64> = list.iter().map(|m| m.id).collect();
            match picker.run(term).await? {
                Pick::Item(i) => {
                    start = Some(i);
                    self.read_messages(term, &area, &ids, i).await?;
                }
                Pick::Key('N', _) => match first_new {
                    Some(i) => {
                        self.read_messages(term, &area, &ids, i).await?;
                    }
                    None => {
                        term.println("|07No new messages here.")?;
                        term.pause().await?;
                    }
                },
                Pick::Key('P', _) => {
                    self.post_message(term, &area, None).await?;
                    start = None;
                }
                _ => return Ok(()),
            }
        }
    }

    /// Read messages one at a time, starting at `ids[index]`.
    async fn read_messages(
        &mut self,
        term: &mut Term,
        area: &Area,
        ids: &[i64],
        mut index: usize,
    ) -> Result<ReadEnd> {
        let (me, _) = self.me();
        while index < ids.len() {
            let id = ids[index];
            let area_id = area.id;
            let msg = self
                .bbs
                .store
                .call(move |c| {
                    let m = messages::get(c, id)?;
                    if m.is_some() {
                        messages::set_last_read(c, me, area_id, id)?;
                    }
                    Ok(m)
                })
                .await?;
            let Some(msg) = msg else {
                index += 1;
                continue;
            };
            self.show_message(term, area, &msg, index + 1, ids.len())
                .await?;

            let can_post = self.level() >= area.post_level;
            let can_delete =
                self.level() >= openbbs_core::users::LEVEL_SYSOP || msg.author_id == Some(me);
            let mut prompt = String::from("|08[|11N|08]|07ext |08[|11P|08]|07rev ");
            if can_post {
                prompt.push_str("|08[|11R|08]|07eply ");
            }
            prompt.push_str("|08[|11A|08]|07gain ");
            if can_delete {
                prompt.push_str("|08[|11D|08]|07elete ");
            }
            prompt.push_str("|08[|11Q|08]|07uit |08» ");
            term.print(&prompt)?;
            loop {
                match term.read_key().await? {
                    Key::Char('n' | 'N' | ' ') | Key::Enter | Key::Right => {
                        index += 1;
                        break;
                    }
                    Key::Char('p' | 'P') | Key::Left => {
                        index = index.saturating_sub(1);
                        break;
                    }
                    Key::Char('a' | 'A') => break,
                    Key::Char('r' | 'R') if can_post => {
                        term.println("|15Reply")?;
                        self.post_message(term, area, Some(&msg)).await?;
                        break;
                    }
                    Key::Char('d' | 'D') if can_delete => {
                        term.println("|15Delete")?;
                        if term.yes_no("|07Delete this message?", false).await? {
                            let id = msg.id;
                            self.bbs
                                .store
                                .call(move |c| messages::delete(c, id))
                                .await?;
                            tracing::info!(message = id, by = me, "message deleted");
                            index += 1;
                        }
                        break;
                    }
                    Key::Char('q' | 'Q') | Key::Esc => {
                        term.clear()?;
                        return Ok(ReadEnd::Quit);
                    }
                    _ => {}
                }
            }
        }
        term.clear()?;
        Ok(ReadEnd::Finished)
    }

    async fn show_message(
        &mut self,
        term: &mut Term,
        area: &Area,
        msg: &Message,
        number: usize,
        total: usize,
    ) -> Result<()> {
        let (cols, rows) = term.size();
        term.clear()?;
        term.println(&format!(
            "|11 {}  |08·  |07Message |15{number}|07 of |15{total}",
            escape_pipes(&area.name)
        ))?;
        term.println(&format!(
            "|03 From: |15{}  |08{}",
            fit(&escape_pipes(&msg.from_name), 30),
            msg.posted_at
        ))?;
        term.println(&format!("|03   To: |07{}", escape_pipes(&msg.to_name)))?;
        term.println(&format!("|03 Subj: |15{}", escape_pipes(&msg.subject)))?;
        let rule = format!("|08{}", "─".repeat(usize::from(cols.saturating_sub(1))));
        term.println(&rule)?;
        let body = wrap(&msg.body, usize::from(cols.saturating_sub(2)));
        let page = usize::from(rows.saturating_sub(8).max(5));
        for (i, line) in body.iter().enumerate() {
            if i > 0 && i % page == 0 {
                term.pause().await?;
            }
            let color = if line.trim_start().split_once('>').is_some_and(|(p, _)| {
                p.trim().len() <= 3 && p.trim().chars().all(|c| c.is_ascii_uppercase())
            }) {
                "|03"
            } else {
                "|07"
            };
            term.println(&format!(" {color}{}", user_colors(line)))?;
        }
        term.println(&rule)?;
        Ok(())
    }

    /// Write a new message, or a reply to `original`.
    async fn post_message(
        &mut self,
        term: &mut Term,
        area: &Area,
        original: Option<&Message>,
    ) -> Result<()> {
        self.node.set_activity(&format!("Posting in {}", area.name));
        term.println("")?;
        let (to, subject, quote) = match original {
            Some(m) => {
                let subject = if m.subject.to_ascii_lowercase().starts_with("re:") {
                    m.subject.clone()
                } else {
                    format!("Re: {}", m.subject)
                };
                let quote = if term.yes_no("|07Quote the original message?", true).await? {
                    editor::quote(&m.from_name, &m.body, 12)
                } else {
                    Vec::new()
                };
                term.println(&format!("|03To: |15{}", escape_pipes(&m.from_name)))?;
                term.println(&format!("|03Subject: |15{}", escape_pipes(&subject)))?;
                (m.from_name.clone(), subject, quote)
            }
            None => {
                let to = term
                    .ask("|03To |08(Enter for All)|03: ", LineOpts::text(24))
                    .await?;
                let subject = term
                    .ask("|03Subject: ", LineOpts::text(SUBJECT_MAX))
                    .await?;
                if subject.is_empty() {
                    term.println("|07No subject, so nothing was posted.")?;
                    term.pause().await?;
                    return Ok(());
                }
                (to, subject, Vec::new())
            }
        };
        let body = match editor::edit(term, quote).await? {
            Edit::Save(body) => body,
            Edit::Abort => {
                term.println("|07Message discarded.")?;
                term.pause().await?;
                return Ok(());
            }
        };
        let (me, handle) = self.me();
        let level = self.level();
        let new = NewMessage {
            area_id: area.id,
            reply_to: original.map(|m| m.id),
            to_name: to,
            subject,
            body,
        };
        let result = self
            .bbs
            .store
            .call(move |c| messages::post(c, me, &handle, level, &new))
            .await;
        match result {
            Ok(id) => {
                tracing::info!(message = id, area = area.name, "message posted");
                term.println(&format!("|10Posted in |15{}|10.", escape_pipes(&area.name)))?;
            }
            Err(e) => term.println(&format!("|12Couldn't post: {e}"))?,
        }
        term.pause().await?;
        Ok(())
    }

    /// Read every new message in every area.
    pub(crate) async fn new_scan(&mut self, term: &mut Term) -> Result<()> {
        self.node.set_activity("New message scan");
        let areas = self.load_areas().await?;
        let mut found = false;
        for area in areas.into_iter().filter(|a| a.new > 0) {
            found = true;
            let (me, _) = self.me();
            let area_id = area.id;
            let (ids, last) = self
                .bbs
                .store
                .call(move |c| {
                    Ok((
                        messages::message_ids(c, area_id)?,
                        messages::last_read(c, me, area_id)?,
                    ))
                })
                .await?;
            let Some(start) = ids.iter().position(|&id| id > last) else {
                continue;
            };
            term.clear()?;
            term.println(&format!(
                "|11Scanning |15{}|11: |14{}|11 new",
                escape_pipes(&area.name),
                area.new
            ))?;
            term.pause().await?;
            if self.read_messages(term, &area, &ids, start).await? == ReadEnd::Quit
                && !term.yes_no("|07Keep scanning other areas?", true).await?
            {
                return Ok(());
            }
        }
        if !found {
            term.println("|07No new messages. You're all caught up.")?;
        } else {
            term.println("|07End of new messages.")?;
        }
        term.pause().await?;
        Ok(())
    }
}

/// `2026-11-09 17:01:11` becomes `11/09/26`.
pub(crate) fn short_date(sqlite: &str) -> String {
    let date = sqlite.get(..10).unwrap_or(sqlite);
    let parts: Vec<&str> = date.split('-').collect();
    match parts.as_slice() {
        [y, m, d] if y.len() == 4 => format!("{m}/{d}/{}", &y[2..]),
        _ => date.to_string(),
    }
}
