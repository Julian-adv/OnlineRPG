# Runtime Performance Optimization (60fps)

## Passability cache: spatial index investigated, not built (2026-07-21)

### Question

`PassabilityCache` is a `HashMap<String, RuntimePassability>` with no spatial
index (`shared/src/pathfinding/mod.rs`). Every collision query iterates **all**
entries and AABB-rejects each. With a 5,000-concurrent-user target, does this
need a region-bucketed index?

### Answer: no. Do not build one yet.

### Findings

**The cache holds 9 entries, not ~170.**

| Source | Files on disk | Cache entries |
|---|---|---|
| `data/terrain/objects/*.json` | 162 | **3** |
| `data/housing/*/*.json` | 5 | **5** |
| `data/dungeons.json` | 1 | **1** |

A region file only leaves an entry if it holds *solid* furniture — 9 types have
footprints (`data/furniture_footprints.json`); everything else is decorative and
`sync_region_furniture` removes the key. Measured: 282 placements across all 162
files, of which **10 are solid, in 3 regions**.

> The boot log used to report *files parsed* (`162 furniture regions`),
> overstating the cache 54× and misleading this very investigation. Fixed to
> count entries.

**Entry count does not scale with players.** All three sources are
admin-authored: housing REST writes go through `require_admin_for_writes`
(`server/src/main.rs`), furniture regions come from map-editor saves, dungeons
from `data/dungeons.csv`. 5,000 users produce the same 9 entries.

**The server never runs A\*.** Zero `find_path` call sites in `server/src`.
Monster AI is owner-authoritative and runs client-side
(`monsterManager.ai_tick_brain`), so each browser pathfinds only for its own
handful of monsters. The "up to 8,000 cache scans per path query" cost of A\*
(2,000 nodes × 4 neighbours) lands on individual tabs, never on the server.

**Server cost is small.** `tick_player_movement` runs at 5 Hz
(`server/src/main.rs`), skips non-moving players, and costs ~3–6 scans of 9
entries per moving player per tick. 5,000 simultaneous movers ≈ under 2M float
comparisons/sec total.

`collision_y` (`server/src/game_state/passability.rs`) accounts for one of
those scans — it derives the Y to collide against rather than trusting the
client's. It costs a second scan only for a leg that crosses a floor grid,
i.e. one near a building or solid furniture; anyone walking open ground
returns early before the stairwell check.

### Revisit when any of these becomes true

1. **More furniture types get footprints.** A furniture entry's AABB is the
   union of every solid piece across a **1024 m** region. Today that spans ≤14 m,
   so AABB rejection works. Marking common decorative objects solid inflates
   those AABBs toward full-region size and **AABB rejection stops rejecting** —
   this is the real trigger, and it is a content decision, not a load one.
2. **Housing opens to player building.** Houses are one entry each and are
   currently admin-gated with no per-player cap. Removing the gate makes entry
   count grow with player count — the only unbounded axis.
3. **Dungeon count grows a lot.** Each entry is 80×80 m with 5–20 floors of
   6,400 cells (up to ~128 KB). One shared instance per entrance, so players
   don't multiply them; authored content does.

### Related fix

The entry counts above are the server's. The **browser** streamed content in as
the player walked and never dropped any of it, so its copy of the cache grew
without bound — the one place entry count did scale with play time.

- **Furniture.** One region is loaded at a time. `furnitureManager.evictDistant`
  keeps the current region **plus its 8 neighbours** — evicting the region just
  crossed out of would drop collision for furniture right across a boundary that
  the server still blocks, desyncing prediction at region seams.
- **Houses.** The larger axis: `loadChunksAround` streams a 3x3 terrain-chunk
  neighbourhood, and `passability_remove_house` only ever ran on explicit
  delete, so crossing a town leaked one entry per house for the whole session.
  `housingManager.evictDistantChunks` sweeps at **radius 2**, one chunk wider
  than the load radius so a player loitering on a boundary can't thrash a chunk
  in and out. It drops the `chunkCache` key itself, not just the houses in it —
  `ensureChunkLoaded` reads a present key as "already loaded" and would
  otherwise never refetch. Since nothing inside a fresh spawn's 3x3 load is ever
  dropped, this can't show less than spawning in place would.

Dungeons already self-evict by distance (`dungeonManager.updateAutoRegister`,
gated on being back on the surface).

Two things did **not** generalise, for different reasons:

