#!/bin/sh
# Validates the bind-mounted config before starting, then drops privileges.
set -eu

. /usr/local/bin/entrypoint-lib.sh

CONFIG=data/config.toml

# Docker silently creates a directory when a bind-mount source is missing, so a
# forgotten config.toml shows up here as a directory rather than a clear error.
if [ -d "$CONFIG" ]; then
    echo "error: $CONFIG is a directory." >&2
    echo "       Docker created it because the bind-mount source does not exist." >&2
    echo "       Create it first:  cp agent-client/data/config.toml.example agent-client/data/config.toml" >&2
    echo "       Then remove the stray directory and start again." >&2
    exit 1
fi

if [ ! -f "$CONFIG" ]; then
    echo "error: $CONFIG not found." >&2
    echo "       cp agent-client/data/config.toml.example agent-client/data/config.toml" >&2
    echo "       and fill in server, terrain and llm before enabling the agent profile." >&2
    exit 1
fi

# npcs/ is mounted read-only from the server; only the cache is ours to write.
mkdir -p data/cache
own_volume data/cache

exec gosu openmmo agent-client "$@"
