//! One walker, for every walk the agent takes.
//!
//! There used to be two — one for a fixed place, one for a moving target —
//! and each had grown its own answer to the same questions: what to do when
//! A* finds no route, when the server snaps us back, when a shut door stands
//! in the way. Every disagreement was a bug. The chase ignored position
//! corrections outright; the commute, given no route, sent nothing at all and
//! froze the body, since the server only reconciles the two sims off a step
//! it can refuse.
//!
//! What a walk is for now lives entirely in [`WalkTo`] and its [`Tuning`]
//! row; the loop is the same one either way.

use std::sync::Arc;
use std::time::{Duration, Instant};

use onlinerpg_shared::housing::{WallDirection, WallVariant};
use onlinerpg_shared::pathfinding::{self, PathWaypoint};
use onlinerpg_shared::{ClientMessage, PlayerId, Position};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use super::movement::{travel_ms, MAX_STEP_DIST};
use crate::geom::PlanarDelta;
use crate::state::SharedState;

/// Minimum distance to a monster before attacking (matches the web client).
const ATTACK_RANGE: f32 = 2.0;
/// Polite stop distance when walking up to another character: close enough to
/// talk, far enough that the models don't overlap.
const APPROACH_RANGE: f32 = 1.0;
/// Stop distance for ground items, inside the server's 2.5 m pickup gate
/// (`MAX_PICKUP_DISTANCE`) so overshoot and float error can't push the
/// PickupItem send back out of range.
const PICKUP_ARRIVE_RANGE: f32 = 2.0;
/// Give up chasing a monster further than this away, so an NPC guard does not
/// run off its patrol after a distant threat.
const MAX_CHASE_DISTANCE: f32 = 20.0;
/// How far the target must move from our last path goal before we re-route.
const REROUTE_THRESHOLD: f32 = 1.5;
/// How often to poll when there is no step to send.
const IDLE_TICK_MS: u64 = 200;

const MAX_CHASE_SECS: f32 = 15.0;
/// Longer than the combat chase: an approach target can be anywhere inside
/// the sight radius.
const MAX_APPROACH_SECS: f32 = 30.0;
/// Shorter than an approach: a dropped item never walks away.
const MAX_PICKUP_WALK_SECS: f32 = 12.0;
/// A point is only ever walked to from inside its own room, so this is a
/// stuck-detector, not a travel budget.
const MAX_POINT_WALK_SECS: f32 = 15.0;

/// How many shut doors one walk may unlatch on its way. A floor carries a
/// handful; the cap only stops a pathological loop.
const MAX_DOORS_PER_WALK: usize = 6;
/// A chase is short and its target is moving — two tries, then let it go.
const MAX_CHASE_DOORS: usize = 2;

/// How many times one walk restarts pathfinding after the server snaps us
/// back (`PositionCorrected`). Corrections are throttled server-side, so a
/// persistent disagreement burns through these in a few seconds and gives up.
const MAX_CORRECTION_REPATHS: usize = 3;

/// Wait for the server's door-toggle reply before re-pathing — that message,
/// not the request, is what reopens the cells for A*.
const DOOR_TOGGLE_WAIT: Duration = Duration::from_millis(400);

/// How many doors a blocked walk probes with A* before giving up. They are
/// tried nearest-first, so the one in our way is normally the first.
const MAX_DOOR_PROBES: usize = 6;

/// Ignore doors further away than this: a door across the map is not what
/// stands between us and the goal, and every probe costs a path search.
const MAX_DOOR_SEARCH_DIST: f32 = 40.0;

