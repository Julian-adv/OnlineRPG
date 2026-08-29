//! Floor layout generation. Everything here must stay deterministic
//! across native and wasm builds — ChaCha8 RNG only, integer math only,
//! fixed draw order (retries re-draw in the same code path, which is
//! fine; changing the *algorithm* shifts the stream and is gated by the
//! golden-hash test).

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use super::doors::wall_openings;
use super::{
    is_locked_depth, FloorLayout, PropKind, PropSpec, Room, SpawnSpec, StairShaft,
    BOSS_MONSTER_TYPE, GRID, MAX_DEPTH, MIN_DEPTH, SHAFT_LEN, SHAFT_W,
};
use crate::housing::WallDirection;

const ROOM_MIN: i32 = 9;
const ROOM_MAX: i32 = 17;
/// Axial size needed by a room that hosts a stair shaft (run + 1 margin each end).
const SHAFT_ROOM_AXIAL: i32 = SHAFT_LEN + 2;
const FLOOR_ATTEMPTS: u32 = 30;
const ROOM_PLACE_ATTEMPTS: u32 = 60;
/// Widest corridor mouth a room wall may have: a 2-wide corridor plus
/// corner slop. A longer run is a corridor hugging the wall, which tears
/// the wall open along its length; such floors are rejected and redrawn.
pub(super) const CORRIDOR_MOUTH_MAX: i32 = 4;

/// Percent of rooms that stay empty of decorative clutter.
const EMPTY_ROOM_PCT: i32 = 30;
/// Per-room clutter count is drawn from this inclusive range (capped by how
/// many wall/corner cells the room actually has).
const PROPS_PER_ROOM: std::ops::RangeInclusive<i32> = 5..=10;
/// Percent chance each prop is drawn from the corner pool rather than a plain
/// wall cell, when both are available — "주로 구석쪽으로".
const CORNER_BIAS: i32 = 80;
/// Percent chance a barrel/crate prop is doubled into a 2-high stack.
const PROP_STACK_PCT: i32 = 35;

const DEPTH_SALT: u64 = 0x9e37_79b9_7f4a_7c15;

#[cfg(test)]
pub fn dungeon_depth(seed: u64) -> u8 {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    rng.gen_range(MIN_DEPTH..=MAX_DEPTH)
}

pub fn generate_dungeon_with(
    seed: u64,
    floors_override: Option<u8>,
    boss: &str,
    entrance_dir: Option<WallDirection>,
) -> Vec<FloorLayout> {
    let mut meta = ChaCha8Rng::seed_from_u64(seed);
    // Always draw every seed-derived value so the meta stream is identical
    // with and without overrides.
    let drawn = meta.gen_range(MIN_DEPTH..=MAX_DEPTH);
    let total = floors_override.map_or(drawn, |f| f.clamp(1, MAX_DEPTH));
    let drawn_along_z = meta.gen_range(0..2) == 1;
    let drawn_reversed = meta.gen_range(0..2) == 1;
    // Compass → shaft: the door opens outward from the entry landing, so the
    // entry sits at the max end (`reversed`) for the positive directions.
    let (along_z, reversed) = match entrance_dir {
        None => (drawn_along_z, drawn_reversed),
        Some(WallDirection::North) => (true, false),
        Some(WallDirection::South) => (true, true),
        Some(WallDirection::West) => (false, false),
        Some(WallDirection::East) => (false, true),
    };

    // Entrance shaft: entry landing straddles the grid center so the
    // world-space entrance position sits on the top landing.
    let half = GRID / 2;
    let entrance_shaft = if along_z {
        StairShaft {
            x: half - 1,
            z: if reversed { half - SHAFT_LEN + 1 } else { half },
            along_z,
            reversed,
        }
    } else {
        StairShaft {
            x: if reversed { half - SHAFT_LEN + 1 } else { half },
            z: half - 1,
            along_z,
            reversed,
        }
    };

    let mut floors = Vec::with_capacity(total as usize);
    let mut up = entrance_shaft;
    for depth in 1..=total {
        let layout = generate_floor(seed, depth, total, up);
        let dead_end = layout.down_shaft.is_none();
        if let Some(down) = layout.down_shaft {
            up = down;
        }
        floors.push(layout);
        // A fallback floor that couldn't fit a down shaft promotes itself
        // to the final floor (chest + boss); stop descending.
        if dead_end {
            break;
        }
    }
    // The boss type comes from the registry, not the RNG; patch it in after
    // the deterministic rolls so the stream is untouched.
    if let Some(last) = floors.last_mut() {
        for spawn in last.spawns.iter_mut().filter(|s| s.is_boss) {
            spawn.monster_type = boss.to_string();
        }
    }
    floors
}

