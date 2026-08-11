//! Movement execution: A*-driven walks, schedule transitions, and the
//! housing-data prefetch that lets pathfinding avoid buildings before the
//! NPC starts moving.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use onlinerpg_shared::furniture::FurniturePlacement;
use onlinerpg_shared::housing::{HouseData, WallDirection, WallVariant};
use onlinerpg_shared::pathfinding::{self, PathWaypoint};
use onlinerpg_shared::{ClientMessage, Position};
use onlinerpg_terrain::coords::{tile_to_region, world_to_tile};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::geom::PlanarDelta;
use crate::state::SharedState;
use onlinerpg_shared::schedule::ScheduleEntry;

use onlinerpg_shared::schedule::resolve_active_schedule;

pub(super) const MOVE_SPEED: f32 = onlinerpg_shared::PLAYER_MOVE_SPEED;

/// Maximum distance per move step (units). Longer segments are subdivided
/// so the NPC walks at MOVE_SPEED instead of teleporting.
pub(super) const MAX_STEP_DIST: f32 = 3.0;
const SCHEDULE_ARRIVAL_RADIUS: f32 = 2.0;

/// How many shut doors one move may open on its way to the goal. A floor
/// carries a handful; the cap only stops a pathological loop.
const MAX_DOORS_PER_MOVE: usize = 6;

/// How many times one move restarts pathfinding after the server snaps us
/// back (`PositionCorrected`). Corrections are throttled server-side, so a
/// persistent disagreement burns through these in a few seconds and gives up.
const MAX_CORRECTION_REPATHS: usize = 3;

/// Forced moves are split into legs under the server's target-distance cap;
/// the margin absorbs whatever the two sims still disagree by.
const FORCE_MOVE_LEG_DIST: f32 = onlinerpg_shared::MAX_MOVE_TARGET_DISTANCE * 0.8;

/// Wait for the server's door-toggle reply before re-pathing — that message,
/// not the request, is what reopens the cells for A*.
const DOOR_TOGGLE_WAIT: Duration = Duration::from_millis(400);

/// How many doors a blocked move probes with A* before giving up. They are
/// tried nearest-first, so the one in our way is normally the first.
const MAX_DOOR_PROBES: usize = 6;

/// Ignore doors further away than this: a door across the map is not what
/// stands between us and the goal, and every probe costs a path search.
const MAX_DOOR_SEARCH_DIST: f32 = 40.0;

/// Housing chunk size in world units (must match server's CHUNK_SIZE).
const HOUSING_CHUNK_SIZE: f32 = 64.0;

/// Move result for path-following
pub(super) enum MoveResult {
    Arrived,
    Blocked,
    Error,
}

/// Check if the active schedule entry changed and execute a move if needed.
/// Returns the new active schedule index.
pub(super) async fn check_schedule_transition(
    state: &Arc<Mutex<SharedState>>,
    schedule: &[ScheduleEntry],
    current: (Option<usize>, Option<u32>),
    label: &str,
) -> (Option<usize>, Option<u32>) {
    let (is_night, game_hour, game_minute) = { state.lock().await.time_context() };
    let new = resolve_active_schedule(schedule, is_night, game_hour, game_minute);
    if new != current {
        // Stop interaction from previous schedule entry if it had an action
        if let Some(prev_i) = current.0 {
            if schedule[prev_i].action.is_some() {
                let mut s = state.lock().await;
                if let Err(e) = s.send_command(ClientMessage::StopInteraction).await {
                    error!("[{label}] Failed to send StopInteraction: {e}");
                }
            }
        }

        if let Some(i) = new.0 {
            let entry = &schedule[i];
            info!(
                "[{label}] Schedule transition: moving to {}",
                entry.display_label()
            );
            // The schedule outranks a follow, and two walkers on one body
            // would only fight.
            if let Some(name) = state.lock().await.cancel_follow() {
                info!("[{label}] Follow of {name} cancelled by a schedule transition");
            }
            execute_schedule_move(state, entry).await;
        }
    }
    new
}

