//! `wasm-bindgen` exports consumed by the web client. Wraps three
//! crate-internal subsystems behind a JS-friendly surface:
//! - the passability cache (per-house geometry + door state),
//! - A* pathfinding queries against that cache, and
//! - the monster-AI brain registry that drives in-browser NPCs.
//!
//! State lives in `thread_local!` cells so each WASM worker has its own
//! cache; JS-facing functions are named `passability_*` / `ai_*` so the
//! TypeScript wrappers can group them by subsystem.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::furniture;
use crate::housing;
use crate::messages::{deserialize_server_msg, serialize_client_msg, ClientMessage};
use crate::monster_ai::{self, BehaviorTree, MonsterBrain, NearbyPlayer};
use crate::pathfinding::{self, PassabilityCache};
use crate::world::Position;

fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsError> {
    let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
    value
        .serialize(&serializer)
        .map_err(|e| JsError::new(&format!("JS conversion failed: {e}")))
}

#[wasm_bindgen]
pub fn serialize_client_message(val: JsValue) -> Result<Vec<u8>, JsError> {
    let msg: ClientMessage = serde_wasm_bindgen::from_value(val)
        .map_err(|e| JsError::new(&format!("Invalid client message: {e}")))?;
    serialize_client_msg(&msg).map_err(|e| JsError::new(&format!("Serialization failed: {e}")))
}

#[wasm_bindgen]
pub fn deserialize_server_message(bytes: &[u8]) -> Result<JsValue, JsError> {
    let msg = deserialize_server_msg(bytes)
        .map_err(|e| JsError::new(&format!("Deserialization failed: {e}")))?;
    to_js(&msg)
}

/// Wire protocol version the bundled wasm was built against; the web client
/// puts it in its `ClientInfo` so a stale cached bundle is refused with a
/// "reload" notice instead of failing later on some other message.
#[wasm_bindgen]
pub fn protocol_version() -> u32 {
    crate::PROTOCOL_VERSION
}

/// Close code the server refuses a stale build with. Exported so the web
/// client compares against the same number the server sends; callers must
/// cache it while wasm is loaded, since `onclose` also fires before that.
#[wasm_bindgen]
pub fn close_code_protocol_mismatch() -> u16 {
    crate::CLOSE_CODE_PROTOCOL_MISMATCH
}

/// XP threshold for a given level, as an f64 for JS interop.
/// Saturates at Number.MAX_SAFE_INTEGER for levels beyond safe integer range.
#[wasm_bindgen]
pub fn xp_for_level(level: u32) -> f64 {
    const MAX_SAFE: u64 = (1u64 << 53) - 1;
    let xp = crate::xp::xp_for_level(level);
    xp.min(MAX_SAFE) as f64
}

/// Cumulative XP threshold for a trained-skill level (fishing etc.), so the
/// client's progress bars use the exact server curve. Skill XP tops out far
/// below safe-integer range, so no saturation is needed.
#[wasm_bindgen]
pub fn skill_xp_for_level(level: u32) -> f64 {
    crate::skills::skill_xp_for_level(level) as f64
}

/// The shared skill level cap (`SKILL_LEVEL_CAP`), for capped-out displays.
#[wasm_bindgen]
pub fn skill_level_cap() -> u32 {
    crate::skills::SKILL_LEVEL_CAP
}

#[wasm_bindgen]
pub fn one_handed_sword_attack_bonus(skill_level: u32) -> i32 {
    crate::skills::one_handed_sword_attack_bonus(skill_level)
}

#[wasm_bindgen]
pub fn shield_skill_guard_bonus(skill_level: u32) -> i32 {
    crate::skills::shield_skill_guard_bonus(skill_level)
}

#[wasm_bindgen]
pub fn armor_skill_guard_bonus(skill: &str, skill_level: u32) -> i32 {
    skill.parse().map_or(0, |skill| {
        crate::skills::armor_skill_guard_bonus(skill, skill_level)
    })
}

#[wasm_bindgen]
pub fn armor_construction_protection(construction: &str, damage_type: &str) -> u32 {
    let Ok(construction) = construction.parse() else {
        return 0;
    };
    let Ok(damage_type) = damage_type.parse() else {
        return 0;
    };
    crate::combat::construction_protection(Some(construction), damage_type)
}

