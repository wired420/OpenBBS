# Setting up OpenBBS

This guide covers two setups:

1. **Development in WSL** (or any Linux or Windows machine) for building and
   trying the board.
2. **Hosting on a Linux VPS** so callers can reach it around the clock.

## 1. Development in WSL

Install the tools once. On Fedora in WSL:

```sh
sudo dnf install -y gcc git openssh-clients
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

On Ubuntu or Debian, the first line is instead
`sudo apt update && sudo apt install -y build-essential git openssh-client`.

Get the code and start a test board:

```sh
git clone https://github.com/wired420/OpenBBS.git
cd OpenBBS
cargo run -- init     # creates openbbs.toml, data/, themes/, doors/
cargo run -- run      # listens on port 2222
```

From another WSL terminal, or from Windows Terminal or PowerShell (WSL
forwards `localhost`):

```sh
ssh -p 2222 new@localhost      # type NEW to sign up; the first account is the sysop
```

The board's files (`openbbs.toml`, `data/`, `themes/`, `doors/`) live in the
folder you ran `init` from. They're ignored by git, so you can experiment
freely. Delete them and run `init` again for a fresh board.

### Building on Windows directly

Install Rust from <https://rustup.rs> (it offers the Visual Studio C++ build
tools, which are needed). Then the same `cargo run -- init` and
`cargo run -- run` commands work in PowerShell.

## 2. Hosting on a Linux VPS

These steps work on Ubuntu, Debian and Fedora with a user who has `sudo`.
Where the commands differ, both are shown.

### Build

Build on the VPS itself (or build in WSL and copy the binary over; both are
x86-64 Linux):

```sh
sudo apt update && sudo apt install -y build-essential git   # Ubuntu/Debian
sudo dnf install -y gcc git                                  # Fedora
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
git clone https://github.com/wired420/OpenBBS.git
cd OpenBBS
cargo build --release
```

Use plain `cargo build --release` on the server. `cargo xtask build` is for
development: it bumps the version and edits CHANGELOG.md and Cargo.toml,
which would then block `git pull` on the server.

The program is now at `target/release/openbbs`.

### Install

Run the board as its own unprivileged user in `/opt/openbbs`:

```sh
sudo useradd --system --home /opt/openbbs --shell /usr/sbin/nologin openbbs
sudo mkdir -p /opt/openbbs
sudo cp target/release/openbbs /opt/openbbs/
sudo chown -R openbbs:openbbs /opt/openbbs
cd /opt/openbbs
sudo -u openbbs ./openbbs init
```

Edit `/opt/openbbs/openbbs.toml`:

- `bbs.name` and `bbs.sysop`: your board's name and your handle.
- `ssh.public_host`: the address callers use, like `bbs.example.com`. It
  appears in file transfer instructions.
- `ssh.listen`: leave it as `0.0.0.0:2222` unless you follow
  "Using port 22" below.

Check it, then install the service:

```sh
sudo -u openbbs ./openbbs check
sudo cp ~/OpenBBS/dist/openbbs.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now openbbs
sudo systemctl status openbbs        # should say "active (running)"
journalctl -u openbbs -f             # live log: logins, uploads, errors
```

### Firewall

Open the BBS port (keep your admin SSH port open too). On Ubuntu or Debian
with ufw:

```sh
sudo ufw allow OpenSSH      # your own admin login, port 22
sudo ufw allow 2222/tcp     # the BBS
sudo ufw enable
```

On Fedora with firewalld (SSH on port 22 is already allowed):

```sh
sudo firewall-cmd --permanent --add-port=2222/tcp
sudo firewall-cmd --reload
```

Many VPS providers also have a firewall in their web control panel; open the
same port there.

Then call in from home and sign up first, so your account becomes the sysop:

```sh
ssh -p 2222 new@bbs.example.com
```

### Using port 22

Callers can then type `ssh bbs.example.com` with no port. Port 22 is usually
taken by the server's own SSH, so move that first, carefully:

1. Edit `/etc/ssh/sshd_config` and set `Port 2200` (any free port).
2. Open the new port (`sudo ufw allow 2200/tcp`, or on Fedora
   `sudo firewall-cmd --permanent --add-port=2200/tcp && sudo firewall-cmd --reload`
   and `sudo semanage port -a -t ssh_port_t -p tcp 2200` for SELinux), then
   restart SSH (`sudo systemctl restart ssh`, or `sshd` on Fedora).
3. **Before closing your current session**, open a new terminal and check
   that `ssh -p 2200 you@bbs.example.com` works. If it doesn't, fix it from
   the session you still have open.
4. In `/opt/openbbs/openbbs.toml`, set `listen = "0.0.0.0:22"`.
5. In `/etc/systemd/system/openbbs.service`, uncomment
   `AmbientCapabilities=CAP_NET_BIND_SERVICE`.
6. Open port 22 for the BBS if your firewall needs it, then
   `sudo systemctl daemon-reload`,
   `sudo systemctl restart openbbs`.

### Backups

Everything the board knows is in `/opt/openbbs`:

- `data/openbbs.db`: users, messages, mail, file listings
- `data/ssh_host_ed25519_key`: the board's identity. Keep it, or callers
  get a "host key changed" warning.
- `data/files/`: the file areas
- `themes/`, `doors/`, `openbbs.toml`: your customizations

A nightly copy of the whole folder is enough. SQLite is safest to copy while
the board is stopped, or use `sqlite3 data/openbbs.db ".backup backup.db"`.

### Upgrading

```sh
cd ~/OpenBBS
git pull
cargo build --release
sudo systemctl stop openbbs
sudo cp target/release/openbbs /opt/openbbs/
sudo systemctl start openbbs
```

Database changes are applied automatically when the new version starts.
Your theme files are never overwritten; compare them with
`crates/openbbs/theme-default/` after an upgrade to pick up new menu items.

### Locked out?

```sh
cd /opt/openbbs
sudo -u openbbs ./openbbs passwd YourHandle      # set a new password
sudo -u openbbs ./openbbs set-level YourHandle 99 # make an account sysop
```