/// Send InteractObject if the schedule entry has an action and object_id.
async fn send_interact_if_needed(s: &mut SharedState, entry: &ScheduleEntry) {
    if let (Some(ref object_type), Some(object_id)) = (&entry.action, entry.object_id) {
        debug!("Sending InteractObject: {object_type} (id={object_id})");
        let cmd = ClientMessage::InteractObject {
            object_type: object_type.clone(),
            object_id,
        };
        if let Err(e) = s.send_command(cmd).await {
            error!("Failed to send InteractObject: {e}");
        }
    }
}

/// Walk to a schedule entry's position and set the final rotation. If the
/// entry has waypoints, visits each one in order before going to `pos`.
async fn execute_schedule_move(state: &Arc<Mutex<SharedState>>, entry: &ScheduleEntry) {
    // Walk through patrol waypoints first (if any)
    for (i, wp) in entry.waypoints.iter().enumerate() {
        let (wx, wz) = (wp[0], wp[2]);
        debug!(
            "Patrol waypoint {}/{}: ({:.1}, {:.1})",
            i + 1,
            entry.waypoints.len(),
            wx,
            wz
        );
        match execute_move(state, wx, wz, entry.floor_level).await {
            MoveResult::Arrived => {}
            MoveResult::Blocked => {
                warn!("Patrol waypoint {i} blocked — skipping ({wx:.1}, {wz:.1})");
            }
            MoveResult::Error => {
                error!("Patrol waypoint {i} error");
            }
        }
    }

    // Go to final position
    let (x, y, z) = (entry.pos[0], entry.pos[1], entry.pos[2]);

    // Check if we're already near the target (including floor level)
    {
        let mut s = state.lock().await;
        if let Some(ref p) = s.self_player {
            let to_target = PlanarDelta::to_xz(&p.position, x, z);
            let same_floor = s.passability_floor() == entry.floor_level;
            if same_floor && to_target.dist < SCHEDULE_ARRIVAL_RADIUS {
                debug!("Already near schedule target — skipping movement");
                send_interact_if_needed(&mut s, entry).await;
                return;
            }
        }
    }

    let arrived = match execute_move(state, x, z, entry.floor_level).await {
        MoveResult::Arrived => true,
        MoveResult::Blocked => {
            // Force-move to schedule position (e.g. cross-floor moves through
            // closed doors). NPCs must follow their schedules.
            warn!(
                "Schedule move blocked — force-moving to ({x:.1}, {z:.1}) floor {}",
                entry.floor_level
            );
            true
        }
        MoveResult::Error => {
            error!("Schedule move error");
            false
        }
    };

    if arrived {
        // Send final position with exact rotation
        let rot_rad = entry.rotation.to_radians();
        let mut s = state.lock().await;
        // Schedules are authored in housing floors, which the wire and the
        // passability cache number the same way. adopt_floor_level so a
        // cross-floor force-move still purges the left floor's monsters.
        s.adopt_floor_level(entry.floor_level as i8);
        let target = Position { x, y, z };
        // A forced move can span the whole map; legs keep every target under
        // the server's distance cap so none is silently refused.
        let from = s.self_player.as_ref().map_or(target, |p| p.position);
        for (i, leg) in force_move_legs(&from, target).into_iter().enumerate() {
            let cmd = ClientMessage::PlayerMove {
                position: leg,
                rotation: rot_rad,
                floor_level: entry.floor_level as i8,
                append: i > 0,
                sprinting: false,
            };
            if let Err(e) = s.send_command(cmd).await {
                error!("Failed to send schedule move: {e}");
                break;
            }
        }

        send_interact_if_needed(&mut s, entry).await;
    }
}

/// Straight-line legs from `from` to `to`, each under the server's move
/// target cap. The last leg is exactly `to`.
fn force_move_legs(from: &Position, to: Position) -> Vec<Position> {
    let delta = PlanarDelta::between(from, &to);
    let legs = (delta.dist / FORCE_MOVE_LEG_DIST).ceil().max(1.0) as u32;
    (1..legs)
        .map(|i| {
            let t = i as f32 / legs as f32;
            Position {
                x: from.x + delta.dx * t,
                y: from.y + (to.y - from.y) * t,
                z: from.z + delta.dz * t,
            }
        })
        .chain(std::iter::once(to))
        .collect()
}

