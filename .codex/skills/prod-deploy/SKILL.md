---
name: prod-deploy
description: Deploy OnlineRPG's current master branch to the production host, monitor the detached build and publish process, restart the game services, and verify production health. Use when the user asks to deploy, ship, release, or push OnlineRPG to prod/production. The restart disconnects all live players.
---

# Production Deploy

Deploy OnlineRPG through the repository's production script and report the exact
commit and service health. Treat this as a live, player-impacting operation.

## Preflight

1. Tell the user that the restart disconnects all connected players before
   launching the deploy.
2. Read the `Production Deployment` section of `README.md` and
   `tools/deploy-prod.sh` if either changed since this skill was last read.
3. Inspect the local release state:

   ```bash
   git status --short --branch
   git log --oneline origin/master..HEAD
   git rev-parse HEAD origin/master
   git log -1 --oneline origin/master
   ```

4. Stop before touching production when:
   - the current branch is not `master`;
   - the working tree is dirty;
   - local `HEAD` is ahead of `origin/master`; or
   - the intended release commit is not on `origin/master`.

   Explain that the prod script pulls `master`. Do not commit or push unless the
   user separately authorizes it.
5. Determine whether the release contains player-visible changes. Inspect the
   commits and diff being released when necessary.
   - For internal-only changes such as logging or behavior-preserving refactors,
     state that no announcement is needed.
   - For player-visible changes, write an announcement before deploying. Read
     `data/announcements/_README.md` and existing announcement files for the
     current style. Use `YYYY-MM-DD-title.md`, required frontmatter, a Korean
     body, and an `[en]` English body. Include the brief restart notice, describe
     the change from the player's perspective, and do not claim a live fix is
     verified when it is not.
   - Announcement files are gitignored operator content. Copy the selected file:

     ```bash
     scp data/announcements/<file>.md prod:~/work/OnlineRPG/data/announcements/
     ```

## Launch

The `prod` SSH alias identifies the production host. Run the deploy detached so
an SSH interruption does not terminate the build. Record its PID so monitoring
can distinguish failure from a slow build:

```bash
ssh prod 'setsid nohup bash ~/work/OnlineRPG/tools/deploy-prod.sh > ~/deploy-latest.log 2>&1 < /dev/null & echo $! > ~/deploy-latest.pid'
```

The script builds both Rust binaries and the client bundle before publishing or
restarting anything. Do not run another deploy concurrently.

## Monitor

Monitor until the final `==> deployed <commit>` marker appears. Do not use
`tail -f`; the marker is the last line, so `tail -f` will not terminate.

Run this through the shell execution tool. If it yields a running session,
continue it with the session wait/input tool until completion while keeping the
user updated:

```bash
ssh prod 'last=0; while :; do
  n=$(wc -l < ~/deploy-latest.log 2>/dev/null || echo "$last")
  if [ "$n" -gt "$last" ]; then
    sed -n "$((last+1)),${n}p" ~/deploy-latest.log \
      | grep -E "==>|error|Error|error\[|failed|FAILED|panic|Killed|No space|fatal|warning" || true
    last=$n
  fi
  tail -5 ~/deploy-latest.log 2>/dev/null | grep -q "==> deployed" && exit 0
  pid=$(cat ~/deploy-latest.pid 2>/dev/null || true)
  if [ -n "$pid" ] && ! kill -0 "$pid" 2>/dev/null; then
    echo "deploy process exited without a success marker"
    tail -50 ~/deploy-latest.log
    exit 1
  fi
  sleep 3
done'
```

A normal deploy takes several minutes. If the monitor fails, inspect the final
log, report the failed stage, and do not claim that production was updated.

## Verify

After the success marker, run:

```bash
ssh prod 'systemctl is-active openmmo-server openmmo-agent-client'
ssh prod 'journalctl -u openmmo-server --since "2 min ago" -p err --no-pager -o cat | tail'
ssh prod 'journalctl -u openmmo-server -n 15 --no-pager -o cat'
```

Confirm:

- `openmmo-server` is `active`;
- startup logs contain no panic or fatal startup error;
- startup reached `Passability cache ready` and `Server started successfully`;
- `openmmo-agent-client` status is reported.

A dead agent client can result from an expired LLM login or outage and does not
make the game-server deploy itself fail, but always flag it.

For a bug fix with an observable log signal, compare that signal before and
after the restart with `journalctl`. A successful build and restart prove only
that the release deployed, not that gameplay behavior is fixed.

## Report

Report:

- the commit from `==> deployed <hash> <subject>`;
- whether `openmmo-server` and `openmmo-agent-client` are active;
- whether an announcement was required and copied;
- any warnings or unverified gameplay claims.
