//! File area screens. Transfers themselves happen over SFTP (see `sftp.rs`);
//! these screens list files, show how to download and upload, and let
//! uploaders and the sysop describe or remove files.

use anyhow::Result;
use openbbs_core::files::{self, DESCRIPTION_MAX, FileArea, FileEntry, human_size};
use openbbs_core::text::escape_pipes;
use openbbs_core::users::LEVEL_SYSOP;

use crate::boards::short_date;
use crate::picker::{Pick, Picker, fit};
use crate::session::Session;
use crate::term::{LineOpts, Term};

impl Session {
    pub(crate) async fn file_areas(&mut self, term: &mut Term) -> Result<()> {
        let mut start = 0;
        loop {
            self.node.set_activity("Browsing file areas");
            let level = self.level();
            let areas = self.bbs.store.call(move |c| files::areas(c, level)).await?;
            let rows: Vec<String> = areas
                .iter()
                .map(|a| {
                    let upload = if level >= a.upload_level {
                        "|10 up"
                    } else {
                        "   "
                    };
                    format!(
                        " |15{} |07{} |03{:>6} {upload}",
                        fit(&escape_pipes(&a.name), 22),
                        fit(&escape_pipes(&a.description), 38),
                        a.file_count
                    )
                })
                .collect();
            let theme = self.bbs.theme();
            let picker = Picker {
                title: "File Areas",
                header: " Area                   Description                             Files",
                rows: &rows,
                hotkeys: &[('U', "How to upload")],
                start,
                empty: "There are no file areas you can use.",
                colors: &theme.colors,
            };
            match picker.run(term).await? {
                Pick::Item(i) => {
                    start = i;
                    self.area_files(term, &areas[i]).await?;
                }
                Pick::Key('U', i) => {
                    let area = i.map(|i| areas[i].clone());
                    self.transfer_help(term, area.as_ref(), None).await?;
                }
                _ => return Ok(()),
            }
        }
    }

    async fn area_files(&mut self, term: &mut Term, area: &FileArea) -> Result<()> {
        let mut start = 0;
        loop {
            self.node
                .set_activity(&format!("In file area {}", area.name));
            let area_id = area.id;
            let list = self
                .bbs
                .store
                .call(move |c| files::list(c, area_id))
                .await?;
            let rows: Vec<String> = list
                .iter()
                .map(|f| {
                    let description = if f.description.is_empty() {
                        "|08(no description)".to_string()
                    } else {
                        format!("|07{}", fit(&escape_pipes(&f.description), 30))
                    };
                    format!(
                        " |15{} |03{:>6} |08{} |03{:>4} {description}",
                        fit(&escape_pipes(&f.filename), 24),
                        human_size(f.size_bytes),
                        short_date(&f.uploaded_at),
                        f.downloads
                    )
                })
                .collect();
            let title = format!(
                "{}  |08·  |07{} files",
                escape_pipes(&area.name),
                list.len()
            );
            let theme = self.bbs.theme();
            let picker = Picker {
                title: &title,
                header: " File name                  Size Date      DLs Description",
                rows: &rows,
                hotkeys: &[('E', "Describe"), ('D', "Delete"), ('U', "Upload")],
                start,
                empty: "No files here yet.",
                colors: &theme.colors,
            };
            match picker.run(term).await? {
                Pick::Item(i) => {
                    start = i;
                    self.transfer_help(term, Some(area), Some(&list[i])).await?;
                }
                Pick::Key('E', Some(i)) => {
                    start = i;
                    self.describe_file(term, &list[i]).await?;
                }
                Pick::Key('D', Some(i)) => {
                    start = i.saturating_sub(1);
                    self.delete_file(term, &list[i]).await?;
                }
                Pick::Key('U', _) => self.transfer_help(term, Some(area), None).await?,
                Pick::Key(..) => {}
                Pick::Quit => return Ok(()),
            }
        }
    }

    fn may_manage(&self, file: &FileEntry) -> bool {
        let (me, _) = self.me();
        self.level() >= LEVEL_SYSOP || file.uploader_id == Some(me)
    }

