//! Door games: the `doors.toml` list and the drop files doors read.
//!
//! A door is an external program. OpenBBS writes the caller's details to a
//! drop file (DOOR.SYS and/or DOOR32.SYS) in a per-node folder, starts the
//! program, and connects its standard input and output to the caller.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// Example `doors.toml`, written by `openbbs init`.
pub const EXAMPLE_DOORS_TOML: &str = r#"# Door games. Each [[door]] adds a door that menus can launch with
# action = "door:ID", and that appears in the Door Games list.
#
# OpenBBS writes DOOR.SYS and DOOR32.SYS for the caller into a folder for
# their node, then runs `command` with `args`, connecting the door's standard
# input and output to the caller. In `command`, `args` and `dir`, these are
# replaced:
#   {node}      node number              {dropdir}  folder holding the drop files
#   {door32}    full path to DOOR32.SYS  {doorsys}  full path to DOOR.SYS
#   {user}      caller's handle          {userid}   caller's user number
#
# id        short name used in menus (letters, numbers, - and _)
# name      shown to callers
# command   program to run
# args      list of arguments
# dir       working folder (optional)
# level     minimum access level (default 10)
# minutes   time the door is told the caller has (default 60)
# encoding  "cp437" (classic DOS doors, the default) or "utf8"
# columns   screen width the door draws for; its output wraps there so it
#           looks right in wider windows (default 80 for cp437 doors, off
#           for utf8; 0 turns it off)
#
# Example (remove the # signs and point it at a real door):
#
# [[door]]
# id = "lord"
# name = "Legend of the Red Dragon"
# command = "/opt/doors/lord/start.sh"
# args = ["{node}", "{door32}"]
# dir = "/opt/doors/lord"
"#;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    #[default]
    Cp437,
    Utf8,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Door {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub dir: Option<String>,
    #[serde(default = "default_level")]
    pub level: i64,
    #[serde(default = "default_minutes")]
    pub minutes: u32,
    #[serde(default)]
    pub encoding: Encoding,
    /// Screen width the door draws for. Output is wrapped at this column so
    /// callers with wider windows see the art as intended. 0 turns it off.
    /// Defaults to 80 for CP437 doors and off for UTF-8 doors.
    #[serde(default)]
    pub columns: Option<usize>,
}

impl Door {
    /// The width to wrap this door's output at, if any.
    pub fn wrap_columns(&self) -> Option<usize> {
        match (self.columns, self.encoding) {
            (Some(0), _) => None,
            (Some(n), _) => Some(n),
            (None, Encoding::Cp437) => Some(80),
            (None, Encoding::Utf8) => None,
        }
    }
}

fn default_level() -> i64 {
    10
}

fn default_minutes() -> u32 {
    60
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DoorsFile {
    #[serde(default, rename = "door")]
    doors: Vec<Door>,
}

/// Parse a `doors.toml`.
pub fn parse(text: &str) -> Result<Vec<Door>> {
    let file: DoorsFile = toml::from_str(text)?;
    let mut seen = Vec::new();
    for door in &file.doors {
        if door.id.is_empty()
            || !door
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!(
                "door id `{}` may only use letters, numbers, - and _",
                door.id
            );
        }
        if seen.contains(&door.id.to_ascii_lowercase()) {
            bail!("door id `{}` is used twice", door.id);
        }
        seen.push(door.id.to_ascii_lowercase());
        if door.command.trim().is_empty() {
            bail!("door `{}` has no command", door.id);
        }
    }
    Ok(file.doors)
}

