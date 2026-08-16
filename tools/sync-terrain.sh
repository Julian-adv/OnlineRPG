#!/usr/bin/env bash
# Push generated terrain data to the deploy host.
#
# Only data/terrain/ is synced. Live server state (game_data.db, housing/,
# npc_token) stays untouched — those are authoritative on the remote.
set -euo pipefail

REMOTE=${REMOTE:-prod}
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
SRC="$REPO/data/terrain/"
DST="$REMOTE:work/OnlineRPG/data/terrain/"

apply=0
opts=(-a --partial --human-readable --stats --exclude=worldgen_preview/)

for arg in "$@"; do
  case $arg in
    --apply) apply=1 ;;
    # Compare by content, not mtime. Needed when a rebake rewrote every file
    # but only some changed — trades local+remote disk reads for network.
    --checksum) opts+=(--checksum) ;;
    # Drop remote tiles no longer produced locally. Verify with a dry run first.
    --delete) opts+=(--delete) ;;
    *) echo "usage: ${0##*/} [--apply] [--checksum] [--delete]" >&2; exit 2 ;;
  esac
done

[[ -d $SRC ]] || { echo "no terrain data at $SRC" >&2; exit 1; }

if ((apply)); then
  opts+=(--info=progress2)
else
  opts+=(--dry-run)
fi

echo "$SRC -> $DST"
rsync "${opts[@]}" "$SRC" "$DST"

((apply)) || echo $'\ndry run only — rerun with --apply to transfer'
