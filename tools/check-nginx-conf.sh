#!/usr/bin/env bash
# Syntax-check docker/nginx.conf.template. The template is a conf.d fragment,
# so it is rendered and wrapped in the minimal surrounding config nginx needs
# before `nginx -t` will look at it. Needs the nginx binary (`apt install
# nginx-core`); the service itself does not have to run.
set -euo pipefail
cd "$(dirname "$0")/.."

command -v nginx >/dev/null || { echo "nginx not installed" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/logs" "$work/cache"

# Only *defined* vars are substituted, exactly as the base image's entrypoint
# does — nginx's own $variables have to survive.
NGINX_LOCAL_RESOLVERS=127.0.0.11 \
    envsubst '$NGINX_LOCAL_RESOLVERS' < docker/nginx.conf.template > "$work/site.conf"

# Two rewrites, both only so the check needs no privileges: the cache lives
# somewhere root owns in the container, and `nginx -t` really opens the listen
# socket, which a normal user may not do on port 80. Nothing else is touched.
sed -i "s#/var/cache/nginx#$work/cache#; s#listen 80;#listen 18080;#" \
    "$work/site.conf"

cat > "$work/nginx.conf" <<CONF
pid $work/nginx.pid;
error_log $work/logs/error.log;
events {}
http {
    access_log $work/logs/access.log;
    client_body_temp_path $work/body;
    proxy_temp_path $work/proxy;
    fastcgi_temp_path $work/fastcgi;
    uwsgi_temp_path $work/uwsgi;
    scgi_temp_path $work/scgi;
    include $work/site.conf;
}
CONF

nginx -t -p "$work" -c "$work/nginx.conf"
