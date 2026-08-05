# syntax=docker/dockerfile:1

# Builds the web bundle (Svelte + wasm) and serves it from nginx.
# Binary assets must be fetched from the pinned Hugging Face dataset first.

# The bundle is built with this placeholder client id (Vite inlines
# import.meta.env at build time) and the entrypoint substitutes the
# deployment's value at startup. Defined once here for both stages.
ARG CLIENT_ID_PLACEHOLDER=__GOOGLE_CLIENT_ID__

FROM node:22-bookworm AS builder

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable \
        --target wasm32-unknown-unknown
ENV PATH="/root/.cargo/bin:${PATH}"

# Pinned so a wasm-pack release cannot silently change the emitted glue code.
ARG WASM_PACK_VERSION=0.15.0
RUN --mount=type=cache,target=/root/.cargo/registry \
    cargo install wasm-pack --version "${WASM_PACK_VERSION}" --locked

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
COPY docker/stub-members.sh docker/
RUN sh docker/stub-members.sh agent-client server tools/terrain-gen

# The public assets are ~750 MB and change rarely; keeping them (and their
# checksum pass) in their own layers means source edits reuse the cache.
COPY client/public/ client/public/
RUN sed -n 's/^file \([0-9a-f]*\) /\1  /p' assets.lock | sha256sum -c --quiet

COPY client/package.json client/package-lock.json client/
WORKDIR /build/client
RUN npm ci

# Enumerated so the asset layer above is not re-copied. A new build input at
# the client root must be added here.
COPY client/index.html client/vite.config.ts client/svelte.config.js \
     client/tsconfig.json client/tsconfig.app.json client/tsconfig.node.json ./
COPY client/src/ src/

ARG CLIENT_ID_PLACEHOLDER
ENV VITE_GOOGLE_CLIENT_ID=${CLIENT_ID_PLACEHOLDER}
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,id=target-wasm,target=/build/target \
    npm run build

FROM nginx:alpine
COPY --from=builder /build/client/dist /usr/share/nginx/html
COPY docker/nginx.conf.template /etc/nginx/templates/default.conf.template
COPY docker/client-entrypoint.sh /docker-entrypoint.d/40-client-id.sh
RUN chmod +x /docker-entrypoint.d/40-client-id.sh
ARG CLIENT_ID_PLACEHOLDER
# The entrypoint needs the same placeholder the bundle was built with, and the
# base image needs the resolver export for the template's ${NGINX_LOCAL_RESOLVERS}.
ENV CLIENT_ID_PLACEHOLDER=${CLIENT_ID_PLACEHOLDER} \
    NGINX_ENTRYPOINT_LOCAL_RESOLVERS=1
EXPOSE 80