fn generate_floor(seed: u64, depth: u8, total: u8, up_shaft: StairShaft) -> FloorLayout {
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ (depth as u64).wrapping_mul(DEPTH_SALT));
    let is_last = depth == total;

    for _ in 0..FLOOR_ATTEMPTS {
        if let Some(layout) = try_generate_floor(&mut rng, depth, is_last, up_shaft) {
            return layout;
        }
    }
    fallback_floor(&mut rng, depth, is_last, up_shaft)
}

fn try_generate_floor(
    rng: &mut ChaCha8Rng,
    depth: u8,
    is_last: bool,
    up_shaft: StairShaft,
) -> Option<FloorLayout> {
    let room_count = rng.gen_range(3..=5) as usize;

    // Room 0 wraps the up shaft (its exit landing is this floor's arrival).
    let mut rooms = vec![place_room_around_shaft(rng, &up_shaft)?];

    // Remaining rooms; the last one is sized to host the down shaft
    // (or the boss + chest on the final floor — same generous sizing).
    let down_along_z = rng.gen_range(0..2) == 1;
    while rooms.len() < room_count {
        let hosts_shaft = rooms.len() == room_count - 1;
        let (w, d) = if hosts_shaft {
            if down_along_z {
                (
                    rng.gen_range(ROOM_MIN..=ROOM_MAX),
                    rng.gen_range(SHAFT_ROOM_AXIAL..=ROOM_MAX),
                )
            } else {
                (
                    rng.gen_range(SHAFT_ROOM_AXIAL..=ROOM_MAX),
                    rng.gen_range(ROOM_MIN..=ROOM_MAX),
                )
            }
        } else {
            (
                rng.gen_range(ROOM_MIN..=ROOM_MAX),
                rng.gen_range(ROOM_MIN..=ROOM_MAX),
            )
        };
        let room = place_room_free(rng, w, d, &rooms, &up_shaft)?;
        rooms.push(room);
    }

    // Down shaft inside the last room.
    let down_shaft = if is_last {
        None
    } else {
        Some(place_shaft_in_room(
            rng,
            rooms.last().unwrap(),
            down_along_z,
        ))
    };

    // Carve rooms, shafts, then room-chain corridors.
    let mut carved = vec![false; (GRID * GRID) as usize];
    for room in &rooms {
        carve_rect(&mut carved, room);
    }
    carve_rect(&mut carved, &up_shaft.rect());
    if let Some(ref down) = down_shaft {
        carve_rect(&mut carved, &down.rect());
    }
    // A locked floor's stair room keeps a single exit: no later corridor may
    // pass through or hug it, or the locked door would have a way around.
    let keep_out = is_locked_depth(depth).then(|| rooms[0].expanded(1));
    for i in 0..rooms.len() - 1 {
        let (from, to) = (rooms[i].center(), rooms[i + 1].center());
        let avoid = keep_out.filter(|_| i > 0);
        let x_first = avoid.is_none_or(|r| !corridor_intersects(from, to, true, &r));
        if !x_first && avoid.is_some_and(|r| corridor_intersects(from, to, false, &r)) {
            return None;
        }
        carve_corridor(&mut carved, from, to, x_first);
    }

    let chest = if is_last {
        Some(rooms.last().unwrap().center())
    } else {
        None
    };

    let mut layout = FloorLayout {
        depth,
        rooms,
        carved,
        up_shaft,
        down_shaft,
        chest,
        spawns: Vec::new(),
        props: Vec::new(),
    };

    let openings = wall_openings(&layout);
    if openings.iter().any(|o| o.len > CORRIDOR_MOUTH_MAX) || !floor_is_connected(&layout) {
        return None;
    }
    if is_locked_depth(depth) && openings.iter().filter(|o| o.room == 0).count() != 1 {
        return None;
    }

    layout.spawns = roll_spawns(rng, &layout);
    layout.props = roll_props(rng, &layout);
    layout.props.extend(roll_wall_torches(rng, &layout));
    Some(layout)
}