/// Load `doors.toml` from the doors folder. A missing file means no doors.
pub fn load(doors_dir: &Path) -> Result<Vec<Door>> {
    let path = doors_dir.join("doors.toml");
    match fs::read_to_string(&path) {
        Ok(text) => parse(&text).with_context(|| format!("in {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Everything the drop files describe.
#[derive(Debug, Clone)]
pub struct DropInfo {
    pub bbs_name: String,
    pub sysop: String,
    pub software: String,
    pub node: u16,
    pub user_id: i64,
    pub handle: String,
    pub real_name: Option<String>,
    pub location: Option<String>,
    pub access_level: i64,
    pub minutes_left: u32,
    pub times_called: i64,
    pub rows: u16,
}

/// DOOR32.SYS: the modern 11-line format.
pub fn door32_sys(info: &DropInfo) -> String {
    let lines = [
        // Comm type 2 (telnet) with handle 0: I/O goes over stdin/stdout.
        "2".to_string(),
        "0".to_string(),
        "38400".to_string(),
        info.software.clone(),
        info.user_id.to_string(),
        info.real_name
            .clone()
            .unwrap_or_else(|| info.handle.clone()),
        info.handle.clone(),
        info.access_level.to_string(),
        info.minutes_left.to_string(),
        // Emulation: 1 = ANSI.
        "1".to_string(),
        info.node.to_string(),
    ];
    lines.join("\r\n") + "\r\n"
}

/// DOOR.SYS: the classic 52-line format.
pub fn door_sys(info: &DropInfo) -> String {
    let real = info
        .real_name
        .clone()
        .unwrap_or_else(|| info.handle.clone());
    let location = info.location.clone().unwrap_or_default();
    let today = "01/01/99".to_string();
    let lines: Vec<String> = vec![
        "COM0:".into(),                       // 1 comm port (0 = local/stdio)
        "38400".into(),                       // 2 baud
        "8".into(),                           // 3 data bits
        info.node.to_string(),                // 4 node
        "38400".into(),                       // 5 DTE rate
        "Y".into(),                           // 6 screen display
        "Y".into(),                           // 7 printer toggle
        "Y".into(),                           // 8 page bell
        "Y".into(),                           // 9 caller alarm
        real,                                 // 10 user full name
        location,                             // 11 location
        "".into(),                            // 12 home phone
        "".into(),                            // 13 work phone
        "".into(),                            // 14 password (never written)
        info.access_level.to_string(),        // 15 security level
        info.times_called.to_string(),        // 16 times on
        today.clone(),                        // 17 last date on
        (info.minutes_left * 60).to_string(), // 18 seconds left
        info.minutes_left.to_string(),        // 19 minutes left
        "GR".into(),                          // 20 graphics mode (ANSI)
        info.rows.to_string(),                // 21 screen length
        "N".into(),                           // 22 expert mode
        "".into(),                            // 23 conferences
        "".into(),                            // 24 conference exited to door
        "".into(),                            // 25 expiration date
        info.user_id.to_string(),             // 26 user record number
        "Y".into(),                           // 27 default protocol
        "0".into(),                           // 28 total uploads
        "0".into(),                           // 29 total downloads
        "0".into(),                           // 30 daily download K
        "999999".into(),                      // 31 max daily download K
        today.clone(),                        // 32 birthdate
        "".into(),                            // 33 main directory
        "".into(),                            // 34 gen directory
        info.sysop.clone(),                   // 35 sysop name
        info.handle.clone(),                  // 36 alias
        "00:00".into(),                       // 37 event time
        "Y".into(),                           // 38 error correcting
        "Y".into(),                           // 39 ANSI supported
        "Y".into(),                           // 40 use record locking
        "7".into(),                           // 41 default color
        "0".into(),                           // 42 time credits
        today,                                // 43 last new files scan
        "00:00".into(),                       // 44 time of this call
        "00:00".into(),                       // 45 time of last call
        "0".into(),                           // 46 max daily files
        "0".into(),                           // 47 files downloaded today
        "0".into(),                           // 48 total K uploaded
        "0".into(),                           // 49 total K downloaded
        "".into(),                            // 50 comment
        "0".into(),                           // 51 doors opened
        "0".into(),                           // 52 messages left
    ];
    lines.join("\r\n") + "\r\n"
}

/// Write both drop files into `dir` (created if needed). Returns their paths
/// as (DOOR32.SYS, DOOR.SYS).
/// Writes both drop files into `dir` and returns their absolute paths.
pub fn write_drop_files(dir: &Path, info: &DropInfo) -> Result<(PathBuf, PathBuf)> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    // Doors can run in their own folder (`dir` in doors.toml), so the paths
    // handed to them must not depend on OpenBBS's working directory.
    let dir = &std::path::absolute(dir)?;
    let door32 = dir.join("DOOR32.SYS");
    let doorsys = dir.join("DOOR.SYS");
    fs::write(&door32, door32_sys(info))?;
    fs::write(&doorsys, door_sys(info))?;
    Ok((door32, doorsys))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> DropInfo {
        DropInfo {
            bbs_name: "Test".into(),
            sysop: "Sysop".into(),
            software: "OpenBBS 0.0.1".into(),
            node: 3,
            user_id: 42,
            handle: "Dark Avenger".into(),
            real_name: None,
            location: Some("Texas".into()),
            access_level: 20,
            minutes_left: 45,
            times_called: 7,
            rows: 24,
        }
    }

    #[test]
    fn door32_has_eleven_lines() {
        let text = door32_sys(&info());
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 11);
        assert_eq!(lines[6], "Dark Avenger");
        assert_eq!(lines[5], "Dark Avenger", "real name falls back to handle");
        assert_eq!(lines[8], "45");
        assert_eq!(lines[10], "3");
        assert!(text.contains("\r\n"));
    }

    #[test]
    fn door_sys_has_fifty_two_lines() {
        let text = door_sys(&info());
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 52);
        assert_eq!(lines[3], "3");
        assert_eq!(lines[14], "20");
        assert_eq!(lines[18], "45");
        assert_eq!(lines[35], "Dark Avenger");
    }

    #[test]
    fn parses_doors_toml() {
        assert!(parse(EXAMPLE_DOORS_TOML).unwrap().is_empty());
        let doors =
            parse("[[door]]\nid='lord'\nname='LORD'\ncommand='/bin/lord'\nargs=['{door32}']\n")
                .unwrap();
        assert_eq!(doors[0].level, 10);
        assert_eq!(doors[0].encoding, Encoding::Cp437);
        assert!(parse("[[door]]\nid='a b'\nname='x'\ncommand='y'\n").is_err());
        assert!(
            parse(
                "[[door]]\nid='a'\nname='x'\ncommand='y'\n[[door]]\nid='A'\nname='x'\ncommand='y'\n"
            )
            .is_err()
        );
    }

    #[test]
    fn drop_file_paths_are_absolute() {
        // A relative folder, as config.paths.data usually is.
        let rel = PathBuf::from(format!("target/drop-test-{}", std::process::id()));
        let (door32, doorsys) = write_drop_files(&rel, &info()).unwrap();
        assert!(door32.is_absolute() && doorsys.is_absolute());
        assert!(door32.exists() && doorsys.exists());
        fs::remove_dir_all(&rel).unwrap();
    }
}
