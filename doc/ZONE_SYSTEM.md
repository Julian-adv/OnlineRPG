# Per-Region Spawn Zones & Town No-Spawn Zones

## Context
Monster spawns were previously defined in a monolithic `world.json`. As the world grows with many monster types and regions, this doesn't scale. This system:
1. Moves spawn rules to per-region zone files (`data/terrain/zones/r{X}_{Z}.json`)
2. Adds rectangular **no-spawn zones** (towns) to the same zone files
3. Adds map editor tools to draw town rectangles and spawn areas

Zone data is kept separate from terrain meta (`data/terrain/meta/`) to avoid coupling gameplay logic with rendering config.

## Zone File Schema

`data/terrain/zones/r-02_+00.json`:
```json
{
  "monsterSpawns": [
    { "monsterType": "scp939", "maxPerPlayer": 3, "maxTotal": 10, "spawnIntervalSecs": 30,
      "minX": -1560.0, "minZ": 410.0, "maxX": -1480.0, "maxZ": 490.0 }
  ],
  "noSpawnZones": [
    { "minX": -1600.0, "minZ": 400.0, "maxX": -1500.0, "maxZ": 500.0, "label": "Starting Town" }
  ]
}
```
Both arrays are optional (default empty). Coordinates are world-space. Both zone types use the same rectangular `minX/minZ/maxX/maxZ` format.

## Architecture

### Shared crate (`shared/src/lib.rs`)
- `NoSpawnZone` struct with `#[serde(rename_all = "camelCase")]` — used directly for both Rust deserialization and JSON serialization (no intermediate struct needed)
- `NoSpawnZone::contains(x, z)` helper for point-in-rect checks

### Terrain crate (`terrain/src/`)
- `coords::zone_path()` — `{base}/zones/r{+XX}_{+ZZ}.json`
- `io::TerrainIO` — `list_zone_regions()`, `read_zone()`, `write_zone()`

### Server
- `world_config.rs` — `MonsterSpawnRule` with rectangular bounds + `maxTotal`, `load_spawn_config_from_regions()` reads all zone files at startup
- `game_state/mod.rs` — `no_spawn_zones` field, constructor takes spawn rules + zones as params
- `game_state/ambient_spawn.rs` — the only consumer: a spawn point inside a zone (+ margin) is dropped
- `terrain/routes.rs` — `GET/PUT /api/terrain/zones/{rx}/{rz}`
- `main.rs` — loads zones from region files at startup, passes to `GameState::new()`

### Clients
Zones never reach a client: the server places every ambient monster itself
(`ambient_spawn.rs`), so no client validates spawn positions any more.

### Map editor
- `stores/editorStore.ts` — `EditorTool` includes `'zone'`, shared stores for zone sub-tool, draw state, form values (`spawnFormMonsterType`, `spawnFormMaxPerPlayer`, etc.), `currentZoneData` (shared between panel and overlay)
- `managers/zoneManager.ts` — `ZoneManager` class for load/save via `/api/terrain/zones/{rx}/{rz}`
- `map-editor/ZoneBrushPanel.svelte` — No-Spawn / Spawn sub-tools with zone list (hover highlights overlay), delete, form inputs for spawn params, draw instructions
- `map-editor/ZoneOverlay.svelte` — terrain-conforming overlays (samples heightmap per cell), red for no-spawn, blue for spawn, yellow preview while drawing, white highlight on hover from panel list. Static zone geometries separated from preview for efficiency, with proper `dispose()` on cleanup.
- `map-editor/MapEditorCursor.svelte` — two-click rectangle drawing, reads spawn params from shared stores (not hardcoded)
- `map-editor/MapEditorPanel.svelte` — Zone tab
- `GameScene.svelte` — creates `ZoneManager`, renders `ZoneOverlay` in editor mode
- `App.svelte` — hides `ChatPanel` and `CharacterAttributesHud` in map editor mode

## Notes
- **Cross-region zones**: A zone drawn in region A may cover region B territory. Server aggregates all zones into a flat list so validation works. Editor shows zones stored in the current region only.
- **Hot-reload**: After editor saves a zone, server won't see it until restart. Acceptable for v1.
