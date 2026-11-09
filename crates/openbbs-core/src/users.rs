//! User accounts, passwords and access levels.

use anyhow::{Context, Result, bail};
use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash};
use rusqlite::{Connection, OptionalExtension, Row, params};

/// Access level given to a brand-new account.
pub const LEVEL_NEW_USER: i64 = 10;
/// Access level that unlocks sysop-only menus and commands.
pub const LEVEL_SYSOP: i64 = 90;
/// Access level given to the very first account on a fresh board.
pub const LEVEL_FIRST_USER: i64 = 99;

pub const HANDLE_MIN: usize = 2;
pub const HANDLE_MAX: usize = 24;
pub const PASSWORD_MIN: usize = 6;

/// Handles that would confuse the login prompt.
const RESERVED_HANDLES: &[&str] = &["new", "all", "sysop", "guest", "bbs"];

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub id: i64,
    pub handle: String,
    pub real_name: Option<String>,
    pub location: Option<String>,
    pub access_level: i64,
    pub created_at: String,
    pub last_login_at: Option<String>,
    pub times_called: i64,
}

impl User {
    pub fn is_sysop(&self) -> bool {
        self.access_level >= LEVEL_SYSOP
    }

    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            handle: row.get("handle")?,
            real_name: row.get("real_name")?,
            location: row.get("location")?,
            access_level: row.get("access_level")?,
            created_at: row.get("created_at")?,
            last_login_at: row.get("last_login_at")?,
            times_called: row.get("times_called")?,
        })
    }
}

const USER_COLUMNS: &str =
    "id, handle, real_name, location, access_level, created_at, last_login_at, times_called";

/// Details collected by the new-user signup.
#[derive(Debug, Clone)]
pub struct NewUser {
    pub handle: String,
    pub password: String,
    pub real_name: Option<String>,
    pub location: Option<String>,
}

/// Check a handle's shape. Doesn't check whether it's taken.
pub fn validate_handle(handle: &str) -> Result<()> {
    let len = handle.chars().count();
    if !(HANDLE_MIN..=HANDLE_MAX).contains(&len) {
        bail!("Handles must be {HANDLE_MIN} to {HANDLE_MAX} characters.");
    }
    if handle != handle.trim() || handle.contains("  ") {
        bail!("Handles can't start or end with a space, or use double spaces.");
    }
    if !handle
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || " _-.".contains(c))
    {
        bail!("Handles may use letters, numbers, spaces, and _ - . only.");
    }
    if !handle
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
    {
        bail!("Handles must start with a letter.");
    }
    if RESERVED_HANDLES
        .iter()
        .any(|r| r.eq_ignore_ascii_case(handle))
    {
        bail!("That handle is reserved.");
    }
    Ok(())
}

pub fn validate_password(password: &str) -> Result<()> {
    if password.chars().count() < PASSWORD_MIN {
        bail!("Passwords must be at least {PASSWORD_MIN} characters.");
    }
    Ok(())
}

/// Hash a password with Argon2id. Slow on purpose; call off the async runtime.
pub fn hash_password(password: &str) -> Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("hashing password: {e}"))
}

/// Check a password against a stored hash. Slow on purpose; call off the async runtime.
pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

pub fn count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT count(*) FROM users", [], |r| r.get(0))?)
}

pub fn handle_taken(conn: &Connection, handle: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM users WHERE handle = ?1",
            [handle],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Create an account from an already-hashed password. The first account on
/// the board becomes the sysop.
pub fn create(conn: &Connection, new: &NewUser, password_hash: &str) -> Result<User> {
    validate_handle(&new.handle)?;
    if handle_taken(conn, &new.handle)? {
        bail!("That handle is already taken.");
    }
    let level = if count(conn)? == 0 {
        LEVEL_FIRST_USER
    } else {
        LEVEL_NEW_USER
    };
    conn.execute(
        "INSERT INTO users (handle, password_hash, real_name, location, access_level)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            new.handle,
            password_hash,
            new.real_name,
            new.location,
            level
        ],
    )
    .context("creating user")?;
    find_by_id(conn, conn.last_insert_rowid())?.context("user vanished after insert")
}

pub fn find_by_id(conn: &Connection, id: i64) -> Result<Option<User>> {
    Ok(conn
        .query_row(
            &format!("SELECT {USER_COLUMNS} FROM users WHERE id = ?1"),
            [id],
            User::from_row,
        )
        .optional()?)
}

/// Look up a user and their password hash by handle (case-insensitive).
pub fn find_for_login(conn: &Connection, handle: &str) -> Result<Option<(User, String)>> {
    Ok(conn
        .query_row(
            &format!("SELECT {USER_COLUMNS}, password_hash FROM users WHERE handle = ?1"),
            [handle],
            |row| Ok((User::from_row(row)?, row.get("password_hash")?)),
        )
        .optional()?)
}

/// Record a successful login and return the updated user.
pub fn record_login(conn: &Connection, id: i64) -> Result<User> {
    conn.execute(
        "UPDATE users SET times_called = times_called + 1, last_login_at = datetime('now')
         WHERE id = ?1",
        [id],
    )?;
    find_by_id(conn, id)?.context("user not found")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn new_user(handle: &str) -> NewUser {
        NewUser {
            handle: handle.into(),
            password: "secret123".into(),
            real_name: None,
            location: Some("Nowhere".into()),
        }
    }

    #[test]
    fn handle_rules() {
        assert!(validate_handle("Dark Avenger").is_ok());
        assert!(validate_handle("x").is_err());
        assert!(validate_handle("1337").is_err());
        assert!(validate_handle("NEW").is_err());
        assert!(validate_handle(" padded").is_err());
        assert!(validate_handle("bad|pipe").is_err());
    }

    #[test]
    fn passwords_hash_and_verify() {
        let hash = hash_password("hunter42").unwrap();
        assert!(verify_password("hunter42", &hash));
        assert!(!verify_password("hunter43", &hash));
        assert!(!verify_password("hunter42", "garbage"));
    }

    #[test]
    fn first_user_is_sysop_and_handles_are_unique() {
        let conn = db::open_in_memory().unwrap();
        let first = create(&conn, &new_user("Clayton"), "h").unwrap();
        assert_eq!(first.access_level, LEVEL_FIRST_USER);
        assert!(first.is_sysop());
        let second = create(&conn, &new_user("Caller"), "h").unwrap();
        assert_eq!(second.access_level, LEVEL_NEW_USER);
        assert!(create(&conn, &new_user("clayton"), "h").is_err());

        let (found, hash) = find_for_login(&conn, "CLAYTON").unwrap().unwrap();
        assert_eq!(found.id, first.id);
        assert_eq!(hash, "h");
        let after = record_login(&conn, first.id).unwrap();
        assert_eq!(after.times_called, 1);
        assert!(after.last_login_at.is_some());
    }
}
