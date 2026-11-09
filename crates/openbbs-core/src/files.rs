//! File areas and the files in them. Each area is a folder under
//! `paths.files`; the database holds descriptions, uploaders and counters.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};

pub const FILENAME_MAX: usize = 128;
pub const DESCRIPTION_MAX: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub struct FileArea {
    pub id: i64,
    pub name: String,
    pub description: String,
    /// Folder name under `paths.files`.
    pub folder: String,
    pub download_level: i64,
    pub upload_level: i64,
    pub file_count: i64,
}

impl FileArea {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            description: row.get("description")?,
            folder: row.get("folder")?,
            download_level: row.get("download_level")?,
            upload_level: row.get("upload_level")?,
            file_count: row.get("file_count")?,
        })
    }

    pub fn path(&self, files_root: &Path) -> PathBuf {
        files_root.join(&self.folder)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileEntry {
    pub id: i64,
    pub area_id: i64,
    pub filename: String,
    pub size_bytes: i64,
    pub description: String,
    pub uploader_id: Option<i64>,
    pub uploader: Option<String>,
    pub uploaded_at: String,
    pub downloads: i64,
}

impl FileEntry {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            area_id: row.get("area_id")?,
            filename: row.get("filename")?,
            size_bytes: row.get("size_bytes")?,
            description: row.get("description")?,
            uploader_id: row.get("uploader_id")?,
            uploader: row.get("uploader")?,
            uploaded_at: row.get("uploaded_at")?,
            downloads: row.get("downloads")?,
        })
    }
}

const AREA_SELECT: &str = "SELECT a.id, a.name, a.description, a.folder, a.download_level,
        a.upload_level, (SELECT count(*) FROM files f WHERE f.area_id = a.id) AS file_count
     FROM file_areas a";

const FILE_SELECT: &str = "SELECT f.id, f.area_id, f.filename, f.size_bytes, f.description,
        f.uploader_id, u.handle AS uploader, f.uploaded_at, f.downloads
     FROM files f LEFT JOIN users u ON u.id = f.uploader_id";

/// Is this a safe file name to store? No paths, no hidden files, no oddities.
pub fn validate_filename(name: &str) -> Result<()> {
    if name.is_empty() || name.chars().count() > FILENAME_MAX {
        bail!("File names must be 1 to {FILENAME_MAX} characters.");
    }
    if name.starts_with('.') || name.ends_with('.') || name.ends_with(' ') {
        bail!("File names can't start with a dot or end with a dot or space.");
    }
    if name
        .chars()
        .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
    {
        bail!("File names can't contain / \\ : * ? \" < > | or control characters.");
    }
    // Names Windows reserves.
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        bail!("That file name is reserved.");
    }
    Ok(())
}

