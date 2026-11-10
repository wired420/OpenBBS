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

5. Configure the door for each node with its own setup program, run the
   same way the BBS runs it so the DOS drive letters match:

   ```sh
   sudo -u openbbs env HOME=/opt/openbbs/data/dosemu/node1 TERM=xterm \
       LANG=C.UTF-8 sh -c "stty rows 25 cols 80; \
       dosemu -t -K /opt/openbbs/doors/lord -E 'LORDCFG.EXE'"
   ```

   DOS screens are 80x25, so `stty` sets that size and `LANG` lets the box
   lines draw. Keep the window at least that big, and resize it (or run
   `reset`) afterwards to get your own size back. Without these, full-screen
   setup programs come out jumbled.

   For node N, point the door at `NODE<N>\DOOR.SYS` (or `DOOR32.SYS`) and
   set communications to FOSSIL on COM1.

   Watch out for node 0: many doors (LORD among them) treat it as local
   play, so a door that works when you start it by hand can still quit at
   once for callers if node 1 and up aren't configured.
6. If a door quits straight away, its complaint is on its local screen,
   which callers never see. Run the command above with your batch file
   instead of the setup program, add `pause` as the batch file's last line,
   and read the message.

### How it works

DOSEMU2 runs with a virtual COM1 connected to its terminal. Its virtual COM
port needs a real terminal, but OpenBBS connects doors with pipes, so the
launcher runs DOSEMU2 under `script` to give it a pseudo-terminal. The door's
local screen isn't sent to the caller; only what it writes to COM1 is.

### Console doors (made for DOORWAY or GATEWAY)

Some DOS games have no modem or FOSSIL code at all. They draw on the PC's
screen and read its keyboard, and BBSes ran them under DOORWAY or GATEWAY
to pass both to the caller. Dope Wars is one. Under the normal launcher
they show a blinking cursor and nothing else.

Add `"console"` as a fifth argument and set `encoding = "utf8"`:

```toml
[[door]]
id = "dopewars"
name = "Dope Wars"
command = "/opt/openbbs/doors/dosemu-door.sh"
args = ["/opt/openbbs/doors/dopewars", "OPENBBS.BAT", "{node}", "{dropdir}", "console"]
encoding = "utf8"
```

In this mode DOSEMU2 has no COM port and sends its 80x25 screen to the
caller, so you don't need DOORWAY. Callers see a little DOSEMU2 start-up
text before the game clears the screen.

### Things to know

- **Paths in a door's own config must be DOS paths.** Inside DOSEMU2 the
  door's folder is a DOS drive (usually `G:`), so a door's setup program
  needs `G:\NODE1\DOOR.SYS`, not `/opt/openbbs/...`.
- **Check that the door's installer finished.** Some installers unzip
  their screens with PKUNZIP and quietly skip it under DOSEMU2. A door with
  no screens shows a blank screen. Look for leftover `.ZIP` files in the
  door's subfolders and unzip them on Linux with `unzip`.
- **Exit codes:** some doors (Exitilus, for one) always exit with code 1,
  so the log says "door finished with an error" after a normal game.

- **Wide windows**: classic doors draw for 80 columns and rely on the
  terminal wrapping there. OpenBBS wraps CP437 door output at column 80 so
  the art lines up in any window size. Set `columns` in doors.toml to
  change the width, or `columns = 0` to turn this off.

- **Give DOSEMU2 KVM access** with `sudo usermod -aG kvm openbbs`, then
  restart the service. Without it DOSEMU2 emulates the CPU, which is much
  slower.

- **Run OpenBBS as a normal user.** When DOSEMU2 runs as root it prints a
  warning that callers would see.

- **Each node gets its own DOSEMU2 home** (`data/dosemu/node<n>`), so two
  callers can play at once. Doors with a shared data file need their own
  multi-node support, as they did on a classic BBS.
- **One caller at a time**: for a door that isn't multi-node safe (Trade
  Wars 2002 v1.03, for one), add `max_callers = 1` to its doors.toml entry.
  A second caller is told who is playing and to try again soon, and the
  Door Games list marks the door "(in use)". The seat is freed when the door
  exits or the caller hangs up. Leave it out, or set 0, for no limit.
- **The systemd service** in `dist/openbbs.service` limits writes to
  `/opt/openbbs`. If doors live elsewhere, add their folder to
  `ReadWritePaths`.
- **Windows hosting**: DOSEMU2 is Linux only. DOS doors on a Windows host
  would need DOSBox-X with a socket connection, which OpenBBS does not
  support yet.
- **Registration keys**: only use doors whose authors released them freely,
  or released registration keys or key generators themselves.
