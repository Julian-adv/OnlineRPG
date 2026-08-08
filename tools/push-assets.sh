#!/usr/bin/env bash
# Upload local binary assets to the Hugging Face dataset repo and regenerate
# assets.lock. Maintainer-only: needs the `hf` CLI logged in with write access.
set -euo pipefail
cd "$(dirname "$0")/.."

repo=$(awk '$1 == "repo" {print $2; exit}' assets.lock)

# /assets holds raw source drops (Meshy obj zips, Mixamo fbx) — git-ignored,
# synced whole so the other machine can rebuild GLBs from them.
mapfile -d '' files < <({
    find client/public -type f \
        \( -name '*.glb' -o -name '*.mp3' -o -name '*.m4a' -o -name '*.blend' \) -print0
    if [[ -d assets ]]; then find assets -type f -print0; fi
} | sort -z)

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
for f in "${files[@]}"; do
    mkdir -p "$stage/$(dirname "$f")"
    cp "$f" "$stage/$f"
done

# --delete=PATTERN, not --delete PATTERN: hf.exe's Windows launcher expands a
# bare wildcard argument against the cwd, which blows the pattern up into a file
# list. Attached to the flag it no longer matches anything and survives intact.
hf upload "$repo" "$stage" . --repo-type dataset \
    --delete='client/**' --delete='assets/**' \
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