#[wasm_bindgen]
pub fn durability_condition(current: u32, max: u32) -> Option<String> {
    crate::inventory::durability_condition(current, max)
        .map(|condition| condition.as_str().to_string())
}

#[wasm_bindgen]
pub fn durability_value_percent(current: u32, max: u32) -> Option<u32> {
    crate::inventory::durability_value_percent(current, max)
}

#[wasm_bindgen]
pub fn healing_skill_hp_bonus(skill_level: u32) -> u32 {
    crate::skills::healing_skill_hp_bonus(skill_level)
}

#[wasm_bindgen]
pub fn weapon_skill_attack_bonus(skill: &str, skill_level: u32) -> i32 {
    skill.parse().map_or(0, |skill| {
        crate::skills::weapon_skill_attack_bonus(skill, skill_level)
    })
}

#[wasm_bindgen]
pub fn weapon_skill_melee_range(skill: &str) -> f32 {
    skill.parse().map_or(
        crate::skills::DEFAULT_WEAPON_MELEE_RANGE_METERS,
        crate::skills::weapon_skill_melee_range,
    )
}

#[wasm_bindgen]
pub fn weapon_skill_attack_cooldown_ms(skill: &str) -> u32 {
    skill.parse().map_or(
        crate::skills::DEFAULT_WEAPON_ATTACK_COOLDOWN_MS,
        crate::skills::weapon_skill_attack_cooldown_ms,
    )
}

/// How long a cast is airborne (`CAST_MS`), so the client can line the splash
/// up with the bobber landing instead of the swing that threw it.
#[wasm_bindgen]
pub fn fishing_cast_ms() -> u32 {
    crate::fishing::CAST_MS
}

/// Cast-vs-walk depth threshold (`MIN_FISHABLE_DEPTH_M`), so the client's
/// click test uses the server's exact water test.
#[wasm_bindgen]
pub fn min_fishable_depth_m() -> f32 {
    crate::fishing::MIN_FISHABLE_DEPTH_M
}

/// Cast range (`MAX_CAST_DISTANCE_METERS`), so the client can walk toward
/// out-of-range water instead of sending a cast the server will reject.
#[wasm_bindgen]
pub fn max_cast_distance_m() -> f32 {
    crate::fishing::MAX_CAST_DISTANCE_METERS
}

// --- Passability cache (WASM global state) ---

thread_local! {
    static PASSABILITY_CACHE: RefCell<PassabilityCache> = RefCell::new(HashMap::new());
}

fn with_cache<R>(f: impl FnOnce(&PassabilityCache) -> R) -> R {
    PASSABILITY_CACHE.with(|c| f(&c.borrow()))
}

fn with_cache_mut<R>(f: impl FnOnce(&mut PassabilityCache) -> R) -> R {
    PASSABILITY_CACHE.with(|c| f(&mut c.borrow_mut()))
}

#[wasm_bindgen]
pub fn passability_add_house(val: JsValue) -> Result<(), JsError> {
    let house: housing::HouseData = serde_wasm_bindgen::from_value(val)
        .map_err(|e| JsError::new(&format!("Invalid HouseData: {e}")))?;
    let rp = pathfinding::build_runtime_passability(&house);
    with_cache_mut(|c| {
        c.insert(house.id.clone(), rp);
        pathfinding::apply_door_overlays(c, &house);
    });
    Ok(())
}

#[wasm_bindgen]
pub fn passability_remove_house(house_id: &str) {
    with_cache_mut(|c| c.remove(house_id));
}

/// One sealed furniture piece, returned to the client for debug visualisation.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FurnitureDebugPieceJs {
    cells: Vec<(i32, i32)>,
    y_base: f32,
}

/// Region object placements (the client's `ObjectPlacement[]`) → the shared
/// `FurniturePlacement`, which deserialises the wire shape directly.
fn to_placements(val: JsValue) -> Result<Vec<furniture::FurniturePlacement>, JsError> {
    serde_wasm_bindgen::from_value(val)
        .map_err(|e| JsError::new(&format!("Invalid furniture placements: {e}")))
}

