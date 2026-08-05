#!/bin/sh
# Vite baked a placeholder client id into the bundle at build time (see
# client.Dockerfile); substituting here lets one image serve any deployment.
# Unset substitutes empty, which is what the login screen reports as
# unconfigured — leaving the placeholder would reach Google as a bogus id.
set -eu

find /usr/share/nginx/html -type f \( -name '*.js' -o -name '*.html' \) \
    | xargs -r grep -l "$CLIENT_ID_PLACEHOLDER" \
    | xargs -r sed -i "s/$CLIENT_ID_PLACEHOLDER/${GOOGLE_CLIENT_ID:-}/g"
