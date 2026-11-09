# Door games

A door is an outside program that callers play from inside the BBS. OpenBBS
writes the caller's details to drop files (`DOOR.SYS` and `DOOR32.SYS`),
starts the program, and connects its input and output to the caller. Doors
are listed in `doors/doors.toml`, which `openbbs init` creates with comments
explaining each setting.

## Native doors

Any program that reads from standard input and writes to standard output
works as a door. That covers Linux doors, scripts, and modern doors with a
"local" or "stdio" mode:

```toml
[[door]]
id = "hello"
name = "Hello World"
command = "/opt/doors/hello/hello.sh"
args = ["{door32}"]
encoding = "utf8"     # leave out for classic CP437 doors
```

## Classic DOS doors with DOSEMU2

Most classic doors are DOS programs that talk to the caller through a FOSSIL
driver on a COM port. On Linux, DOSEMU2 can run them and connect COM1 to
standard input and output, which plugs straight into OpenBBS.

This was tested end to end with DOSEMU2 2.0pre9 on Ubuntu 24.04: a caller on
SSH ran a FOSSIL test door that read DOOR32.SYS and talked to them through
COM1. Real doors still need their own setup, so try each one before opening
it to callers.

### Install DOSEMU2

On Fedora (including WSL), from DOSEMU2's own COPR repository (this route
wasn't tried here):

```sh
sudo dnf copr enable stsp/dosemu2
sudo dnf install -y dosemu2
```

On Ubuntu, from DOSEMU2's PPA:

```sh
sudo add-apt-repository ppa:dosemu2/ppa
sudo apt update
sudo apt install -y dosemu2
```

Check it works by running `dosemu -t` and typing `exitemu` to leave. The
launcher also uses `script`, which comes with util-linux on every major
distro.

### Set up a door

1. Put the door in its own folder, for example `/opt/doors/lord`, owned by
   the `openbbs` user.
2. Write a batch file there that starts the door. The node number arrives as
   `%1`, and the drop files are copied to `NODE<n>\` in the door's folder:

   ```bat
   @echo off
   rem Load the FOSSIL driver that ships with DOSEMU2, then start the door.
   fossil.com
   lord.exe /N%1 /PNODE%1
   ```

   `fossil.com` is built into DOSEMU2 and already on the DOS path. Each door has its own command line options for the node and drop file
   path, so check its documentation.
3. Copy the launcher into place:

   ```sh
   sudo install -m 755 dist/doors/dosemu-door.sh /opt/openbbs/doors/
   ```

4. Add the door to `doors/doors.toml`:

   ```toml
   [[door]]
   id = "lord"
   name = "Legend of the Red Dragon"
   command = "/opt/openbbs/doors/dosemu-door.sh"
   args = ["/opt/doors/lord", "START.BAT", "{node}", "{dropdir}"]
   minutes = 30
   ```

5. Run the door once from the sysop's own login, and set it up with the
   door's own config program if it has one (run that locally with
   `dosemu` in the door's folder).

### How it works

DOSEMU2 runs with a virtual COM1 connected to its terminal. Its virtual COM
port needs a real terminal, but OpenBBS connects doors with pipes, so the
launcher runs DOSEMU2 under `script` to give it a pseudo-terminal. The door's
local screen isn't sent to the caller; only what it writes to COM1 is.

### Things to know

- **Run OpenBBS as a normal user.** When DOSEMU2 runs as root it prints a
  warning that callers would see.

- **Each node gets its own DOSEMU2 home** (`data/dosemu/node<n>`), so two
  callers can play at once. Doors with a shared data file need their own
  multi-node support, as they did on a classic BBS.
- **The systemd service** in `dist/openbbs.service` limits writes to
  `/opt/openbbs`. If doors live elsewhere, add their folder to
  `ReadWritePaths`.
- **Windows hosting**: DOSEMU2 is Linux only. DOS doors on a Windows host
  would need DOSBox-X with a socket connection, which OpenBBS does not
  support yet.
- **Registration keys**: only use doors whose authors released them freely,
  or released registration keys or key generators themselves.
