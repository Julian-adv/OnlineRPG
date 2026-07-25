//! Procedural nethack-style dungeon generation, shared verbatim between
//! the native server and the WASM web client. Layouts are fully
//! deterministic from a seed (derived from the entrance id), so neither
//! side ever sends geometry over the wire — both regenerate the same
//! rooms, corridors and stair shafts and only entity state is networked.
//!
//! Determinism rules (enforced by the golden-hash test in `tests`):
//! - RNG is `ChaCha8Rng` only. `SmallRng` is a different algorithm on
//!   wasm32 vs 64-bit native and must never be used here.
//! - No `HashMap`/`HashSet` iteration, no platform-dependent float math
//!   in anything that influences the layout.
//!
//! Coordinate model: each floor is a `GRID`×`GRID` field of 1m cells
//! centered on the dungeon entrance. Depth `d` (1-based) lives at world
//! `y = entrance.y - d * DUNGEON_FLOOR_HEIGHT` and registers in the
//! passability cache as floor index `passability_floor_for_depth(d)`.
//! The offset keeps dungeon *interior* floors clear of housing floor
//! levels 0-3 — `is_cardinal_move_blocked` matches by floor index alone,
//! so reusing 0..3 would make dungeon walls block players walking on the
//! surface above the dungeon footprint. Index 0 itself is the surface
//! (see `surface_passability_cells`), which is genuinely that floor.
//!
//! Floors connect through 2×`SHAFT_LEN` stair shafts that occupy the
//! same cells on both adjacent floors; the entry landing belongs to the
//! shallower floor, the exit landing to the deeper one, and A* walks
//! them via the existing housing stairwell intermediate-key machinery.

mod doors;
mod gen;
mod registry;
mod stairs;
#[cfg(test)]
mod tests;

pub use doors::{closed_door_segs, interior_doors, InteriorDoorSpec};
pub use registry::{entrance, entrance_at, entrances, footprint_contains, DungeonEntranceDef};
pub use stairs::{
    entrance_ramp_height_at, floor_height_at, ground_y_for_floor, shaft_run_pos, LANDING_CELLS,
};

use serde::Serialize;
use std::collections::HashSet;
use std::sync::LazyLock;

use crate::pathfinding::{
    PassabilityCache, RuntimeFloorGrid, RuntimePassability, StairwellInfo, EDGE_E, EDGE_N, EDGE_S,
    EDGE_W,
};
use crate::world::Position;

/// Side length of a dungeon floor in 1m cells.
pub const GRID: i32 = 80;
const HALF_GRID: i32 = GRID / 2;

/// Vertical distance between consecutive dungeon floors.
pub const DUNGEON_FLOOR_HEIGHT: f32 = 4.0;

/// Collision wall height registered in the passability grids. Kept below
/// `DUNGEON_FLOOR_HEIGHT` so the depth-1 Y window tops out 1m under the
/// entrance and never captures players walking on the surface above.
pub const DUNGEON_WALL_HEIGHT: f32 = 3.0;

/// Passability floor index of depth 1. Sits one slot above the housing
/// range (`0..=housing::MAX_FLOOR_LEVEL`) so the two systems can never
/// collide in floor-keyed collision queries. Derived from the housing max
/// so growing housing shifts the dungeon range automatically — this index
/// is recomputed at runtime and never persisted, so the shift is safe.
pub const DUNGEON_FLOOR_INDEX_BASE: u8 = crate::housing::MAX_FLOOR_LEVEL + 1;

pub const MIN_DEPTH: u8 = 5;
pub const MAX_DEPTH: u8 = 20;

/// Stair shaft footprint: `SHAFT_W` cells wide, `SHAFT_LEN` along the run.
/// The run must stay ≤ 16 cells — stairwell intermediate floor keys get
/// `FLOOR_SCALE = 16` slots between two regular floors.
pub const SHAFT_W: i32 = 2;
pub const SHAFT_LEN: i32 = 8;

/// Monster type spawned on the final floor next to the treasure chest.
/// Global single-dungeon shortcut; when a second dungeon lands this belongs
/// in dungeons.csv next to `floors` (see `registry`).
pub const BOSS_MONSTER_TYPE: &str = "goblin_boss";

/// How far a player's Y may sit from a floor's world Y and still be accepted
/// as standing on it. Part of the wire contract: a client declaring a dungeon
/// floor is refused outside this band (`validated_dungeon_floor`), and any
/// client computing its own Y underground must stay inside it.
pub const FLOOR_Y_TOLERANCE: f32 = 2.5;