/// Register (or replace) a region's solid furniture under `key` in the same
/// passability cache houses and dungeons use. Takes the raw region object
/// placements; solidity and footprint cells are resolved by `furniture` (shared
/// with the agent-client and server). Movement collision and click-to-move A*
/// then both treat the sealed cells as impassable — a character can neither walk
/// through the furniture nor path through it. A region with no solid furniture
/// removes the entry. Returns the sealed pieces (cells + floor Y) for the debug
/// overlay.
#[wasm_bindgen]
pub fn passability_set_furniture(key: &str, val: JsValue) -> Result<JsValue, JsError> {
    let placements = to_placements(val)?;
    let pieces = furniture::furniture_pieces(&placements);
    let debug: Vec<FurnitureDebugPieceJs> = pieces
        .iter()
        .map(|p| FurnitureDebugPieceJs {
            cells: p.cells.clone(),
            y_base: p.y_base,
        })
        .collect();
    with_cache_mut(
        |c| match pathfinding::build_furniture_passability(&pieces) {
            Some(rp) => {
                c.insert(key.to_string(), rp);
            }
            None => {
                c.remove(key);
            }
        },
    );
    to_js(&debug)
}

#[wasm_bindgen]
pub fn passability_remove_furniture(key: &str) {
    with_cache_mut(|c| {
        c.remove(key);
    });
}

/// Whether an object type is solid furniture (blocks movement). The editor uses
/// this to snap solid furniture to 90° yaw so its footprint lands on whole cells.
#[wasm_bindgen]
pub fn furniture_is_solid(type_id: &str) -> bool {
    furniture::is_solid(type_id)
}

#[wasm_bindgen]
pub fn passability_update_door(
    house_id: &str,
    room_val: JsValue,
    wall_dir_val: JsValue,
    segment_index: u32,
    is_open: bool,
) -> Result<(), JsError> {
    let room: housing::RoomData = serde_wasm_bindgen::from_value(room_val)
        .map_err(|e| JsError::new(&format!("Invalid RoomData: {e}")))?;
    let wall_dir: housing::WallDirection = serde_wasm_bindgen::from_value(wall_dir_val)
        .map_err(|e| JsError::new(&format!("Invalid WallDirection: {e}")))?;
    with_cache_mut(|c| {
        pathfinding::update_door_edge(
            c,
            house_id,
            &room,
            wall_dir,
            segment_index as usize,
            is_open,
        );
    });
    Ok(())
}

#[wasm_bindgen]
pub fn passability_find_path(
    start_x: f32,
    start_z: f32,
    start_floor: u8,
    goal_x: f32,
    goal_z: f32,
    goal_floor: u8,
) -> Result<JsValue, JsError> {
    let result = with_cache(|c| {
        pathfinding::find_and_smooth_path(
            start_x,
            start_z,
            start_floor,
            goal_x,
            goal_z,
            goal_floor,
            c,
            pathfinding::DEFAULT_MAX_NODES,
        )
    });
    to_js(&PathResultJs {
        waypoints: result
            .waypoints
            .iter()
            .map(|w| WaypointJs {
                x: w.x,
                z: w.z,
                floor: w.floor,
            })
            .collect(),
        found: result.found,
    })
}

/// The browser's only movement check, so it uses the mover variant: a player
/// furniture has sealed in can step back out. Pathfinding and smoothing run
/// entirely Rust-side and never come through here.
#[wasm_bindgen]
pub fn passability_is_movement_blocked(
    from_x: f32,
    from_z: f32,
    to_x: f32,
    to_z: f32,
    floor_level: u8,
    y: f32,
) -> bool {
    with_cache(|c| {
        pathfinding::is_movement_blocked_for_mover(
            c,
            from_x,
            from_z,
            to_x,
            to_z,
            floor_level,
            Some(y),
        )
    })
}

#[wasm_bindgen]
pub fn passability_is_circle_blocked(x: f32, z: f32, r: f32, floor_level: u8, y: f32) -> bool {
    with_cache(|c| pathfinding::is_circle_blocked_on_floor(c, x, z, r, floor_level, Some(y)))
}

#[wasm_bindgen]
pub fn passability_is_cardinal_move_blocked(
    cell_x: i32,
    cell_z: i32,
    dx: i32,
    dz: i32,
    floor_level: u8,
) -> bool {
    with_cache(|c| pathfinding::is_cardinal_move_blocked(c, cell_x, cell_z, dx, dz, floor_level))
}

