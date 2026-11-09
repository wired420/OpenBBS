-- Users and access levels.
CREATE TABLE users (
    id            INTEGER PRIMARY KEY,
    handle        TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT NOT NULL,
    real_name     TEXT,
    location      TEXT,
    email         TEXT,
    access_level  INTEGER NOT NULL DEFAULT 10,
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    last_login_at TEXT,
    times_called  INTEGER NOT NULL DEFAULT 0
);

-- Message boards: conferences hold areas, areas hold messages.
CREATE TABLE message_conferences (
    id           INTEGER PRIMARY KEY,
    name         TEXT NOT NULL UNIQUE,
    description  TEXT NOT NULL DEFAULT '',
    read_level   INTEGER NOT NULL DEFAULT 10,
    sort_order   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE message_areas (
    id            INTEGER PRIMARY KEY,
    conference_id INTEGER NOT NULL REFERENCES message_conferences(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    description   TEXT NOT NULL DEFAULT '',
    read_level    INTEGER NOT NULL DEFAULT 10,
    post_level    INTEGER NOT NULL DEFAULT 10,
    -- Set when the area is linked to a message network later (e.g. an echo tag).
    network_tag   TEXT,
    sort_order    INTEGER NOT NULL DEFAULT 0,
    UNIQUE (conference_id, name)
);

CREATE TABLE messages (
    id         INTEGER PRIMARY KEY,
    area_id    INTEGER NOT NULL REFERENCES message_areas(id) ON DELETE CASCADE,
    reply_to   INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    author_id  INTEGER REFERENCES users(id) ON DELETE SET NULL,
    -- Kept so messages from network peers (or deleted users) still show a name.
    from_name  TEXT NOT NULL,
    to_name    TEXT NOT NULL DEFAULT 'All',
    subject    TEXT NOT NULL,
    body       TEXT NOT NULL,
    posted_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX messages_area ON messages(area_id, id);

-- Tracks how far each user has read in each area.
CREATE TABLE message_lastread (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    area_id INTEGER NOT NULL REFERENCES message_areas(id) ON DELETE CASCADE,
    last_id INTEGER NOT NULL,
    PRIMARY KEY (user_id, area_id)
);

-- Private user-to-user mail.
CREATE TABLE mail (
    id        INTEGER PRIMARY KEY,
    from_id   INTEGER REFERENCES users(id) ON DELETE SET NULL,
    to_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    subject   TEXT NOT NULL,
    body      TEXT NOT NULL,
    sent_at   TEXT NOT NULL DEFAULT (datetime('now')),
    read_at   TEXT
);
CREATE INDEX mail_to ON mail(to_id, id);

-- File areas and the files in them.
CREATE TABLE file_areas (
    id             INTEGER PRIMARY KEY,
    name           TEXT NOT NULL UNIQUE,
    description    TEXT NOT NULL DEFAULT '',
    -- Folder under paths.files.
    folder         TEXT NOT NULL UNIQUE,
    download_level INTEGER NOT NULL DEFAULT 10,
    upload_level   INTEGER NOT NULL DEFAULT 10,
    sort_order     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE files (
    id           INTEGER PRIMARY KEY,
    area_id      INTEGER NOT NULL REFERENCES file_areas(id) ON DELETE CASCADE,
    filename     TEXT NOT NULL,
    size_bytes   INTEGER NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    uploader_id  INTEGER REFERENCES users(id) ON DELETE SET NULL,
    uploaded_at  TEXT NOT NULL DEFAULT (datetime('now')),
    downloads    INTEGER NOT NULL DEFAULT 0,
    UNIQUE (area_id, filename)
);

-- Starter content so a fresh board isn't empty.
INSERT INTO message_conferences (id, name, description) VALUES
    (1, 'Main', 'Local discussion');
INSERT INTO message_areas (conference_id, name, description) VALUES
    (1, 'General', 'Anything goes'),
    (1, 'Sysop Announcements', 'News from the sysop');
UPDATE message_areas SET post_level = 90 WHERE name = 'Sysop Announcements';
INSERT INTO file_areas (name, description, folder) VALUES
    ('Uploads', 'New uploads awaiting review', 'uploads');
