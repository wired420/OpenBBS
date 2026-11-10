//! Running door games: drop files, launching, and passing the caller's
//! terminal through to the door's standard input and output.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use openbbs_core::colwrap::ColumnWrap;
use openbbs_core::doors::{self, Door, DropInfo, Encoding};
use openbbs_core::text::escape_pipes;
use openbbs_core::{VERSION, cp437};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::picker::{Pick, Picker, fit};
use crate::session::Session;
use crate::term::{Hangup, Term};

/// Most lines of a door's stderr written to the log per run.
const MAX_STDERR_LINES: usize = 50;

/// Replace `{name}` placeholders.
fn fill(template: &str, values: &HashMap<&str, String>) -> String {
    let mut out = template.to_string();
    for (key, value) in values {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out
}

impl Session {
    fn load_doors(&self) -> Result<Vec<Door>> {
        doors::load(&self.bbs.config.paths.doors)
    }

    /// The Door Games list.
    pub(crate) async fn door_list(&mut self, term: &mut Term) -> Result<()> {
        let mut start = 0;
        loop {
            self.node.set_activity("Browsing doors");
            let level = self.level();
            let all = match self.load_doors() {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!("doors.toml: {e:#}");
                    term.println("|12The door list has an error. Tell the sysop.")?;
                    term.pause().await?;
                    return Ok(());
                }
            };
            let list: Vec<Door> = all.into_iter().filter(|d| level >= d.level).collect();
            let rows: Vec<String> = list
                .iter()
                .map(|d| format!(" |15{} |08{}", fit(&escape_pipes(&d.name), 50), d.id))
                .collect();
            let theme = self.bbs.theme();
            let picker = Picker {
                title: "Door Games",
                header: " Game",
                rows: &rows,
                hotkeys: &[],
                start,
                empty: "No doors are installed yet.",
                colors: &theme.colors,
            };
            match picker.run(term).await? {
                Pick::Item(i) => {
                    start = i;
                    let id = list[i].id.clone();
                    self.run_door(term, &id).await?;
                }
                _ => return Ok(()),
            }
        }
    }

    /// Launch a door by id.
    pub(crate) async fn run_door(&mut self, term: &mut Term, id: &str) -> Result<()> {
        let door = self
            .load_doors()?
            .into_iter()
            .find(|d| d.id.eq_ignore_ascii_case(id));
        let Some(door) = door else {
            term.println(&format!("|12There's no door called {}.", escape_pipes(id)))?;
            term.pause().await?;
            return Ok(());
        };
        if self.level() < door.level {
            term.println("|12You don't have access to that door.")?;
            term.pause().await?;
            return Ok(());
        }
        let Some(user) = self.user.clone() else {
            return Ok(());
        };
        let config = &self.bbs.config;
        let node = self.node.number();
        let drop_dir = config.paths.data.join("nodes").join(format!("node{node}"));
        let info = DropInfo {
            bbs_name: config.bbs.name.clone(),
            sysop: config.bbs.sysop.clone(),
            software: format!("OpenBBS {VERSION}"),
            node,
            user_id: user.id,
            handle: user.handle.clone(),
            real_name: user.real_name.clone(),
            location: user.location.clone(),
            access_level: user.access_level,
            minutes_left: door.minutes,
            times_called: user.times_called,
            rows: term.size().1,
        };
        let (door32, doorsys) = doors::write_drop_files(&drop_dir, &info)?;
        let drop_dir = door32.parent().map(Path::to_path_buf).unwrap_or(drop_dir);
        let values: HashMap<&str, String> = HashMap::from([
            ("node", node.to_string()),
            ("dropdir", drop_dir.display().to_string()),
            ("door32", door32.display().to_string()),
            ("doorsys", doorsys.display().to_string()),
            ("user", user.handle.clone()),
            ("userid", user.id.to_string()),
        ]);
        let mut command = Command::new(fill(&door.command, &values));
        command
            .args(door.args.iter().map(|a| fill(a, &values)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = &door.dir {
            command.current_dir(fill(dir, &values));
        }

        self.node.set_activity(&format!("Playing {}", door.name));
        tracing::info!(door = door.id, user = user.handle, node, "door started");
        term.clear()?;
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                tracing::error!(door = door.id, "couldn't start door: {e}");
                term.println(&format!(
                    "|12{} couldn't start. Tell the sysop.",
                    escape_pipes(&door.name)
                ))?;
                term.pause().await?;
                return Ok(());
            }
        };
        let mut stdin = child.stdin.take().context("door stdin")?;
        let mut stdout = child.stdout.take().context("door stdout")?;
        // Anything the door prints to stderr goes to the server log, so a door
        // that fails to start leaves a reason behind. Capped so a chatty door
        // can't flood the log.
        if let Some(stderr) = child.stderr.take() {
            let id = door.id.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                let mut logged = 0;
                while let Ok(Some(line)) = lines.next_line().await {
                    if logged < MAX_STDERR_LINES && !line.trim().is_empty() {
                        tracing::warn!(door = id, "door stderr: {}", line.trim_end());
                        logged += 1;
                    }
                }
            });
        }
        let mut buf = vec![0u8; 8192];
        let mut utf8_carry: Vec<u8> = Vec::new();
        let mut wrap = door.wrap_columns().map(ColumnWrap::new);
        let outcome: Result<(), Hangup> = loop {
            tokio::select! {
                read = stdout.read(&mut buf) => {
                    let n = match read {
                        Ok(0) | Err(_) => break Ok(()),
                        Ok(n) => n,
                    };
                    let text = match door.encoding {
                        Encoding::Cp437 => cp437::program_output(&buf[..n]),
                        Encoding::Utf8 => {
                            utf8_carry.extend_from_slice(&buf[..n]);
                            let valid = match std::str::from_utf8(&utf8_carry) {
                                Ok(_) => utf8_carry.len(),
                                Err(e) if e.error_len().is_none() => e.valid_up_to(),
                                Err(_) => utf8_carry.len(),
                            };
                            let text = String::from_utf8_lossy(&utf8_carry[..valid]).into_owned();
                            utf8_carry.drain(..valid);
                            text
                        }
                    };
                    let text = match wrap.as_mut() {
                        Some(wrap) => wrap.process(&text),
                        None => text,
                    };
                    if let Err(e) = term.write_raw(text.into_bytes()) {
                        break Err(e);
                    }
                }
                input = term.read_raw() => {
                    match input {
                        Ok(bytes) => {
                            if stdin.write_all(&bytes).await.is_err() || stdin.flush().await.is_err() {
                                break Ok(());
                            }
                        }
                        Err(e) => break Err(e),
                    }
                }
            }
        };
        drop(stdin);
        // Give the door a moment to exit on its own, then make sure it's gone.
        let status = match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
            Ok(status) => status.ok(),
            Err(_) => {
                let _ = child.kill().await;
                None
            }
        };
        match status {
            Some(status) if !status.success() => tracing::warn!(
                door = door.id,
                user = user.handle,
                "door finished with an error ({status})"
            ),
            _ => tracing::info!(door = door.id, user = user.handle, "door finished"),
        }
        outcome?;
        term.write("\x1b[0m\x1b[?25h\r\n")?;
        term.println(&format!(
            "|07Back at |15{}|07 from |15{}|07.",
            escape_pipes(&config.bbs.name),
            escape_pipes(&door.name)
        ))?;
        term.pause().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_placeholders() {
        let values = HashMap::from([
            ("node", "2".to_string()),
            ("door32", "/x/DOOR32.SYS".into()),
        ]);
        assert_eq!(
            fill("-N{node} {door32} {other}", &values),
            "-N2 /x/DOOR32.SYS {other}"
        );
    }
}
