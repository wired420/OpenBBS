//! Private user-to-user mail.

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::messages::validate;
use crate::users;

#[derive(Debug, Clone, PartialEq)]
pub struct Mail {
    pub id: i64,
    pub from_id: Option<i64>,
    pub from_name: String,
    pub to_id: i64,
    pub subject: String,
    pub body: String,
    pub sent_at: String,
    pub read_at: Option<String>,
}

impl Mail {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            from_id: row.get("from_id")?,
            from_name: row.get("from_name")?,
            to_id: row.get("to_id")?,
            subject: row.get("subject")?,
            body: row.get("body")?,
            sent_at: row.get("sent_at")?,
            read_at: row.get("read_at")?,
        })
    }

    pub fn is_unread(&self) -> bool {
        self.read_at.is_none()
    }
}

const SELECT: &str = "SELECT m.id, m.from_id, coalesce(u.handle, '(deleted user)') AS from_name,
        m.to_id, m.subject, m.body, m.sent_at, m.read_at
     FROM mail m LEFT JOIN users u ON u.id = m.from_id";

/// A user's mailbox, newest first.
pub fn inbox(conn: &Connection, user_id: i64) -> Result<Vec<Mail>> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE m.to_id = ?1 ORDER BY m.id DESC"))?;
    let rows = stmt.query_map([user_id], Mail::from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn unread_count(conn: &Connection, user_id: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM mail WHERE to_id = ?1 AND read_at IS NULL",
        [user_id],
        |r| r.get(0),
    )?)
}

/// Fetch one letter, only if it belongs to `user_id`.
pub fn get(conn: &Connection, user_id: i64, id: i64) -> Result<Option<Mail>> {
    Ok(conn
        .query_row(
            &format!("{SELECT} WHERE m.id = ?1 AND m.to_id = ?2"),
            [id, user_id],
            Mail::from_row,
        )
        .optional()?)
}

pub fn mark_read(conn: &Connection, user_id: i64, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE mail SET read_at = datetime('now')
         WHERE id = ?1 AND to_id = ?2 AND read_at IS NULL",
        [id, user_id],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, user_id: i64, id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM mail WHERE id = ?1 AND to_id = ?2",
        [id, user_id],
    )?;
    Ok(())
}

/// Send mail to a handle. Returns the recipient's exact handle.
pub fn send(
    conn: &Connection,
    from_id: i64,
    to_handle: &str,
    subject: &str,
    body: &str,
) -> Result<String> {
    validate(subject, body)?;
    let (to, _) = users::find_for_login(conn, to_handle.trim())?
        .with_context(|| format!("There's no user named {}.", to_handle.trim()))?;
    if to.id == from_id {
        bail!("You can't send mail to yourself.");
    }
    conn.execute(
        "INSERT INTO mail (from_id, to_id, subject, body) VALUES (?1, ?2, ?3, ?4)",
        params![from_id, to.id, subject.trim(), body],
    )?;
    Ok(to.handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn send_read_delete() {
        let conn = db::open_in_memory().unwrap();
        let new = |h: &str| users::NewUser {
            handle: h.into(),
            password: "x".into(),
            real_name: None,
            location: None,
        };
        let a = users::create(&conn, &new("Alice"), "h").unwrap();
        let b = users::create(&conn, &new("Bob"), "h").unwrap();

        assert_eq!(
            send(&conn, a.id, "bob", "Hi", "How are you?").unwrap(),
            "Bob"
        );
        assert!(send(&conn, a.id, "nobody", "Hi", "x").is_err());
        assert!(send(&conn, a.id, "Alice", "Hi", "x").is_err());

        assert_eq!(unread_count(&conn, b.id).unwrap(), 1);
        let letters = inbox(&conn, b.id).unwrap();
        assert_eq!(letters[0].from_name, "Alice");
        // Alice can't read Bob's mail.
        assert!(get(&conn, a.id, letters[0].id).unwrap().is_none());
        mark_read(&conn, b.id, letters[0].id).unwrap();
        assert_eq!(unread_count(&conn, b.id).unwrap(), 0);
        delete(&conn, a.id, letters[0].id).unwrap();
        assert_eq!(
            inbox(&conn, b.id).unwrap().len(),
            1,
            "only the owner can delete"
        );
        delete(&conn, b.id, letters[0].id).unwrap();
        assert!(inbox(&conn, b.id).unwrap().is_empty());
    }
}
