# Changelog

All notable changes to OpenBBS are recorded here. Each release build
(`cargo xtask build`) bumps the patch number and turns the **Unreleased**
section into that version's entry, so add new notes under **Unreleased** as you
go.

## [Unreleased]

### Added
- Console mode for the DOSEMU2 launcher (`"console"` as its fifth
  argument) runs DOS games made for DOORWAY or GATEWAY, which draw on the
  screen instead of using a COM port. Dope Wars runs this way.
- `scripts/publish-wiki.sh` publishes the README, setup guide and door guide
  to the GitHub wiki.
- Door output wraps at 80 columns, as on a classic terminal, so DOS door art
  lines up in wider windows. Set `columns` per door in doors.toml (0 turns
  it off). On by default for CP437 doors.
- `max_callers` in doors.toml limits how many callers can be in a door at
  once. Set `max_callers = 1` for doors that only handle one player at a
  time. Callers who find it full are told who is playing, and the Door
  Games list shows "(in use)".

### Fixed
- A caller hanging up in a DOS door left DOSEMU2 running. OpenBBS stopped
  the launcher script but not what it started. Doors now run in their own
  process group, and the whole group is stopped when the door ends.
- Console-mode DOS doors showed a blank screen under the systemd service,
  which sets no terminal type. The launcher now sets `TERM=xterm` for them.
- Doors that clear the screen with a form feed (byte 12) now clear it,
  instead of showing a ♀ character. Bell and backspace also pass through.
- The setup guide builds the server with `cargo build --release`.
  `cargo xtask build` there bumped the version locally and blocked `git pull`.
- Doors with `dir` set in doors.toml quit at once: `{dropdir}`, `{door32}`
  and `{doorsys}` were relative to OpenBBS's folder. They are now full paths.
- The DOSEMU2 launcher tells DOS doors the caller is on COM1. DOOR.SYS said
  `COM0:` (local, no caller), so doors such as LORD started in local mode
  and quit.
- DOS doors run under the systemd service. The DOSEMU2 launcher ran DOSEMU2
  through the `openbbs` account's login shell, which is `nologin`, so every
  door quit as soon as it started.
- A door that fails now leaves a reason in the server log: its error output
  and exit status are logged instead of thrown away.
- The DOSEMU2 launcher checks that `dosemu`, `script` and the door's batch
  file exist, and says which one is missing. When DOSEMU2 fails, its error
  output and the errors from its boot log reach the server log.

## [0.0.1] - 2026-11-09

### Added
- Rust workspace that builds on Linux (including WSL) and Windows.
- MIT license, README and this changelog.
- `cargo xtask build`, which bumps the patch version and stamps this changelog
  before a release build; `cargo xtask bump minor|major` for manual bumps.
- Config file (`openbbs.toml`) with defaults, and `openbbs init` to create it.
- SQLite database with versioned migrations, and the first tables: users,
  message areas, messages, private mail, file areas and files.
- `openbbs check` to validate the config and database.
- GitHub Actions CI building and testing on Linux and Windows.
- SSH server (`openbbs run`) in pure Rust. Creates its host key on first run,
  supports several callers at once (`bbs.max_nodes`) and turns callers away
  politely when the board is full.
- Login with handle and password, and new user signup. Passwords are hashed
  with Argon2id. The first account becomes the sysop. Connecting as
  `ssh yourhandle@host` skips straight to the password prompt.
- Access levels: new users get 10, sysop items need 90.
- Menu system driven by TOML files in the theme folder. Lightbar menus (arrow
  keys + Enter) and classic hotkey menus, with optional custom ANSI screens.
  Menu edits apply right away, without a restart.
- Default theme with an ANSI welcome logo, main menu, sysop menu, who's
  online, user list and account page. `openbbs init` copies it to
  `themes/default` for editing.
- CP437 to UTF-8 conversion so classic `.ans` art displays correctly, pipe
  color codes (`|00`-`|23`) and `@TOKENS@` such as `@USER@` and `@BBSNAME@`.
- Idle callers are disconnected after `bbs.idle_minutes`.
- Message boards: area list with total and new counts, message lists,
  a reader with next/previous/reply, posting with To and Subject, quoted
  replies, per-user read pointers, a new message scan across every area,
  and "catch up". Areas can limit who reads and who posts by access level.
  Sysops and authors can delete messages.
- Private mail: inbox with unread markers, reading, sending, replying with
  quotes, and deleting. Callers are told about new mail and new posts at
  login and can jump straight to their mail.
- A line editor for messages and mail with automatic word wrap and `/S`
  save, `/A` abort, `/L` list and `/D` delete-line commands.
- Scrolling lightbar lists (arrows, Page Up/Down, Home/End) used for areas,
  messages and mail.
- New menu actions: `messages`, `newscan`, `mail` and `sendmail`.
- File areas over SFTP: log in with `sftp yourhandle@host` (handle and BBS
  password) to browse areas as folders, download, and upload where your
  access level allows. Uploads land in a temporary file and only appear once
  complete; uploads over `files.max_upload_mb` are refused. Downloads are
  counted.
- File area screens: area and file lists with sizes, dates and download
  counts, file details with ready-to-paste `sftp` commands, and describe or
  delete for the uploader and sysop.
- Files the sysop copies into an area folder are picked up at startup or with
  Rescan File Areas on the sysop menu.
- `ssh yourhandle@host` now checks your BBS password at the SSH level and
  goes straight to the main menu. Other user names (like `new` or `bbs`)
  still reach the login prompt as before.
- New config settings: `ssh.public_host` (shown in transfer instructions) and
  `files.max_upload_mb`.
- Door games: list doors in `doors/doors.toml` (an example is written by
  `openbbs init`). OpenBBS writes DOOR.SYS and DOOR32.SYS for the caller's
  node, runs the door, and connects its input and output to the caller, with
  CP437 output converted for modern terminals. Doors can require an access
  level. Menu actions `doors` and `door:ID`.
- Sysop console on the sysop menu: manage users (access level, password
  reset, delete), add/edit/delete message areas and file areas, and rescan
  file folders.
- `openbbs passwd HANDLE` and `openbbs set-level HANDLE LEVEL` for fixing
  accounts from the command line, even when the sysop is locked out.
- Command-line errors print one clear line instead of a stack trace.
- Setup guide (`docs/SETUP.md`) for development in WSL and hosting on a
  Linux VPS, with a systemd service (`dist/openbbs.service`), firewall,
  backup and upgrade steps.
- Door guide (`docs/DOORS.md`) and a DOSEMU2 launcher
  (`dist/doors/dosemu-door.sh`) for classic DOS doors.