/// A* node budget for long in-dungeon path queries. Maze floors plus the
/// open-surface leak through the entrance stairwell can exhaust the
/// housing default (2000) on cross-floor routes; short chase paths are
/// unaffected. Searches that exhaust the budget still return a partial
/// path toward the goal.
pub const DUNGEON_PATH_MAX_NODES: usize = 20000;

/// How far (in cells, Chebyshev distance) to search outward from a kill
/// for walkable floor when a loot drop would otherwise land in a wall.
/// Rooms are larger than this, so a carved cell is always found well
/// before the limit; it's only a guard against pathological geometry.
const DROP_SEARCH_RING_MAX: i32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Room {
    pub x: i32,
    pub z: i32,
    pub w: i32,
    pub d: i32,
}

impl Room {
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.z + self.d / 2)
    }

    /// Half-open cell membership, mirrored by the client's `rectContains`.
    pub fn contains(&self, x: i32, z: i32) -> bool {
        x >= self.x && x < self.x + self.w && z >= self.z && z < self.z + self.d
    }

    fn expanded(&self, by: i32) -> Room {
        Room {
            x: self.x - by,
            z: self.z - by,
            w: self.w + by * 2,
            d: self.d + by * 2,
        }
    }

    fn intersects(&self, other: &Room) -> bool {
        self.x < other.x + other.w
            && self.x + self.w > other.x
            && self.z < other.z + other.d
            && self.z + self.d > other.z
    }
}

/// A vertical stair shaft connecting two adjacent floors. The footprint
/// is identical on both floors; `reversed` selects which physical end is
/// the entry landing (on the shallower floor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StairShaft {
    /// Min-corner cell of the footprint.
    pub x: i32,
    pub z: i32,
    /// Run axis: true = along +Z, false = along +X.
    pub along_z: bool,
    /// false = entry (shallow) landing at the min end, true = at the max end.
    pub reversed: bool,
}

impl StairShaft {
    pub fn rect(&self) -> Room {
        if self.along_z {
            Room {
                x: self.x,
                z: self.z,
                w: SHAFT_W,
                d: SHAFT_LEN,
            }
        } else {
            Room {
                x: self.x,
                z: self.z,
                w: SHAFT_LEN,
                d: SHAFT_W,
            }
        }
    }

    pub fn contains(&self, x: i32, z: i32) -> bool {
        self.rect().contains(x, z)
    }

    /// Cell at run position `i` (0 = entry end), lateral offset `w`.
    pub fn step_cell(&self, i: i32, w: i32) -> (i32, i32) {
        let run = if self.reversed { SHAFT_LEN - 1 - i } else { i };
        if self.along_z {
            (self.x + w, self.z + run)
        } else {
            (self.x + run, self.z + w)
        }
    }

    /// Entry landing cell (shallower floor), first lateral column.
    pub fn entry_cell(&self) -> (i32, i32) {
        self.step_cell(0, 0)
    }