- **The sweep itself.** A shared Rust-side pass over the wasm cache would
  desync all three producers: each keeps a JS-side "already loaded" guard —
  `chunkCache.has(key)`, `lastLoadedRegion`, `dungeonManager.id` — that a silent
  removal from wasm never clears, stranding the content unloadable for the rest
  of the session. Eviction has to run through the manager that owns the guard.
- **The load/evict pairing.** This one is a convention rather than shared code:
  each streaming producer exposes *one* position-driven entry point that owns
  its own ordering — `housingManager.updateStreaming`,
  `dungeonManager.updateFromPlayerPosition`. Leaving callers to pair the two
  halves is how the "load first, evict after" rule ended up rediscovered and
  re-commented at each call site.

## Round 2 (2026-04-09)

### Problem

Same heavy scene at DPR 1.5 showing 53-57fps. Loop profiler revealed game loop CPU work was only ~1.2ms — the remaining ~17ms was entirely GPU-bound (Threlte main render + CSM shadow passes).

### Results

| Optimization | avgDelta | FPS |
|---|---|---|
| Baseline (DPR 1.5, MSAA, shadow 2048x2, placeholder at origin) | 18.13ms | 53-57 |
| + Alternate-frame refraction/reflection | -- | 55-58 |
| + DPR 1.0 | 17.54ms | 55-58 |
| + Placeholder PointLight offscreen after compile | 17.24ms | 56-58 |
| + Disable MSAA | ~16.5ms | 60 |
| + Restore native DPR 1.5, CSM 2→1 | -- | 60 |
| + Restore CSM 2 + shadow 2048 (final) | -- | 60 |

### Changes

#### 1. Disable MSAA, Cap DPR (biggest win)

**File**: `client/src/lib/utils/renderer.ts`

WebGPU MSAA 4x resolve cost ~0.7ms/frame. At native DPR 1.5, the higher pixel density provides sufficient edge quality without MSAA. DPR capped at 1.5 to prevent excessive resolution on ultra-high-DPI displays.

#### 2. Placeholder PointLight Offscreen After Compilation

**File**: `client/src/lib/components/GameScene.svelte`

The intensity=0 placeholder PointLight (for pipeline pre-compilation) rendered a 6-face cube shadow map (512x512) every frame even though it contributed nothing visually. After `isSceneCompiling` becomes false, the light moves to `OFFSCREEN_Y` so its shadow frustum captures no objects.

#### 3. Alternate-Frame Refraction/Reflection

**File**: `client/src/lib/components/game-scene/multi-pass-rendering.ts`

Instead of rendering both refraction and reflection every frame, they alternate: even frames render refraction, odd frames render reflection. Previous frame's texture is reused. Water effects change slowly enough that one-frame latency is invisible. First frame after warmup renders both to initialize textures.

#### 4. Alternate-Frame Grass Compute

**File**: `client/src/lib/components/game-scene/GameSceneGrassLayer.svelte`

Up to 27 `renderer.compute()` dispatches per frame (3 grass types × 9 sub-chunks). Now dispatched every other frame. Wind animation is time-based so skipping frames produces no visual discontinuity.

#### 5. Alternate-Frame Wetness Capture

**File**: `client/src/lib/components/game-scene/GameSceneWaterLayer.svelte`

Wetness capture+decay (2 render calls per water tile) now runs every other frame. Water material uniforms still update every frame to maintain wave animation quality. Decay formula uses actual dt, so skipping frames is mathematically safe.

#### 6. Cache computeSunLightSnapshot

**File**: `client/src/lib/components/GameScene.svelte`

Was called twice per frame with identical arguments (once for water uniforms, once for scene lighting). Now computed once and passed to both consumers.

#### 7. Pre-allocate ReflectionRenderManager Color

**File**: `client/src/lib/managers/reflectionRenderManager.ts`

`new THREE.Color()` was allocated in `render()` and `clear()` every call to save/restore clear color. Replaced with a pre-allocated instance field `_savedClearColor`.

### Key Findings

- **MSAA is expensive on WebGPU**: 4x MSAA resolve added ~0.7ms/frame. On DPR ≥ 1.5 displays, native resolution provides adequate anti-aliasing without MSAA.
- **Zero-intensity lights still render shadows**: Three.js does not skip shadow map rendering for lights with intensity=0. A PointLight shadow = 6 cube face renders per frame.
- **DPR matters less than expected**: Reducing DPR from 1.5→1.0 (2.25x fewer pixels) only saved ~0.6ms, confirming the bottleneck was draw call / vertex processing, not fragment fill rate.
- **Alternate-frame rendering is free for slow-changing effects**: Water refraction/reflection, wetness decay, and grass wind animation all tolerate one-frame latency with no visible artifacts.