/// Execute a move to the target position using A* pathfinding, opening any
/// shut door that seals the route. Most of a dungeon sits behind those doors —
/// the stairs down included — and a shut front door can leave a resident with
/// no way out of their own house, so a blocked path is a cue to go unlatch
/// something, not to give up.
pub(super) async fn execute_move(
    state: &Arc<Mutex<SharedState>>,
    goal_x: f32,
    goal_z: f32,
    goal_floor: u8,
) -> MoveResult {
    let mut doors_opened = 0;
    let mut repaths = 0;
    while doors_opened <= MAX_DOORS_PER_MOVE && repaths <= MAX_CORRECTION_REPATHS {
        let before = state.lock().await.position_corrections;
        match walk_path(state, goal_x, goal_z, goal_floor).await {
            MoveResult::Blocked => {
                // A refused step is a disagreement with the server, not a shut
                // door. The correction already resynced our predicted position
                // to the authoritative one (`relocate_self`), so a fresh path
                // from there — not a door hunt — is what reconciles the sims.
                if state.lock().await.position_corrections != before {
                    repaths += 1;
                    info!("Re-pathing after a server position correction ({repaths}/{MAX_CORRECTION_REPATHS})");
                    continue;
                }
                if !open_blocking_door(state).await {
                    return MoveResult::Blocked;
                }
                doors_opened += 1;
            }
            other => return other,
        }
    }
    MoveResult::Blocked
}

/// One A* path, walked to the end. Subdivides long legs so the NPC walks at
/// `MOVE_SPEED` instead of teleporting.
///
/// A search that cannot reach the goal still returns the leg that gets closest
/// (`found: false` with waypoints). That leg is worth walking — it carries us to
/// the wall or door in the way — but it is not an arrival, so it reports
/// `Blocked`: the caller opens a door and retries, and the agent is never told
/// it reached a floor it never got to.
///
/// Such a leg is only walked as far as it stays on the floor it started on.
/// With the way down sealed, the closest node A* can reach is the *surface*
/// above the target — walking that whole leg climbs back out of the dungeon,
/// and the door standing in the way is then two floors behind us.
async fn walk_path(
    state: &Arc<Mutex<SharedState>>,
    goal_x: f32,
    goal_z: f32,
    goal_floor: u8,
) -> MoveResult {
    let (path_result, start_floor) = {
        let s = state.lock().await;
        (
            s.find_path_to(goal_x, goal_z, goal_floor),
            s.passability_floor(),
        )
    };

    if path_result.waypoints.is_empty() {
        if !path_result.found {
            return MoveResult::Blocked;
        }
        return MoveResult::Arrived;
    }

    let walked: &[_] = if path_result.found {
        &path_result.waypoints
    } else {
        let keep = path_result
            .waypoints
            .iter()
            .position(|wp| wp.floor != start_floor)
            .unwrap_or(path_result.waypoints.len());
        &path_result.waypoints[..keep]
    };

    match walk_waypoints(state, walked).await {
        MoveResult::Arrived if !path_result.found => MoveResult::Blocked,
        other => other,
    }
}

