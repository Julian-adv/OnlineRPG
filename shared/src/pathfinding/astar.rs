//! A* search over the virtual 1m world grid with floor-level awareness.
//! Combines two cell types: regular floor cells (cardinal expansion via
//! the cell-edge mask) and stairwell intermediate cells (axis-only
//! expansion via the precomputed stair map). The goal is reachable at
//! any floor key on the goal floor — A* doesn't have to walk back to a
//! landing first if the goal cell is mid-stairwell.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap, HashSet};

use super::query::is_cardinal_move_blocked;
use super::stair::{build_stair_cells, floor_to_key, is_regular_key, key_to_floor, AStarKey};
use super::{PassabilityCache, PathResult, PathWaypoint, DIRS};

/// Default max nodes for A* expansion. Sized for multi-floor house traversal.
pub const DEFAULT_MAX_NODES: usize = 2000;

#[derive(Clone)]
struct AStarNode {
    x: i32,
    z: i32,
    /// Floor key: regular floors are multiples of FLOOR_SCALE,
    /// intermediate stairwell cells use values in between.
    fk: u16,
    g: u32,
    f: u32,
}

impl Eq for AStarNode {}
impl PartialEq for AStarNode {
    fn eq(&self, other: &Self) -> bool {
        self.f == other.f
    }
}
impl Ord for AStarNode {
    fn cmp(&self, other: &Self) -> Ordering {
        self.f.cmp(&other.f)
    }
}
impl PartialOrd for AStarNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

struct ClosedEntry {
    g: u32,
    parent: AStarKey,
}