/// Where a walk is headed. One row here and one in [`WalkTo::tuning`] is the
/// whole of what makes a chase different from a commute.
pub(super) enum WalkTo<'a> {
    Monster(&'a str),
    Character(&'a PlayerId),
    GroundItem(u64),
    /// A fixed spot nearby (the cell beside a chest), with the distance the
    /// caller needs us to end up inside.
    Point {
        pos: Position,
        floor_level: i8,
        arrive_range: f32,
    },
    /// Somewhere on the map, however far: the commute, the patrol leg, the
    /// walk home. No leash and no deadline — only arrival, or a route that
    /// does not exist.
    Place {
        x: f32,
        z: f32,
        floor: u8,
    },
}

/// The distances and deadlines one walk runs by.
struct Tuning {
    /// The walk ends successfully within this distance of the target.
    arrive_range: f32,
    /// How far short of the target the path goal stops, so a step can never
    /// land on top of a character. Zero pathfinds straight at the target —
    /// the arrive check stops us first.
    path_pullback: f32,
    /// How close a direct step (no path) walks before it stops short.
    step_stop_dist: f32,
    /// Give up once the target is further away than this.
    max_distance: f32,
    max_secs: f32,
    /// Arriving also needs a clear attack line — stopping at range through a
    /// shut door is no arrival at all.
    needs_clear_line: bool,
    max_doors: usize,
    /// A destination that cannot move is worth giving up on the moment A*
    /// says it cannot be reached; a target that walks may stand somewhere
    /// reachable on the next poll.
    give_up_when_unreachable: bool,
    /// Whether walking the route A* found is itself the arrival. Where the
    /// goal is a cell rather than a range, it has to be: A* smooths its last
    /// waypoint onto the goal, and a walk that lands a few centimetres short
    /// would otherwise re-search the same route forever.
    route_end_is_arrival: bool,
}

impl WalkTo<'_> {
    fn position(&self, s: &SharedState) -> Option<Position> {
        match self {
            Self::Monster(id) => s.nearby_monsters.get(*id).map(|m| m.position),
            Self::Character(id) => s.nearby_players.get(*id).map(|p| p.position),
            Self::GroundItem(id) => s.ground_item(*id).map(|i| i.position),
            Self::Point { pos, .. } => Some(*pos),
            Self::Place { x, z, .. } => Some(Position {
                x: *x,
                y: 0.0,
                z: *z,
            }),
        }
    }

    /// Passability floor index to path toward, converted from the target's
    /// wire `floor_level`.
    fn floor(&self, s: &SharedState) -> u8 {
        let level = match self {
            Self::Monster(id) => s.nearby_monsters.get(*id).map(|m| m.floor_level),
            Self::Character(id) => s.nearby_players.get(*id).map(|p| p.floor_level),
            Self::GroundItem(id) => s.ground_item(*id).map(|i| i.floor_level),
            Self::Point { floor_level, .. } => Some(*floor_level),
            Self::Place { floor, .. } => return *floor,
        };
        onlinerpg_shared::dungeon::passability_floor_for_level(level.unwrap_or(0))
    }

    fn tuning(&self) -> Tuning {
        match self {
            // Characters carry a little arrive slack over APPROACH_RANGE so
            // reaching the pulled-back path goal always counts as in range.
            Self::Character(_) => Tuning {
                arrive_range: APPROACH_RANGE + 0.2,
                path_pullback: APPROACH_RANGE,
                step_stop_dist: APPROACH_RANGE,
                max_distance: crate::state::NPC_SIGHT_RADIUS,
                max_secs: MAX_APPROACH_SECS,
                needs_clear_line: false,
                max_doors: MAX_CHASE_DOORS,
                give_up_when_unreachable: false,
                route_end_is_arrival: false,
            },
            Self::Monster(_) => Tuning {
                arrive_range: ATTACK_RANGE,
                path_pullback: 0.0,
                step_stop_dist: ATTACK_RANGE - 0.5,
                max_distance: MAX_CHASE_DISTANCE,
                max_secs: MAX_CHASE_SECS,
                needs_clear_line: true,
                max_doors: MAX_CHASE_DOORS,
                give_up_when_unreachable: false,
                route_end_is_arrival: false,
            },
            Self::GroundItem(_) => Tuning {
                arrive_range: PICKUP_ARRIVE_RANGE,
                path_pullback: 0.0,
                step_stop_dist: PICKUP_ARRIVE_RANGE - 0.5,
                max_distance: crate::state::NPC_SIGHT_RADIUS,
                max_secs: MAX_PICKUP_WALK_SECS,
                needs_clear_line: false,
                max_doors: MAX_CHASE_DOORS,
                give_up_when_unreachable: false,
                route_end_is_arrival: false,
            },
            // A fixed point is only ever offered once we share its room, so
            // the sight radius is slack, not a leash.
            Self::Point { arrive_range, .. } => Tuning {
                arrive_range: *arrive_range,
                path_pullback: 0.0,
                step_stop_dist: (arrive_range - 0.5).max(0.5),
                max_distance: crate::state::NPC_SIGHT_RADIUS,
                max_secs: MAX_POINT_WALK_SECS,
                needs_clear_line: false,
                max_doors: MAX_CHASE_DOORS,
                give_up_when_unreachable: true,
                route_end_is_arrival: false,
            },
            // Arrival is the goal cell itself: A* smooths its last waypoint
            // onto it, so anything short of standing there is still walking.
            Self::Place { .. } => Tuning {
                arrive_range: 0.1,
                path_pullback: 0.0,
                step_stop_dist: 0.0,
                max_distance: f32::INFINITY,
                max_secs: f32::INFINITY,
                needs_clear_line: false,
                max_doors: MAX_DOORS_PER_WALK,
                give_up_when_unreachable: true,
                route_end_is_arrival: true,
            },
        }
    }
}

