# syntax=docker/dockerfile:1

# Builds the game server and the terrain baker from one workspace compile.
# Both binaries ship in the same image: compose runs the baker as a separate
# `terrain-init` service via a different entrypoint, which keeps the published
# image count at three while still isolating bake failures from server startup.

FROM rust:1-bookworm AS builder
WORKDIR /build

# build.rs turns data-src/*.csv into data/*.json, which the binaries then embed
# with include_str!. The generated JSON never needs to reach the runtime image.
#
# data/ also holds three *tracked* JSON files that shared/ embeds directly
# (furniture_footprints, material-impact-sounds, monster_attack_clips), so the
# directory is a build input even though its generated members are gitignored.
COPY Cargo.toml Cargo.lock clippy.toml ./
COPY data/ data/
COPY shared/ shared/
COPY terrain/ terrain/
COPY server/ server/
COPY tools/terrain-gen/ tools/terrain-gen/
# terrain-gen embeds the furniture catalog when placing objects. It is a small
# tracked JSON that happens to live beside the LFS models, so copy just the file
# rather than dragging client/public into the build context.
COPY client/public/models/objects/catalog.json client/public/models/objects/
COPY tools/cargo-build-data.rs tools/
COPY data-src/ data-src/

# agent-client is a workspace member, so cargo needs its manifest to resolve the
# workspace at all. Stub the sources instead of copying them: cargo does not run
# build scripts for members outside the -p selection, and a stub keeps this
# image's cache from busting on unrelated agent-client edits.
COPY agent-client/Cargo.toml agent-client/
RUN mkdir -p agent-client/src \
    && echo 'fn main() {}' > agent-client/src/main.rs \
    && echo 'fn main() {}' > agent-client/build.rs

RUN cargo build --release --locked -p onlinerpg-server -p terrain-gen

FROM debian:bookworm-slim AS server
# curl backs the compose healthcheck; gosu drops root once the entrypoint has
# fixed volume ownership.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl gosu \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -g 10001 openmmo \
    && useradd -u 10001 -g 10001 -m -s /usr/sbin/nologin openmmo

COPY --from=builder /build/target/release/onlinerpg-server /usr/local/bin/
COPY --from=builder /build/target/release/terrain-gen /usr/local/bin/
COPY docker/server-entrypoint.sh docker/terrain-bake.sh /usr/local/bin/
RUN chmod +x /usr/local/bin/server-entrypoint.sh /usr/local/bin/terrain-bake.sh

# Tracked files a fresh volume would otherwise mask. Zone files define town
# no-spawn areas and monster spawn rectangles, and terrain-gen does not
# generate them; NPC schedules and personas drive the official NPCs.
COPY data/terrain/zones/ /opt/openmmo/seed/terrain/zones/
COPY data/announcements/_README.md /opt/openmmo/seed/announcements/
COPY agent-client/data/npcs/ /opt/openmmo/seed/npcs/

ENV STATE_DIR=/state \
    NPC_DATA_DIR=/npcs \
    TERRAIN_DIR=/terrain
WORKDIR /app
EXPOSE 10006 10007
ENTRYPOINT ["/usr/local/bin/server-entrypoint.sh"]