/// Walk an already-found route, subdividing long legs so the NPC moves at
/// `MOVE_SPEED` instead of teleporting. Split out so a caller that has just
/// proved a route (the door probe) can walk it without searching again.
async fn walk_waypoints(state: &Arc<Mutex<SharedState>>, waypoints: &[PathWaypoint]) -> MoveResult {
    let corrections = state.lock().await.position_corrections;

    for wp in waypoints {
        loop {
            let travel_ms = {
                let mut s = state.lock().await;
                // The server snapped us back: this path walks into a step it
                // refuses, so drop it rather than grind the same wall.
                if s.position_corrections != corrections {
                    warn!("Path abandoned after a position correction");
                    return MoveResult::Blocked;
                }
                let player = match &s.self_player {
                    Some(p) => p,
                    None => return MoveResult::Error,
                };

                let to_wp = PlanarDelta::to_xz(&player.position, wp.x, wp.z);
                if to_wp.dist < 0.1 {
                    break;
                }

                let (step_x, step_z, step_dist) = if to_wp.dist <= MAX_STEP_DIST {
                    (wp.x, wp.z, to_wp.dist)
                } else {
                    let ratio = MAX_STEP_DIST / to_wp.dist;
                    (
                        player.position.x + to_wp.dx * ratio,
                        player.position.z + to_wp.dz * ratio,
                        MAX_STEP_DIST,
                    )
                };

                if let Err(e) = s
                    .send_step(step_x, step_z, wp.floor, to_wp.rotation())
                    .await
                {
                    error!("Failed to send move waypoint: {e}");
                    return MoveResult::Error;
                }
                ((step_dist / MOVE_SPEED) * 1000.0) as u64
            };

            tokio::time::sleep(Duration::from_millis(travel_ms.max(50))).await;
        }
    }

    MoveResult::Arrived
}

/// A shut door standing between us and where we want to go: how to open it,
/// and the cell centers on either side — one of them is our side.
struct DoorCandidate {
    label: String,
    toggle: ClientMessage,
    sides: [(f32, f32); 2],
}

/// Walk to the nearest shut door on our floor that we can actually reach and
/// open it. Returns false when no such door exists — then the goal really is
/// unreachable. Covers dungeon corridor doors and house doors alike: a shut
/// front door leaves a resident NPC with no route out of their own house just
/// as surely as a shut crypt door hides the stairs down.
pub(super) async fn open_blocking_door(state: &Arc<Mutex<SharedState>>) -> bool {
    let Some((door, route)) = pick_reachable_door(state).await else {
        return false;
    };

    // The probe already proved this route; walk the waypoints it found rather
    // than paying for the same search again.
    if matches!(walk_waypoints(state, &route).await, MoveResult::Error) {
        return false;
    }

    info!("Opening {} to get through", door.label);
    let mut s = state.lock().await;
    if let Err(e) = s.send_command(door.toggle).await {
        error!("Failed to send door toggle: {e}");
        return false;
    }
    drop(s);
    // The server's reply (DoorToggled / DungeonDoorToggled) is what reopens the
    // cells for A*; re-pathing before it lands would just find the same wall.
    tokio::time::sleep(DOOR_TOGGLE_WAIT).await;
    true
}

/// The closest shut door on our floor with a side we can path to, and the route
/// there. Opening it widens the reachable set; if the goal is still walled off,
/// the next round picks the next one (this one no longer counts as shut).
///
/// Each probe is a full path search, so the candidates are filtered by distance
/// and sorted nearest-first before any of them runs — the door in our way is
/// normally the first, and the rest are never searched for.
async fn pick_reachable_door(
    state: &Arc<Mutex<SharedState>>,
) -> Option<(DoorCandidate, Vec<PathWaypoint>)> {
    let s = state.lock().await;
    let position = s.self_player.as_ref()?.position;
    let floor = s.passability_floor();
    let reach = |x: f32, z: f32| PlanarDelta::xz(position.x, position.z, x, z).dist;

    let mut doors = closed_doors_on_our_floor(&s);
    let mut sides: Vec<(f32, usize, (f32, f32))> = doors
        .iter()
        .enumerate()
        .flat_map(|(i, door)| door.sides.map(|side| (reach(side.0, side.1), i, side)))
        .filter(|(dist, _, _)| *dist <= MAX_DOOR_SEARCH_DIST)
        .collect();
    sides.sort_by(|a, b| a.0.total_cmp(&b.0));

    for (_, index, side) in sides.into_iter().take(MAX_DOOR_PROBES) {
        let route = s.find_path_to(side.0, side.1, floor);
        if route.found {
            return Some((doors.swap_remove(index), route.waypoints));
        }
    }
    None
}

