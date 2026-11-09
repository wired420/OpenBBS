//! Message boards: conferences, areas, messages and per-user read pointers.

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};

pub const SUBJECT_MAX: usize = 60;
pub const BODY_MAX_LINES: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub struct Conference {
    pub id: i64,
    pub name: String,
    pub description: String,
}

/// A message area plus counts for one user.
#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    pub id: i64,
    pub conference_id: i64,
    pub conference: String,
    pub name: String,
    pub description: String,
    pub post_level: i64,
    pub total: i64,
    /// Messages newer than the user's last-read pointer.
    pub new: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub id: i64,
    pub area_id: i64,
    pub reply_to: Option<i64>,
    pub author_id: Option<i64>,
    pub from_name: String,
    pub to_name: String,
    pub subject: String,
    pub body: String,
    pub posted_at: String,
}

impl Message {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            area_id: row.get("area_id")?,
            reply_to: row.get("reply_to")?,
            author_id: row.get("author_id")?,
            from_name: row.get("from_name")?,
            to_name: row.get("to_name")?,
            subject: row.get("subject")?,
            body: row.get("body")?,
            posted_at: row.get("posted_at")?,
        })
    }
}

const MESSAGE_COLUMNS: &str =
    "id, area_id, reply_to, author_id, from_name, to_name, subject, body, posted_at";