/// File areas the user can download from (with file counts).
pub fn areas(conn: &Connection, level: i64) -> Result<Vec<FileArea>> {
    let mut stmt = conn.prepare(&format!(
        "{AREA_SELECT} WHERE a.download_level <= ?1 ORDER BY a.sort_order, a.name"
    ))?;
    let rows = stmt.query_map([level], FileArea::from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Every file area, regardless of level.
pub fn all_areas(conn: &Connection) -> Result<Vec<FileArea>> {
    areas(conn, i64::MAX)
}

pub fn area(conn: &Connection, id: i64) -> Result<Option<FileArea>> {
    Ok(conn
        .query_row(
            &format!("{AREA_SELECT} WHERE a.id = ?1"),
            [id],
            FileArea::from_row,
        )
        .optional()?)
}

/// Look up an area by its folder name (case-insensitive).
pub fn area_by_folder(conn: &Connection, folder: &str) -> Result<Option<FileArea>> {
    Ok(conn
        .query_row(
            &format!("{AREA_SELECT} WHERE a.folder = ?1 COLLATE NOCASE"),
            [folder],
            FileArea::from_row,
        )
        .optional()?)
}

pub fn list(conn: &Connection, area_id: i64) -> Result<Vec<FileEntry>> {
    let mut stmt = conn.prepare(&format!(
        "{FILE_SELECT} WHERE f.area_id = ?1 ORDER BY f.filename COLLATE NOCASE"
    ))?;
    let rows = stmt.query_map([area_id], FileEntry::from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<FileEntry>> {
    Ok(conn
        .query_row(
            &format!("{FILE_SELECT} WHERE f.id = ?1"),
            [id],
            FileEntry::from_row,
        )
        .optional()?)
}

pub fn find(conn: &Connection, area_id: i64, filename: &str) -> Result<Option<FileEntry>> {
    Ok(conn
        .query_row(
            &format!("{FILE_SELECT} WHERE f.area_id = ?1 AND f.filename = ?2 COLLATE NOCASE"),
            params![area_id, filename],
            FileEntry::from_row,
        )
        .optional()?)
}

/// Record a finished upload (or replace the entry for a re-upload).
pub fn record_upload(
    conn: &Connection,
    area_id: i64,
    filename: &str,
    size: u64,
    uploader_id: Option<i64>,
) -> Result<i64> {
    validate_filename(filename)?;
    let size = i64::try_from(size).unwrap_or(i64::MAX);
    conn.execute(
        "INSERT INTO files (area_id, filename, size_bytes, uploader_id) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (area_id, filename) DO UPDATE SET size_bytes = excluded.size_bytes,
             uploader_id = excluded.uploader_id, uploaded_at = datetime('now')",
        params![area_id, filename, size, uploader_id],
    )?;
    find(conn, area_id, filename)?
        .map(|f| f.id)
        .context("upload vanished")
}

pub fn record_download(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE files SET downloads = downloads + 1 WHERE id = ?1",
        [id],
    )?;
    Ok(())
}

pub fn set_description(conn: &Connection, id: i64, description: &str) -> Result<()> {
    let description: String = description.trim().chars().take(DESCRIPTION_MAX).collect();
    conn.execute(
        "UPDATE files SET description = ?2 WHERE id = ?1",
        params![id, description],
    )?;
    Ok(())
}

/// Delete a file's entry and the file on disk.
pub fn delete(conn: &Connection, files_root: &Path, id: i64) -> Result<()> {
    let Some(entry) = get(conn, id)? else {
        return Ok(());
    };
    let area = area(conn, entry.area_id)?.context("file area missing")?;
    let path = area.path(files_root).join(&entry.filename);
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("deleting {}", path.display())),
    }
    conn.execute("DELETE FROM files WHERE id = ?1", [id])?;
    Ok(())
}

/// A file area's settings, for the sysop.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaSettings {
    pub name: String,
    pub description: String,
    pub folder: String,
    pub download_level: i64,
    pub upload_level: i64,
}

fn check_area(settings: &AreaSettings) -> Result<()> {
    if settings.name.trim().is_empty() || settings.name.chars().count() > 40 {
        bail!("Area names must be 1 to 40 characters.");
    }
    if settings.folder.is_empty()
        || settings.folder.len() > 40
        || !settings
            .folder
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("Folder names may use letters, numbers, - and _ only.");
    }
    for level in [settings.download_level, settings.upload_level] {
        if !(0..=99).contains(&level) {
            bail!("Levels run from 0 to 99.");
        }
    }
    Ok(())
}

pub fn add_area(conn: &Connection, files_root: &Path, settings: &AreaSettings) -> Result<i64> {
    check_area(settings)?;
    conn.execute(
        "INSERT INTO file_areas (name, description, folder, download_level, upload_level)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            settings.name.trim(),
            settings.description.trim(),
            settings.folder.to_ascii_lowercase(),
            settings.download_level,
            settings.upload_level
        ],
    )
    .map_err(|e| anyhow::anyhow!("Couldn't add the area (is the name or folder taken?): {e}"))?;
    fs::create_dir_all(files_root.join(settings.folder.to_ascii_lowercase()))?;
    Ok(conn.last_insert_rowid())
}

/// Change an area's name, description and levels (the folder stays).
pub fn update_area(conn: &Connection, id: i64, settings: &AreaSettings) -> Result<()> {
    check_area(settings)?;
    conn.execute(
        "UPDATE file_areas SET name = ?2, description = ?3, download_level = ?4,
             upload_level = ?5 WHERE id = ?1",
        params![
            id,
            settings.name.trim(),
            settings.description.trim(),
            settings.download_level,
            settings.upload_level
        ],
    )?;
    Ok(())
}

/// Remove an empty file area. The folder on disk is left alone.
pub fn delete_area(conn: &Connection, id: i64) -> Result<()> {
    let count: i64 =
        conn.query_row("SELECT count(*) FROM files WHERE area_id = ?1", [id], |r| {
            r.get(0)
        })?;
    if count > 0 {
        bail!("The area still has {count} files. Delete them first.");
    }
    conn.execute("DELETE FROM file_areas WHERE id = ?1", [id])?;
    Ok(())
}

/// What [`sync`] changed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub added: usize,
    pub removed: usize,
    pub updated: usize,
}