/// Log label. `Display` rather than a `-> String` helper so the monster arm
/// keeps borrowing its id instead of allocating a copy for a log line.
impl std::fmt::Display for WalkTo<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Monster(id) => f.write_str(id),
            Self::Character(id) => write!(f, "{id}"),
            Self::GroundItem(id) => write!(f, "item {id}"),
            Self::Point { pos, .. } => write!(f, "point ({:.1}, {:.1})", pos.x, pos.z),
            Self::Place { x, z, .. } => write!(f, "({x:.1}, {z:.1})"),
        }
    }
}

/// Why a walk gave up, tagged at the point where the loop knew the ground
/// truth — so callers report the real reason instead of guessing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum LostReason {
    /// The target left the world state: died, despawned, or out of sight.
    TargetGone,
    /// We died (or vanished) on the way.
    PlayerDied,
    /// The target sits this far away, beyond the walk's leash.
    TooFar(f32),
    /// The clock ran out before arriving.
    Timeout,
    /// No route on this floor — walls or shut doors all the way around.
    NoPath,
}

impl LostReason {
    /// Target-neutral clause completing "You could not reach X — {clause}."
    pub(super) fn clause(&self) -> String {
        match self {
            Self::TargetGone => "your target is no longer there".to_string(),
            Self::PlayerDied => "you died on the way".to_string(),
            Self::TooFar(d) => format!("your target is {d:.0}m away, beyond your reach"),
            Self::Timeout => "you ran out of time before arriving".to_string(),
            Self::NoPath => "no route leads there from here".to_string(),
        }
    }
}

/// How a walk ended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Walked {
    Arrived,
    /// No route, and no door left to open.
    Blocked,
    /// The target stopped being worth walking to.
    Lost(LostReason),
    Error,
}

