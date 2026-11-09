# OpenBBS

A 100% open source bulletin board system, written in Rust, for the modern
internet. Callers connect over SSH today, with a web client planned once SSH is
solid.

OpenBBS aims for the classic BBS experience — message boards, private mail,
file areas, door games, ANSI art menus and a sysop console — with menus as easy
to set up as Synchronet's and screens that look better.

## Status

Early development, working toward **v0.0.1 Beta**.

## Building

OpenBBS builds on Linux (including WSL) and Windows with a stable Rust
toolchain (<https://rustup.rs>).

```sh
# Release build. Bumps the patch version and stamps CHANGELOG.md first.
cargo xtask build

# Day-to-day development build (no version bump).
cargo build
cargo test
```

Other version commands (the patch bump is automatic; minor and major are bumped
by hand when the project owner decides):

```sh
cargo xtask version          # print the current version
cargo xtask bump minor       # 0.0.7 -> 0.1.0
cargo xtask bump major       # 0.4.2 -> 1.0.0
```

## Running

```sh
cargo run -- init            # write openbbs.toml, the database, host key and default theme
cargo run -- run             # start the board (SSH on port 2222 by default)
```

Then call it from any SSH client:

```sh
ssh -p 2222 new@localhost        # type NEW at the prompt to sign up
ssh -p 2222 yourhandle@localhost # skips straight to the password prompt
```

The first account created becomes the sysop. `openbbs check` validates the
config and database without starting the server.

## Customizing

Everything callers see lives in `themes/default/`:

- `menus/*.toml`: menus, their hotkeys, actions and access levels. Comments in
  `menus/main.toml` list every action. Edits apply immediately.
- `screens/*.ans` / `*.txt`: ANSI art and text screens. Classic CP437 `.ans`
  files work as-is. Text can use pipe color codes (`|15` bright white, `|01`
  blue, ...) and tokens like `@USER@`, `@BBSNAME@` and `@NODE@`.
- `theme.toml`: colors for the menus OpenBBS draws itself.

## License

MIT — see [LICENSE](LICENSE). Use it however you like; just keep the copyright
and license notice intact.
