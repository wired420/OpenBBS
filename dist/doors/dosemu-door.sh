#!/bin/sh
# Run a DOS door game under DOSEMU2 for OpenBBS.
#
# Tested with DOSEMU2 2.0pre9 on Ubuntu 24.04. See docs/DOORS.md.
#
# doors.toml entry:
#   [[door]]
#   id = "lord"
#   name = "Legend of the Red Dragon"
#   command = "/opt/openbbs/doors/dosemu-door.sh"
#   args = ["/opt/doors/lord", "START.BAT", "{node}", "{dropdir}"]
#
# Arguments:
#   1  the door's folder on Linux (becomes a DOS drive)
#   2  the batch file in that folder that starts the door
#   3  node number
#   4  folder holding DOOR.SYS and DOOR32.SYS for this node
#
# The batch file gets the node number as %1. The drop files are copied to
# NODE<n>\ inside the door's folder, so from the door's folder a door reads NODE1\DOOR.SYS.
set -eu

# Errors printed here end up in the OpenBBS log (journalctl -u openbbs).
for tool in dosemu script realpath; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "dosemu-door.sh: '$tool' is not installed or not on PATH" >&2
        exit 1
    fi
done
if [ ! -f "$1/$2" ]; then
    echo "dosemu-door.sh: $1/$2 not found (check the door's args)" >&2
    exit 1
fi

door_dir=$(realpath "$1")
batch=$2
node=$3
drop_dir=$(realpath "$4")

node_dir="$door_dir/NODE$node"
mkdir -p "$node_dir"
# OpenBBS writes drop files for doors that use standard input and output
# (DOOR.SYS says COM0:, which doors read as "local, no caller"). Under
# DOSEMU2 the caller is on COM1, so say so. DOS programs also expect CR LF
# line endings.
if [ -f "$drop_dir/DOOR.SYS" ]; then
    sed -e '1s/.*/COM1:/' -e 's/\r*$/\r/' "$drop_dir/DOOR.SYS" > "$node_dir/DOOR.SYS"
fi
if [ -f "$drop_dir/DOOR32.SYS" ]; then
    # Line 1: comm type 1 (serial). Line 2: port handle 0 (COM1).
    sed -e '1s/.*/1/' -e '2s/.*/0/' -e 's/\r*$/\r/' "$drop_dir/DOOR32.SYS" > "$node_dir/DOOR32.SYS"
fi

# Each node gets its own DOSEMU2 home, so two callers in the same door
# don't share a C: drive or boot log.
export HOME="${DOSEMU_HOME_BASE:-/opt/openbbs/data/dosemu}/node$node"
mkdir -p "$HOME"

# DOSEMU2's virtual COM port needs a terminal, and OpenBBS connects doors
# with pipes, so `script` (from util-linux) gives it a pseudo-terminal.
# Inside DOS, COM1 is the caller: FOSSIL doors talk to them through it, and
# the door's local screen is not shown.
#   -t   terminal mode, no window
#   -K   make the door's folder a DOS drive and switch to it
#   -E   run this DOS command, then exit
# `script` runs the command with $SHELL, falling back to the account's login
# shell. The openbbs service account's shell is nologin, which refuses to
# run anything, so name a real shell.
export SHELL=/bin/sh
# DOSEMU2's own errors go to a log file, because inside `script` they would
# be sent to the caller. If the door fails, the end of that log and of
# DOSEMU2's boot log are printed to stderr, which OpenBBS logs.
log="$HOME/dosemu-errors.log"
status=0
script -qefc "dosemu -t -quiet -I 'serial { virtual com 1 }' \
    -K '$door_dir' -E '$batch $node' 2>'$log'" /dev/null || status=$?
if [ "$status" -ne 0 ]; then
    echo "dosemu-door.sh: DOSEMU2 exited with status $status" >&2
    [ -s "$log" ] && tail -n 20 "$log" >&2
    [ -f "$HOME/.dosemu/boot.log" ] && grep -iE 'error|fail|denied|kvm' "$HOME/.dosemu/boot.log" | tail -n 20 >&2
fi
exit "$status"
