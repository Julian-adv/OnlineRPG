---
description: "Deploy the current master to the prod server (build both binaries + client bundle on the prod host, publish, restart the systemd units) and verify it came up. Use when the user asks to deploy, ship, or push to prod. Restarts the game — it disconnects every live player."
---

You are deploying OnlineRPG to production. The deploy script (`tools/deploy-prod.sh`)
runs **on the prod host**: it `git pull --ff-only`s master, builds both Rust
binaries and the client bundle, rsyncs the bundle to the webroot, then restarts
both systemd units. Restarting disconnects everyone currently playing.

The prod host is the `prod` SSH alias. See README.md "Production Deployment" for
the reference.

## 1. Preflight — before touching prod

Gitignored operator data (announcements, banned names, terrain, housing) does
not ride the deploy — sync it **before** launching so the deploy's restart
loads it; syncing after costs prod a second restart.

- **Push first.** The script pulls `master` on prod, so anything not on
  `origin/master` will not deploy. Run `git status` and `git log origin/master..HEAD`;
  if HEAD is ahead of origin or the tree is dirty, stop and get it committed and
  pushed (this repo commits straight to master — no feature branch). Confirm the
  commit you intend to ship is the one at `origin/master`.
- **Player-facing change? Write an announcement first.** Announcements show on the
  login screen. They live in `data/announcements/` but are **gitignored**
  (operator content — `.gitignore` excludes `*.md` except `_README.md`), so they
  do **not** ride the deploy. Format is in `data/announcements/_README.md`
  (`YYYY-MM-DD-title.md`, `title`/`title_en`/`category` frontmatter, `[en]` marker
  for the English body). Match the existing files' habits: include the
  server-restart notice ("업데이트 적용을 위해 서버가 잠시 재시작됩니다…"), write from
  the player's point of view (what they saw, not the mechanism), and if the fix is
  unverified in live play, say so. Then copy it up:
  ```bash
  scp data/announcements/<file>.md prod:~/work/OnlineRPG/data/announcements/
  ```
  A pure internal change (logging, refactor with no player-visible effect) needs
  no announcement — say so and skip it.
- **Banned names.** `data/banned_names.txt` is gitignored too (real names);
  the local copy on this host is the master. Diff against prod first — a
  prod-only line is an operator edit to merge locally, not overwrite — then
  push:
  ```bash
  ssh prod 'cat ~/work/OnlineRPG/data/banned_names.txt' | diff - data/banned_names.txt
  scp data/banned_names.txt prod:~/work/OnlineRPG/data/banned_names.txt
  ```
  Identical files → say so and skip the scp.
- **Terrain and housing sync — check both every deploy.** `data/terrain/` and
  `data/housing/` are not in git, so the deploy does not carry them. Sync
  **before** launching so the deploy's restart loads them — syncing after
  costs prod a second restart (server + agent-client, which caches houses
  for pathfinding).
  - Terrain: `REMOTE=prod bash tools/sync-terrain.sh` (mtime-based dry run;
    do **not** add `--checksum` — 1.16M files, it reads every one). Review
    the transferred-files count (a handful is normal), rerun with `--apply`,
    then spot-check an md5sum on prod.
  - Housing: houses, rooms, and furniture placements all live in the
    per-house JSONs under `data/housing/`; the sync script excludes the
    directory on purpose because the server writes live state (door
    open/close) into these files and prod is authoritative. Handle one file
    at a time, never a bulk rsync. Diff both directions, deletions included:
    ```bash
    rsync -ain --delete data/housing/ prod:work/OnlineRPG/data/housing/
    ```
    Ignore `*.bak-*` lines and `t`-only (mtime) lines. What remains:
    - `<f+++` / `<f.s` (new or changed here): fetch prod's copy and diff it —
      prod-only changes (placed furniture, new houses, doors) would be lost;
      merge if both sides changed. Back it up on prod as
      `<file>.bak-YYYYMMDD`, then scp the local file over.
    - `*deleting <house>.json` (removed here): confirm the house is gone from
      the local editor on purpose, check the prod copy's `ownerId` and
      furniture (a player-owned house is not deleted without asking), then
      `mv` it to `<file>.bak-YYYYMMDD` on prod. Do not leave it — its terrain
      edits (grass regrown, flattening undone) do ride the terrain sync, so a
      house left behind stands on regrown grass.
  Nothing to sync when neither directory has local edits — say so and move on.

## 2. Launch the deploy, detached

A foreground run dies with the SSH connection and loses the whole build, so
detach it:
```bash
ssh prod 'setsid nohup bash ~/work/OnlineRPG/tools/deploy-prod.sh > ~/deploy-latest.log 2>&1 < /dev/null &'
```
The script builds everything before it touches live state (rsync + restarts at the
very end), so an interruption before that leaves the old bundle and old server
running as a matched pair — never a half-deploy.