/// Walk to `to` until we are inside its arrive range.
///
/// `background` marks steps no current action asked for (the follow task), so
/// they stay out of the action-progress count.
pub(super) async fn walk(
    state: &Arc<Mutex<SharedState>>,
    to: &WalkTo<'_>,
    background: bool,
    sprint: Option<bool>,
) -> Walked {
    let tuning = to.tuning();
    let started = Instant::now();
    let mut route: Vec<PathWaypoint> = Vec::new();
    let mut leg = 0usize;
    let mut doors_opened = 0usize;
    let mut repaths = 0usize;
    let mut unreachable = false;
    let mut last_goal: Option<(f32, f32)> = None;
    let mut corrections = state.lock().await.position_corrections;

    loop {
        if started.elapsed().as_secs_f32() > tuning.max_secs {
            warn!("Walk to {to} timed out");
            return Walked::Lost(LostReason::Timeout);
        }

        let (target_floor, goal, snapped) = {
            let s = state.lock().await;
            let Some(target_pos) = to.position(&s) else {
                return Walked::Lost(LostReason::TargetGone);
            };
            let Some(me) = s.self_player.as_ref().filter(|p| p.health > 0) else {
                return Walked::Lost(LostReason::PlayerDied);
            };
            let to_target = PlanarDelta::between(&me.position, &target_pos);
            let arrived = to_target.dist <= tuning.arrive_range
                && !(tuning.needs_clear_line && s.attack_line_blocked(target_pos.x, target_pos.z));
            if arrived {
                return Walked::Arrived;
            }
            if to_target.dist > tuning.max_distance {
                info!(
                    "Giving up on {to}: {:.1}m away (>{:.1}m)",
                    to_target.dist, tuning.max_distance
                );
                return Walked::Lost(LostReason::TooFar(to_target.dist));
            }
            // The path goal, pulled back toward us so a route never ends
            // inside the target character.
            let goal = if tuning.path_pullback > 0.0 && to_target.dist > tuning.path_pullback {
                let ratio = (to_target.dist - tuning.path_pullback) / to_target.dist;
                (
                    me.position.x + to_target.dx * ratio,
                    me.position.z + to_target.dz * ratio,
                )
            } else {
                (target_pos.x, target_pos.z)
            };
            let snapped = s.position_corrections != corrections;
            (to.floor(&s), goal, snapped)
        };

        // The server snapped us back: whatever we were walking, it walks into
        // a step it refuses, so drop the route and search again from where it
        // says we are (`relocate_self` has already moved us there).
        if snapped {
            corrections = state.lock().await.position_corrections;
            repaths += 1;
            if repaths > MAX_CORRECTION_REPATHS {
                info!("Walk to {to} gave up after {repaths} position corrections");
                return Walked::Blocked;
            }
            debug!("Re-pathing after a server position correction ({repaths})");
            route.clear();
            leg = 0;
        }

        let goal_moved = match last_goal {
            Some((x, z)) => PlanarDelta::xz(x, z, goal.0, goal.1).dist > REROUTE_THRESHOLD,
            None => true,
        };
        if route.is_empty() || leg >= route.len() || goal_moved {
            let (found, waypoints, start_floor) = {
                let s = state.lock().await;
                let r = s.find_path_to(goal.0, goal.1, target_floor);
                (r.found, r.waypoints, s.passability_floor())
            };
            // A search that cannot reach the goal still hands back the leg
            // that gets closest. That leg is worth walking — it carries us to
            // the wall or the door in the way — but only as far as it stays
            // on the floor it started on: with the way down sealed, the
            // closest node A* can reach is the *surface* above the target,
            // and walking that climbs back out of the dungeon.
            unreachable = !found;
            route = if found {
                waypoints
            } else {
                let keep = waypoints
                    .iter()
                    .position(|wp| wp.floor != start_floor)
                    .unwrap_or(waypoints.len());
                waypoints[..keep].to_vec()
            };
            leg = 0;
            last_goal = Some(goal);
        }

        match step_along(state, &route, &mut leg, background, sprint).await {
            Step::Sent(ms) => {
                tokio::time::sleep(Duration::from_millis(ms.max(50))).await;
                continue;
            }
            Step::Error => return Walked::Error,
            Step::Nothing => {}
        }

        // Nothing left to walk.
        if !unreachable {
            // A* had a route and we walked it out. Where the goal is a cell
            // that is the arrival; where it is a range around something that
            // moves, the something moved — look again.
            if tuning.route_end_is_arrival {
                return Walked::Arrived;
            }
            route.clear();
            tokio::time::sleep(Duration::from_millis(IDLE_TICK_MS)).await;
            continue;
        }
        // No route at all, or we are standing on the closest cell A* could
        // reach — which is where a shut door usually is.
        if doors_opened < tuning.max_doors && open_blocking_door(state, background, sprint).await {
            doors_opened += 1;
            route.clear();
            continue;
        }
        // Underground a missing route is a wall or a shut door, never the
        // open ground a straight line assumes.
        if state.lock().await.self_floor_level < 0 {
            info!("No route to {to} on this floor");
            return if tuning.give_up_when_unreachable {
                Walked::Blocked
            } else {
                Walked::Lost(LostReason::NoPath)
            };
        }
        // On the surface, inch toward it anyway. The ground may simply be
        // un-pathable rather than walled, and either way the server needs a
        // step it can refuse before it will tell us where we really are.
        match nudge(state, goal, tuning.step_stop_dist, background, sprint).await {
            Step::Sent(ms) => tokio::time::sleep(Duration::from_millis(ms.max(50))).await,
            Step::Error => return Walked::Error,
            Step::Nothing => tokio::time::sleep(Duration::from_millis(IDLE_TICK_MS)).await,
        }
        if tuning.give_up_when_unreachable {
            return Walked::Blocked;
        }
    }
}

