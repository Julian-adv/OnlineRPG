# syntax=docker/dockerfile:1

# Two runtime targets share one compile:
#   slim  (default) - HTTP LLM backends only (none/openrouter/openai)
#   codex           - adds node + the codex CLI, which shells out per prompt
# The codex target needs a logged-in ~/.codex mounted in, so it cannot be
# verified by a from-scratch compose run the way the slim target can.

FROM rust:1-bookworm AS builder
WORKDIR /build

COPY Cargo.toml Cargo.lock clippy.toml ./
# shared/ embeds tracked JSON from data/ and the object catalog with
# include_str!. The catalog is a small tracked file, so it is copied alone
# rather than dragging all of client/public into the image.
COPY data/ data/
COPY client/public/models/objects/catalog.json client/public/models/objects/
COPY shared/ shared/
COPY terrain/ terrain/
COPY agent-client/ agent-client/
COPY tools/cargo-build-data.rs tools/
COPY data-src/ data-src/

COPY server/Cargo.toml server/
COPY tools/terrain-gen/Cargo.toml tools/terrain-gen/
COPY docker/stub-members.sh docker/
RUN sh docker/stub-members.sh server tools/terrain-gen

# The cache mounts keep dependency compiles out of the layer, so the binary
# must be copied out within the same RUN.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,id=target-agent-client,target=/build/target \
    cargo build --release --locked -p agent-client \
    && mkdir -p /out \
    && cp target/release/agent-client /out/

FROM debian:bookworm-slim AS slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl gosu \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -g 10001 openmmo \
    && useradd -u 10001 -g 10001 -m -s /usr/sbin/nologin openmmo

COPY --from=builder /out/agent-client /usr/local/bin/
COPY docker/entrypoint-lib.sh docker/agent-entrypoint.sh /usr/local/bin/
RUN chmod +x /usr/local/bin/agent-entrypoint.sh

# config.toml is gitignored deployment state and arrives as a bind mount.
# npcs/ arrives from the server's volume, read-only.
COPY agent-client/data/animation_durations.json \
     agent-client/data/system_prompt.txt \
     /app/agent-client/data/
COPY agent-client/data/templates/ /app/agent-client/data/templates/
COPY agent-client/data/prompts/ /app/agent-client/data/prompts/
COPY agent-client/data/user_prompts/ /app/agent-client/data/user_prompts/

# The binary resolves data/config.toml and ../data/npc_token relative to CWD.
WORKDIR /app/agent-client
ENTRYPOINT ["/usr/local/bin/agent-entrypoint.sh"]

FROM slim AS codex
RUN apt-get update \
    && apt-get install -y --no-install-recommends gnupg \
    && curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && npm install -g @openai/codex \
    && npm cache clean --force \
    && rm -rf /var/lib/apt/lists/*