/// Conferences with at least one area the user can read.
pub fn conferences(conn: &Connection, level: i64) -> Result<Vec<Conference>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.name, c.description FROM message_conferences c
         WHERE c.read_level <= ?1
           AND EXISTS (SELECT 1 FROM message_areas a
                       WHERE a.conference_id = c.id AND a.read_level <= ?1)
         ORDER BY c.sort_order, c.name",
    )?;
    let rows = stmt.query_map([level], |r| {
        Ok(Conference {
            id: r.get(0)?,
            name: r.get(1)?,
            description: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Areas the user can read, with total and new counts. `conference` limits
/// the list to one conference.
pub fn areas(
    conn: &Connection,
    user_id: i64,
    level: i64,
    conference: Option<i64>,
) -> Result<Vec<Area>> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.conference_id, c.name, a.name, a.description, a.post_level,
                (SELECT count(*) FROM messages m WHERE m.area_id = a.id),
                (SELECT count(*) FROM messages m WHERE m.area_id = a.id
                   AND m.id > coalesce((SELECT last_id FROM message_lastread l
                                        WHERE l.user_id = ?1 AND l.area_id = a.id), 0))
         FROM message_areas a JOIN message_conferences c ON c.id = a.conference_id
         WHERE a.read_level <= ?2 AND c.read_level <= ?2
           AND (?3 IS NULL OR a.conference_id = ?3)
         ORDER BY c.sort_order, c.name, a.sort_order, a.name",
    )?;
    let rows = stmt.query_map(params![user_id, level, conference], |r| {
        Ok(Area {
            id: r.get(0)?,
            conference_id: r.get(1)?,
            conference: r.get(2)?,
            name: r.get(3)?,
            description: r.get(4)?,
            post_level: r.get(5)?,
            total: r.get(6)?,
            new: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// One area, if the user can read it.
pub fn area(conn: &Connection, user_id: i64, level: i64, area_id: i64) -> Result<Option<Area>> {
    Ok(areas(conn, user_id, level, None)?
        .into_iter()
        .find(|a| a.id == area_id))
}

/// Message ids in an area, oldest first.
pub fn message_ids(conn: &Connection, area_id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare("SELECT id FROM messages WHERE area_id = ?1 ORDER BY id")?;
    let ids = stmt.query_map([area_id], |r| r.get(0))?;
    Ok(ids.collect::<rusqlite::Result<_>>()?)
}

/// Headers (no bodies needed, but cheap enough) for listing an area.
pub fn list(conn: &Connection, area_id: i64) -> Result<Vec<Message>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {MESSAGE_COLUMNS} FROM messages WHERE area_id = ?1 ORDER BY id"
    ))?;
    let rows = stmt.query_map([area_id], Message::from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Message>> {
    Ok(conn
        .query_row(
            &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"),
            [id],
            Message::from_row,
        )
        .optional()?)
}

/// A new post.
#[derive(Debug, Clone)]
pub struct NewMessage {
    pub area_id: i64,
    pub reply_to: Option<i64>,
    pub to_name: String,
    pub subject: String,
    pub body: String,
}

pub fn validate(subject: &str, body: &str) -> Result<()> {
    if subject.trim().is_empty() {
        bail!("A subject is required.");
    }
    if subject.chars().count() > SUBJECT_MAX {
        bail!("Subjects can be at most {SUBJECT_MAX} characters.");
    }
    if body.trim().is_empty() {
        bail!("The message is empty.");
    }
    Ok(())
}

/// Post a message as `author`. Checks the area's post level. Returns the new
/// message id, and moves the author's read pointer past it.
pub fn post(
    conn: &Connection,
    author_id: i64,
    author_name: &str,
    level: i64,
    new: &NewMessage,
) -> Result<i64> {
    validate(&new.subject, &new.body)?;
    let area = area(conn, author_id, level, new.area_id)?.context("no such message area")?;
    if level < area.post_level {
        bail!("You can't post in {}.", area.name);
    }
    let to = if new.to_name.trim().is_empty() {
        "All"
    } else {
        new.to_name.trim()
    };
    conn.execute(
        "INSERT INTO messages (area_id, reply_to, author_id, from_name, to_name, subject, body)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            new.area_id,
            new.reply_to,
            author_id,
            author_name,
            to,
            new.subject.trim(),
            new.body
        ],
    )?;
    let id = conn.last_insert_rowid();
    // Don't show people their own post as new, unless they had unread ones before it.
    let had_unread = area.new > 0;
    if !had_unread {
        set_last_read(conn, author_id, new.area_id, id)?;
    }
    Ok(id)
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM messages WHERE id = ?1", [id])?;
    Ok(())
}

pub fn last_read(conn: &Connection, user_id: i64, area_id: i64) -> Result<i64> {
    Ok(conn
        .query_row(
            "SELECT last_id FROM message_lastread WHERE user_id = ?1 AND area_id = ?2",
            [user_id, area_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

/// Move the read pointer forward (never backward).
pub fn set_last_read(conn: &Connection, user_id: i64, area_id: i64, message_id: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO message_lastread (user_id, area_id, last_id) VALUES (?1, ?2, ?3)
         ON CONFLICT (user_id, area_id) DO UPDATE SET last_id = max(last_id, excluded.last_id)",
        [user_id, area_id, message_id],
    )?;
    Ok(())
}

/// Mark everything in every readable area as read.
pub fn catch_up(conn: &Connection, user_id: i64, level: i64) -> Result<()> {
    for area in areas(conn, user_id, level, None)? {
        if let Some(last) = message_ids(conn, area.id)?.last() {
            set_last_read(conn, user_id, area.id, *last)?;
        }
    }
    Ok(())
}

/// Total new messages across all readable areas.
pub fn total_new(conn: &Connection, user_id: i64, level: i64) -> Result<i64> {
    Ok(areas(conn, user_id, level, None)?
        .iter()
        .map(|a| a.new)
        .sum())
}

/// A message area's settings, for the sysop.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaSettings {
    pub conference_id: i64,
    pub name: String,
    pub description: String,
    pub read_level: i64,
    pub post_level: i64,
}

fn check_settings(settings: &AreaSettings) -> Result<()> {
    if settings.name.trim().is_empty() || settings.name.chars().count() > 40 {
        bail!("Area names must be 1 to 40 characters.");
    }
    for level in [settings.read_level, settings.post_level] {
        if !(0..=99).contains(&level) {
            bail!("Levels run from 0 to 99.");
        }
    }
    Ok(())
}

/// Every area with its settings, regardless of level.
pub fn all_area_settings(conn: &Connection) -> Result<Vec<(i64, AreaSettings, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.conference_id, a.name, a.description, a.read_level, a.post_level,
                (SELECT count(*) FROM messages m WHERE m.area_id = a.id)
         FROM message_areas a JOIN message_conferences c ON c.id = a.conference_id
         ORDER BY c.sort_order, c.name, a.sort_order, a.name",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get(0)?,
            AreaSettings {
                conference_id: r.get(1)?,
                name: r.get(2)?,
                description: r.get(3)?,
                read_level: r.get(4)?,
                post_level: r.get(5)?,
            },
            r.get(6)?,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn add_area(conn: &Connection, settings: &AreaSettings) -> Result<i64> {
    check_settings(settings)?;
    conn.execute(
        "INSERT INTO message_areas (conference_id, name, description, read_level, post_level)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            settings.conference_id,
            settings.name.trim(),
            settings.description.trim(),
            settings.read_level,
            settings.post_level
        ],
    )
    .map_err(|e| anyhow::anyhow!("Couldn't add the area (is the name taken?): {e}"))?;
    Ok(conn.last_insert_rowid())
}

pub fn update_area(conn: &Connection, id: i64, settings: &AreaSettings) -> Result<()> {
    check_settings(settings)?;
    conn.execute(
        "UPDATE message_areas SET name = ?2, description = ?3, read_level = ?4, post_level = ?5
         WHERE id = ?1",
        params![
            id,
            settings.name.trim(),
            settings.description.trim(),
            settings.read_level,
            settings.post_level
        ],
    )?;
    Ok(())
}

/// Delete an area and every message in it.
pub fn delete_area(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM message_areas WHERE id = ?1", [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db, users};

    fn setup() -> (Connection, i64, i64) {
        let conn = db::open_in_memory().unwrap();
        let new = |h: &str| users::NewUser {
            handle: h.into(),
            password: "x".into(),
            real_name: None,
            location: None,
        };
        let sysop = users::create(&conn, &new("Sysop Sam"), "h").unwrap();
        let caller = users::create(&conn, &new("Caller"), "h").unwrap();
        (conn, sysop.id, caller.id)
    }

    fn general(conn: &Connection) -> i64 {
        conn.query_row(
            "SELECT id FROM message_areas WHERE name = 'General'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn msg(area_id: i64, subject: &str) -> NewMessage {
        NewMessage {
            area_id,
            reply_to: None,
            to_name: String::new(),
            subject: subject.into(),
            body: "Hello there".into(),
        }
    }

    #[test]
    fn post_read_and_new_counts() {
        let (conn, sysop, caller) = setup();
        let area = general(&conn);
        let first = post(&conn, sysop, "Sysop Sam", 99, &msg(area, "First")).unwrap();
        post(&conn, sysop, "Sysop Sam", 99, &msg(area, "Second")).unwrap();

        // The author doesn't see their own posts as new.
        assert_eq!(total_new(&conn, sysop, 99).unwrap(), 0);
        // Another caller sees both.
        assert_eq!(total_new(&conn, caller, 10).unwrap(), 2);
        set_last_read(&conn, caller, area, first).unwrap();
        assert_eq!(total_new(&conn, caller, 10).unwrap(), 1);
        // Pointers never move backward.
        set_last_read(&conn, caller, area, 0).unwrap();
        assert_eq!(last_read(&conn, caller, area).unwrap(), first);
        catch_up(&conn, caller, 10).unwrap();
        assert_eq!(total_new(&conn, caller, 10).unwrap(), 0);

        let m = get(&conn, first).unwrap().unwrap();
        assert_eq!(m.to_name, "All");
        assert_eq!(list(&conn, area).unwrap().len(), 2);
        delete(&conn, first).unwrap();
        assert_eq!(message_ids(&conn, area).unwrap().len(), 1);
    }

    #[test]
    fn post_levels_are_enforced() {
        let (conn, _sysop, caller) = setup();
        let announcements: i64 = conn
            .query_row(
                "SELECT id FROM message_areas WHERE name = 'Sysop Announcements'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let err = post(&conn, caller, "Caller", 10, &msg(announcements, "Hi")).unwrap_err();
        assert!(err.to_string().contains("can't post"));
        assert!(post(&conn, caller, "Caller", 10, &msg(general(&conn), "")).is_err());
    }

    #[test]
    fn sysop_area_management() {
        let (conn, sysop, _) = setup();
        let settings = AreaSettings {
            conference_id: 1,
            name: "Retro Games".into(),
            description: "Old school".into(),
            read_level: 10,
            post_level: 10,
        };
        let id = add_area(&conn, &settings).unwrap();
        assert!(
            add_area(&conn, &settings).is_err(),
            "names are unique per conference"
        );
        post(&conn, sysop, "Sysop Sam", 99, &msg(id, "Hi")).unwrap();
        let mut changed = settings.clone();
        changed.post_level = 50;
        update_area(&conn, id, &changed).unwrap();
        let all = all_area_settings(&conn).unwrap();
        let found = all.iter().find(|(i, ..)| *i == id).unwrap();
        assert_eq!((found.1.post_level, found.2), (50, 1));
        delete_area(&conn, id).unwrap();
        assert!(
            get(&conn, 1).unwrap().is_none(),
            "messages go with the area"
        );
    }

    #[test]
    fn conference_listing() {
        let (conn, _, caller) = setup();
        let confs = conferences(&conn, 10).unwrap();
        assert_eq!(confs.len(), 1);
        assert_eq!(
            areas(&conn, caller, 10, Some(confs[0].id)).unwrap().len(),
            2
        );
    }
}