/// Find a path on a virtual 1m world grid with floor-level awareness.
///
/// `start_floor`/`goal_floor` are floor *keys*, not "the floor nearest that Y":
/// an endpoint standing mid-stairwell belongs to the shallower connected floor
/// here. Resolve endpoints with [`super::start_floor_at`] rather than
/// [`super::get_floor_at_position`], or the search reaches the stairs only
/// through their far landing.
#[allow(clippy::too_many_arguments)]
pub fn find_path(
    start_x: f32,
    start_z: f32,
    start_floor: u8,
    goal_x: f32,
    goal_z: f32,
    goal_floor: u8,
    cache: &PassabilityCache,
    max_nodes: usize,
) -> PathResult {
    let sx = start_x.floor() as i32;
    let sz = start_z.floor() as i32;
    let gx = goal_x.floor() as i32;
    let gz = goal_z.floor() as i32;

    if sx == gx && sz == gz && start_floor == goal_floor {
        return PathResult {
            waypoints: vec![PathWaypoint {
                x: goal_x,
                z: goal_z,
                floor: goal_floor,
            }],
            found: true,
        };
    }

    let stair_cells = build_stair_cells(cache);

    // Build set of (x, z, real_floor) for stairwell cells.
    // Used to block regular-floor cardinal moves into stairwell interior cells,
    // but only for floors that the stairwell actually connects.
    let mut stair_positions: HashSet<(i32, i32, u8)> = HashSet::new();
    for &(x, z, fk) in stair_cells.keys() {
        stair_positions.insert((x, z, key_to_floor(fk)));
    }

    // Confine the search to a single floor when start and goal share it: a
    // same-floor room goal is always reachable without leaving the floor, so the
    // stairwell is never a legitimate shortcut. The exception is a click whose
    // target is the stairwell itself. Intermediate stair cells are keyed to the
    // shallower connected floor, so a landing→mid-stair click may have equal
    // start/goal floor numbers but still needs stair-axis expansion.
    let start_regular_key = (sx, sz, floor_to_key(start_floor));
    let start_is_stair_interior = stair_positions.contains(&(sx, sz, start_floor))
        && !stair_cells.contains_key(&start_regular_key);
    let goal_is_stair_position = stair_positions.contains(&(gx, gz, goal_floor));
    let confine_to_floor =
        start_floor == goal_floor && !start_is_stair_interior && !goal_is_stair_position;

    let start_fk = floor_to_key(start_floor);

    let mut open = BinaryHeap::new();
    let mut closed: HashMap<AStarKey, ClosedEntry> = HashMap::new();

    let h = |x: i32, z: i32, fk: u16| -> u32 {
        let real_f = key_to_floor(fk) as i32;
        let goal_f = goal_floor as i32;
        (x - gx).unsigned_abs() + (z - gz).unsigned_abs() + (real_f - goal_f).unsigned_abs() * 2
    };

    let start_h = h(sx, sz, start_fk);
    let start_key: AStarKey = (sx, sz, start_fk);
    open.push(Reverse(AStarNode {
        x: sx,
        z: sz,
        fk: start_fk,
        g: 0,
        f: start_h,
    }));
    closed.insert(
        start_key,
        ClosedEntry {
            g: 0,
            parent: start_key,
        },
    );

    // If start is on a stairwell intermediate cell, also seed that key
    // so the player doesn't have to walk back to entry landing first.
    // Skipped under floor confinement: those seeds only exist to start a
    // cross-floor climb/descent.
    if !confine_to_floor {
        for &key in stair_cells.keys() {
            let (kx, kz, kfk) = key;
            if kx == sx && kz == sz && key_to_floor(kfk) == start_floor && kfk != start_fk {
                let sh = h(sx, sz, kfk);
                open.push(Reverse(AStarNode {
                    x: sx,
                    z: sz,
                    fk: kfk,
                    g: 0,
                    f: sh,
                }));
                closed.insert(
                    key,
                    ClosedEntry {
                        g: 0,
                        parent: start_key,
                    },
                );
            }
        }
    }

    let mut best_h = start_h;
    let mut best_key = start_key;
    let mut expanded = 0;

    while let Some(Reverse(cur)) = open.pop() {
        if expanded >= max_nodes {
            break;
        }
        expanded += 1;

        let cur_key: AStarKey = (cur.x, cur.z, cur.fk);

        // Accept goal at exact fk or any intermediate stair fk on the same floor.
        // This handles clicking mid-stairwell where the cell is only reachable
        // via stair expansion with an intermediate fk, not the regular floor fk.
        if cur.x == gx && cur.z == gz && key_to_floor(cur.fk) == goal_floor {
            return PathResult {
                waypoints: reconstruct_path_vf(&closed, start_key, cur_key, goal_x, goal_z),
                found: true,
            };
        }

        if let Some(entry) = closed.get(&cur_key) {
            if cur.g > entry.g {
                continue;
            }
        }

        let on_regular = is_regular_key(cur.fk);
        let cur_floor = key_to_floor(cur.fk);

        // --- Regular floor expansion (cardinal neighbors) ---
        // Skip stairwell interior cells that aren't landings on this regular floor
        let is_stair_interior = stair_positions.contains(&(cur.x, cur.z, cur_floor))
            && !stair_cells.contains_key(&cur_key);
        if on_regular && !is_stair_interior {
            for &(dx, dz) in &DIRS {
                let nx = cur.x + dx;
                let nz = cur.z + dz;
                let new_g = cur.g + 1;

                if is_cardinal_move_blocked(cache, cur.x, cur.z, dx, dz, cur_floor) {
                    continue;
                }

                // Block moves into stairwell interior cells on regular floor.
                // A cell (nx, nz) is a stairwell interior if it belongs to a
                // stairwell but has no stair_cells entry at the current fk
                // (only landings match regular floor keys).
                if stair_positions.contains(&(nx, nz, cur_floor))
                    && !stair_cells.contains_key(&(nx, nz, cur.fk))
                {
                    continue;
                }

                let nkey: AStarKey = (nx, nz, cur.fk);
                if let Some(existing) = closed.get(&nkey) {
                    if existing.g <= new_g {
                        continue;
                    }
                }

                closed.insert(
                    nkey,
                    ClosedEntry {
                        g: new_g,
                        parent: cur_key,
                    },
                );
                let nh = h(nx, nz, cur.fk);
                open.push(Reverse(AStarNode {
                    x: nx,
                    z: nz,
                    fk: cur.fk,
                    g: new_g,
                    f: new_g + nh,
                }));
                if nh < best_h {
                    best_h = nh;
                    best_key = nkey;
                }
            }
        }

        // --- Stairwell axis expansion (prev/next along stairwell) ---
        // Disabled under floor confinement so the search never leaves
        // start_floor via the stairs; the bool short-circuits the per-node
        // stair-cell lookup on that (dungeon-monster) hot path.
        if !confine_to_floor {
            if let Some(sc) = stair_cells.get(&cur_key) {
                for neighbor in [&sc.prev, &sc.next].into_iter().flatten() {
                    let new_g = cur.g + 1;
                    let nkey: AStarKey = (neighbor.x, neighbor.z, neighbor.fk);
                    if let Some(existing) = closed.get(&nkey) {
                        if existing.g <= new_g {
                            continue;
                        }
                    }
                    closed.insert(
                        nkey,
                        ClosedEntry {
                            g: new_g,
                            parent: cur_key,
                        },
                    );
                    let nh = h(neighbor.x, neighbor.z, neighbor.fk);
                    open.push(Reverse(AStarNode {
                        x: neighbor.x,
                        z: neighbor.z,
                        fk: neighbor.fk,
                        g: new_g,
                        f: new_g + nh,
                    }));
                    if nh < best_h {
                        best_h = nh;
                        best_key = nkey;
                    }
                }
            }
        }
    }

    // Partial path to closest node
    let (bx, bz, _) = best_key;
    if best_key != start_key {
        return PathResult {
            waypoints: reconstruct_path_vf(
                &closed,
                start_key,
                best_key,
                bx as f32 + 0.5,
                bz as f32 + 0.5,
            ),
            found: false,
        };
    }

    PathResult {
        waypoints: Vec::new(),
        found: false,
    }
}

fn reconstruct_path_vf(
    closed: &HashMap<AStarKey, ClosedEntry>,
    start: AStarKey,
    end: AStarKey,
    final_x: f32,
    final_z: f32,
) -> Vec<PathWaypoint> {
    let mut path = Vec::new();
    let mut key = end;

    while key != start {
        let (cx, cz, fk) = key;
        // Only emit waypoints at regular floor levels (entry/exit landings).
        // Intermediate stairwell cells are skipped — the client interpolates
        // between the entry and exit landings, and GameSceneHousingLayer
        // handles the Y offset based on stairwell position.
        if is_regular_key(fk) {
            path.push(PathWaypoint {
                x: cx as f32 + 0.5,
                z: cz as f32 + 0.5,
                floor: key_to_floor(fk),
            });
        }
        let entry = match closed.get(&key) {
            Some(e) => e,
            None => break,
        };
        key = entry.parent;
    }

    path.reverse();

    if let Some(last) = path.last_mut() {
        last.x = final_x;
        last.z = final_z;
    }

    path
}
