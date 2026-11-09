//! The sysop console: manage users, message areas and file areas from
//! inside the BBS. Every screen here requires sysop access.

use anyhow::Result;
use openbbs_core::text::escape_pipes;
use openbbs_core::users::{self, LEVEL_SYSOP};
use openbbs_core::{files, messages};

use crate::boards::short_date;
use crate::picker::{Pick, Picker, fit};
use crate::session::Session;
use crate::store::blocking;
use crate::term::{LineOpts, Term, TermResult};

/// Ask for text, showing the current value; Enter keeps it.
async fn ask_text(term: &mut Term, label: &str, current: &str, max: usize) -> TermResult<String> {
    let prompt = if current.is_empty() {
        format!("|03{label}: ")
    } else {
        format!("|03{label} |08[{}]|03: ", escape_pipes(current))
    };
    let answer = term.ask(&prompt, LineOpts::text(max)).await?;
    Ok(if answer.is_empty() {
        current.to_string()
    } else {
        answer
    })
}

/// Ask for a level from 0 to 99; Enter keeps `current`.
async fn ask_level(term: &mut Term, label: &str, current: i64) -> TermResult<i64> {
    loop {
        let answer = term
            .ask(
                &format!("|03{label} |08[{current}]|03: "),
                LineOpts::text(2),
            )
            .await?;
        if answer.is_empty() {
            return Ok(current);
        }
        match answer.parse::<i64>() {
            Ok(n) if (0..=99).contains(&n) => return Ok(n),
            _ => term.println("|12Enter a number from 0 to 99.")?,
        }
    }
}

impl Session {
    fn is_sysop(&self) -> bool {
        self.level() >= LEVEL_SYSOP
    }

    async fn show_result(&self, term: &mut Term, result: Result<()>, done: &str) -> Result<()> {
        match result {
            Ok(()) => term.println(&format!("|10{done}"))?,
            Err(e) => term.println(&format!("|12{e}"))?,
        }
        term.pause().await?;
        Ok(())
    }

    /// Sysop: list and edit users.
    pub(crate) async fn sysop_users(&mut self, term: &mut Term) -> Result<()> {
        if !self.is_sysop() {
            return Ok(());
        }
        let mut start = 0;
        loop {
            self.node.set_activity("Sysop: users");
            let list = self.bbs.store.call(users::list_all).await?;
            let rows: Vec<String> = list
                .iter()
                .map(|u| {
                    format!(
                        " |15{} |03{:>5} |07{:>6} |08{} |07{}",
                        fit(&u.handle, 24),
                        u.access_level,
                        u.times_called,
                        fit(
                            &u.last_login_at
                                .as_deref()
                                .map(short_date)
                                .unwrap_or_default(),
                            8
                        ),
                        fit(&escape_pipes(u.location.as_deref().unwrap_or("")), 24)
                    )
                })
                .collect();
            let theme = self.bbs.theme();
            let picker = Picker {
                title: "Sysop · Users",
                header: " Handle                   Level  Calls Last on  Location",
                rows: &rows,
                hotkeys: &[('L', "Level"), ('P', "Password"), ('D', "Delete")],
                start,
                empty: "No users.",
                colors: &theme.colors,
            };
            let (me, _) = self.me();
            match picker.run(term).await? {
                Pick::Item(i) | Pick::Key('L', Some(i)) => {
                    start = i;
                    let user = &list[i];
                    term.println(&format!("|07Access level for |15{}|07:", user.handle))?;
                    let level = ask_level(term, "Level", user.access_level).await?;
                    if user.id == me && level < LEVEL_SYSOP {
                        term.println("|12You can't take away your own sysop access.")?;
                        term.pause().await?;
                        continue;
                    }
                    let id = user.id;
                    let result = self
                        .bbs
                        .store
                        .call(move |c| users::set_level(c, id, level))
                        .await;
                    self.show_result(term, result, "Level updated.").await?;
                }
                Pick::Key('P', Some(i)) => {
                    start = i;
                    let user = &list[i];
                    term.println(&format!("|07New password for |15{}|07:", user.handle))?;
                    let pw = term.ask("|03Password: ", LineOpts::password()).await?;
                    if pw.is_empty() {
                        continue;
                    }
                    if let Err(e) = users::validate_password(&pw) {
                        self.show_result(term, Err(e), "").await?;
                        continue;
                    }
                    let hash = blocking(move || users::hash_password(&pw)).await??;
                    let id = user.id;
                    let result = self
                        .bbs
                        .store
                        .call(move |c| users::set_password_hash(c, id, &hash))
                        .await;
                    tracing::info!(user = user.handle, "sysop reset password");
                    self.show_result(term, result, "Password changed.").await?;
                }
                Pick::Key('D', Some(i)) => {
                    start = i.saturating_sub(1);
                    let user = &list[i];
                    if user.id == me {
                        term.println("|12You can't delete your own account.")?;
                        term.pause().await?;
                        continue;
                    }
                    if term
                        .yes_no(
                            &format!(
                                "|07Delete |15{}|07 and their mailbox? Their posts stay.",
                                user.handle
                            ),
                            false,
                        )
                        .await?
                    {
                        let id = user.id;
                        let result = self.bbs.store.call(move |c| users::delete(c, id)).await;
                        tracing::info!(user = user.handle, "sysop deleted user");
                        self.show_result(term, result, "User deleted.").await?;
                    }
                }
                Pick::Key(..) => {}
                Pick::Quit => return Ok(()),
            }
        }
    }

