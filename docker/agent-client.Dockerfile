# syntax=docker/dockerfile:1

# Two runtime targets share one compile:
#   slim  (default) - HTTP LLM backends only (none/openrouter/openai)
#   codex           - adds node + the codex CLI, which shells out per prompt
# The codex target needs a logged-in ~/.codex mounted in, so it cannot be
# verified by a from-scratch compose run the way the slim target can.

FROM rust:1-bookworm AS builder
WORKDIR /build

COPY Cargo.toml Cargo.lock clippy.toml ./
# shared/ embeds three tracked JSON files from data/ with include_str!.
COPY data/ data/
COPY shared/ shared/
COPY terrain/ terrain/
COPY agent-client/ agent-client/
COPY tools/cargo-build-data.rs tools/
COPY data-src/ data-src/

# Mirror of server.Dockerfile: stub the members we are not building so cargo can
# still resolve the workspace without pulling their sources into this context.
COPY server/Cargo.toml server/
COPY tools/terrain-gen/Cargo.toml tools/terrain-gen/
RUN mkdir -p server/src tools/terrain-gen/src \
    && echo 'fn main() {}' > server/src/main.rs \
    && echo 'fn main() {}' > server/build.rs \
    && echo 'fn main() {}' > tools/terrain-gen/src/main.rs

RUN cargo build --release --locked -p agent-client

FROM debian:bookworm-slim AS slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl gosu \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -g 10001 openmmo \
    && useradd -u 10001 -g 10001 -m -s /usr/sbin/nologin openmmo

COPY --from=builder /build/target/release/agent-client /usr/local/bin/
COPY docker/agent-entrypoint.sh /usr/local/bin/
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
USER root
RUN apt-get update \
    && apt-get install -y --no-install-recommends gnupg \
    && curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && npm install -g @openai/codex \
    && npm cache clean --force \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app/agent-client
