#!/bin/sh
# Stubs the workspace members an image does not compile, so cargo can resolve
# the workspace from their manifests alone. Build scripts of members outside
# the -p selection never run, so the build.rs stub is inert.
set -eu

for member in "$@"; do
    mkdir -p "$member/src"
    echo 'fn main() {}' > "$member/src/main.rs"
    echo 'fn main() {}' > "$member/build.rs"
done
