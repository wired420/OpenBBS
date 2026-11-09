# Changelog

All notable changes to OpenBBS are recorded here. Each release build
(`cargo xtask build`) bumps the patch number and turns the **Unreleased**
section into that version's entry, so add new notes under **Unreleased** as you
go.

## [Unreleased]

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