    async fn describe_file(&mut self, term: &mut Term, file: &FileEntry) -> Result<()> {
        if !self.may_manage(file) {
            term.println("|12Only the uploader or the sysop can describe this file.")?;
            term.pause().await?;
            return Ok(());
        }
        term.println(&format!(
            "|07Description for |15{}|07:",
            escape_pipes(&file.filename)
        ))?;
        let text = term.ask("|03> ", LineOpts::text(DESCRIPTION_MAX)).await?;
        if text.is_empty() {
            return Ok(());
        }
        let id = file.id;
        self.bbs
            .store
            .call(move |c| files::set_description(c, id, &text))
            .await?;
        Ok(())
    }

    async fn delete_file(&mut self, term: &mut Term, file: &FileEntry) -> Result<()> {
        if !self.may_manage(file) {
            term.println("|12Only the uploader or the sysop can delete this file.")?;
            term.pause().await?;
            return Ok(());
        }
        if term
            .yes_no(
                &format!("|07Delete |15{}|07 for good?", escape_pipes(&file.filename)),
                false,
            )
            .await?
        {
            let root = self.bbs.config.paths.files.clone();
            let id = file.id;
            self.bbs
                .store
                .call(move |c| files::delete(c, &root, id))
                .await?;
            tracing::info!(file = file.filename, "file deleted");
        }
        Ok(())
    }

    /// File details plus how to download (or upload) it with SFTP.
    async fn transfer_help(
        &mut self,
        term: &mut Term,
        area: Option<&FileArea>,
        file: Option<&FileEntry>,
    ) -> Result<()> {
        let (_, handle) = self.me();
        let ssh = &self.bbs.config.ssh;
        let port = ssh.port();
        let host = ssh.display_host();
        let login = if handle.contains(' ') {
            format!("\"{handle}\"")
        } else {
            handle.clone()
        };
        let port_flag = if port == 22 {
            String::new()
        } else {
            format!("-P {port} ")
        };
        term.clear()?;
        if let Some(f) = file {
            term.println(&format!("|11 {}", escape_pipes(&f.filename)))?;
            term.println("|08 ─────────────────────────────────────────────────────────────")?;
            let field = |label: &str, value: String| format!("|03 {label:<12}|15{value}");
            term.println(&field("Size", human_size(f.size_bytes)))?;
            term.println(&field("Uploaded", format!("{} UTC", f.uploaded_at)))?;
            term.println(&field(
                "Uploader",
                f.uploader.clone().unwrap_or_else(|| "Sysop".into()),
            ))?;
            term.println(&field("Downloads", f.downloads.to_string()))?;
            if !f.description.is_empty() {
                term.println(&field("Description", escape_pipes(&f.description)))?;
            }
            term.println("")?;
        }
        term.println("|11 Transferring files")?;
        term.println("|07 Files move over |15SFTP|07, which every modern SSH client supports")?;
        term.println("|07 (OpenSSH, FileZilla, WinSCP, Cyberduck). Log in with your handle")?;
        term.println("|07 and BBS password:")?;
        term.println("")?;
        term.println(&format!(
            "|15   sftp {port_flag}{}@{host}",
            escape_pipes(&login)
        ))?;
        term.println("")?;
        let folder = area.map(|a| a.folder.as_str()).unwrap_or("uploads");
        match file {
            Some(f) => term.println(&format!(
                "|07 Then: |15get \"/{}/{}\"",
                escape_pipes(folder),
                escape_pipes(&f.filename)
            ))?,
            None => {
                let can_upload = area.is_none_or(|a| self.level() >= a.upload_level);
                if can_upload {
                    term.println(&format!(
                        "|07 To upload: |15put yourfile.zip \"/{}/\"",
                        escape_pipes(folder)
                    ))?;
                    term.println(&format!(
                        "|08 Uploads up to {} MB. Add a description here afterwards with E.",
                        self.bbs.config.files.max_upload_mb
                    ))?;
                } else {
                    term.println("|12 You can't upload to this area.")?;
                }
            }
        }
        term.println("")?;
        term.pause().await?;
        Ok(())
    }

    /// Sysop: make the file database match the folders on disk.
    pub(crate) async fn rescan_files(&mut self, term: &mut Term) -> Result<()> {
        if self.level() < LEVEL_SYSOP {
            return Ok(());
        }
        let root = self.bbs.config.paths.files.clone();
        let report = self.bbs.store.call(move |c| files::sync(c, &root)).await?;
        term.println(&format!(
            "|07Rescanned file areas: |15{}|07 added, |15{}|07 removed, |15{}|07 updated.",
            report.added, report.removed, report.updated
        ))?;
        term.pause().await?;
        Ok(())
    }
}
