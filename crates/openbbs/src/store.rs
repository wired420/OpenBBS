//! Shared database handle for async code.

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use openbbs_core::db;
use rusqlite::Connection;

/// A database connection shared by every session. Work runs on Tokio's
/// blocking pool so SQLite and password hashing never stall the network.
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self::from_connection(db::open(path)?))
    }

    pub fn open_in_memory() -> Result<Self> {
        Ok(Self::from_connection(db::open_in_memory()?))
    }

    fn from_connection(conn: Connection) -> Self {
        Self {
            conn: Arc::new(Mutex::new(conn)),
        }
    }

    /// Run `f` with the connection on the blocking pool.
    pub async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().map_err(|_| anyhow!("database lock poisoned"))?;
            f(&guard)
        })
        .await?
    }
}

/// Run CPU-heavy work (password hashing) on the blocking pool.
pub async fn blocking<T, F>(f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    Ok(tokio::task::spawn_blocking(f).await?)
}
