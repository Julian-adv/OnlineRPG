#!/bin/sh
# Fills an empty terrain volume, then exits. Compose runs this as a one-shot
# `terrain-init` service that the server waits on.
#
# Timing note: a bake costs ~3 minutes almost regardless of region range,
# because erosion and road generation simulate the whole 32x32 km world before
# any tiles are written. The range only controls disk usage (~65 MB/region).
set -eu

. /usr/local/bin/entrypoint-lib.sh

TERRAIN_DIR="${TERRAIN_DIR:-/terrain}"
REGION_MIN="${TERRAIN_REGION_MIN:--2}"
REGION_MAX="${TERRAIN_REGION_MAX:-1}"
SEED_ZONES=/opt/openmmo/seed/terrain/zones

mkdir -p "$TERRAIN_DIR"

seed_zones() {
    # Zone files carry town no-spawn areas and monster spawn rectangles. They
    # are tracked in git and terrain-gen never writes them, so without this the
    # world bakes fine but monsters spawn inside towns.
    seed_into "$SEED_ZONES" "$TERRAIN_DIR/zones"
    chown -R openmmo:openmmo "$TERRAIN_DIR/zones"
}

# worldgen.json is written last, so its presence means a bake ran to completion.
# A half-finished bake leaves tiles but no marker and is redone on next start.
if [ -f "$TERRAIN_DIR/worldgen.json" ] && [ -z "${TERRAIN_FORCE_BAKE:-}" ]; then
    echo "terrain-init: worldgen.json present, skipping bake"
    seed_zones
    own_volume "$TERRAIN_DIR"
    exit 0
fi

echo "terrain-init: baking regions ${REGION_MIN}..${REGION_MAX} on both axes"
echo "terrain-init: this takes about 3 minutes; global simulation dominates"

terrain-gen bake \
    --out "$TERRAIN_DIR" \
    --region-x-min "$REGION_MIN" \
    --region-x-max "$REGION_MAX" \
    --region-z-min "$REGION_MIN" \
    --region-z-max "$REGION_MAX"

seed_zones
# The bake ran as root and rewrote the tree, so own_volume's fast path
# would wrongly skip.
chown -R openmmo:openmmo "$TERRAIN_DIR"
echo "terrain-init: done"
