//! SFTP access to the file areas, for callers who logged in to SSH with
//! their BBS handle and password.
//!
//! The SFTP root lists the file areas the caller may download from, as
//! folders. Downloads count toward a file's download total. Uploads are
//! allowed in areas where the caller meets the upload level; they're written
//! to a temporary file and only appear once complete.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use openbbs_core::files::{self, FileArea};
use openbbs_core::users::{LEVEL_SYSOP, User};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};
use russh_sftp::server::StatusReply;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::session::Bbs;

/// Most bytes returned by one read request.
const MAX_READ: u32 = 256 * 1024;
/// Most handles one client may have open.
const MAX_HANDLES: usize = 64;

enum Open {
    Dir {
        entries: Vec<File>,
        sent: bool,
    },
    Read {
        file: fs::File,
        file_id: i64,
        counted: bool,
    },
    Write {
        file: fs::File,
        temp: PathBuf,
        target: PathBuf,
        area_id: i64,
        name: String,
        written: u64,
        failed: bool,
    },
}

pub struct SftpSession {
    bbs: Arc<Bbs>,
    user: User,
    handles: HashMap<String, Open>,
    next_handle: u64,
}

/// A path inside the virtual tree.
enum Target {
    Root,
    Area(FileArea),
    File(FileArea, String),
}

fn status(code: StatusCode, message: &str) -> StatusReply {
    StatusReply::new(code).with_message(message)
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".into(),
        language_tag: "en-US".into(),
    }
}

/// Clean up a client path: resolve `.` and `..`, always absolute.
fn normalize(path: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p.to_string()),
        }
    }
    parts
}

fn dir_attrs() -> FileAttributes {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u32::try_from(d.as_secs()).unwrap_or(u32::MAX))
        .ok();
    let mut attrs = FileAttributes {
        permissions: Some(0o755),
        mtime: now,
        atime: now,
        ..Default::default()
    };
    attrs.set_dir(true);
    attrs
}

impl SftpSession {
    pub fn new(bbs: Arc<Bbs>, user: User) -> Self {
        Self {
            bbs,
            user,
            handles: HashMap::new(),
            next_handle: 1,
        }
    }

    fn files_root(&self) -> PathBuf {
        self.bbs.config.paths.files.clone()
    }

    async fn resolve(&self, path: &str) -> Result<Target, StatusReply> {
        let parts = normalize(path);
        let level = self.user.access_level;
        match parts.as_slice() {
            [] => Ok(Target::Root),
            [folder, rest @ ..] if rest.len() <= 1 => {
                let folder = folder.clone();
                let area = self
                    .bbs
                    .store
                    .call(move |c| files::area_by_folder(c, &folder))
                    .await
                    .map_err(|_| status(StatusCode::Failure, "database error"))?
                    .filter(|a| a.download_level <= level)
                    .ok_or_else(|| status(StatusCode::NoSuchFile, "no such folder"))?;
                match rest {
                    [] => Ok(Target::Area(area)),
                    [name] => Ok(Target::File(area, name.clone())),
                    _ => unreachable!(),
                }
            }
            _ => Err(status(StatusCode::NoSuchFile, "no such file")),
        }
    }

    fn add_handle(&mut self, open: Open) -> Result<String, StatusReply> {
        if self.handles.len() >= MAX_HANDLES {
            return Err(status(StatusCode::Failure, "too many open files"));
        }
        let handle = format!("h{}", self.next_handle);
        self.next_handle += 1;
        self.handles.insert(handle.clone(), open);
        Ok(handle)
    }

    async fn file_attrs(&self, area: &FileArea, name: &str) -> Result<FileAttributes, StatusReply> {
        let area_id = area.id;
        let lookup = name.to_string();
        let entry = self
            .bbs
            .store
            .call(move |c| files::find(c, area_id, &lookup))
            .await
            .map_err(|_| status(StatusCode::Failure, "database error"))?
            .ok_or_else(|| status(StatusCode::NoSuchFile, "no such file"))?;
        let path = area.path(&self.files_root()).join(&entry.filename);
        let meta = fs::metadata(&path)
            .await
            .map_err(|_| status(StatusCode::NoSuchFile, "no such file"))?;
        let mut attrs = FileAttributes::from(&meta);
        attrs.permissions = Some(0o644);
        attrs.set_regular(true);
        attrs.uid = None;
        attrs.gid = None;
        Ok(attrs)
    }

    async fn stat_path(&self, path: &str) -> Result<FileAttributes, StatusReply> {
        match self.resolve(path).await? {
            Target::Root | Target::Area(_) => Ok(dir_attrs()),
            Target::File(area, name) => self.file_attrs(&area, &name).await,
        }
    }
}

impl russh_sftp::server::Handler for SftpSession {
    type Error = StatusReply;