    /// Exit landing cell (deeper floor), first lateral column.
    pub fn exit_cell(&self) -> (i32, i32) {
        self.step_cell(SHAFT_LEN - 1, 0)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnSpec {
    pub x: i32,
    pub z: i32,
    pub monster_type: String,
    pub is_boss: bool,
    /// Proactive (선공형) monster: attacks players on sight instead of only
    /// retaliating when hit. Designated per entry in [`spawn_table`].
    pub aggressive: bool,
}

/// Clutter prop dropped into a room. Every kind but [`PropKind::TorchWall`]
/// becomes a 1×1 collision pillar in the passability grid (see
/// [`floor_passability_cells_full`]), so a mover routes around it and a path
/// can never end on its cell — unlike the treasure chest, which carries no
/// collision. Breaking a barrel or crate opens its cell again. The string
/// variants match the object-catalog ids (`barrel`/`crate`/`chest`/`torch_wall`)
/// the client loads the GLB for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PropKind {
    Barrel,
    Crate,
    Chest,
    /// Wall-mounted torch: hangs on a room's north or east wall (one per room).
    /// Never spawned by [`PropKind`] clutter rolls — placed by its own pass.
    TorchWall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropSpec {
    pub x: i32,
    pub z: i32,
    pub kind: PropKind,
    /// How many of `kind` are stacked vertically (1 or 2). Chests never stack.
    pub stack: u8,
    /// Yaw in whole degrees (0..360). Meaning depends on `kind`: for clutter
    /// (barrel/crate/chest) it's a random jitter for variety; for `TorchWall`
    /// it's the room-facing direction the client mounts it by (north wall → 0,
    /// east wall → 270).
    pub rotation: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FloorLayout {
    /// 1-based depth below the surface.
    pub depth: u8,
    pub rooms: Vec<Room>,
    /// GRID*GRID row-major walkability (rooms + corridors + shafts).
    pub carved: Vec<bool>,
    /// Shaft arriving from the floor above (or from the surface at depth 1).
    pub up_shaft: StairShaft,
    /// Shaft descending to the next floor; `None` on the final floor.
    pub down_shaft: Option<StairShaft>,
    /// Treasure chest cell, only on the final floor.
    pub chest: Option<(i32, i32)>,
    pub spawns: Vec<SpawnSpec>,
    /// Decorative barrels/crates/chests clustered in room corners. Cosmetic
    /// only (no collision) — see [`PropSpec`].
    pub props: Vec<PropSpec>,
}

impl FloorLayout {
    pub fn is_carved(&self, x: i32, z: i32) -> bool {
        (0..GRID).contains(&x) && (0..GRID).contains(&z) && self.carved[(x + z * GRID) as usize]
    }

    /// Room covering this cell. `None` in a corridor or on a stair landing
    /// that no room's rect reaches.
    pub fn room_at(&self, x: i32, z: i32) -> Option<&Room> {
        self.rooms.iter().find(|r| r.contains(x, z))
    }

    /// Pick a walkable world position to drop loot near a monster's death
    /// spot, so the item never lands inside a wall. Pickup is a pure
    /// proximity check (no pathfinding), so an item in an uncarved cell can
    /// be unreachable — a player can never get close enough through the
    /// wall.
    ///
    /// `preferred` is the desired scatter point (the death position plus a
    /// random offset). If it already sits on carved floor it's kept as-is.
    /// Otherwise the search widens in Chebyshev rings around the *death*
    /// cell and snaps to the carved cell whose center is nearest `preferred`,
    /// finally falling back to the death cell itself — the monster stood
    /// there, so it is carved.
    pub fn walkable_drop_position(
        &self,
        entrance: &Position,
        death: &Position,
        preferred: &Position,
    ) -> Position {
        let surface_y = floor_world_y(entrance.y, self.depth);

        // 1. Keep the scattered point when it already lands on floor.
        let (px, pz) = world_to_cell(entrance, preferred.x, preferred.z);
        if self.is_carved(px, pz) {
            return Position {
                x: preferred.x,
                y: surface_y,
                z: preferred.z,
            };
        }

        // 2. Widen outward from the death cell; at each ring snap to the
        //    carved cell whose center is closest to the preferred point so
        //    the drop still trends in the scatter direction.
        let (dcx, dcz) = world_to_cell(entrance, death.x, death.z);
        for ring in 1..=DROP_SEARCH_RING_MAX {
            let mut best: Option<((i32, i32), f32)> = None;
            for cz in (dcz - ring)..=(dcz + ring) {
                for cx in (dcx - ring)..=(dcx + ring) {
                    // Only the perimeter cells are new at this ring.
                    if (cx - dcx).abs() != ring && (cz - dcz).abs() != ring {
                        continue;
                    }
                    if !self.is_carved(cx, cz) {
                        continue;
                    }
                    let c = cell_center(entrance, self.depth, (cx, cz));
                    let d2 = (c.x - preferred.x).powi(2) + (c.z - preferred.z).powi(2);
                    if best.is_none_or(|(_, b)| d2 < b) {
                        best = Some(((cx, cz), d2));
                    }
                }
            }
            if let Some((cell, _)) = best {
                return cell_center(entrance, self.depth, cell);
            }
        }

        // 3. Fallback: the death cell itself (the monster occupied it).
        cell_center(entrance, self.depth, (dcx, dcz))
    }
}

/// FNV-1a 64 over the entrance id. Implemented inline because
/// `DefaultHasher` is not stable across Rust releases and the seed must
/// match between independently-built server and client binaries.
pub fn dungeon_seed(entrance_id: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in entrance_id.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Total floor count for a dungeon, 5..=20, derived from the seed.
#[cfg(test)]
pub(crate) fn dungeon_depth(seed: u64) -> u8 {
    gen::dungeon_depth(seed)
}

/// Generate every floor of the dungeon. Cheap enough (≤20 grids of 56×56
/// cells) that callers always generate the full dungeon and index into it.
/// Test-only: real dungeons must go through `generate_dungeon_for` so the
/// csv floor override applies; this seed-only form exists for property tests
/// over arbitrary seeds.
#[cfg(test)]
pub(crate) fn generate_dungeon(seed: u64) -> Vec<FloorLayout> {
    gen::generate_dungeon_with(seed, None)
}

/// Generate a dungeon by entrance id: seed derived from the id, floor count
/// from the registry's `floors` override when present. This is what both the
/// server and the wasm client use for real dungeons; `generate_dungeon` stays
/// seed-only for property tests over arbitrary seeds.
pub fn generate_dungeon_for(entrance_id: &str) -> Vec<FloorLayout> {
    let floors = entrance(entrance_id).and_then(|d| d.floors);
    gen::generate_dungeon_with(dungeon_seed(entrance_id), floors)
}

pub fn passability_floor_for_depth(depth: u8) -> u8 {
    DUNGEON_FLOOR_INDEX_BASE + depth - 1
}

/// Map an entity's wire `floor_level` onto its passability floor index.
/// The wire encodes dungeons as negative depth, while the passability cache
/// stacks them above housing — so the two disagree exactly underground.
pub fn passability_floor_for_level(floor_level: i8) -> u8 {
    if floor_level < 0 {
        passability_floor_for_depth(floor_level.unsigned_abs())
    } else {
        floor_level as u8
    }
}

/// Inverse of [`passability_floor_for_level`]: recover the wire `floor_level`
/// a passability floor index stands for. Path waypoints carry cache indices,
/// so anything turning a path back into move packets needs this.
pub fn floor_level_for_passability(floor: u8) -> i8 {
    if floor >= DUNGEON_FLOOR_INDEX_BASE {
        -((floor - DUNGEON_FLOOR_INDEX_BASE + 1) as i8)
    } else {
        floor as i8
    }
}

/// A* node budget for a search between two passability floors. Dungeon routes
/// are long mazes that also leak into the open surface through the entrance
/// shaft, and exhaust the housing default; anything above ground does not.
/// Lives here so every caller — agent, browser and the shared monster brains —
/// gets the same budget instead of each remembering the rule.
pub fn path_max_nodes(start_floor: u8, goal_floor: u8) -> usize {
    if start_floor >= DUNGEON_FLOOR_INDEX_BASE || goal_floor >= DUNGEON_FLOOR_INDEX_BASE {
        DUNGEON_PATH_MAX_NODES
    } else {
        crate::pathfinding::DEFAULT_MAX_NODES
    }
}

pub fn floor_world_y(entrance_y: f32, depth: u8) -> f32 {
    entrance_y - depth as f32 * DUNGEON_FLOOR_HEIGHT
}

/// World min-corner of the cell grid. Floored so cell edges sit on
/// integer world coordinates like housing grids do.
pub fn dungeon_origin(entrance_x: f32, entrance_z: f32) -> (f32, f32) {
    (
        entrance_x.floor() - HALF_GRID as f32,
        entrance_z.floor() - HALF_GRID as f32,
    )
}

/// World-space center of a grid cell.
pub fn cell_center(entrance: &Position, depth: u8, cell: (i32, i32)) -> Position {
    let (ox, oz) = dungeon_origin(entrance.x, entrance.z);
    Position {
        x: ox + cell.0 as f32 + 0.5,
        y: floor_world_y(entrance.y, depth),
        z: oz + cell.1 as f32 + 0.5,
    }
}

/// Grid cell containing a world-space XZ position (inverse of `cell_center`).
pub fn world_to_cell(entrance: &Position, x: f32, z: f32) -> (i32, i32) {
    let (ox, oz) = dungeon_origin(entrance.x, entrance.z);
    ((x - ox).floor() as i32, (z - oz).floor() as i32)
}

/// Passability cache key for a dungeon (one entry covers every floor).
pub fn dungeon_cache_key(entrance_id: &str) -> String {
    format!("dungeon:{entrance_id}")
}

/// One weighted entry in a depth's spawn table. `aggressive` makes that dungeon
/// spawn proactive (선공형 — attacks on sight). Sourced per monster from the
/// `dungeonMinDepth`/`dungeonMaxDepth`/`dungeonWeight`/`dungeonAggressive`
/// columns of `data-src/monsters.csv`.
#[derive(Debug, Clone)]
pub struct SpawnEntry {
    pub monster_type: String,
    pub weight: u32,
    pub aggressive: bool,
}

/// Per-depth spawn tables indexed by depth (`0..=MAX_DEPTH`), built once from
/// the monster table. The dungeon generator runs in the shared crate on both
/// native (server) and wasm32 (client), so the data is baked in at compile
/// time via `include_str!` — runtime file IO would risk desync. We read the
/// SOURCE csv directly (not the generated `data/monsters.json`) because that
/// JSON is produced by a build script whose ordering relative to this crate
/// isn't guaranteed; reading the csv keeps a `cargo build` after a csv edit
/// self-consistent. Entries stay in csv row order — stable and identical on
/// both sides — which the weighted pick in `roll_spawns` relies on.
static SPAWN_TABLES: LazyLock<Vec<Vec<SpawnEntry>>> =
    LazyLock::new(|| build_spawn_tables(include_str!("../../../data-src/monsters.csv")));

fn build_spawn_tables(csv: &str) -> Vec<Vec<SpawnEntry>> {
    let mut tables: Vec<Vec<SpawnEntry>> = vec![Vec::new(); MAX_DEPTH as usize + 1];
    let mut lines = csv.lines();
    let Some(header) = lines.next() else {
        return tables;
    };
    let cols: Vec<&str> = header.split(',').map(str::trim).collect();
    let col = |name: &str| {
        cols.iter()
            .position(|c| *c == name)
            .unwrap_or_else(|| panic!("monsters.csv missing `{name}` column"))
    };
    let (id_col, min_col, max_col, weight_col, aggr_col) = (
        col("id"),
        col("dungeonMinDepth"),
        col("dungeonMaxDepth"),
        col("dungeonWeight"),
        col("dungeonAggressive"),
    );

    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        let field = |i: usize| fields.get(i).map(|s| s.trim()).unwrap_or("");
        // Only rows with a min depth are dungeon spawns; the boss is placed
        // separately (see `roll_spawns`) and leaves these columns blank.
        let Ok(min) = field(min_col).parse::<u8>() else {
            continue;
        };
        // Blank (or anything past the deepest floor) means "down to the
        // bottom"; values above MAX_DEPTH clamp rather than extend it.
        let max = field(max_col)
            .parse::<u8>()
            .unwrap_or(MAX_DEPTH)
            .min(MAX_DEPTH);
        let weight = field(weight_col).parse::<u32>().unwrap_or(1).max(1);
        let entry = SpawnEntry {
            monster_type: field(id_col).to_string(),
            weight,
            aggressive: field(aggr_col) == "true",
        };
        for depth in min..=max {
            tables[depth as usize].push(entry.clone());
        }
    }
    tables
}

/// Weighted monster entries that can spawn at `depth`, in stable csv order, or
/// an empty slice if none cover it. Tune via the `dungeon*` columns of
/// monsters.csv.
pub fn spawn_table(depth: u8) -> &'static [SpawnEntry] {
    SPAWN_TABLES
        .get(depth as usize)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// Effective monster level at a given depth. Shallow floors use the
/// definition level untouched; below depth 4 monsters gain +1 level per
/// two floors, capped at 20.
pub fn monster_level_for_depth(def_level: u8, depth: u8) -> u8 {
    if depth <= 4 {
        def_level
    } else {
        (def_level as u32 + (depth as u32 - 4) / 2).min(20) as u8
    }
}

/// All four edge bits set: a fully sealed, impassable cell. Used to turn a
/// decorative prop's cell into a 1×1 collision pillar (see `roll_props`).
pub(crate) const EDGE_ALL: u8 = EDGE_N | EDGE_E | EDGE_S | EDGE_W;

/// Edge-bitmask cells for one floor, derived from its carved mask plus walls
/// turning each stair shaft into a dead-end whose only opening on this floor is
/// the landing this floor stands on. The shaft footprint sits inside a room, so
/// without the far landing's run-end walled, A* treats it as a cut-through and
/// marches a same-floor monster across it onto steps that render at the *other*
/// floor's height; the side walls also keep a descending player from stepping
/// sideways off the stairs mid-run. The steps themselves stay open along the run
/// so a descending player (collision-checked against this floor once their Y
/// drops into range) still walks through.
pub fn floor_passability_cells(layout: &FloorLayout) -> Vec<u8> {
    floor_passability_cells_inner(layout, &[], &[])
}

/// OR an edge bit into a floor cell, ignoring out-of-grid coordinates. Shared by
/// the shaft-walling and door-sealing passes, both of which seal grid boundaries.
fn or_edge_bit(cells: &mut [u8], x: i32, z: i32, bit: u8) {
    if (0..GRID).contains(&x) && (0..GRID).contains(&z) {
        cells[(x + z * GRID) as usize] |= bit;
    }
}

/// Like [`floor_passability_cells`] but leaves the cells of broken props open
/// and seals shut interior doors. `broken` holds indices into `layout.props`
/// whose 1×1 collision pillar has been destroyed, restoring movement through
/// them (recomputing from the carved mask, rather than clearing the cell's
/// edges, keeps any genuine wall the prop sat against intact).
/// `closed_door_segs` is a flat list of floor-local grid quads
/// `(ax, az, bx, bz)`, one per *closed* corridor-mouth door, matching the
/// client's `DungeonDoorSeg` (open doors are omitted by the caller). A
/// horizontal run (`az == bz`) is a north-wall door; a vertical run
/// (`ax == bx`) an east-wall door. Used to rebuild a floor's passability so
/// pathfinding (and thus monster AI) treats a shut door as a wall.
pub fn floor_passability_cells_full(
    layout: &FloorLayout,
    broken: &[u32],
    closed_door_segs: &[i32],
) -> Vec<u8> {
    floor_passability_cells_inner(layout, broken, closed_door_segs)
}

fn floor_passability_cells_inner(
    layout: &FloorLayout,
    broken: &[u32],
    closed_door_segs: &[i32],
) -> Vec<u8> {
    let mut cells = vec![0u8; (GRID * GRID) as usize];

    for z in 0..GRID {
        for x in 0..GRID {
            if !layout.is_carved(x, z) {
                continue;
            }
            let idx = (x + z * GRID) as usize;
            if !layout.is_carved(x, z - 1) {
                cells[idx] |= EDGE_N;
            }
            if !layout.is_carved(x, z + 1) {
                cells[idx] |= EDGE_S;
            }
            if !layout.is_carved(x + 1, z) {
                cells[idx] |= EDGE_E;
            }
            if !layout.is_carved(x - 1, z) {
                cells[idx] |= EDGE_W;
            }
        }
    }

    // Wall a shaft so its footprint is a dead-end on this floor, opening only
    // at the landing this floor stands on. `legit_is_exit` is true for the
    // up-shaft (you arrive at and step off its deep/exit landing) and false for
    // the down-shaft (you step onto its shallow/entry landing to descend).
    //   * lateral side walls keep a descending player from sidestepping off the
    //     run into a flush room — the up-shaft exit landing is the only row left
    //     open sideways, so you can step out at the bottom;
    //   * the *far* landing (the one belonging to the other floor) gets its
    //     room-facing run-end walled too. That's the one new opening the old
    //     code left, and it let same-floor A* march a monster straight across
    //     the footprint and onto steps that render at the other floor's height.
    // The steps in between stay open along the run on purpose: a descending
    // player is collision-checked against *this* floor's grid once their Y
    // drops into its range (~a quarter of the way down, see `is_movement_blocked`
    // floor selection), so the run must stay walkable. Cross-floor pathfinding
    // is unaffected either way — it walks the shaft via the stairwell expansion,
    // which ignores these edge bits.
    let mut wall_shaft = |shaft: &StairShaft, legit_is_exit: bool| {
        let r = shaft.rect();
        if shaft.along_z {
            let exit_z = shaft.exit_cell().1;
            let far_z = if legit_is_exit {
                shaft.entry_cell().1
            } else {
                exit_z
            };
            for z in r.z..r.z + r.d {
                if !(legit_is_exit && z == exit_z) {
                    or_edge_bit(&mut cells, r.x, z, EDGE_W);
                    or_edge_bit(&mut cells, r.x - 1, z, EDGE_E);
                    or_edge_bit(&mut cells, r.x + r.w - 1, z, EDGE_E);
                    or_edge_bit(&mut cells, r.x + r.w, z, EDGE_W);
                }
            }
            // Far landing's outer run-end (points away from the steps, into the
            // wrapping room).
            for x in r.x..r.x + r.w {
                if far_z == r.z {
                    or_edge_bit(&mut cells, x, far_z, EDGE_N);
                    or_edge_bit(&mut cells, x, far_z - 1, EDGE_S);
                } else {
                    or_edge_bit(&mut cells, x, far_z, EDGE_S);
                    or_edge_bit(&mut cells, x, far_z + 1, EDGE_N);
                }
            }
        } else {
            let exit_x = shaft.exit_cell().0;
            let far_x = if legit_is_exit {
                shaft.entry_cell().0
            } else {
                exit_x
            };
            for x in r.x..r.x + r.w {
                if !(legit_is_exit && x == exit_x) {
                    or_edge_bit(&mut cells, x, r.z, EDGE_N);
                    or_edge_bit(&mut cells, x, r.z - 1, EDGE_S);
                    or_edge_bit(&mut cells, x, r.z + r.d - 1, EDGE_S);
                    or_edge_bit(&mut cells, x, r.z + r.d, EDGE_N);
                }
            }
            for z in r.z..r.z + r.d {
                if far_x == r.x {
                    or_edge_bit(&mut cells, far_x, z, EDGE_W);
                    or_edge_bit(&mut cells, far_x - 1, z, EDGE_E);
                } else {
                    or_edge_bit(&mut cells, far_x, z, EDGE_E);
                    or_edge_bit(&mut cells, far_x + 1, z, EDGE_W);
                }
            }
        }
    };

    wall_shaft(&layout.up_shaft, true);
    if let Some(ref down) = layout.down_shaft {
        wall_shaft(down, false);
    }

    // Decorative props are solid obstacles: seal every edge of their cell so a
    // player or monster can neither enter nor leave it. Sealing the prop cell
    // alone is enough — the move check ORs both cells' edge bits, so it becomes
    // a 1×1 pillar. `prop_cell_ok` keeps props off corridor mouths and stair
    // landings, but those local rules can't guarantee global connectivity on
    // their own, so `roll_props` runs a reachability backstop (with each prop's
    // seal applied here) and rejects any prop that would close a route.
    for (i, p) in layout.props.iter().enumerate() {
        if broken.contains(&(i as u32)) {
            continue;
        }
        // Wall torches hang high on the wall, not on the floor — they're purely
        // cosmetic and never block a player or monster.
        if matches!(p.kind, PropKind::TorchWall) {
            continue;
        }
        if p.x >= 0 && p.x < GRID && p.z >= 0 && p.z < GRID {
            cells[(p.x + p.z * GRID) as usize] |= EDGE_ALL;
        }
    }

    // Seal shut interior doors at corridor mouths. The carve pass above leaves a
    // mouth fully open (both the room cell and the corridor cell are carved), so
    // a closed door has to *add* the boundary's edge bits — the inverse of a
    // housing door, which is shut by default and clears bits when opened. Each
    // quad mirrors `buildInteriorDoor`'s seg: a north-wall door seals EDGE_N on
    // the room cell and EDGE_S on the corridor cell across the opening; an
    // east-wall door seals EDGE_E / EDGE_W across the two columns. ORing both
    // cells' bits is what `is_*_blocked` checks, so sealing one side suffices,
    // but we seal both for symmetry with the carve pass.
    for q in closed_door_segs.chunks_exact(4) {
        let (ax, az, bx, bz) = (q[0], q[1], q[2], q[3]);
        if az == bz {
            // North-wall door: opening spans x in [ax, bx) on wall line z = az.
            let z = az;
            for x in ax..bx {
                or_edge_bit(&mut cells, x, z, EDGE_N);
                or_edge_bit(&mut cells, x, z - 1, EDGE_S);
            }
        } else if ax == bx {
            // East-wall door: opening spans z in [az, bz) on wall line x = ax.
            let x = ax;
            for z in az..bz {
                or_edge_bit(&mut cells, x - 1, z, EDGE_E);
                or_edge_bit(&mut cells, x, z, EDGE_W);
            }
        }
    }

    cells
}

/// Floor-0 (surface) cells over the dungeon footprint: empty but for the
/// entrance shaft's lateral side walls, which stop a player walking down from
/// stepping off the run.
///
/// Mostly it just has to exist. Floor selection keys a mover to the grid whose
/// `y_base` is nearest ([`crate::pathfinding::get_floor_at_position`]), so
/// without a surface grid someone on the open ground above a dungeon is keyed
/// to depth 1 and collides with the walls under their feet. The entry run-end
/// (the mouth) stays open on purpose: it is the only way in, and the stairwell
/// consult refuses a move only when *every* connected floor does — so depth 1's
/// grid, which seals that same end against same-floor monsters, would otherwise
/// decide alone and wall the dungeon shut.
fn surface_passability_cells(up_shaft: &StairShaft) -> Vec<u8> {
    let mut cells = vec![0u8; (GRID * GRID) as usize];
    let (near, far) = if up_shaft.along_z {
        (EDGE_W, EDGE_E)
    } else {
        (EDGE_N, EDGE_S)
    };
    for i in 0..SHAFT_LEN {
        for (w, bit) in [(0, near), (SHAFT_W - 1, far)] {
            let (x, z) = up_shaft.step_cell(i, w);
            or_edge_bit(&mut cells, x, z, bit);
        }
    }
    cells
}

/// Build the runtime passability entry covering every floor of the
/// dungeon, including the surface-entrance stairwell (floor 0 → depth 1)
/// and one stairwell per inter-floor shaft. Register it under
/// `dungeon_cache_key(..)` in the same cache houses live in; all existing
/// collision/A* queries then work unchanged. Interior doors are sealed
/// shut (their default state); callers with live door/prop state rebuild
/// floors via `floor_passability_cells_full`.
pub fn dungeon_passability(entrance: &Position, layouts: &[FloorLayout]) -> RuntimePassability {
    let (ox, oz) = dungeon_origin(entrance.x, entrance.z);

    let mut floors: Vec<RuntimeFloorGrid> = layouts
        .iter()
        .map(|layout| RuntimeFloorGrid {
            floor_level: passability_floor_for_depth(layout.depth),
            origin_x: 0,
            origin_z: 0,
            width: GRID as u8,
            depth: GRID as u8,
            y_base: floor_world_y(entrance.y, layout.depth),
            wall_height: DUNGEON_WALL_HEIGHT,
            cells: floor_passability_cells_full(layout, &[], &closed_door_segs(layout, None)),
        })
        .collect();

    let shaft_info = |shaft: &StairShaft, lower_floor: u8, upper_floor: u8| {
        let r = shaft.rect();
        StairwellInfo {
            local_min_x: r.x,
            local_min_z: r.z,
            local_max_x: r.x + r.w,
            local_max_z: r.z + r.d,
            lower_floor,
            upper_floor,
            along_z: shaft.along_z,
            reversed: shaft.reversed,
        }
    };

    let mut stairwells = Vec::new();
    if let Some(first) = layouts.first() {
        // Surface (floor 0) down to depth 1. In the stairwell encoding
        // "lower" is the entry end: i=0 lands on the surface.
        floors.push(RuntimeFloorGrid {
            floor_level: 0,
            origin_x: 0,
            origin_z: 0,
            width: GRID as u8,
            depth: GRID as u8,
            y_base: entrance.y,
            wall_height: DUNGEON_WALL_HEIGHT,
            cells: surface_passability_cells(&first.up_shaft),
        });
        stairwells.push(shaft_info(
            &first.up_shaft,
            0,
            passability_floor_for_depth(1),
        ));
    }
    for layout in layouts {
        if let Some(ref down) = layout.down_shaft {
            stairwells.push(shaft_info(
                down,
                passability_floor_for_depth(layout.depth),
                passability_floor_for_depth(layout.depth + 1),
            ));
        }
    }

    RuntimePassability {
        house_origin_x: ox,
        house_origin_z: oz,
        min_x: ox,
        max_x: ox + GRID as f32,
        min_z: oz,
        max_z: oz + GRID as f32,
        floors,
        stairwells,
        yields_to_trapped_mover: false,
    }
}

/// One dungeon floor's cells under its current dynamic state: `broken` props
/// (indices into that floor's `props`) open their cells, and every interior
/// door not in `open_doors` seals its corridor mouth. `None` when the dungeon
/// has no such floor (depth 0 is the cosmetic surface entrance door).
///
/// Both sets go through this one call because the floor is regenerated from
/// the layout each time — applying one alone would drop the other.
///
/// Split from [`set_floor_cells`] so a caller never has to compute 6400 cells
/// while holding its passability lock: the server's movement tick reads that
/// same lock for every moving player.
pub fn floor_cells(
    layouts: &[FloorLayout],
    depth: u8,
    broken: &[u32],
    open_doors: Option<&HashSet<u32>>,
) -> Option<Vec<u8>> {
    let layout = layouts.get(depth.checked_sub(1)? as usize)?;
    Some(floor_passability_cells_full(
        layout,
        broken,
        &closed_door_segs(layout, open_doors),
    ))
}

/// Install [`floor_cells`]' result over the dungeon's registered floor. A
/// no-op when the dungeon isn't in `cache` (nobody is near it).
pub fn set_floor_cells(cache: &mut PassabilityCache, entrance_id: &str, depth: u8, cells: Vec<u8>) {
    let floor_level = passability_floor_for_depth(depth);
    if let Some(rp) = cache.get_mut(&dungeon_cache_key(entrance_id)) {
        if let Some(floor) = rp.floors.iter_mut().find(|f| f.floor_level == floor_level) {
            floor.cells = cells;
        }
    }
}