#[wasm_bindgen]
pub fn passability_get_floor_at(x: f32, z: f32, y: f32) -> u8 {
    with_cache(|c| pathfinding::get_floor_at_position(c, x, z, y))
}

/// Floor an A* endpoint at (x, z, y) must be keyed to — the same as
/// `passability_get_floor_at` off the stairs, the stairwell's lower floor on an
/// intermediate step. See `pathfinding::start_floor_at`.
#[wasm_bindgen]
pub fn passability_start_floor_at(x: f32, z: f32, y: f32) -> u8 {
    with_cache(|c| pathfinding::start_floor_at(c, x, z, y))
}

#[wasm_bindgen]
pub fn passability_get_floor_y_base(x: f32, z: f32, floor_level: u8) -> f32 {
    with_cache(|c| pathfinding::get_floor_y_base(c, x, z, floor_level).unwrap_or(f32::NAN))
}

// --- Dungeon (procedural, seed-deterministic) ---

thread_local! {
    static DUNGEON_LAYOUTS: RefCell<HashMap<String, Rc<Vec<crate::dungeon::FloorLayout>>>> =
        RefCell::new(HashMap::new());
}

/// Layouts are deterministic per entrance id, so generate once and memoize —
/// regeneration costs milliseconds in wasm and several exports need them per
/// floor transition. The registry is tiny, so entries are never evicted.
///
/// The hit path allocates nothing: the height queries below run per entity per
/// frame, and `entry()` would key every one of them with a fresh `String`.
fn dungeon_layouts(entrance_id: &str) -> Rc<Vec<crate::dungeon::FloorLayout>> {
    DUNGEON_LAYOUTS.with(|c| {
        if let Some(layouts) = c.borrow().get(entrance_id) {
            return layouts.clone();
        }
        let layouts = Rc::new(crate::dungeon::generate_dungeon_for(entrance_id));
        c.borrow_mut()
            .insert(entrance_id.to_string(), layouts.clone());
        layouts
    })
}

/// Full layout of every floor of a dungeon, generated from the entrance
/// id. Identical to what the server generates natively from the same id.
#[wasm_bindgen]
pub fn dungeon_layout(entrance_id: &str) -> Result<JsValue, JsError> {
    to_js(&*dungeon_layouts(entrance_id))
}

/// Interior-door specs for one floor: wall side, opening span, wall line and
/// door id (see `dungeon::doors`).
#[wasm_bindgen]
pub fn dungeon_interior_doors(entrance_id: &str, depth: u8) -> Result<JsValue, JsError> {
    let doors = if depth == 0 {
        Vec::new()
    } else {
        dungeon_layouts(entrance_id)
            .get((depth - 1) as usize)
            .map(crate::dungeon::interior_doors)
            .unwrap_or_default()
    };
    to_js(&doors)
}

/// Shared dungeon constants so the TS side never hardcodes them.
#[wasm_bindgen]
pub fn dungeon_constants() -> Result<JsValue, JsError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct DungeonConstants {
        grid: i32,
        floor_height: f32,
        wall_height: f32,
        floor_index_base: u8,
        shaft_w: i32,
        shaft_len: i32,
        landing_cells: f32,
        max_depth: u8,
        path_max_nodes: u32,
        event_delivery_radius: f32,
    }
    to_js(&DungeonConstants {
        grid: crate::dungeon::GRID,
        floor_height: crate::dungeon::DUNGEON_FLOOR_HEIGHT,
        wall_height: crate::dungeon::DUNGEON_WALL_HEIGHT,
        floor_index_base: crate::dungeon::DUNGEON_FLOOR_INDEX_BASE,
        shaft_w: crate::dungeon::SHAFT_W,
        shaft_len: crate::dungeon::SHAFT_LEN,
        landing_cells: crate::dungeon::LANDING_CELLS,
        max_depth: crate::dungeon::MAX_DEPTH,
        path_max_nodes: crate::dungeon::DUNGEON_PATH_MAX_NODES as u32,
        event_delivery_radius: crate::EVENT_DELIVERY_RADIUS,
    })
}