/// Room 0: random dims/position constrained to contain the up shaft with
/// a ≥1-cell margin on every side.
fn place_room_around_shaft(rng: &mut ChaCha8Rng, shaft: &StairShaft) -> Option<Room> {
    let r = shaft.rect();
    let (w, d) = if shaft.along_z {
        (
            rng.gen_range(ROOM_MIN..=ROOM_MAX),
            rng.gen_range(SHAFT_ROOM_AXIAL..=ROOM_MAX),
        )
    } else {
        (
            rng.gen_range(SHAFT_ROOM_AXIAL..=ROOM_MAX),
            rng.gen_range(ROOM_MIN..=ROOM_MAX),
        )
    };

    let x = range_pick(rng, (r.x + r.w + 1 - w).max(1), (r.x - 1).min(GRID - 1 - w))?;
    let z = range_pick(rng, (r.z + r.d + 1 - d).max(1), (r.z - 1).min(GRID - 1 - d))?;
    Some(Room { x, z, w, d })
}

fn place_room_free(
    rng: &mut ChaCha8Rng,
    w: i32,
    d: i32,
    rooms: &[Room],
    up_shaft: &StairShaft,
) -> Option<Room> {
    let shaft_zone = up_shaft.rect().expanded(1);
    for _ in 0..ROOM_PLACE_ATTEMPTS {
        let x = rng.gen_range(1..=GRID - 1 - w);
        let z = rng.gen_range(1..=GRID - 1 - d);
        let candidate = Room { x, z, w, d };
        let grown = candidate.expanded(1);
        if grown.intersects(&shaft_zone) {
            continue;
        }
        if rooms.iter().any(|r| grown.intersects(r)) {
            continue;
        }
        return Some(candidate);
    }
    None
}

fn place_shaft_in_room(rng: &mut ChaCha8Rng, room: &Room, along_z: bool) -> StairShaft {
    let (sw, sl) = if along_z {
        (SHAFT_W, SHAFT_LEN)
    } else {
        (SHAFT_LEN, SHAFT_W)
    };
    // Interior placement with a 1-cell margin; room sizing guarantees fit.
    let x = rng.gen_range(room.x + 1..=room.x + room.w - 1 - sw);
    let z = rng.gen_range(room.z + 1..=room.z + room.d - 1 - sl);
    StairShaft {
        x,
        z,
        along_z,
        reversed: rng.gen_range(0..2) == 1,
    }
}

fn range_pick(rng: &mut ChaCha8Rng, lo: i32, hi: i32) -> Option<i32> {
    if lo > hi {
        return None;
    }
    Some(rng.gen_range(lo..=hi))
}

fn carve_rect(carved: &mut [bool], r: &Room) {
    for z in r.z.max(0)..(r.z + r.d).min(GRID) {
        for x in r.x.max(0)..(r.x + r.w).min(GRID) {
            carved[(x + z * GRID) as usize] = true;
        }
    }
}

/// The two 2-wide legs of an L corridor: X leg then Z leg (`x_first`), or
/// Z leg then X leg.
fn corridor_legs(from: (i32, i32), to: (i32, i32), x_first: bool) -> [Room; 2] {
    let (x0, z0) = from;
    let (x1, z1) = to;
    let x_leg = |z: i32| Room {
        x: x0.min(x1),
        z,
        w: (x0 - x1).abs() + 1,
        d: 2,
    };
    let z_leg = |x: i32| Room {
        x,
        z: z0.min(z1),
        w: 2,
        d: (z0 - z1).abs() + 1,
    };
    if x_first {
        [x_leg(z0), z_leg(x1)]
    } else {
        [z_leg(x0), x_leg(z1)]
    }
}

fn corridor_intersects(from: (i32, i32), to: (i32, i32), x_first: bool, rect: &Room) -> bool {
    corridor_legs(from, to, x_first)
        .iter()
        .any(|leg| leg.intersects(rect))
}

/// 2-cell-wide L corridor.
fn carve_corridor(carved: &mut [bool], from: (i32, i32), to: (i32, i32), x_first: bool) {
    for leg in corridor_legs(from, to, x_first) {
        for z in leg.z..leg.z + leg.d {
            for x in leg.x..leg.x + leg.w {
                if (1..GRID - 1).contains(&x) && (1..GRID - 1).contains(&z) {
                    carved[(x + z * GRID) as usize] = true;
                }
            }
        }
    }
}

