//! SQLite storage and schema migrations.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Schema migrations, applied in order. Never edit one that has shipped;
/// add a new file instead.
const MIGRATIONS: &[(&str, &str)] = &[(
    "0001_initial",
    include_str!("../migrations/0001_initial.sql"),
)];

/// Open (creating if needed) the database at `path` and bring its schema up to date.
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
    }
    let conn =
        Connection::open(path).with_context(|| format!("opening database {}", path.display()))?;
    prepare(conn)
}

/// Open a throwaway in-memory database (used by tests).
pub fn open_in_memory() -> Result<Connection> {
    prepare(Connection::open_in_memory()?)
}

fn prepare(mut conn: Connection) -> Result<Connection> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    migrate(&mut conn)?;
    Ok(conn)
}

/// The schema version the database is at (number of migrations applied).
pub fn schema_version(conn: &Connection) -> Result<usize> {
    let v: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    Ok(v as usize)
}

/// The schema version this build expects.
pub fn latest_schema_version() -> usize {
    MIGRATIONS.len()
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let current = schema_version(conn)?;
    if current > MIGRATIONS.len() {
        anyhow::bail!(
            "database schema version {current} is newer than this build supports ({}); \
             upgrade OpenBBS",
            MIGRATIONS.len()
        );
    }
    for (index, (name, sql)) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)
            .with_context(|| format!("applying migration {name}"))?;
        tx.pragma_update(None, "user_version", (index + 1) as i64)?;
        tx.commit()?;
        tracing::info!(migration = name, "applied database migration");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_fresh_database() {
        let conn = open_in_memory().unwrap();
        assert_eq!(schema_version(&conn).unwrap(), latest_schema_version());
        let areas: i64 = conn
            .query_row("SELECT count(*) FROM message_areas", [], |r| r.get(0))
            .unwrap();
        assert_eq!(areas, 2);
    }

    #[test]
    fn reopening_is_a_no_op() {
        let dir = std::env::temp_dir().join(format!("openbbs-test-{}", std::process::id()));
        let path = dir.join("test.db");
        drop(open(&path).unwrap());
        let conn = open(&path).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), latest_schema_version());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
