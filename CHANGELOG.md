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
