#!/usr/bin/env bash
# Report binary assets whose working-tree bytes differ from assets.lock, or that
# the lock doesn't list. Prints nothing and exits 0 when in sync; exits 1 otherwise.
# Same file set as push-assets.sh.
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
import hashlib, os, subprocess, sys

lock = {}
for line in open("assets.lock"):
    parts = line.rstrip("\n").split(" ", 2)
    if parts[0] == "file":
        lock[parts[2]] = parts[1]

files = subprocess.run(
    ["bash", "-c",
     r"find client/public -type f \( -name '*.glb' -o -name '*.mp3' -o -name '*.m4a' -o -name '*.blend' \) -print0;"
     r"[ -d assets ] && find assets -type f ! -name '*.blend1' -print0 || true"],
    capture_output=True, check=True).stdout.decode().split("\0")
files = sorted(f for f in files if f)

bad = False
for f in files:
    if f not in lock:
        print(f"new      {f}"); bad = True
        continue
    h = hashlib.sha256()
    with open(f, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    if h.hexdigest() != lock[f]:
        print(f"modified {f}"); bad = True
for f in lock:
    if f not in files:
        print(f"missing  {f}"); bad = True
sys.exit(1 if bad else 0)
PY