/// What one step attempt did.
enum Step {
    /// Sent, and this is how long the server takes to walk it.
    Sent(u64),
    /// Nothing to send from here.
    Nothing,
    Error,
}

/// Send one `MAX_STEP_DIST`-bounded step toward `route[leg]`, advancing `leg`
/// as waypoints are reached. Subdividing is what keeps the body walking at
/// `MOVE_SPEED` instead of teleporting.
async fn step_along(
    state: &Arc<Mutex<SharedState>>,
    route: &[PathWaypoint],
    leg: &mut usize,
    background: bool,
    sprint: Option<bool>,
) -> Step {
    while *leg < route.len() {
        let wp = &route[*leg];
        let mut s = state.lock().await;
        let Some(me) = s.self_player.as_ref().map(|p| p.position) else {
            return Step::Error;
        };
        let to_wp = PlanarDelta::to_xz(&me, wp.x, wp.z);
        if to_wp.dist < 0.1 {
            *leg += 1;
            continue;
        }
        let (x, z, dist) = if to_wp.dist <= MAX_STEP_DIST {
            *leg += 1;
            (wp.x, wp.z, to_wp.dist)
        } else {
            let ratio = MAX_STEP_DIST / to_wp.dist;
            (
                me.x + to_wp.dx * ratio,
                me.z + to_wp.dz * ratio,
                MAX_STEP_DIST,
            )
        };
        return match s
            .send_step(x, z, wp.floor, to_wp.rotation(), background, sprint)
            .await
        {
            Ok(sprinting) => Step::Sent(travel_ms(dist, sprinting, s.self_move_mult)),
            Err(e) => {
                error!("Failed to send a walk step: {e}");
                Step::Error
            }
        };
    }
    Step::Nothing
}