/// Cells reachable from the up-shaft exit landing over a given edge-bitmask
/// grid. Walks the same edges real collision uses and treats stair-shaft
/// interior cells as blocked (they're only traversable vertically). Taking
/// `cells` as a parameter lets prop placement re-run this with props sealed in.
fn floor_reachable(
    layout: &FloorLayout,
    cells: &[u8],
    visited: &mut [bool],
    queue: &mut std::collections::VecDeque<(i32, i32)>,
) {
    visited.iter_mut().for_each(|v| *v = false);
    queue.clear();
    let start = layout.up_shaft.exit_cell();

    let is_interior = |x: i32, z: i32| -> bool {
        // Up shaft: only the exit row is a landing on this floor.
        if layout.up_shaft.contains(x, z) {
            let exit = layout.up_shaft.exit_cell();
            let on_exit_row = if layout.up_shaft.along_z {
                z == exit.1
            } else {
                x == exit.0
            };
            if !on_exit_row {
                return true;
            }
        }
        if let Some(ref down) = layout.down_shaft {
            if down.contains(x, z) {
                let entry = down.entry_cell();
                let on_entry_row = if down.along_z {
                    z == entry.1
                } else {
                    x == entry.0
                };
                if !on_entry_row {
                    return true;
                }
            }
        }
        false
    };

    visited[(start.0 + start.1 * GRID) as usize] = true;
    queue.push_back(start);

    const EDGE_BITS: [(i32, i32, u8, u8); 4] = [
        (0, -1, super::EDGE_N, super::EDGE_S),
        (0, 1, super::EDGE_S, super::EDGE_N),
        (1, 0, super::EDGE_E, super::EDGE_W),
        (-1, 0, super::EDGE_W, super::EDGE_E),
    ];

    while let Some((x, z)) = queue.pop_front() {
        for (dx, dz, leave, enter) in EDGE_BITS {
            let nx = x + dx;
            let nz = z + dz;
            if !(0..GRID).contains(&nx) || !(0..GRID).contains(&nz) {
                continue;
            }
            let nidx = (nx + nz * GRID) as usize;
            if visited[nidx] || !layout.carved[nidx] {
                continue;
            }
            if cells[(x + z * GRID) as usize] & leave != 0 || cells[nidx] & enter != 0 {
                continue;
            }
            if is_interior(nx, nz) {
                continue;
            }
            visited[nidx] = true;
            queue.push_back((nx, nz));
        }
    }
}

/// Whether every room center, the down-shaft entry, and the chest are in a
/// reachable set produced by [`floor_reachable`]. The chest cell is sealed for
/// good, so it counts as reached when a neighbour is — that's where the player
/// stands to open it.
fn floor_targets_reachable(layout: &FloorLayout, visited: &[bool]) -> bool {
    let visited_at =
        |(x, z): (i32, i32)| layout.is_carved(x, z) && visited[(x + z * GRID) as usize];
    let reachable = |cell: (i32, i32)| {
        if layout.chest == Some(cell) {
            layout.approach_cells(cell).any(visited_at)
        } else {
            visited_at(cell)
        }
    };

    for room in &layout.rooms {
        if !reachable(room.center()) {
            return false;
        }
    }
    if let Some(ref down) = layout.down_shaft {
        if !reachable(down.entry_cell()) {
            return false;
        }
    }
    if let Some(chest) = layout.chest {
        if !reachable(chest) {
            return false;
        }
    }
    true
}

/// Edge-aware reachability gate run during generation (before props exist):
/// every room center, the down-shaft entry, and the chest must be reachable
/// from the up-shaft exit landing.
fn floor_is_connected(layout: &FloorLayout) -> bool {
    let cells = super::floor_passability_cells(layout);
    let mut visited = vec![false; (GRID * GRID) as usize];
    let mut queue = std::collections::VecDeque::new();
    floor_reachable(layout, &cells, &mut visited, &mut queue);
    floor_targets_reachable(layout, &visited)
}