/// Generate the dungeon's passability (all floors + stair shafts,
/// including the surface entrance stairwell) and register it in the same
/// cache houses use. Movement collision, click-to-move A* and monster AI
/// pathing then work in the dungeon unchanged.
#[wasm_bindgen]
pub fn dungeon_add_passability(
    entrance_id: &str,
    entrance_x: f32,
    entrance_y: f32,
    entrance_z: f32,
) {
    let rp = crate::dungeon::dungeon_passability(
        &Position {
            x: entrance_x,
            y: entrance_y,
            z: entrance_z,
        },
        &dungeon_layouts(entrance_id),
    );
    with_cache_mut(|c| {
        c.insert(crate::dungeon::dungeon_cache_key(entrance_id), rp);
    });
}

#[wasm_bindgen]
pub fn dungeon_remove_passability(entrance_id: &str) {
    with_cache_mut(|c| c.remove(&crate::dungeon::dungeon_cache_key(entrance_id)));
}

/// Run one of the shared stair-shaft geometry queries for a registry dungeon,
/// mapping "no answer" onto `NaN` for JS. The entrance comes from the shared
/// registry rather than the caller: `data/dungeons.json`, which the browser
/// embeds, is generated from the same `data-src/dungeons.csv` the registry
/// reads, so there is no second placement to keep in step.
fn dungeon_geometry(
    entrance_id: &str,
    query: impl FnOnce(&Position, &[crate::dungeon::FloorLayout]) -> Option<f32>,
) -> f32 {
    let Some(def) = crate::dungeon::entrance(entrance_id) else {
        return f32::NAN;
    };
    query(&def.position(), &dungeon_layouts(entrance_id)).unwrap_or(f32::NAN)
}

/// Ground height on dungeon floor `depth` at (x, z), stair ramps included, or
/// `NaN` when the dungeon has no such floor. The single Y model every client
/// shares — see `dungeon::stairs`.
#[wasm_bindgen]
pub fn dungeon_floor_height_at(entrance_id: &str, depth: u8, x: f32, z: f32) -> f32 {
    dungeon_geometry(entrance_id, |entrance, layouts| {
        crate::dungeon::floor_height_at(entrance, layouts, depth, x, z)
    })
}

/// Surface entrance ramp height at (x, z), or `NaN` off the entrance shaft —
/// out there the terrain sampler owns Y.
#[wasm_bindgen]
pub fn dungeon_entrance_ramp_height_at(entrance_id: &str, x: f32, z: f32) -> f32 {
    dungeon_geometry(entrance_id, |entrance, layouts| {
        crate::dungeon::entrance_ramp_height_at(entrance, layouts, x, z)
    })
}

/// Run position along one of floor `depth`'s shafts, in `[0, shaft_len)` from
/// the entry (shallow) end; `NaN` off the footprint. `down` picks the shaft
/// descending to `depth + 1` instead of the one arriving at `depth`.
#[wasm_bindgen]
pub fn dungeon_shaft_run_pos(entrance_id: &str, depth: u8, down: bool, x: f32, z: f32) -> f32 {
    dungeon_geometry(entrance_id, |entrance, layouts| {
        let layout = layouts.get(depth.checked_sub(1)? as usize)?;
        let shaft = if down {
            layout.down_shaft.as_ref()?
        } else {
            &layout.up_shaft
        };
        crate::dungeon::shaft_run_pos(entrance, shaft, x, z)
    })
}

/// Rebuild one dungeon floor's passability with its current dynamic state:
/// `broken` props (indices into that floor's `props`) destroyed, opening their
/// cells, and every interior door not in `open_door_ids` sealed (the closed
/// segments are derived from the layout, same as the server). Both the
/// broken-prop set and the open-door set route the full current state through
/// here (on-entry snapshots and live toggles alike), so the two never clobber
/// each other.
#[wasm_bindgen]
pub fn dungeon_rebuild_floor(entrance_id: &str, depth: u8, broken: &[u32], open_door_ids: &[u32]) {
    let open: HashSet<u32> = open_door_ids.iter().copied().collect();
    let cells =
        crate::dungeon::floor_cells(&dungeon_layouts(entrance_id), depth, broken, Some(&open));
    if let Some(cells) = cells {
        with_cache_mut(|c| crate::dungeon::set_floor_cells(c, entrance_id, depth, cells));
    }
}