    /// Sysop: add, edit and delete message areas.
    pub(crate) async fn sysop_message_areas(&mut self, term: &mut Term) -> Result<()> {
        if !self.is_sysop() {
            return Ok(());
        }
        let mut start = 0;
        loop {
            self.node.set_activity("Sysop: message areas");
            let list = self.bbs.store.call(messages::all_area_settings).await?;
            let rows: Vec<String> = list
                .iter()
                .map(|(_, a, count)| {
                    format!(
                        " |15{} |07{} |03{:>4} {:>4} |07{:>6}",
                        fit(&escape_pipes(&a.name), 24),
                        fit(&escape_pipes(&a.description), 28),
                        a.read_level,
                        a.post_level,
                        count
                    )
                })
                .collect();
            let theme = self.bbs.theme();
            let picker = Picker {
                title: "Sysop · Message Areas",
                header: " Area                     Description                  Read Post   Msgs",
                rows: &rows,
                hotkeys: &[('A', "Add"), ('E', "Edit"), ('D', "Delete")],
                start,
                empty: "No message areas. Press A to add one.",
                colors: &theme.colors,
            };
            match picker.run(term).await? {
                Pick::Key('A', _) => {
                    let blank = messages::AreaSettings {
                        conference_id: 1,
                        name: String::new(),
                        description: String::new(),
                        read_level: 10,
                        post_level: 10,
                    };
                    term.println("|11New message area")?;
                    let Some(settings) = self.edit_message_area(term, blank).await? else {
                        continue;
                    };
                    let result = self
                        .bbs
                        .store
                        .call(move |c| messages::add_area(c, &settings).map(|_| ()))
                        .await;
                    self.show_result(term, result, "Area added.").await?;
                }
                Pick::Item(i) | Pick::Key('E', Some(i)) => {
                    start = i;
                    let (id, current, _) = list[i].clone();
                    term.println("|11Edit message area |08(Enter keeps the current value)")?;
                    let Some(settings) = self.edit_message_area(term, current).await? else {
                        continue;
                    };
                    let result = self
                        .bbs
                        .store
                        .call(move |c| messages::update_area(c, id, &settings))
                        .await;
                    self.show_result(term, result, "Area updated.").await?;
                }
                Pick::Key('D', Some(i)) => {
                    start = i.saturating_sub(1);
                    let (id, area, count) = &list[i];
                    if term
                        .yes_no(
                            &format!(
                                "|07Delete |15{}|07 and its |15{count}|07 messages for good?",
                                escape_pipes(&area.name)
                            ),
                            false,
                        )
                        .await?
                    {
                        let id = *id;
                        let result = self
                            .bbs
                            .store
                            .call(move |c| messages::delete_area(c, id))
                            .await;
                        self.show_result(term, result, "Area deleted.").await?;
                    }
                }
                Pick::Key(..) => {}
                Pick::Quit => return Ok(()),
            }
        }
    }