/// Every shut door on the floor we stand on: dungeon corridor doors when we
/// are underground, house doors when we are not.
fn closed_doors_on_our_floor(s: &SharedState) -> Vec<DoorCandidate> {
    if s.self_floor_level < 0 {
        let Some(dungeon) = s.dungeon_here() else {
            return Vec::new();
        };
        let depth = s.self_floor_level.unsigned_abs();
        let open = s
            .world_cache
            .read()
            .unwrap()
            .open_dungeon_doors(&dungeon.id, depth);
        return dungeon
            .closed_doors(depth, &open)
            .into_iter()
            .map(|d| DoorCandidate {
                label: format!("dungeon door {} on floor {depth}", d.door_id),
                toggle: ClientMessage::ToggleDungeonDoor {
                    entrance_id: dungeon.id.clone(),
                    depth,
                    door_id: d.door_id,
                },
                sides: d.sides,
            })
            .collect();
    }

    let floor = s.self_floor_level as u8;
    let world = s.world_cache.read().unwrap();
    let mut out = Vec::new();
    for house in world.houses().values() {
        // Cells are indexed from the house origin; the floor grid's own origin
        // cancels out (see `pathfinding::update_door_edge`).
        let ox = house.origin.x.floor() as i32;
        let oz = house.origin.z.floor() as i32;
        for (room_index, room) in house.rooms.iter().enumerate() {
            if room.floor_level != floor {
                continue;
            }
            for dir in [
                WallDirection::North,
                WallDirection::South,
                WallDirection::East,
                WallDirection::West,
            ] {
                for (seg, wall) in room.wall(dir).iter().enumerate() {
                    // Windows are openable too, but they are not a way through.
                    if wall.variant != WallVariant::WithDoor || wall.is_open {
                        continue;
                    }
                    let ((dx, dz, _), (adx, adz, _)) = pathfinding::door_cells(room, dir, seg);
                    let (rx, rz) = (ox + room.local_x, oz + room.local_z);
                    out.push(DoorCandidate {
                        label: format!("{} door (room {room_index}, {dir:?} {seg})", house.id),
                        toggle: ClientMessage::ToggleDoor {
                            house_id: house.id.clone(),
                            room_index: room_index as u32,
                            wall_dir: dir,
                            segment_index: seg as u32,
                        },
                        sides: [
                            ((rx + dx) as f32 + 0.5, (rz + dz) as f32 + 0.5),
                            ((rx + adx) as f32 + 0.5, (rz + adz) as f32 + 0.5),
                        ],
                    });
                }
            }
        }
    }
    out
}

/// Raw region object placements, as served by `/api/terrain/objects/{rx}/{rz}`.
#[derive(serde::Deserialize)]
struct RegionObjects {
    #[serde(default)]
    placements: Vec<FurniturePlacement>,
}

/// Insert the region (16×16 tiles) containing a world position into the set.
fn insert_region(regions: &mut HashSet<(i32, i32)>, x: f32, z: f32) {
    regions.insert((
        tile_to_region(world_to_tile(x)),
        tile_to_region(world_to_tile(z)),
    ));
}

/// Every position pathfinding data should cover: a schedule's stops and patrol
/// waypoints, or — for an agent with no schedule — wherever it currently is.
pub(super) fn coverage_positions(
    schedule: &[ScheduleEntry],
    position: Option<onlinerpg_shared::Position>,
) -> Vec<(f32, f32)> {
    if schedule.is_empty() {
        return position.map(|p| (p.x, p.z)).into_iter().collect();
    }
    schedule
        .iter()
        .flat_map(|e| {
            std::iter::once((e.pos[0], e.pos[2])).chain(e.waypoints.iter().map(|wp| (wp[0], wp[2])))
        })
        .collect()
}

