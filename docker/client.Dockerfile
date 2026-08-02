# syntax=docker/dockerfile:1

# Builds the web bundle (Svelte + wasm) and serves it from nginx.
# Binary assets must be fetched from the pinned Hugging Face dataset first.

FROM node:22-bookworm AS builder

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable \
        --target wasm32-unknown-unknown
ENV PATH="/root/.cargo/bin:${PATH}"

# Pinned so a wasm-pack release cannot silently change the emitted glue code.
ARG WASM_PACK_VERSION=0.15.0
RUN cargo install wasm-pack --version "${WASM_PACK_VERSION}" --locked

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY assets.lock ./
# shared/ embeds three tracked JSON files from data/ with include_str!.
COPY data/ data/
COPY shared/ shared/
COPY terrain/ terrain/
COPY tools/ tools/
COPY data-src/ data-src/
# build:wasm runs generate:animations, which writes
# agent-client/data/animation_durations.json as a side effect. Nothing in the
# bundle reads it, but the script fails if the directory is missing.
COPY agent-client/data/animation_durations.json agent-client/data/

# wasm-pack resolves the whole cargo workspace before building shared/, so the
# members we do not compile still need manifests. Stub their sources.
COPY agent-client/Cargo.toml agent-client/
COPY server/Cargo.toml server/
COPY tools/terrain-gen/Cargo.toml tools/terrain-gen/
RUN mkdir -p agent-client/src server/src tools/terrain-gen/src \
    && echo 'fn main() {}' > agent-client/src/main.rs \
    && echo 'fn main() {}' > agent-client/build.rs \
    && echo 'fn main() {}' > server/src/main.rs \
    && echo 'fn main() {}' > server/build.rs \
    && echo 'fn main() {}' > tools/terrain-gen/src/main.rs
COPY client/ client/

WORKDIR /build
RUN set -eu; \
    while IFS= read -r line; do \
        case "$line" in \
            "file "*) \
                entry=${line#file }; \
                hash=${entry%% *}; \
                path=${entry#* }; \
                test -f "$path" || { echo "error: missing asset: $path" >&2; exit 1; }; \
                actual=$(sha256sum "$path" | cut -d ' ' -f 1); \
                test "$actual" = "$hash" || { echo "error: checksum mismatch: $path" >&2; exit 1; }; \
                ;; \
        esac; \
    done < assets.lock

WORKDIR /build/client
RUN npm ci
RUN npm run build

FROM nginx:alpine
COPY --from=builder /build/client/dist /usr/share/nginx/html
COPY docker/nginx.conf /etc/nginx/conf.d/default.conf
COPY docker/client-entrypoint.sh /docker-entrypoint.d/40-resolver.sh
RUN chmod +x /docker-entrypoint.d/40-resolver.sh
EXPOSE 80