    fn unimplemented(&self) -> Self::Error {
        status(StatusCode::OpUnsupported, "not supported")
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        tracing::info!(user = self.user.handle, "sftp session started");
        Ok(Version::new())
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let clean = format!("/{}", normalize(&path).join("/"));
        Ok(Name {
            id,
            files: vec![File::dummy(clean)],
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        Ok(Attrs {
            id,
            attrs: self.stat_path(&path).await?,
        })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.stat(id, path).await
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let attrs = match self.handles.get(&handle) {
            Some(Open::Dir { .. }) => dir_attrs(),
            Some(Open::Read { file, .. }) | Some(Open::Write { file, .. }) => {
                let meta = file
                    .metadata()
                    .await
                    .map_err(|_| status(StatusCode::Failure, "stat failed"))?;
                let mut attrs = FileAttributes::from(&meta);
                attrs.uid = None;
                attrs.gid = None;
                attrs
            }
            None => return Err(status(StatusCode::Failure, "bad handle")),
        };
        Ok(Attrs { id, attrs })
    }

    async fn setstat(
        &mut self,
        id: u32,
        _path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        // Clients set times and modes after uploads; there's nothing to keep.
        Ok(ok(id))
    }

    async fn fsetstat(
        &mut self,
        id: u32,
        _handle: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        Ok(ok(id))
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let entries = match self.resolve(&path).await? {
            Target::Root => {
                let level = self.user.access_level;
                let areas = self
                    .bbs
                    .store
                    .call(move |c| files::areas(c, level))
                    .await
                    .map_err(|_| status(StatusCode::Failure, "database error"))?;
                areas
                    .into_iter()
                    .map(|a| File::new(a.folder, dir_attrs()))
                    .collect()
            }
            Target::Area(area) => {
                let area_id = area.id;
                let list = self
                    .bbs
                    .store
                    .call(move |c| files::list(c, area_id))
                    .await
                    .map_err(|_| status(StatusCode::Failure, "database error"))?;
                let dir = area.path(&self.files_root());
                let mut entries = Vec::new();
                for entry in list {
                    if let Ok(meta) = fs::metadata(dir.join(&entry.filename)).await {
                        let mut attrs = FileAttributes::from(&meta);
                        attrs.permissions = Some(0o644);
                        attrs.set_regular(true);
                        attrs.uid = None;
                        attrs.gid = None;
                        entries.push(File::new(entry.filename, attrs));
                    }
                }
                entries
            }
            Target::File(..) => return Err(status(StatusCode::NoSuchFile, "not a folder")),
        };
        let mut entries = entries;
        entries.insert(0, File::new(".", dir_attrs()));
        entries.insert(1, File::new("..", dir_attrs()));
        let handle = self.add_handle(Open::Dir {
            entries,
            sent: false,
        })?;
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        match self.handles.get_mut(&handle) {
            Some(Open::Dir { entries, sent }) if !*sent => {
                *sent = true;
                Ok(Name {
                    id,
                    files: entries.clone(),
                })
            }
            Some(Open::Dir { .. }) => Err(StatusCode::Eof.into()),
            _ => Err(status(StatusCode::Failure, "bad handle")),
        }
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        let Target::File(area, name) = self.resolve(&filename).await? else {
            return Err(status(StatusCode::Failure, "that's a folder"));
        };
        let wants_write = pflags.intersects(
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::APPEND,
        );
        let dir = area.path(&self.files_root());
        if !wants_write {
            let area_id = area.id;
            let lookup = name.clone();
            let entry = self
                .bbs
                .store
                .call(move |c| files::find(c, area_id, &lookup))
                .await
                .map_err(|_| status(StatusCode::Failure, "database error"))?
                .ok_or_else(|| status(StatusCode::NoSuchFile, "no such file"))?;
            let file = fs::File::open(dir.join(&entry.filename))
                .await
                .map_err(|_| status(StatusCode::NoSuchFile, "no such file"))?;
            let handle = self.add_handle(Open::Read {
                file,
                file_id: entry.id,
                counted: false,
            })?;
            return Ok(Handle { id, handle });
        }

        // Upload.
        if self.user.access_level < area.upload_level {
            return Err(status(
                StatusCode::PermissionDenied,
                "you can't upload to this area",
            ));
        }
        files::validate_filename(&name)
            .map_err(|e| status(StatusCode::PermissionDenied, &e.to_string()))?;
        let area_id = area.id;
        let lookup = name.clone();
        let existing = self
            .bbs
            .store
            .call(move |c| files::find(c, area_id, &lookup))
            .await
            .map_err(|_| status(StatusCode::Failure, "database error"))?;
        if let Some(existing) = &existing {
            let own = existing.uploader_id == Some(self.user.id);
            if !own && self.user.access_level < LEVEL_SYSOP {
                return Err(status(
                    StatusCode::PermissionDenied,
                    "a file with that name already exists",
                ));
            }
        }
        // Keep the case of an existing entry so it gets replaced, not duplicated.
        let final_name = existing.map(|e| e.filename).unwrap_or(name);
        fs::create_dir_all(&dir)
            .await
            .map_err(|_| status(StatusCode::Failure, "can't create area folder"))?;
        let temp = dir.join(format!(".upload-{}-{}", self.user.id, self.next_handle));
        let file = fs::File::create(&temp)
            .await
            .map_err(|_| status(StatusCode::Failure, "can't create file"))?;
        let handle = self.add_handle(Open::Write {
            file,
            temp,
            target: dir.join(&final_name),
            area_id,
            name: final_name,
            written: 0,
            failed: false,
        })?;
        Ok(Handle { id, handle })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let Some(Open::Read {
            file,
            counted,
            file_id,
        }) = self.handles.get_mut(&handle)
        else {
            return Err(status(StatusCode::Failure, "bad handle"));
        };
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(|_| status(StatusCode::Failure, "seek failed"))?;
        let mut buf = vec![0u8; len.min(MAX_READ) as usize];
        let n = file
            .read(&mut buf)
            .await
            .map_err(|_| status(StatusCode::Failure, "read failed"))?;
        if n == 0 {
            return Err(StatusCode::Eof.into());
        }
        buf.truncate(n);
        if !*counted {
            *counted = true;
            let file_id = *file_id;
            let _ = self
                .bbs
                .store
                .call(move |c| files::record_download(c, file_id))
                .await;
        }
        Ok(Data { id, data: buf })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let limit = self.bbs.config.files.max_upload_mb * 1024 * 1024;
        let Some(Open::Write {
            file,
            written,
            failed,
            ..
        }) = self.handles.get_mut(&handle)
        else {
            return Err(status(StatusCode::Failure, "bad handle"));
        };
        let end = offset + data.len() as u64;
        if end > limit {
            *failed = true;
            return Err(status(
                StatusCode::Failure,
                "file is larger than the upload limit",
            ));
        }
        let result = async {
            file.seek(std::io::SeekFrom::Start(offset)).await?;
            file.write_all(&data).await
        }
        .await;
        if result.is_err() {
            *failed = true;
            return Err(status(StatusCode::Failure, "write failed"));
        }
        *written = (*written).max(end);
        Ok(ok(id))
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        match self.handles.remove(&handle) {
            Some(Open::Write {
                mut file,
                temp,
                target,
                area_id,
                name,
                written,
                failed,
            }) => {
                let flushed = file.flush().await.is_ok();
                drop(file);
                if failed || !flushed {
                    let _ = fs::remove_file(&temp).await;
                    return Err(status(StatusCode::Failure, "upload failed"));
                }
                if fs::rename(&temp, &target).await.is_err() {
                    let _ = fs::remove_file(&temp).await;
                    return Err(status(StatusCode::Failure, "couldn't save the upload"));
                }
                let uploader = self.user.id;
                let recorded_name = name.clone();
                self.bbs
                    .store
                    .call(move |c| {
                        files::record_upload(c, area_id, &recorded_name, written, Some(uploader))
                    })
                    .await
                    .map_err(|_| status(StatusCode::Failure, "database error"))?;
                tracing::info!(
                    user = self.user.handle,
                    file = name,
                    bytes = written,
                    "upload"
                );
                Ok(ok(id))
            }
            Some(_) => Ok(ok(id)),
            None => Err(status(StatusCode::Failure, "bad handle")),
        }
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        let Target::File(area, name) = self.resolve(&filename).await? else {
            return Err(status(StatusCode::PermissionDenied, "can't remove that"));
        };
        let area_id = area.id;
        let entry = self
            .bbs
            .store
            .call(move |c| files::find(c, area_id, &name))
            .await
            .map_err(|_| status(StatusCode::Failure, "database error"))?
            .ok_or_else(|| status(StatusCode::NoSuchFile, "no such file"))?;
        let own = entry.uploader_id == Some(self.user.id);
        if !own && self.user.access_level < LEVEL_SYSOP {
            return Err(status(
                StatusCode::PermissionDenied,
                "only the sysop or uploader can remove files",
            ));
        }
        let root = self.files_root();
        let file_id = entry.id;
        self.bbs
            .store
            .call(move |c| files::delete(c, &root, file_id))
            .await
            .map_err(|_| status(StatusCode::Failure, "couldn't remove the file"))?;
        Ok(ok(id))
    }
}

impl Drop for SftpSession {
    fn drop(&mut self) {
        // Clean up any uploads the client abandoned.
        for open in self.handles.values() {
            if let Open::Write { temp, .. } = open {
                let _ = std::fs::remove_file(temp);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_paths() {
        assert!(normalize("/").is_empty());
        assert_eq!(
            normalize("/uploads/../uploads/./a.zip"),
            vec!["uploads", "a.zip"]
        );
        assert_eq!(normalize("../../etc/passwd"), vec!["etc", "passwd"]);
        assert_eq!(normalize("uploads\\x"), vec!["uploads", "x"]);
    }
}
