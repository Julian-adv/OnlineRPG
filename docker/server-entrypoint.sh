#!/bin/sh
# Prepares the mounted volumes, then hands off to the server as a normal user.
#
# Runs as root only long enough to fix ownership: a fresh named volume is
# root-owned, and the server writes npc_token with 0600, so a container that
# never dropped privileges would leave files the host cannot read back.
set -eu

. /usr/local/bin/entrypoint-lib.sh

STATE_DIR="${STATE_DIR:-/state}"
NPC_DATA_DIR="${NPC_DATA_DIR:-/npcs}"
SEED_DIR=/opt/openmmo/seed

mkdir -p "$STATE_DIR" "$NPC_DATA_DIR"

seed_into "$SEED_DIR/announcements" "$STATE_DIR/announcements"
# Banned character names: an operator may edit the copy in the volume.
seed_into "$SEED_DIR/state" "$STATE_DIR"
# NPC schedules and personas are tracked in git and are read *and written* by
# the map editor over REST, so they belong to the server's volume.
seed_into "$SEED_DIR/npcs" "$NPC_DATA_DIR"

# The seeded trees are small; the state volume (housing, DB) is not.
chown -R openmmo:openmmo "$STATE_DIR/announcements" "$STATE_DIR/banned_names.txt" "$NPC_DATA_DIR"
own_volume "$STATE_DIR"

exec gosu openmmo onlinerpg-server "$@"