fn roll_spawns(rng: &mut ChaCha8Rng, layout: &FloorLayout) -> Vec<SpawnSpec> {
    let in_shaft = |x: i32, z: i32| cell_in_any_shaft(layout, x, z);

    // Deterministic candidate order: row-major scan.
    let mut candidates: Vec<(i32, i32)> = Vec::new();
    for z in 0..GRID {
        for x in 0..GRID {
            if !layout.carved[(x + z * GRID) as usize] || in_shaft(x, z) {
                continue;
            }
            // Arrivals get a quiet room: nothing spawns where stairs land.
            if cell_in_stair_room(layout, x, z) {
                continue;
            }
            if layout.chest == Some((x, z)) {
                continue;
            }
            candidates.push((x, z));
        }
    }

    let mut spawns = Vec::new();

    if let Some((cx, cz)) = layout.chest {
        // Boss guards the chest on the final floor.
        let boss_cell = [(cx + 2, cz), (cx - 2, cz), (cx, cz + 2), (cx, cz - 2)]
            .into_iter()
            .find(|&(x, z)| layout.is_carved(x, z) && !in_shaft(x, z))
            .unwrap_or((cx, cz));
        spawns.push(SpawnSpec {
            x: boss_cell.0,
            z: boss_cell.1,
            monster_type: BOSS_MONSTER_TYPE.to_string(),
            is_boss: true,
            // The treasure guardian hunts intruders on sight.
            aggressive: true,
        });
    }

    let table = super::spawn_table(layout.depth);
    let total_weight: u32 = table.iter().map(|e| e.weight).sum();
    if total_weight == 0 {
        return spawns;
    }
    // First floor is the entry floor: keep it lighter so newcomers ease in.
    let count = if layout.depth == 1 {
        rng.gen_range(3..=5) as usize
    } else {
        rng.gen_range(5..=9) as usize
    };

    for _ in 0..count.min(candidates.len()) {
        // `as u32`: `usize` is u32 on wasm32 but u64 on native, and
        // `gen_range` over each width consumes a different amount of the RNG
        // stream — desyncing client (wasm) from server (native) here, and in
        // every draw after. Fixed-width keeps the layout identical on both.
        let idx = rng.gen_range(0..candidates.len() as u32) as usize;
        let (x, z) = candidates.swap_remove(idx);
        let mut roll = rng.gen_range(0..total_weight);
        let mut chosen = &table[0];
        for entry in table {
            if roll < entry.weight {
                chosen = entry;
                break;
            }
            roll -= entry.weight;
        }
        spawns.push(SpawnSpec {
            x,
            z,
            monster_type: chosen.monster_type.clone(),
            is_boss: false,
            aggressive: chosen.aggressive,
        });
    }

    spawns
}

pub(super) fn cell_in_any_room(layout: &FloorLayout, x: i32, z: i32) -> bool {
    layout.rooms.iter().any(|r| r.contains(x, z))
}

/// Whether the cell lies in a room holding a stair shaft (up or down).
pub(super) fn cell_in_stair_room(layout: &FloorLayout, x: i32, z: i32) -> bool {
    layout.rooms.iter().any(|r| {
        r.contains(x, z)
            && (r.intersects(&layout.up_shaft.rect())
                || layout
                    .down_shaft
                    .as_ref()
                    .is_some_and(|s| r.intersects(&s.rect())))
    })
}

pub(super) fn cell_in_any_shaft(layout: &FloorLayout, x: i32, z: i32) -> bool {
    layout.up_shaft.contains(x, z) || layout.down_shaft.as_ref().is_some_and(|s| s.contains(x, z))
}

/// Carved floor that belongs to a connecting corridor (not a room, not a shaft).
pub(super) fn cell_is_corridor(layout: &FloorLayout, x: i32, z: i32) -> bool {
    layout.is_carved(x, z) && !cell_in_any_room(layout, x, z) && !cell_in_any_shaft(layout, x, z)
}

/// Number of orthogonal neighbours that are solid rock (uncarved). 0 = open
/// floor, 1 = flush against a wall, ≥2 = a corner nook.
fn wall_sides(layout: &FloorLayout, x: i32, z: i32) -> u8 {
    let mut n = 0;
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        if !layout.is_carved(x + dx, z + dz) {
            n += 1;
        }
    }
    n
}

/// Wall a chest backs onto, as the cell delta toward it: the first uncarved
/// neighbour in the client's N, S, W, E order (no wall → the client defaults
/// to backing north).
pub(super) fn chest_back_delta(layout: &FloorLayout, x: i32, z: i32) -> (i32, i32) {
    [(0, -1), (0, 1), (-1, 0), (1, 0)]
        .into_iter()
        .find(|&(dx, dz)| !layout.is_carved(x + dx, z + dz))
        .unwrap_or((0, -1))
}

/// The two cells flanking a chest along its long side (which runs along the
/// backed wall); the ~1.5m model overflows the 1m cell into these flanks, so
/// they must hold no other solid prop.
pub(super) fn flank_cells(x: i32, z: i32, back: (i32, i32)) -> [(i32, i32); 2] {
    match back {
        (0, _) => [(x - 1, z), (x + 1, z)],
        _ => [(x, z - 1), (x, z + 1)],
    }
}

/// Yaw the client renders a chest with: hinge onto the backed wall
/// (N → 0°, S → 180°, W → 90°, E → 270°).
pub(super) fn chest_yaw(back: (i32, i32)) -> u16 {
    match back {
        (0, -1) => 0,
        (0, 1) => 180,
        (-1, 0) => 90,
        _ => 270,
    }
}