    async fn edit_message_area(
        &mut self,
        term: &mut Term,
        mut a: messages::AreaSettings,
    ) -> Result<Option<messages::AreaSettings>> {
        a.name = ask_text(term, "Name", &a.name, 40).await?;
        if a.name.trim().is_empty() {
            return Ok(None);
        }
        a.description = ask_text(term, "Description", &a.description, 60).await?;
        a.read_level = ask_level(term, "Level to read", a.read_level).await?;
        a.post_level = ask_level(term, "Level to post", a.post_level).await?;
        Ok(Some(a))
    }

    /// Sysop: add, edit and delete file areas.
    pub(crate) async fn sysop_file_areas(&mut self, term: &mut Term) -> Result<()> {
        if !self.is_sysop() {
            return Ok(());
        }
        let mut start = 0;
        loop {
            self.node.set_activity("Sysop: file areas");
            let list = self.bbs.store.call(files::all_areas).await?;
            let rows: Vec<String> = list
                .iter()
                .map(|a| {
                    format!(
                        " |15{} |07{} |03{:>4} {:>4} |07{:>6}",
                        fit(&escape_pipes(&a.name), 22),
                        fit(&a.folder, 16),
                        a.download_level,
                        a.upload_level,
                        a.file_count
                    )
                })
                .collect();
            let theme = self.bbs.theme();
            let picker = Picker {
                title: "Sysop · File Areas",
                header: " Area                   Folder            DL   UL    Files",
                rows: &rows,
                hotkeys: &[('A', "Add"), ('E', "Edit"), ('D', "Delete")],
                start,
                empty: "No file areas. Press A to add one.",
                colors: &theme.colors,
            };
            let root = self.bbs.config.paths.files.clone();
            match picker.run(term).await? {
                Pick::Key('A', _) => {
                    term.println("|11New file area")?;
                    let name = ask_text(term, "Name", "", 40).await?;
                    if name.trim().is_empty() {
                        continue;
                    }
                    let suggested: String = name
                        .to_ascii_lowercase()
                        .chars()
                        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                        .collect();
                    let folder = ask_text(term, "Folder", &suggested, 40).await?;
                    let description = ask_text(term, "Description", "", 60).await?;
                    let download_level = ask_level(term, "Level to download", 10).await?;
                    let upload_level = ask_level(term, "Level to upload", 10).await?;
                    let settings = files::AreaSettings {
                        name,
                        description,
                        folder,
                        download_level,
                        upload_level,
                    };
                    let result = self
                        .bbs
                        .store
                        .call(move |c| files::add_area(c, &root, &settings).map(|_| ()))
                        .await;
                    self.show_result(term, result, "Area added.").await?;
                }
                Pick::Item(i) | Pick::Key('E', Some(i)) => {
                    start = i;
                    let a = list[i].clone();
                    term.println("|11Edit file area |08(Enter keeps the current value)")?;
                    let name = ask_text(term, "Name", &a.name, 40).await?;
                    let description = ask_text(term, "Description", &a.description, 60).await?;
                    let download_level =
                        ask_level(term, "Level to download", a.download_level).await?;
                    let upload_level = ask_level(term, "Level to upload", a.upload_level).await?;
                    let settings = files::AreaSettings {
                        name,
                        description,
                        folder: a.folder.clone(),
                        download_level,
                        upload_level,
                    };
                    let id = a.id;
                    let result = self
                        .bbs
                        .store
                        .call(move |c| files::update_area(c, id, &settings))
                        .await;
                    self.show_result(term, result, "Area updated.").await?;
                }
                Pick::Key('D', Some(i)) => {
                    start = i.saturating_sub(1);
                    let a = &list[i];
                    if term
                        .yes_no(
                            &format!("|07Delete the file area |15{}|07?", escape_pipes(&a.name)),
                            false,
                        )
                        .await?
                    {
                        let id = a.id;
                        let result = self
                            .bbs
                            .store
                            .call(move |c| files::delete_area(c, id))
                            .await;
                        self.show_result(term, result, "Area deleted.").await?;
                    }
                }
                Pick::Key(..) => {}
                Pick::Quit => return Ok(()),
            }
        }
    }
}
