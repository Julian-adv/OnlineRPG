#!/usr/bin/env bash
# Upload local binary assets to the Hugging Face dataset repo and regenerate
# assets.lock. Maintainer-only: needs the `hf` CLI logged in with write access.
set -euo pipefail
cd "$(dirname "$0")/.."

repo=$(awk '$1 == "repo" {print $2; exit}' assets.lock)

mapfile -d '' files < <(find client/public -type f \
    \( -name '*.glb' -o -name '*.mp3' -o -name '*.m4a' -o -name '*.blend' \) -print0 | sort -z)

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
for f in "${files[@]}"; do
    mkdir -p "$stage/$(dirname "$f")"
    cp "$f" "$stage/$f"
done

hf upload "$repo" "$stage" . --repo-type dataset --delete 'client/**' \
    --commit-message "Sync assets from working tree"

rev=$(python3 -c "import json, urllib.request; \
print(json.load(urllib.request.urlopen('https://huggingface.co/api/datasets/$repo'))['sha'])")

{
    echo "repo $repo"
    echo "revision $rev"
    for f in "${files[@]}"; do
        echo "file $(sha256sum "$f" | awk '{print $1}') $f"
    done
} > assets.lock

echo "assets.lock updated to revision $rev — commit it to publish."
