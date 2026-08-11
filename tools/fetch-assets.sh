#!/usr/bin/env bash
# Download binary assets pinned in assets.lock from Hugging Face.
# Idempotent: files whose sha256 already matches are skipped.
# Optional arg: path prefix to limit which entries are fetched.
set -euo pipefail
cd "$(dirname "$0")/.."

prefix=${1:-}
lock=assets.lock
repo=$(awk '$1 == "repo" {print $2; exit}' "$lock")
rev=$(awk '$1 == "revision" {print $2; exit}' "$lock")

urlencode() {
    local LC_ALL=C s=$1 out='' c i
    for ((i = 0; i < ${#s}; i++)); do
        c=${s:i:1}
        case $c in
            [A-Za-z0-9._~/-]) out+=$c ;;
            *) printf -v c '%%%02X' "'$c"; out+=$c ;;
        esac
    done
    printf '%s' "$out"
}

checked=0 fetched=0
while IFS= read -r line; do
    [[ $line == "file "* ]] || continue
    read -r _ hash path <<<"$line"
    [[ -z $prefix || $path == "$prefix"* ]] || continue
    checked=$((checked + 1))
    if [[ -f $path && $(sha256sum "$path" | awk '{print $1}') == "$hash" ]]; then
        continue
    fi
    mkdir -p "$(dirname "$path")"
    url="https://huggingface.co/datasets/$repo/resolve/$rev/$(urlencode "$path")"
    tmp="$path.hf-tmp"
    curl -fsSL --retry 3 -o "$tmp" "$url"
    if [[ $(sha256sum "$tmp" | awk '{print $1}') != "$hash" ]]; then
        rm -f "$tmp"
        echo "error: checksum mismatch for $path" >&2
        exit 1
    fi
    mv "$tmp" "$path"
    fetched=$((fetched + 1))
    echo "fetched $path"
done < "$lock"

echo "assets ok: $checked files, $fetched fetched"