/// Whether `(x, z)` is a sound spot to drop a clutter prop: carved room floor,
/// clear of the stairs/chest/monsters, and not standing in a doorway. The last
/// rule is the whole point of the placement constraints — a prop may hug a wall
/// or sit beside the stair shaft, but must never block a corridor mouth or the
/// landing cell you step through to take the stairs.
fn prop_cell_ok(
    layout: &FloorLayout,
    landings: &[(i32, i32)],
    taken: &[bool],
    x: i32,
    z: i32,
) -> bool {
    if !layout.is_carved(x, z) || cell_in_any_shaft(layout, x, z) {
        return false;
    }
    if layout.chest == Some((x, z)) || taken[(x + z * GRID) as usize] {
        return false;
    }
    if layout.spawns.iter().any(|s| s.x == x && s.z == z) {
        return false;
    }
    // Don't choke a corridor entrance or a stair-landing approach: reject any
    // cell orthogonally touching a corridor cell or a landing cell. Cells beside
    // the (walled-off) shaft body are fine — "계단 옆쪽에 붙이는 것은 괜찮음".
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let (nx, nz) = (x + dx, z + dz);
        if cell_is_corridor(layout, nx, nz) || landings.contains(&(nx, nz)) {
            return false;
        }
    }
    true
}

/// Stair-landing cells to keep clear of props: this floor's up-shaft exit row
/// and the down-shaft entry row. A prop dropped in front of one would wall off
/// the stairs.
fn collect_landing_cells(layout: &FloorLayout) -> Vec<(i32, i32)> {
    let mut landings: Vec<(i32, i32)> = Vec::new();
    for w in 0..SHAFT_W {
        landings.push(layout.up_shaft.step_cell(SHAFT_LEN - 1, w));
    }
    if let Some(ref down) = layout.down_shaft {
        for w in 0..SHAFT_W {
            landings.push(down.step_cell(0, w));
        }
    }
    landings
}

/// Hang at most one wall torch per room at the centre of its north or east wall,
/// the side chosen at random. Purely cosmetic like [`roll_props`] — the torch
/// sits high on the wall and is never added to the passability grid. It still
/// honours the same clutter placement rules ([`prop_cell_ok`]: clear of the
/// chest/spawns/shafts, never choking a corridor mouth or stair landing, and one
/// prop per cell — `taken` is seeded with the clutter already placed), plus a
/// solid cell on the chosen side to actually mount on. The `rotation` field
/// carries the room-facing yaw the client renders with (north wall → 0°, east
/// wall → 270°). Deterministic: rooms visited in order, exactly one integer draw
/// each (the `as u32` cast keeps wasm and native in lockstep — see `roll_props`).
fn roll_wall_torches(rng: &mut ChaCha8Rng, layout: &FloorLayout) -> Vec<PropSpec> {
    let landings = collect_landing_cells(layout);
    // Seed `taken` with the clutter already placed so a torch never shares a cell.
    let mut taken = vec![false; (GRID * GRID) as usize];
    for p in &layout.props {
        taken[(p.x + p.z * GRID) as usize] = true;
    }

    let mut torches = Vec::new();
    for room in &layout.rooms {
        // North wall centre: top (−Z) row, mid-width. East wall centre: right
        // (+X) column, mid-depth. A side works only when the cell is a sound prop
        // spot and the cell just outside it is solid (a real wall to hang on).
        let (nx, nz) = (room.x + room.w / 2, room.z);
        let (ex, ez) = (room.x + room.w - 1, room.z + room.d / 2);
        // (valid?, x, z, room-facing yaw) for each candidate wall.
        let sides = [
            (
                !layout.is_carved(nx, nz - 1) && prop_cell_ok(layout, &landings, &taken, nx, nz),
                nx,
                nz,
                0u16,
            ),
            (
                !layout.is_carved(ex + 1, ez) && prop_cell_ok(layout, &landings, &taken, ex, ez),
                ex,
                ez,
                270u16,
            ),
        ];
        // Take the first valid side, north-first or east-first per the coin flip.
        let north_first = rng.gen_range(0..2u32) == 0;
        let pick = if north_first {
            sides.iter().find(|s| s.0)
        } else {
            sides.iter().rev().find(|s| s.0)
        };
        if let Some(&(_, x, z, rotation)) = pick {
            taken[(x + z * GRID) as usize] = true;
            torches.push(PropSpec {
                x,
                z,
                kind: PropKind::TorchWall,
                stack: 1,
                rotation,
            });
        }
    }
    torches
}

