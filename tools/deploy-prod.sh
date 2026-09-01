#!/usr/bin/env bash
# Build the current master on this host and put it live.
#
# Run it on the deploy host itself (prod). Overridable: REPO, WEBROOT, SERVICE.
set -euo pipefail

# git pull below rewrites this file while bash is still reading it, so re-exec
# from a snapshot first.
if [[ ${DEPLOY_FROM_SNAPSHOT:-} != 1 ]]; then
    snap=$(mktemp)
    cat "$0" > "$snap"
    DEPLOY_FROM_SNAPSHOT=1 exec bash "$snap" "$@"
fi
trap 'rm -f "$0"' EXIT

REPO=${REPO:-$HOME/work/OnlineRPG}
WEBROOT=${WEBROOT:-/var/www/openmmo}
SERVICE=${SERVICE:-openmmo-server}
AGENT_SERVICE=${AGENT_SERVICE:-openmmo-agent-client}

cd "$REPO"

echo "==> git pull"
git pull --ff-only

echo "==> assets"
bash tools/fetch-assets.sh

# Untracked (real names): synced from the dev host in the /deploy preflight.
# The server logs "Loaded N banned character name(s)" at restart.
if [[ ! -f data/banned_names.txt ]]; then
    echo "warning: data/banned_names.txt missing — restart loads an empty list" >&2
fi

echo "==> server (release)"
cargo build --release -p onlinerpg-server

echo "==> agent client (release)"
cargo build --release -p agent-client

echo "==> client deps"
(cd client && npm ci)

# `npm run build` runs build:wasm first, so the bundle always embeds fresh wasm.
echo "==> client bundle"
(cd client && npm run build)

# Both artifacts exist now. Swap the static files and restart together, so the
# wasm the browser downloads and the running server never disagree on the
# wire protocol.
echo "==> publish to $WEBROOT"
sudo rsync -a --delete client/dist/ "$WEBROOT/"
sudo chown -R www-data:www-data "$WEBROOT"

echo "==> restart $SERVICE"
sudo systemctl restart "$SERVICE"
sleep 3
sudo systemctl is-active "$SERVICE"

# The NPCs reconnect to a restarted server on their own, but their binary just
# changed too. Not every host runs them, so skip when the unit is absent.
if systemctl cat "$AGENT_SERVICE.service" >/dev/null 2>&1; then
    echo "==> restart $AGENT_SERVICE"
    sudo systemctl restart "$AGENT_SERVICE"
    sleep 3
    # A dead agent client (expired codex login, LLM outage) must not fail the
    # deploy — the game itself is already live at this point.
    sudo systemctl is-active "$AGENT_SERVICE" ||
        echo "warning: $AGENT_SERVICE is not running — check 'journalctl -u $AGENT_SERVICE'" >&2
fi

echo "==> deployed $(git log --oneline -1)"