## 3. Watch it to completion — with a monitor that ends itself

The log ends at `==> deployed <commit>`, and that marker is the script's **last**
line. So a plain `tail -f` never terminates: nothing is written after the marker,
so even a `grep`/`awk` that exits on it leaves `tail` blocked on the file with no
SIGPIPE to kill it, and the monitor lingers until timeout. Don't use `tail -f`.

Poll the log on prod instead and `exit` when the marker lands, so the monitor
closes itself. Pass this to the Monitor tool (single ssh, streams progress +
failures, self-terminating):
```bash
ssh prod 'last=0; while :; do
  n=$(wc -l < ~/deploy-latest.log 2>/dev/null || echo "$last")
  if [ "$n" -gt "$last" ]; then
    sed -n "$((last+1)),${n}p" ~/deploy-latest.log \
      | grep -E "==>|warning|error|Error|error\[|failed|FAILED|panic|Killed|No space|fatal"
    last=$n
  fi
  tail -5 ~/deploy-latest.log | grep -q "==> deployed" && exit 0
  sleep 3
done'
```
The `==>` in the filter catches every progress marker (git pull → builds → publish
→ restarts → deployed); the error signatures catch a build that aborts under
`set -euo pipefail` without ever reaching the marker. A typical run is a few
minutes (two release builds + wasm + Vite bundle). Keep the Monitor's own timeout
as a backstop in case the deploy hangs and neither the marker nor an error appears.

## 4. Verify it came up

```bash
ssh prod 'systemctl is-active openmmo-server openmmo-agent-client'
ssh prod 'journalctl -u openmmo-server --since "2 min ago" -p err --no-pager -o cat | tail'
ssh prod 'journalctl -u openmmo-server -n 15 --no-pager -o cat'   # startup + passability cache line
```
Confirm both units are `active`, the startup log shows no panics, and the
"Passability cache ready" / "Server started successfully" lines are present.
A dead `openmmo-agent-client` (expired LLM login, outage) does **not** fail the
deploy — the game is already live — but flag it.

## 5. Agent-client release — when the deploy needs one

If `PROTOCOL_VERSION` ([shared/src/lib.rs](../../shared/src/lib.rs)) changed since the
last `agent-client-v*` tag, every distributed agent-client is refused by the new
server ("update agent-client"), so a GitHub release must ship with the deploy.
Check with:
```bash
git log $(git describe --tags --match 'agent-client-v*' --abbrev=0)..HEAD --oneline -- shared/ agent-client/
```
A meaningful `agent-client/` change without a protocol bump also warrants one;
pure server/web-client work does not — say so and skip.

1. Tag the deployed commit `agent-client-vX.Y.0` and push the tag.
2. Linux tarball, on this host:
   ```bash
   GOOGLE_CLI_CLIENT_SECRET=$(cat ~/.config/openmmo/cli-secret) bash tools/package-agent-client.sh
   ```
3. Windows zip, on `pc4090` (repo `C:\Users\jake\work\OnlineRPG`): bring the repo
   to the tagged commit first — it often holds uncommitted local files; back them
   up (rename to `*.bak`), never delete. Build with **pwsh 7, not powershell.exe**
   (5.1 writes backslash zip paths that break extraction outside Windows):
   ```bash
   ssh pc4090 'pwsh -NoProfile -Command "cd C:\Users\jake\work\OnlineRPG; $env:GOOGLE_CLI_CLIENT_SECRET=(Get-Content C:\Users\jake\.config\openmmo\cli-secret -Raw).Trim(); .\tools\package-agent-client.ps1"'
   ```
   scp the zip back and verify the archive has zero backslash entry paths.
4. `gh release create agent-client-vX.Y.0 <tarball> <zip>` — match the previous
   release's Korean notes: why the protocol bumped, old versions refused at
   connect, what changed for agents, asset list.
5. Add an in-game announcement telling agent operators to re-download (step 1's
   announcement flow; a server restart is needed to show it).

Details: doc/REMOTE_AGENT_CLIENT.md "패키징 메모".

## 6. Report

Tell the user the deployed commit (`==> deployed <hash>`), that both units are up,
whether the announcement shipped, and the agent-client release URL if one was
needed. If the deploy was to fix a bug with a log
signal (e.g. the `Blocked move` warns), compare its rate before vs after the
restart with `journalctl` rather than claiming success from a clean build alone —
a clean build only proves it compiled, not that the fix worked in play.

$ARGUMENTS
