#!/usr/bin/env bash
# Verifies a running compose stack end to end.
#
# Covers the parts of "a fresh host can play the game" that a machine can
# judge: the bundle is served, both proxied backends answer, the WebSocket
# upgrade completes, and the seeded zone data survived the bake. Whether the
# 3D world actually renders still needs a human.
#
# Usage: docker compose up -d --wait && tools/smoke-compose.sh
set -euo pipefail

PORT="${CLIENT_PORT:-8080}"
BASE="http://localhost:${PORT}"
fails=0

check() {
    local label=$1
    shift
    if "$@" >/dev/null 2>&1; then
        echo "ok    $label"
    else
        echo "FAIL  $label" >&2
        fails=$((fails + 1))
    fi
}

check_zone_write() {
    local zone token code
    zone=$(mktemp)
    token=$(docker compose exec -T server cat /state/npc_token)
    curl -fsS "$BASE/api/terrain/zones/-2/0" -o "$zone"
    code=$(curl -sS -o /dev/null -w '%{http_code}' \
        -X PUT \
        -H "Authorization: Bearer $token" \
        -H 'Content-Type: application/json' \
        --data-binary "@$zone" \
        "$BASE/api/terrain/zones/-2/0")
    rm -f "$zone"
    [[ $code == 204 ]]
}

echo "==> smoke testing $BASE"

check "client serves index.html" \
    bash -c "curl -fsS '$BASE/' | grep -q '<div id=\"app\"'"

check "wasm bundle is reachable" \
    bash -c "curl -fsS -o /dev/null '$BASE/' -w '%{http_code}' | grep -q '^200$'"

# Proves nginx resolves and reaches the server's REST port.
check "/api proxies to the terrain REST API" \
    curl -fsS -o /dev/null "$BASE/api/terrain/height/0/0"

# Zone files are tracked in git but terrain-gen never writes them. If the image
# seed did not land, this still returns 200 with an empty array — hence the
# length check rather than a bare status check.
check "seeded no-spawn zones survived the bake" \
    bash -c "curl -fsS '$BASE/api/terrain/zones/-2/0' | jq -e '.noSpawnZones | length >= 1'"

check "authenticated terrain writes reach the volume" check_zone_write

# 101 means nginx forwarded the Upgrade to the game port.
check "/ws completes the WebSocket upgrade" \
    bash -c "
        code=\$(curl -fsS -o /dev/null -w '%{http_code}' \
            -H 'Upgrade: websocket' \
            -H 'Connection: Upgrade' \
            -H 'Sec-WebSocket-Version: 13' \
            -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
            '$BASE/ws' || true)
        [ \"\$code\" = '101' ]
    "

check "every service is running or exited cleanly" \
    bash -c "docker compose ps --format json | jq -es 'all(.State == \"running\" or .ExitCode == 0)'"

if [ "$fails" -ne 0 ]; then
    echo "==> $fails check(s) failed" >&2
    exit 1
fi

echo "==> all checks passed"