fn pick_prop_kind(rng: &mut ChaCha8Rng) -> PropKind {
    // Barrels and crates are the common dungeon clutter; chests are the rare
    // find. Weighted 5 : 4 : 2 over a draw of 11.
    match rng.gen_range(0..11) {
        0..=4 => PropKind::Barrel,
        5..=8 => PropKind::Crate,
        _ => PropKind::Chest,
    }
}

/// Scatter a few decorative barrels/crates/chests through the rooms, biased
/// toward corners and walls. Props are *solid* — [`floor_passability_cells`]
/// seals their cell — so placement keeps corridor mouths and stair landings
/// clear (see [`prop_cell_ok`]) and, as a backstop, rejects any individual prop
/// that would wall a room off (re-checking reachability with it sealed). Runs
/// after the layout is otherwise final. Deterministic: rooms visited in order,
/// candidate cells gathered row-major, every random draw integer, and the
/// connectivity check is pure — same seed, same clutter on server and client.
fn roll_props(rng: &mut ChaCha8Rng, layout: &FloorLayout) -> Vec<PropSpec> {
    // Landing cells (this floor's own up-shaft exit row + the down-shaft entry
    // row): the spots a prop in front of would wall off the stairs.
    let landings = collect_landing_cells(layout);

    let mut taken = vec![false; (GRID * GRID) as usize];
    // The treasure chest renders yaw-0 (long side along X); reserve its
    // flanks like a placed chest's so clutter can't clip its body.
    if let Some((cx, cz)) = layout.chest {
        for (fx, fz) in flank_cells(cx, cz, (0, -1)) {
            if layout.is_carved(fx, fz) {
                taken[(fx + fz * GRID) as usize] = true;
            }
        }
    }
    let mut props = Vec::new();
    // Base passability for the connectivity backstop. `layout.props` is empty at
    // this point, so this has no prop seals yet; we add each kept prop's seal as
    // we commit it and test a tentative seal before committing. The BFS buffers
    // are allocated once and reused across every tentative check.
    let mut cells = super::floor_passability_cells(layout);
    let mut visited = vec![false; (GRID * GRID) as usize];
    let mut queue = std::collections::VecDeque::new();

    for room in &layout.rooms {
        if rng.gen_range(0..100) < EMPTY_ROOM_PCT {
            continue;
        }

        // Partition the room's eligible cells into corner nooks and plain wall
        // cells (row-major scan keeps the order stable across builds).
        let mut corners: Vec<(i32, i32)> = Vec::new();
        let mut edges: Vec<(i32, i32)> = Vec::new();
        for z in room.z..room.z + room.d {
            for x in room.x..room.x + room.w {
                if !prop_cell_ok(layout, &landings, &taken, x, z) {
                    continue;
                }
                match wall_sides(layout, x, z) {
                    n if n >= 2 => corners.push((x, z)),
                    1 => edges.push((x, z)),
                    _ => {}
                }
            }
        }
        if corners.is_empty() && edges.is_empty() {
            continue;
        }

        let target = rng.gen_range(PROPS_PER_ROOM);
        for _ in 0..target {
            // Prefer a corner; fall back to a wall cell (and vice-versa when one
            // pool runs dry).
            let from_corner = if corners.is_empty() {
                false
            } else if edges.is_empty() {
                true
            } else {
                rng.gen_range(0..100) < CORNER_BIAS
            };
            let pool = if from_corner {
                &mut corners
            } else {
                &mut edges
            };
            if pool.is_empty() {
                break;
            }
            // `as u32`: see roll_spawns — `gen_range` over `usize` (u32 on
            // wasm, u64 on native) draws differently per platform, desyncing the
            // RNG. This is the draw whose divergence shows up as props sitting in
            // different cells on the client vs the server.
            let (x, z) = pool.swap_remove(rng.gen_range(0..pool.len() as u32) as usize);
            let idx = (x + z * GRID) as usize;
            taken[idx] = true;

            // Backstop: don't let this prop seal off any room/chest/stairs.
            // Tentatively wall the cell and re-check reachability; restore the
            // cell's original edges and skip if it disconnects something. (No
            // RNG drawn for a rejected prop, so the stream stays deterministic
            // across server/client.)
            let saved = cells[idx];
            cells[idx] |= super::EDGE_ALL;
            floor_reachable(layout, &cells, &mut visited, &mut queue);
            if !floor_targets_reachable(layout, &visited) {
                cells[idx] = saved;
                continue;
            }

            // A chest's ~1.5m body overflows its 1m cell along the backed
            // wall, and loot spilled from it would roll down a stair shaft in
            // front: demote to a crate when a flank is already taken or the
            // opening faces a shaft, and reserve both flanks once a chest
            // stands. The demote path takes the stack draw a chest would
            // skip, but the branch is a pure function of shared state, so
            // server and client still walk identical streams. (Flank indices
            // need no bounds check: rooms keep a 1-cell border inside the
            // grid.)
            let mut kind = pick_prop_kind(rng);
            let mut chest_back = None;
            if kind == PropKind::Chest {
                let back = chest_back_delta(layout, x, z);
                let flanks = flank_cells(x, z, back);
                let front = (x - back.0, z - back.1);
                if flanks
                    .iter()
                    .any(|&(fx, fz)| taken[(fx + fz * GRID) as usize])
                    || cell_in_any_shaft(layout, front.0, front.1)
                {
                    kind = PropKind::Crate;
                } else {
                    for (fx, fz) in flanks {
                        taken[(fx + fz * GRID) as usize] = true;
                        corners.retain(|&c| c != (fx, fz));
                        edges.retain(|&c| c != (fx, fz));
                    }
                    chest_back = Some(back);
                }
            }
            let stack = if kind != PropKind::Chest && rng.gen_range(0..100) < PROP_STACK_PCT {
                2
            } else {
                1
            };
            // Chests carry their back-wall yaw (like torches) so the client
            // never re-derives the wall pick; the draw still happens to keep
            // the stream layout-independent.
            let mut rotation = rng.gen_range(0..360) as u16;
            if let Some(back) = chest_back {
                rotation = chest_yaw(back);
            }
            props.push(PropSpec {
                x,
                z,
                kind,
                stack,
                rotation,
            });
        }
    }

    props
}

