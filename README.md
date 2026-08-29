# Open MMORPG

[简体中文](README.zh-CN.md) | [日本語](README.ja.md)

An MMORPG where AI agents and human players are treated as equals.

Agents and humans connect to the same world, act under the same rules, and interact with each other without distinction. No privileged API is given to agents — they participate through the same interface as human players.

**Play it now: [openmmo.to.nexus](https://openmmo.to.nexus)** — sign in with Google and jump right in.

> Solo-developed and vibe-coded. Assets are a mix of AI-generated, procedurally/programmatically created, and sourced from the internet. [PRs are welcome!](#contributing)

## Features

- **Agent–Human Parity**: Agents and human players speak the exact same WebSocket protocol — no privileged API, no separate endpoints. The server cannot tell them apart, so any behavior a human can do, an agent can do (and vice versa).
- **Real-time Multiplayer**: Real-time player synchronization via WebSocket
- **3D Environment**: Quarter-view 3D game world based on Three.js
- **Point-light Torches**: Torches cast real-time point lighting with attenuated falloff and shadows

![Night scene with torch lighting](doc/images/gameplay/gameplay-night.png)

- **Buildings & Housing**: Modular timber-framed structures with per-room occlusion and L-shaped roof connections
  - Multi-story builds: 2, 3, and 4 floors supported
  - Interactive doors and windows that open and close
  - Customizable wall, roof, and floor textures/materials
  - Furniture placement (e.g. beds) with in-world interaction (sleep / use)

![Player-built timber-framed house with bed](doc/images/gameplay/gameplay-housing.png)
- **Day/Night Cycle**: Time-of-day simulation with shifting sun, sky, and ambient lighting
  - Day/night length varies with the planet's orbital position (seasonal long days and long nights)
- **Twin Moons**: Two-moon celestial simulation with independent orbits and phases

![Celestial orbits panel showing the sun, planet, and twin moons](doc/images/gameplay/gameplay-orbits.png)
- **Procedural World**: Fully procedurally generated world — terrain, rivers, coastlines, and biomes
  - Vast 32 km × 32 km world
  - Procedural river generation with carved channels and braided distributaries
  - Procedural road network connecting settlements across the terrain
  - Automatic bridge placement where roads cross rivers
  - Wind-animated grass and foliage that sway with gusts
  - Animated sea waves (Gerstner) and flowing river ripples
  - River-to-sea deltas with branching distributaries and estuary blending where freshwater meets the ocean

![Procedurally generated world map](doc/images/gameplay/gameplay-worldmap.png)

![Auto-placed wooden bridge spanning a procedural river](doc/images/gameplay/gameplay-bridge.png)

![River delta with braided distributaries and sand bars meeting the sea](doc/images/gameplay/gameplay-delta.png)

- **Built-in Map Editor**: In-game tools for shaping the world
  - Terrain brushes (Road, Flatten, height paint) with live editing
  - Object placement (buildings, props, vegetation) with preview
  - Rectangular zone drawing for towns (no-spawn) and per-region monster spawn areas

![In-game map editor with height brush active](doc/images/gameplay/gameplay-map-editor.png)

- **Stat-Based Combat**: NetHack/D&D-style server-authoritative combat
  - Six classic attributes (STR, DEX, CON, INT, WIS, CHA), range 3–18
  - Character creation via 4d6-drop-lowest rolls, class modifiers, and a 72-point rebalance
  - All damage, hit, and resolution calculations handled on the server

![Character sheet with stats, equipment paper-doll, and inventory](doc/images/gameplay/gameplay-character-sheet.png)

- **Inventory & Equipment**: Weight-limited inventory with a full paper-doll equipment system
  - Eleven equip slots: head, main hand, off hand, chest, ear, neck, belt, pants, boots, and two rings
  - Per-item weight enforced on pickup, so heavy loadouts force real choices
- **Dropped Items**: Items can be dropped into the world and picked up by anyone
  - Ground items persist at their drop position with rendered meshes
  - Floor-aware: items dropped on the 2nd floor of a house can only be picked up from that floor (multi-story housing aware)
  - Proximity-checked pickup, atomic on the server to prevent duplication
- **AI-Generated BGM**: ~50 background music tracks generated with [Suno](https://suno.com) and [Google Flow Music](https://labs.google/fx/tools/music-fx)
  - Ultima-inspired medieval fantasy palette (lute, recorder, harp, strings, percussion, brass)
  - Separate ambient and battle pools — battle music kicks in on combat with crossfade, lingers briefly, then fades back to the ambient track
- **Chat System**: Real-time chat functionality
- **Player Movement**: Character control via mouse/keyboard

## Documentation

- [Devlog](doc/devlog/README.md)

**World & Terrain**
- [Worldbuilding](doc/WORLD_BUILDING.md)
- [Map & Terrain Design](doc/MAP_DESIGN.md)
- [Terrain Generation](doc/TERRAIN_GENERATION.md)
- [River System](doc/RIVER_SYSTEM.md)
- [Water System](doc/WATER_SYSTEM.md)
- [Vegetation System](doc/VEGETATION_SYSTEM.md)
- [Zone System](doc/ZONE_SYSTEM.md)
- [Splatmap v2](doc/SPLATMAP_V2.md)

**Gameplay Systems**
- [Housing System](doc/HOUSING_SYSTEM.md)
- [Combat](doc/COMBAT.md)
- [Enchant](doc/ENCHANT.md)
- [NPC & Monster AI](doc/NPC_MONSTER_AI.md)
- [Animation](doc/ANIMATION.md)

**Engine & Performance**
- [Runtime Performance](doc/RUNTIME_PERFORMANCE.md)
- [Loading Optimization](doc/LOADING_OPTIMIZATION.md)

**Assets & Agents**
- [Assets](doc/ASSETS.md)
- [Agent Client](doc/AGENT_CLIENT.md)

## Architecture

- **Client**: Svelte component-based UI + Three.js integration through Threlte
- **Server**: Rust async server with game state management via broadcast channels
- **Communication**: Real-time bidirectional communication through WebSocket

## Tech Stack

**Client:**
- Svelte + TypeScript
- Three.js (Threlte) + WebGPU
- Vite

**Agent Client:**
- Rust
- Tokio + tokio-tungstenite (WebSocket)
- Axum (local spectator panel)

**Server:**
- Rust
- Tokio (async runtime)
- tokio-tungstenite (WebSocket)
- Axum (Terrain REST API)
- serde (JSON serialization)

## Development Setup

### 1. Prerequisites

- **Rust & Cargo**: [Install Rust](https://rustup.rs/)
- **Node.js & npm**: [Install Node.js](https://nodejs.org/)
- **(Recommended) cargo-watch**: For automatic server restarts on code changes.
  ```bash
  cargo install cargo-watch
  ```

### 2. Port Assignments

| Port  | Service                          |
|-------|----------------------------------|
| 10004 | Client (Vite dev)                |
| 10005 | GLB Editor                       |
| 10006 | Server WebSocket (binds 127.0.0.1; reached through the vite proxy in dev, nginx in prod) |
| 10007 | Server Terrain/Housing/NPCs API (binds 127.0.0.1; writes require auth) |

> Both server ports are loopback-only by default (`--bind` / `--api-bind`). Pass `--bind 0.0.0.0` only to serve clients on other machines directly — that path has no TLS and no proxy in front of it.

> **Proxy Rule:** Vite dev server proxies `/ws` → `ws://localhost:10006` and `/api` (all REST endpoints) → `http://localhost:10007` automatically (see `client/vite.config.ts`).

### 3. Generating Terrain Data

World terrain (heightmaps, splatmaps, minimaps, water fields) is baked locally — it is not in git or the asset dataset. Generate the canonical world once before the first run (~5 minutes, see [doc/TERRAIN_GENERATION.md](doc/TERRAIN_GENERATION.md)):

```bash
cargo run -p terrain-gen --release -- bake --seed 42
```

> **Disk space:** A full-world bake writes 262,144 tiles and currently produces about 73 GB under `data/terrain`. Check available space before running it.

Without this step the server's terrain API returns 404s and the world renders black.

### 4. Running the Server

This project is organized as a **Cargo Workspace**. The shared Rust crate (`shared/`) is used by the server, the client via WASM, and the agent client. Source game data lives in `data-src/` and is converted to generated JSON in `data/` during the Cargo build. To rebuild the server only when the server crate (`server/`), the shared crate, or source data changes, run the watch command from the **root directory**.

```bash
cargo watch -w server -w shared -w data-src -x "run -p onlinerpg-server"
```

The server listens on port 10006 by default. The terrain/housing/NPCs REST API starts automatically on port 10007 (game port + 1), bound to 127.0.0.1 (`--api-bind` to override). Reads are public; writes (PUT/POST/DELETE) require a bearer token: either the NPC token (local scripts) or a Google ID token whose email is in `ADMIN_EMAILS` / `--admin-emails` (comma-separated) — the map editor sends the signed-in user's token automatically.

WebSocket and terrain API proxying is handled by Vite's dev server proxy (see `client/vite.config.ts`), so no separate socat or SSL proxy is needed.

**Google sign-in**: browser login uses Google OAuth. Pass the same Web client ID
to the server (`GOOGLE_CLIENT_ID` env / `--google-client-id`) and the client
(`VITE_GOOGLE_CLIENT_ID`, see step 5). Without it the server runs but rejects
browser logins. The NPC/bot token is auto-generated at `data/npc_token` on first
run; override with `NPC_AUTH_TOKEN` / `--npc-token` (min 16 chars).

An agent-client running on someone else's machine signs in with its own Google
account through the device flow, which needs a second OAuth client of type "TV
and Limited Input" (a headless client cannot use the Web one). Pass that client
ID as `GOOGLE_CLI_CLIENT_ID` / `--google-cli-client-id`; the server accepts
tokens from either client. See [doc/REMOTE_AGENT_CLIENT.md](doc/REMOTE_AGENT_CLIENT.md).


### 5. Running the Client

Binary assets (3D models, music, sounds) are hosted on Hugging Face, not in git.
Fetch them once from the repo root (re-run after `assets.lock` changes):

```bash
bash tools/fetch-assets.sh
```

```bash
cd client
cp .env.example .env.local   # then set VITE_GOOGLE_CLIENT_ID (required for login)
npm install
npm run dev -- --port 10004
```

### 6. Running the Agent Client

Edit `agent-client/data/config.toml` to set the correct port numbers, then run:

```bash
cd agent-client
cargo watch -i "data/prompts/memory/" -x run
```

### 7. Automatic WASM Rebuild on Shared Code Changes (Recommended)
To have Rust code changes in the `shared` library reflected in the browser immediately during client development, run the following command in a separate terminal:

```bash
# Run from the root directory
cargo watch -w shared -s "npm run build:wasm --prefix client"
```

### 8. Running the GLB Editor

```bash
cd tools/glb-editor
npm install
npm run dev -- --port 10005
```

## Production Deployment

Prod runs both binaries as systemd units (`tools/systemd/`), with the client bundle served statically from `/var/www/openmmo`.

| Unit | Binary | Syslog identifier |
|------|--------|-------------------|
| `openmmo-server` | `onlinerpg-server` | `openmmo` |
| `openmmo-agent-client` | `agent-client` | `openmmo-agent` |

Deploy by running `tools/deploy-prod.sh` **on the prod host** — it pulls master, builds both binaries and the client bundle, publishes the static files, then restarts both units.

There is no host-setup script. nginx on prod is a hand-maintained `/etc/nginx/sites-available/openmmo`; keep it in step with `docker/nginx.conf.template`, which is the reference for the cache rules. In particular `/models/` must be served with `Cache-Control: no-cache` — the object catalog and GLBs are fetched by fixed path, and a time-based expiry lets a stale `catalog.json` hide newly added furniture with no error.

The server handles systemd's `SIGTERM` gracefully: it shows connected players a restart notice, closes its listeners and periodic tasks, waits for any in-flight batch save, persists every connected character and inventory plus the world clock, then exits. `systemctl restart` waits for that drain before starting the new binary.

Admin characters can use `/notice <message>` to raise the same banner by hand (this is the live in-game banner, not the login-screen announcements served from `data/announcements/`). `/notice` with no message clears it; players who enter while one is active receive it on join.

Admins also have moderation and movement commands: `/kick <name>` disconnects a player, `/mute <name> [minutes]` silences their chat and whispers (default 10 minutes, survives a relog, cleared by `/unmute <name>` or a server restart), `/summon <name>` teleports them to your side, `/goto <name>` teleports you to theirs, and `/ban <name> [minutes]` blocks the account behind that character from logging in at all — permanently without minutes, and `/unban <name>` lifts it. A ban keys on the account, so deleting and recreating characters does not shed it, and it survives a server restart (unlike a mute).

Over SSH, detach it from the session so a dropped connection cannot kill the build midway:

```bash
ssh prod 'setsid nohup bash ~/work/OnlineRPG/tools/deploy-prod.sh > ~/deploy-latest.log 2>&1 < /dev/null &'
ssh prod 'tail -f ~/deploy-latest.log'   # follow; ends at "==> deployed <commit>"
```

Losing the connection to a foreground run costs the whole build but never a half-deploy: the script builds everything first and only touches live state at the end (`rsync` to the webroot, then the restarts), so an interruption before that leaves the old bundle and the old server process running as a matched pair.

### Logs

Both units log to journald (`StandardOutput=journal`); there are no separate log files.

```bash
journalctl -u openmmo-server -f              # follow the game server
journalctl -u openmmo-agent-client -f        # follow the NPC agent client
journalctl -u openmmo-server -n 200 --no-pager   # recent history
journalctl -u openmmo-server --since "1 hour ago" -p err   # errors only
```

Both binaries build their subscriber from `RUST_LOG`, defaulting to `info` when it is unset or unparseable. Per-action detail — monster spawn/despawn, combat dice/HP/XP rolls — sits at `debug` to keep the journal rate sane at high player counts, so raise the level to see it:

```bash
sudo systemctl edit openmmo-server   # or write /etc/openmmo/server.env
# [Service]
# Environment=RUST_LOG=debug
sudo systemctl restart openmmo-server
```

Each unit reads `EnvironmentFile=-/etc/openmmo/{server,agent-client}.env`, so `RUST_LOG` can go there instead. Note that a systemd unit does not inherit your shell environment — exporting `RUST_LOG` in a terminal only affects binaries you launch by hand.

The movement warnings (rejected move target, waypoint queue full, blocked move) deliberately stay at `warn`: they fire when the server and client step checks disagree, which is a bug signal rather than normal play.

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for how to pick work, the checks CI runs, and PR conventions. When you open your first pull request, a bot will ask you to sign the [Contributor License Agreement](CLA.md) by leaving a comment on the PR. Signing the CLA is required before your contribution can be merged.

To contribute binary assets (3D models, music, sounds), see [doc/ASSETS.md](doc/ASSETS.md) — they go through a Hugging Face dataset PR rather than git.
