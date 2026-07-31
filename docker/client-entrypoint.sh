#!/bin/sh
# Substitutes the container's DNS server into the nginx config.
#
# The upstream is resolved through a variable so nginx can start before the
# server container exists, and a `resolver` directive is mandatory for that.
# Reading resolv.conf keeps this correct on any compose network mode, unlike
# hardcoding Docker's embedded 127.0.0.11.
set -eu

CONF=/etc/nginx/conf.d/default.conf
RESOLVER=$(awk '/^nameserver/ { print $2; exit }' /etc/resolv.conf)

if [ -z "${RESOLVER:-}" ]; then
    RESOLVER=127.0.0.11
fi

sed -i "s/__RESOLVER__/${RESOLVER}/" "$CONF"

# Vite baked __GOOGLE_CLIENT_ID__ into the bundle at build time (see
# client.Dockerfile). Substituting here lets one image serve any deployment;
# left unset, the login screen reports the placeholder as unconfigured.
if [ -n "${GOOGLE_CLIENT_ID:-}" ]; then
    grep -rl __GOOGLE_CLIENT_ID__ /usr/share/nginx/html \
        | xargs -r sed -i "s/__GOOGLE_CLIENT_ID__/${GOOGLE_CLIENT_ID}/g"
fi