/// Debug: dump one floor's per-cell edge bitmask (N=1, E=2, S=4, W=8) plus
/// its world min-corner origin and Y, so the client can draw a passability
/// wireframe. Returns null when the dungeon isn't registered or the floor
/// level isn't present.
#[wasm_bindgen]
pub fn dungeon_passability_floor_cells(
    entrance_id: &str,
    floor_level: u8,
) -> Result<JsValue, JsError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct FloorCellsJs {
        origin_x: f32,
        origin_z: f32,
        width: u8,
        depth: u8,
        y_base: f32,
        cells: Vec<u8>,
    }
    with_cache(|c| {
        let key = crate::dungeon::dungeon_cache_key(entrance_id);
        let Some(rp) = c.get(&key) else {
            return Ok(JsValue::NULL);
        };
        let Some(f) = rp.floors.iter().find(|f| f.floor_level == floor_level) else {
            return Ok(JsValue::NULL);
        };
        to_js(&FloorCellsJs {
            origin_x: rp.house_origin_x + f.origin_x as f32,
            origin_z: rp.house_origin_z + f.origin_z as f32,
            width: f.width,
            depth: f.depth,
            y_base: f.y_base,
            cells: f.cells.clone(),
        })
    })
}

/// `passability_find_path` with an explicit node budget — dungeon floors
/// are mazes and cross-floor routes can exhaust the housing default.
#[wasm_bindgen]
pub fn passability_find_path_budget(
    start_x: f32,
    start_z: f32,
    start_floor: u8,
    goal_x: f32,
    goal_z: f32,
    goal_floor: u8,
    max_nodes: u32,
) -> Result<JsValue, JsError> {
    let result = with_cache(|c| {
        pathfinding::find_and_smooth_path(
            start_x,
            start_z,
            start_floor,
            goal_x,
            goal_z,
            goal_floor,
            c,
            max_nodes as usize,
        )
    });
    to_js(&PathResultJs {
        waypoints: result
            .waypoints
            .iter()
            .map(|w| WaypointJs {
                x: w.x,
                z: w.z,
                floor: w.floor,
            })
            .collect(),
        found: result.found,
    })
}

#[wasm_bindgen]
pub fn passability_debug_info() -> Result<JsValue, JsError> {
    with_cache(|c| {
        let entries: Vec<String> = c.iter().map(|(id, rp)| {
            let total_cells: usize = rp.floors.iter().map(|f| f.cells.len()).sum();
            let non_zero: usize = rp.floors.iter()
                .flat_map(|f| f.cells.iter())
                .filter(|&&b| b != 0)
                .count();
            format!(
                "{}: origin=({:.1},{:.1}) aabb=({:.1},{:.1})→({:.1},{:.1}) floors={} stairwells={} cells={} non_zero={}",
                id, rp.house_origin_x, rp.house_origin_z,
                rp.min_x, rp.min_z, rp.max_x, rp.max_z,
                rp.floors.len(), rp.stairwells.len(),
                total_cells, non_zero
            )
        }).collect();
        to_js(&entries)
    })
}

// Serializable types for WASM return values
#[derive(Serialize)]
struct WaypointJs {
    x: f32,
    z: f32,
    floor: u8,
}

#[derive(Serialize)]
struct PathResultJs {
    waypoints: Vec<WaypointJs>,
    found: bool,
}

// --- Monster AI WASM bindings ---

thread_local! {
    static MONSTER_BRAINS: RefCell<HashMap<String, MonsterBrain>> = RefCell::new(HashMap::new());
    static AI_BEHAVIOR_TREES: RefCell<HashMap<String, BehaviorTree>> = RefCell::new(HashMap::new());
}

struct WasmPathProvider;
impl monster_ai::PathProvider for WasmPathProvider {
    fn find_path(
        &self,
        start_x: f32,
        start_z: f32,
        start_floor: u8,
        goal_x: f32,
        goal_z: f32,
        goal_floor: u8,
    ) -> pathfinding::PathResult {
        with_cache(|c| {
            pathfinding::find_and_smooth_path(
                start_x,
                start_z,
                start_floor,
                goal_x,
                goal_z,
                goal_floor,
                c,
                pathfinding::DEFAULT_MAX_NODES,
            )
        })
    }
}