/// Make the database match the folders on disk: files the sysop copied in
/// are added, entries whose file is gone are removed, sizes are refreshed.
/// Creates missing area folders.
pub fn sync(conn: &Connection, files_root: &Path) -> Result<SyncReport> {
    let mut report = SyncReport::default();
    for area in all_areas(conn)? {
        let dir = area.path(files_root);
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let mut on_disk = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let meta = entry.metadata()?;
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if !meta.is_file() || validate_filename(&name).is_err() {
                continue;
            }
            match find(conn, area.id, &name)? {
                None => {
                    record_upload(conn, area.id, &name, meta.len(), None)?;
                    report.added += 1;
                }
                Some(known) if known.size_bytes != meta.len() as i64 => {
                    conn.execute(
                        "UPDATE files SET size_bytes = ?2 WHERE id = ?1",
                        params![known.id, meta.len() as i64],
                    )?;
                    report.updated += 1;
                }
                Some(_) => {}
            }
            on_disk.push(name.to_lowercase());
        }
        for known in list(conn, area.id)? {
            if !on_disk.contains(&known.filename.to_lowercase()) {
                conn.execute("DELETE FROM files WHERE id = ?1", [known.id])?;
                report.removed += 1;
            }
        }
    }
    Ok(report)
}

/// `1536` becomes `1.5K`.
pub fn human_size(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut size = bytes.max(0) as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}B")
    } else if size < 10.0 {
        format!("{size:.1}{}", UNITS[unit])
    } else {
        format!("{size:.0}{}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn filename_rules() {
        assert!(validate_filename("game.zip").is_ok());
        assert!(validate_filename("My File (1).txt").is_ok());
        for bad in [
            "", ".hidden", "../x", "a/b", "a\\b", "con.txt", "COM1", "x.", "a|b",
        ] {
            assert!(validate_filename(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn sync_upload_download_delete() {
        let conn = db::open_in_memory().unwrap();
        let root = std::env::temp_dir().join(format!("openbbs-files-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        // First sync creates the Uploads folder.
        assert_eq!(sync(&conn, &root).unwrap(), SyncReport::default());
        let uploads = &areas(&conn, 10).unwrap()[0];
        let dir = uploads.path(&root);
        assert!(dir.is_dir());

        fs::write(dir.join("readme.txt"), b"hello").unwrap();
        fs::write(dir.join(".hidden"), b"x").unwrap();
        let report = sync(&conn, &root).unwrap();
        assert_eq!(report.added, 1);
        let entry = find(&conn, uploads.id, "README.TXT").unwrap().unwrap();
        assert_eq!(entry.size_bytes, 5);

        record_download(&conn, entry.id).unwrap();
        set_description(&conn, entry.id, "  Read me first  ").unwrap();
        let entry = get(&conn, entry.id).unwrap().unwrap();
        assert_eq!(
            (entry.downloads, entry.description.as_str()),
            (1, "Read me first")
        );
        assert_eq!(areas(&conn, 10).unwrap()[0].file_count, 1);

        fs::write(dir.join("readme.txt"), b"hello again").unwrap();
        assert_eq!(sync(&conn, &root).unwrap().updated, 1);

        delete(&conn, &root, entry.id).unwrap();
        assert!(!dir.join("readme.txt").exists());
        assert!(list(&conn, uploads.id).unwrap().is_empty());

        fs::write(dir.join("gone.txt"), b"x").unwrap();
        sync(&conn, &root).unwrap();
        fs::remove_file(dir.join("gone.txt")).unwrap();
        assert_eq!(sync(&conn, &root).unwrap().removed, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sysop_area_management() {
        let conn = db::open_in_memory().unwrap();
        let root = std::env::temp_dir().join(format!("openbbs-fa-{}", std::process::id()));
        let settings = AreaSettings {
            name: "Utilities".into(),
            description: "Handy tools".into(),
            folder: "Utils".into(),
            download_level: 10,
            upload_level: 20,
        };
        let id = add_area(&conn, &root, &settings).unwrap();
        assert!(root.join("utils").is_dir());
        assert!(area_by_folder(&conn, "UTILS").unwrap().is_some());
        let mut bad = settings.clone();
        bad.folder = "../etc".into();
        assert!(add_area(&conn, &root, &bad).is_err());
        record_upload(&conn, id, "tool.zip", 3, None).unwrap();
        assert!(
            delete_area(&conn, id).is_err(),
            "areas with files can't be deleted"
        );
        let file = find(&conn, id, "tool.zip").unwrap().unwrap();
        conn.execute("DELETE FROM files WHERE id = ?1", [file.id])
            .unwrap();
        delete_area(&conn, id).unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(512), "512B");
        assert_eq!(human_size(1536), "1.5K");
        assert_eq!(human_size(50 * 1024 * 1024), "50M");
    }
}