---

## Round 1

### Problem

Heavy scene (4-story buildings x2, 1-story houses x3, trees, grass, character) showing 55fps instead of target 60fps. Frame budget: 16.67ms, actual: ~18.2ms — needed to save ~1.5ms per frame.

### Results

| Optimization | FPS | Improvement |
|---|---|---|
| Baseline | 55 | -- |
| + Dynamic grass compute count | 57 | +2 |
| + Remove terrain castShadow | 58-60 | +2 |
| + Remove door/shutter castShadow | 60 (stable) | +1 |

### Render Pipeline (per frame)

The game renders multiple passes per frame:

1. **Update logic** (CPU): player, animations, grass compute dispatch, housing detection
2. **Wetness pass**: 256x256 RT per water tile (negligible)
3. **Refraction pass**: half-res render -- terrain + housing (hides water, entities, grass, trees)
4. **Reflection pass**: half-res render -- entities only (hides terrain, water, housing, grass, trees)
5. **Shadow pass**: CSM 2 cascades x 2048x2048 -- all castShadow objects
6. **Main render**: full scene at full resolution

### Changes

#### 1. Dynamic Grass Compute Dispatch Count

**File**: `client/src/lib/components/game-scene/GameSceneGrassLayer.svelte`

**Problem**: Grass wind simulation uses GPU compute shaders dispatched per sub-chunk (3x3 grid = 9 sub-chunks x 3 types = up to 27 dispatches). Each dispatch was fixed at full buffer capacity (131,072 for short/tall grass, 2,048 for flowers) regardless of actual blade count. If a sub-chunk had 5,000 blades, 126,000 GPU threads were wasted.

**Fix**: Set `computeUpdate.count` to actual blade count before each dispatch.

```typescript
// Before: always dispatches capacity (131K) threads
renderer.compute(slot.ctx.computeUpdate)

// After: dispatch only actual blade count
;(slot.ctx.computeUpdate as { count: number }).count = slot.ctx.count
renderer.compute(slot.ctx.computeUpdate)
```

Three.js `ComputeNode.count` is dynamically writable (`ComputeNode.js:setCount()`). The buffer stays allocated at full capacity, but only active indices are processed. Safe because unused indices' output is never read (`mesh.count` limits rendering).

#### 2. Remove Terrain Shadow Casting

**File**: `client/src/lib/components/SplatTerrain.svelte`

**Problem**: All 9 splat terrain tiles had `castShadow = true`. Terrain is mostly flat ground -- it doesn't need to cast shadows onto other objects. Each tile was rendered into the shadow map for both CSM cascades = 18 unnecessary shadow draw calls.

**Fix**: Remove `castShadow` from SplatTerrain, keep `receiveShadow` so terrain still receives shadows from trees, buildings, etc.

#### 3. Remove Door/Shutter Shadow Casting

**File**: `client/src/lib/utils/house-geo-walls.ts`

**Problem**: Every door panel and window shutter was an individual mesh with `castShadow = true`. A 4-story building can have many doors and windows, each creating 1-2 shadow draw calls x 2 CSM cascades. These tiny objects produce shadows invisible in isometric view.

**Fix**: Remove `castShadow` from door panels and window shutters. The building walls (merged meshes) still cast shadows normally.

### Profiling Tools

- **Loop profiler**: `GameScene.svelte` has a built-in profiler tracking per-section CPU time (grassUpdate, refractionPass, reflectionPass, housingUpdate, etc.). Enable via `setLoopProfileEnabled(true)` on the game scene context. Output goes to browser console as `[LoopProfile]` grouped tables every 1 second.
- **Browser DevTools**: Chrome Performance tab shows GPU timing. Look for long "GPU" blocks in the flame chart.
- **renderer.info**: Three.js renderer exposes draw call and triangle counts per render call (auto-resets each `render()`).

### Key Findings

- **GPU-bound, not CPU-bound**: CPU-side optimizations (skipping render pass submissions, reducing JS work) had minimal impact. GPU workload reduction (fewer compute threads, fewer shadow draw calls) had direct impact.
- **Shadow maps are expensive**: CSM with 2 cascades doubles shadow rendering. Every `castShadow = true` mesh is rendered once per cascade. Small objects (doors, shutters) and flat geometry (terrain) should not cast shadows unless visually necessary.
- **Compute dispatch count matters**: WebGPU compute dispatches process all threads up to the specified count. If actual data is a fraction of buffer capacity, most GPU threads run on empty data. Always set dispatch count to actual work size.
