#!/usr/bin/env bash
# Build a tarball someone can unpack on their own machine and run.
#
# The agent-client needs its data/ directory next to the binary (prompts,
# animation timings) but none of the 3 GB terrain tree — that comes over HTTP.
# Output: dist/agent-client-<commit>-<target>.tar.gz
#
# Builds natively, so the artifact inherits the build host's glibc floor
# (currently 2.39 = Ubuntu 24.04+). That is the documented audience for the
# download; anyone older builds from source, which needs nothing but a Rust
# toolchain. Set TARGET to cross-compile (x86_64-unknown-linux-musl gives a
# static binary, but ring needs a musl-targeting cc for that).
set -euo pipefail

REPO=${REPO:-$(cd "$(dirname "$0")/.." && pwd)}
OUT_DIR=${OUT_DIR:-$REPO/dist}
# Host only: the WebSocket lives at /ws (the reverse proxy upgrades that path
# and serves the game page at /), and the tile API at /api/terrain.
HOST=${HOST:-openmmo.to.nexus}

# Google's device flow requires the installed-app secret in the token
# exchange. It is not confidential (RFC 8252 section 8.5) and every shipped
# copy needs it, but it stays out of the repo: committing it trips secret
# scanners, so it is injected here from the packaging environment instead.
CLIENT_SECRET=${GOOGLE_CLI_CLIENT_SECRET:-}
if [[ -z $CLIENT_SECRET ]]; then
    echo "error: set GOOGLE_CLI_CLIENT_SECRET (Google Cloud → the CLI OAuth client)." >&2
    echo "       Without it the packaged client cannot complete Google sign-in." >&2
    exit 1
fi
# v0.2.0 shipped a value with the GO chopped off, and every packaged sign-in
# failed with invalid_client. Refuse to build rather than find out downstream.
if [[ $CLIENT_SECRET != GOCSPX-* ]]; then
    echo "error: GOOGLE_CLI_CLIENT_SECRET does not start with GOCSPX-." >&2
    echo "       Google installed-app secrets always do; this one is mangled." >&2
    exit 1
fi

TARGET=${TARGET:-}

cd "$REPO"
# The artifact is named by the crate version; the release tag must be
# agent-client-v<version>, and the version must be bumped every release —
# the shipped self-updater compares it against the latest tag.
version=$(grep -m1 '^version' agent-client/Cargo.toml | cut -d'"' -f2)
if [[ -n $TARGET ]]; then
    cargo build --release --target "$TARGET" -p agent-client
    binary="target/$TARGET/release/agent-client"
    suffix=${TARGET%%-unknown-linux-*}-${TARGET##*-}
else
    cargo build --release -p agent-client
    binary="target/release/agent-client"
    # Name the glibc floor, not the build host: it is what decides whether
    # the download runs on someone else's machine.
    glibc=$(objdump -T "$binary" | grep -oE "GLIBC_[0-9.]+" | sort -uV | tail -1)
    suffix="$(uname -m)-${glibc,,}"
fi
name="agent-client-v$version-$suffix"
stage="$OUT_DIR/$name"

rm -rf "$stage"
mkdir -p "$stage/data"
cp "$binary" "$stage/"
# No data/templates: those are operator NPC roles (merchant, guard). A user
# agent layers its own role file over the shared data/system_prompt.txt.
cp agent-client/data/system_prompt.txt agent-client/data/animation_durations.json "$stage/data/"
# Ship both roles to copy from, and start the editable one off as newcomer:
# a fresh agent that learns the world by playing it.
mkdir -p "$stage/data/user_prompts"
cp agent-client/data/user_prompts/*.txt "$stage/data/user_prompts/"
cp agent-client/data/user_prompts/newcomer.txt "$stage/data/user_prompt.txt"

# Shared with package-agent-client.ps1 so the shipped config cannot drift.
# Registry NPC personas are operator-side; a user agent plays its own character.
config=$(<"$REPO/tools/agent-client-config.toml.in")
config=${config//@HOST@/$HOST}
config=${config//@CLIENT_SECRET@/$CLIENT_SECRET}
printf '%s\n' "$config" > "$stage/data/config.toml"

cp "$REPO/doc/AGENT_CLIENT_QUICKSTART.md" "$stage/README.md"

tar -czf "$OUT_DIR/$name.tar.gz" -C "$OUT_DIR" "$name"
rm -rf "$stage"
echo "==> $OUT_DIR/$name.tar.gz"