/// Fetch region object placements around `positions` and register their solid
/// furniture in the passability cache, so the bot paths around furniture
/// exactly like the browser client (both go through the shared `furniture`
/// resolution). A region is ~1024m, so this usually touches one or two.
pub(super) async fn fetch_furniture_around(
    world_cache: &Arc<std::sync::RwLock<crate::state::WorldCache>>,
    positions: &[(f32, f32)],
    api_base_url: &str,
    label: &str,
) {
    let mut regions = HashSet::new();
    for (x, z) in positions {
        insert_region(&mut regions, *x, *z);
    }

    let client = reqwest::Client::new();
    let fetches = regions.iter().map(|&(rx, rz)| {
        let client = &client;
        let url = format!("{api_base_url}/api/terrain/objects/{rx}/{rz}");
        async move {
            let resp = match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => resp.json::<RegionObjects>().await.ok(),
                _ => None,
            };
            (rx, rz, resp)
        }
    });
    let results = futures_util::future::join_all(fetches).await;

    let mut world = world_cache.write().unwrap();
    let mut synced_regions = 0usize;
    for (rx, rz, resp) in results {
        let Some(resp) = resp else { continue };
        world.sync_furniture(rx, rz, &resp.placements);
        synced_regions += 1;
    }
    if synced_regions > 0 {
        debug!("[{label}] Synced furniture for {synced_regions} region(s)");
    }
}

/// Insert a position's chunk and its 8 neighbors into the set.
fn insert_chunk_neighbors(chunks: &mut HashSet<(i32, i32)>, x: f32, z: f32) {
    let cx = (x / HOUSING_CHUNK_SIZE).floor() as i32;
    let cz = (z / HOUSING_CHUNK_SIZE).floor() as i32;
    for dx in -1..=1i32 {
        for dz in -1..=1i32 {
            chunks.insert((cx + dx, cz + dz));
        }
    }
}

/// Fetch houses from the HTTP API for every chunk `positions` touches (plus
/// their neighbors), so pathfinding can avoid buildings.
pub(super) async fn fetch_houses_around(
    world_cache: &Arc<std::sync::RwLock<crate::state::WorldCache>>,
    positions: &[(f32, f32)],
    api_base_url: &str,
    label: &str,
) {
    let mut chunks = HashSet::new();
    for (x, z) in positions {
        insert_chunk_neighbors(&mut chunks, *x, *z);
    }

    debug!(
        "[{label}] Fetching houses for {} chunk(s): {:?}",
        chunks.len(),
        chunks
    );
    let client = reqwest::Client::new();
    let fetches = chunks.iter().map(|&(cx, cz)| {
        let client = &client;
        let url = format!("{api_base_url}/api/housing/area/{cx}/{cz}");
        async move {
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    resp.json::<Vec<HouseData>>().await.unwrap_or_default()
                }
                Ok(resp) => {
                    warn!(
                        "[{label}] Housing API returned {} for chunk ({cx},{cz})",
                        resp.status()
                    );
                    Vec::new()
                }
                Err(e) => {
                    warn!("[{label}] Failed to fetch houses for chunk ({cx},{cz}): {e}");
                    Vec::new()
                }
            }
        }
    });
    let results = futures_util::future::join_all(fetches).await;

    let all_houses: Vec<HouseData> = results.into_iter().flatten().collect();
    if all_houses.is_empty() {
        info!("[{label}] No houses found in any chunk");
    } else {
        let count = all_houses.len();
        let mut world = world_cache.write().unwrap();
        for house in all_houses {
            world.add_house(house);
        }
        info!("[{label}] Loaded {count} house(s) for pathfinding");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_move_legs_stay_under_cap_and_end_exactly() {
        let from = Position {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        };
        let to = Position {
            x: 90.0,
            y: 4.0,
            z: 90.0,
        };
        let legs = force_move_legs(&from, to);
        assert!(legs.len() > 1);
        let mut prev = from;
        for leg in &legs {
            assert!(
                PlanarDelta::between(&prev, leg).dist < onlinerpg_shared::MAX_MOVE_TARGET_DISTANCE
            );
            prev = *leg;
        }
        assert_eq!(*legs.last().unwrap(), to);
    }

    #[test]
    fn short_force_move_is_a_single_exact_leg() {
        let from = Position {
            x: 10.0,
            y: 0.0,
            z: 10.0,
        };
        let to = Position {
            x: 13.0,
            y: 0.0,
            z: 14.0,
        };
        let legs = force_move_legs(&from, to);
        assert_eq!(legs, vec![to]);
    }
}
