#!/bin/sh
# Prepares the mounted volumes, then hands off to the server as a normal user.
#
# Runs as root only long enough to fix ownership: a fresh named volume is
# root-owned, and the server writes npc_token with 0600, so a container that
# never dropped privileges would leave files the host cannot read back.
set -eu

STATE_DIR="${STATE_DIR:-/state}"
NPC_DATA_DIR="${NPC_DATA_DIR:-/npcs}"
SEED_DIR=/opt/openmmo/seed

mkdir -p "$STATE_DIR" "$NPC_DATA_DIR"

# Seed tracked files that an empty volume would otherwise hide. `cp -Rn` never
# overwrites and exits 0 when it skips, so an operator's edits and a restored
# backup both survive while a genuine failure still stops the container.
if [ -d "$SEED_DIR/announcements" ]; then
    mkdir -p "$STATE_DIR/announcements"
    cp -Rn "$SEED_DIR/announcements/." "$STATE_DIR/announcements/"
fi

# NPC schedules and personas are tracked in git and are read *and written* by
# the map editor over REST, so they belong to the server's volume.
if [ -d "$SEED_DIR/npcs" ]; then
    cp -Rn "$SEED_DIR/npcs/." "$NPC_DATA_DIR/"
fi

chown -R 10001:10001 "$STATE_DIR" "$NPC_DATA_DIR"

exec gosu openmmo onlinerpg-server "$@"