#[wasm_bindgen]
pub fn ai_load_behavior_trees(json: &str) -> Result<(), JsError> {
    let trees = monster_ai::load_behavior_trees(json)
        .map_err(|e| JsError::new(&format!("Failed to parse behavior trees: {e}")))?;
    AI_BEHAVIOR_TREES.with(|t| *t.borrow_mut() = trees);
    Ok(())
}

#[wasm_bindgen]
pub fn ai_create_brain(val: JsValue) -> Result<(), JsError> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct CreateBrainArgs {
        monster_id: String,
        monster_type: String,
        position: Position,
        health: u32,
        max_health: u32,
        walk_speed: f32,
        run_speed: f32,
        attack_range: f32,
        chase_range: f32,
        attack_cooldown: f32,
        behavior: String,
        /// Passability floor for path queries (dungeon monsters use their
        /// depth's floor index; defaults to overworld).
        #[serde(default)]
        path_floor: u8,
    }

    let args: CreateBrainArgs = serde_wasm_bindgen::from_value(val)
        .map_err(|e| JsError::new(&format!("Invalid brain args: {e}")))?;

    let mut brain = MonsterBrain::new(
        args.monster_id.clone(),
        args.monster_type,
        args.behavior,
        args.position,
        args.health,
        args.max_health,
        args.walk_speed,
        args.run_speed,
        args.attack_range,
        args.chase_range,
        args.attack_cooldown,
    );
    brain.path_floor = args.path_floor;

    MONSTER_BRAINS.with(|b| b.borrow_mut().insert(args.monster_id, brain));
    Ok(())
}

#[wasm_bindgen]
pub fn ai_remove_brain(monster_id: &str) {
    MONSTER_BRAINS.with(|b| b.borrow_mut().remove(monster_id));
}

#[wasm_bindgen]
pub fn ai_tick_brain(
    monster_id: &str,
    delta_ms: f32,
    nearby_players: JsValue,
) -> Result<JsValue, JsError> {
    let players: Vec<NearbyPlayer> = serde_wasm_bindgen::from_value(nearby_players)
        .map_err(|e| JsError::new(&format!("Invalid nearby_players: {e}")))?;

    let result = MONSTER_BRAINS.with(|brains| {
        let mut brains = brains.borrow_mut();
        let brain = match brains.get_mut(monster_id) {
            Some(b) => b,
            None => return None,
        };

        let mut rng = rand::thread_rng();
        AI_BEHAVIOR_TREES.with(|trees| {
            let trees = trees.borrow();
            monster_ai::behavior_tree_for(&trees, &brain.behavior).map(|tree| {
                brain.tick_with_behavior_tree(delta_ms, &players, tree, &WasmPathProvider, &mut rng)
            })
        })
    });

    match result {
        Some(r) => to_js(&r),
        None => to_js(&serde_json::Value::Null),
    }
}

/// `attacker_id` is `f64`, not `u64`: wasm-bindgen maps `u64` to a JS BigInt,
/// and the client holds player ids as plain numbers. Exact by `PlayerId`'s
/// below-2^53 invariant.
#[wasm_bindgen]
pub fn ai_handle_hit(
    monster_id: &str,
    attacker_id: f64,
    hit: bool,
    damage: u32,
) -> Result<JsValue, JsError> {
    let attacker_id = crate::PlayerId::from(attacker_id as u64);
    let commands = MONSTER_BRAINS.with(|brains| {
        let mut brains = brains.borrow_mut();
        let brain = match brains.get_mut(monster_id) {
            Some(b) => b,
            None => return vec![],
        };

        brain.handle_hit_with_behavior_tree(&attacker_id, hit, damage)
    });

    to_js(&commands)
}

/// Re-sync an owned monster's brain to the server's position after a refused move.
#[wasm_bindgen]
pub fn ai_apply_authoritative_position(monster_id: &str, x: f32, y: f32, z: f32) {
    MONSTER_BRAINS.with(|brains| {
        if let Some(brain) = brains.borrow_mut().get_mut(monster_id) {
            brain.apply_authoritative_position(Position { x, y, z });
        }
    });
}

#[wasm_bindgen]
pub fn ai_handle_death(monster_id: &str) {
    MONSTER_BRAINS.with(|brains| {
        if let Some(brain) = brains.borrow_mut().get_mut(monster_id) {
            brain.handle_death();
        }
    });
}