/// One direct step toward `goal`, stopping `stop_dist` short of it — the
/// fall-through for when A* has no leg to walk.
async fn nudge(
    state: &Arc<Mutex<SharedState>>,
    goal: (f32, f32),
    stop_dist: f32,
    background: bool,
    sprint: Option<bool>,
) -> Step {
    let mut s = state.lock().await;
    let Some(me) = s.self_player.as_ref().map(|p| p.position) else {
        return Step::Error;
    };
    let to_goal = PlanarDelta::to_xz(&me, goal.0, goal.1);
    if to_goal.dist <= stop_dist.max(0.1) {
        return Step::Nothing;
    }
    let dist = (to_goal.dist - stop_dist).min(MAX_STEP_DIST);
    let ratio = dist / to_goal.dist;
    let floor = s.passability_floor();
    match s
        .send_step(
            me.x + to_goal.dx * ratio,
            me.z + to_goal.dz * ratio,
            floor,
            to_goal.rotation(),
            background,
            sprint,
        )
        .await
    {
        Ok(sprinting) => {
            debug!("A* had no leg to walk — nudging {dist:.1}m on");
            Step::Sent(travel_ms(dist, sprinting, s.self_move_mult))
        }
        Err(e) => {
            error!("Failed to send a nudge step: {e}");
            Step::Error
        }
    }
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
async fn open_blocking_door(
    state: &Arc<Mutex<SharedState>>,
    background: bool,
    sprint: Option<bool>,
) -> bool {
    let Some((door, route)) = pick_reachable_door(state).await else {
        return false;
    };

    // The probe already proved this route; walk the waypoints it found rather
    // than paying for the same search again.
    let mut leg = 0usize;
    loop {
        match step_along(state, &route, &mut leg, background, sprint).await {
            Step::Sent(ms) => tokio::time::sleep(Duration::from_millis(ms.max(50))).await,
            Step::Nothing => break,
            Step::Error => return false,
        }
    }

    info!("Opening {} to get through", door.label);
    let mut s = state.lock().await;
    if let Err(e) = s.send_flagged_command(door.toggle, background).await {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::tests::{test_player, test_state};
    use onlinerpg_shared::housing::{HouseData, PassabilityGrid};

    /// A block of cells walled on every edge: A* will not leave one.
    fn boxed_in(x: f32, z: f32, floor_level: u8) -> HouseData {
        HouseData {
            id: "box".to_string(),
            owner_id: "test".to_string(),
            origin: Position { x, y: 0.0, z },
            rooms: Vec::new(),
            passability: vec![PassabilityGrid {
                floor_level,
                origin_x: 0,
                origin_z: 0,
                width: 2,
                depth: 2,
                cells: vec![1 | 2 | 4 | 8; 4],
            }],
        }
    }

    async fn boxed_in_at(
        x: f32,
        z: f32,
        floor_level: i8,
    ) -> (
        Arc<Mutex<SharedState>>,
        tokio::sync::mpsc::Receiver<ClientMessage>,
    ) {
        let (mut s, rx) = test_state();
        s.self_player = Some(test_player(x, z));
        s.self_player_id = Some(PlayerId::from(1));
        s.self_floor_level = floor_level;
        let floor = s.passability_floor();
        s.world_cache
            .write()
            .unwrap()
            .add_house(boxed_in(x.floor() - 1.0, z.floor() - 1.0, floor));
        assert!(
            !s.find_path_to(x + 30.0, z, floor).found,
            "A* has no leg here"
        );
        (Arc::new(Mutex::new(s)), rx)
    }

    /// A walk A* cannot route must still put a step on the wire: the server
    /// reconciles the two sims off a step it refuses — its correction and its
    /// sealed-cell rescue both need one — so silence freezes the body while
    /// the caller reissues the same walk forever.
    #[tokio::test(start_paused = true)]
    async fn a_walk_with_no_route_still_steps() {
        let (state, mut rx) = boxed_in_at(0.5, 0.5, 0).await;
        let floor = state.lock().await.passability_floor();
        let to = WalkTo::Place {
            x: 30.5,
            z: 0.5,
            floor,
        };
        assert_eq!(walk(&state, &to, false, Some(false)).await, Walked::Blocked);
        assert!(
            matches!(rx.try_recv(), Ok(ClientMessage::PlayerMove { .. })),
            "a blocked walk must still tell the server where it wanted to go"
        );
    }

    /// Underground a missing route is a wall, not un-pathable ground.
    #[tokio::test(start_paused = true)]
    async fn a_walk_with_no_route_underground_sends_nothing() {
        let (state, mut rx) = boxed_in_at(0.5, 0.5, -1).await;
        let floor = state.lock().await.passability_floor();
        let to = WalkTo::Place {
            x: 30.5,
            z: 0.5,
            floor,
        };
        walk(&state, &to, false, Some(false)).await;
        assert!(rx.try_recv().is_err(), "no line walked through a dungeon");
    }
}