/// Guaranteed-valid degenerate layout: one large room wrapping the up
/// shaft (clamped to the grid), with the down shaft placed by
/// deterministic scan. Only reached if `FLOOR_ATTEMPTS` randomized
/// layouts all failed validation, which should be vanishingly rare.
fn fallback_floor(
    rng: &mut ChaCha8Rng,
    depth: u8,
    is_last: bool,
    up_shaft: StairShaft,
) -> FloorLayout {
    let r = up_shaft.rect();
    let x0 = (r.x - 8).max(1);
    let z0 = (r.z - 8).max(1);
    let x1 = (r.x + r.w + 8).min(GRID - 1);
    let z1 = (r.z + r.d + 8).min(GRID - 1);
    let room = Room {
        x: x0,
        z: z0,
        w: x1 - x0,
        d: z1 - z0,
    };

    let down_shaft = if is_last {
        None
    } else {
        let up_zone = r.expanded(1);
        let mut found = None;
        'scan: for z in room.z + 1..room.z + room.d - 1 - SHAFT_LEN {
            for x in room.x + 1..room.x + room.w - 1 - SHAFT_W {
                let cand = StairShaft {
                    x,
                    z,
                    along_z: true,
                    reversed: false,
                };
                if !cand.rect().expanded(1).intersects(&up_zone) {
                    found = Some(cand);
                    break 'scan;
                }
            }
        }
        // The wrap room is ≥17 cells on each axis around the shaft, so a
        // slot virtually always exists; if not, this floor promotes
        // itself to the final floor (generate_dungeon stops descending).
        found
    };

    let mut carved = vec![false; (GRID * GRID) as usize];
    carve_rect(&mut carved, &room);
    carve_rect(&mut carved, &up_shaft.rect());
    if let Some(ref down) = down_shaft {
        carve_rect(&mut carved, &down.rect());
    }

    let in_shaft = |x: i32, z: i32| {
        up_shaft.contains(x, z) || down_shaft.as_ref().is_some_and(|s| s.contains(x, z))
    };
    let chest = if is_last || down_shaft.is_none() {
        let (cx, cz) = room.center();

        [
            (cx, cz),
            (room.x + 2, room.z + 2),
            (room.x + room.w - 3, room.z + 2),
            (room.x + 2, room.z + room.d - 3),
            (room.x + room.w - 3, room.z + room.d - 3),
        ]
        .into_iter()
        .find(|&(x, z)| !in_shaft(x, z))
    } else {
        None
    };

    let mut layout = FloorLayout {
        depth,
        rooms: vec![room],
        carved,
        up_shaft,
        down_shaft,
        chest,
        spawns: Vec::new(),
        props: Vec::new(),
    };
    layout.spawns = roll_spawns(rng, &layout);
    layout.props = roll_props(rng, &layout);
    layout.props.extend(roll_wall_torches(rng, &layout));
    layout
}
