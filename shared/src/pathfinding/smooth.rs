//! Greedy line-of-sight path smoothing on top of `find_path`.
//! `find_and_smooth_path` is the single entry-point most callers use:
//! it runs A*, prepends the player's continuous start position, then
//! collapses cardinal A* zig-zags into the longest straight runs each
//! cell-edge mask still permits. Smoothing only spans within a single
//! floor; floor-transition waypoints (stairwell entry/exit) are anchored
//! so the path stays cardinal across the seam.

use super::astar::find_path;
use super::query::{
    is_cardinal_move_blocked, is_circle_blocked_on_floor, segment_obstacles, SegmentObstacles,
};
use super::{PassabilityCache, PathResult, PathWaypoint};

/// Player collision half-width, mirroring the client's `PLAYER_RADIUS` used by
/// the continuous mover (`player-physics.ts`). Smoothing rejects any diagonal
/// whose interior would bring a body of this radius into a wall.
const PLAYER_RADIUS: f32 = 0.3;

/// Greedy line-of-sight path smoothing. Only smooths within the same floor level.
fn smooth_path(waypoints: &[PathWaypoint], cache: &PassabilityCache) -> Vec<PathWaypoint> {
    if waypoints.len() <= 2 {
        return waypoints.to_vec();
    }

    let mut result = vec![waypoints[0].clone()];
    let mut anchor = 0;

    while anchor < waypoints.len() - 1 {
        let mut farthest = anchor + 1;

        // Don't smooth from floor-transition points (stairwell exit/entry).
        // The first step after a floor change must stay cardinal to avoid
        // diagonal paths that clip stairwell side-walls.
        let is_floor_transition =
            anchor > 0 && waypoints[anchor].floor != waypoints[anchor - 1].floor;

        if !is_floor_transition {
            for probe in anchor + 2..waypoints.len() {
                if waypoints[probe].floor != waypoints[anchor].floor {
                    break;
                }
                if is_line_passable(&waypoints[anchor], &waypoints[probe], cache) {
                    farthest = probe;
                } else {
                    break;
                }
            }
        }

        result.push(waypoints[farthest].clone());
        anchor = farthest;
    }

    result
}

/// Line-of-sight check for path smoothing: the point-thickness Bresenham cell
/// walk, plus a body-radius sweep that rejects diagonals whose interior would
/// clip a convex wall corner the player's body can't clear.
pub(super) fn is_line_passable(
    from: &PathWaypoint,
    to: &PathWaypoint,
    cache: &PassabilityCache,
) -> bool {
    if !cells_line_passable(from, to, cache) {
        return false;
    }
    // Cell-edge traversal permits diagonals whose interior grazes a convex wall
    // corner within the body radius — the continuous mover then refuses to
    // cross, stranding anything without a wall-slide fallback (monsters, agents).
    // Reject such a segment ONLY when it's a genuine mid-path "notch": both
    // endpoints clear of walls but the interior isn't. When an endpoint sits
    // against a wall, a near-wall start/goal is expected and the mover just
    // stops short there, so leave the segment passable.
    let r = PLAYER_RADIUS;
    if is_circle_blocked_on_floor(cache, from.x, from.z, r, from.floor, None)
        || is_circle_blocked_on_floor(cache, to.x, to.z, r, from.floor, None)
    {
        return true;
    }
    !body_clips_wall(from, to, cache)
}

/// Sample the segment interior for a wall the body radius can't clear.
fn body_clips_wall(from: &PathWaypoint, to: &PathWaypoint, cache: &PassabilityCache) -> bool {
    let floor = from.floor;
    let r = PLAYER_RADIUS;
    let dx = to.x - from.x;
    let dz = to.z - from.z;
    let len = (dx * dx + dz * dz).sqrt();
    // Step finer than the radius so a corner notch can't slip between samples.
    let steps = (len / (r * 0.5)).ceil() as i32;
    for i in 1..steps {
        let t = i as f32 / steps as f32;
        if is_circle_blocked_on_floor(cache, from.x + dx * t, from.z + dz * t, r, floor, None) {
            return true;
        }
    }
    false
}

/// Bresenham cell-edge line-of-sight: the original point-thickness check.
fn cells_line_passable(from: &PathWaypoint, to: &PathWaypoint, cache: &PassabilityCache) -> bool {
    let floor = from.floor;
    let x0 = from.x.floor() as i32;
    let z0 = from.z.floor() as i32;
    let x1 = to.x.floor() as i32;
    let z1 = to.z.floor() as i32;

    if x0 == x1 && z0 == z1 {
        return true;
    }

    let dx = (x1 - x0).abs();
    let dz = (z1 - z0).abs();
    let sx = (x1 - x0).signum();
    let sz = (z1 - z0).signum();

    let mut x = x0;
    let mut z = z0;
    let mut err = dx - dz;

    loop {
        if x == x1 && z == z1 {
            return true;
        }

        let e2 = 2 * err;
        let step_x = e2 > -dz;
        let step_z = e2 < dx;

        if step_x && step_z {
            // Diagonal: both L-paths must be clear
            if is_cardinal_move_blocked(cache, x, z, sx, 0, floor)
                || is_cardinal_move_blocked(cache, x + sx, z, 0, sz, floor)
            {
                return false;
            }
            if is_cardinal_move_blocked(cache, x, z, 0, sz, floor)
                || is_cardinal_move_blocked(cache, x, z + sz, sx, 0, floor)
            {
                return false;
            }
            x += sx;
            z += sz;
            err += dx - dz;
        } else if step_x {
            if is_cardinal_move_blocked(cache, x, z, sx, 0, floor) {
                return false;
            }
            x += sx;
            err -= dz;
        } else {
            if is_cardinal_move_blocked(cache, x, z, 0, sz, floor) {
                return false;
            }
            z += sz;
            err += dx;
        }
    }
}

/// Same-floor direct shortcut: when the straight start→goal segment passes the
/// exact line-of-sight test smoothing applies, A* could only produce a path
/// that collapses to this segment, so the search is skipped. Stairwell contact
/// falls back to A* (see `segment_obstacles`), and a segment nowhere near any
/// obstacle is accepted without walking the line.
fn direct_line_ok(
    start_x: f32,
    start_z: f32,
    goal_x: f32,
    goal_z: f32,
    floor: u8,
    cache: &PassabilityCache,
) -> bool {
    let min_x = start_x.min(goal_x);
    let max_x = start_x.max(goal_x);
    let min_z = start_z.min(goal_z);
    let max_z = start_z.max(goal_z);
    match segment_obstacles(cache, min_x, max_x, min_z, max_z, floor, PLAYER_RADIUS) {
        SegmentObstacles::Stairwell => false,
        SegmentObstacles::None => true,
        SegmentObstacles::Walls => {
            let from = PathWaypoint {
                x: start_x,
                z: start_z,
                floor,
            };
            let to = PathWaypoint {
                x: goal_x,
                z: goal_z,
                floor,
            };
            is_line_passable(&from, &to, cache)
        }
    }
}

/// Convenience: find path and smooth it in one call.
#[allow(clippy::too_many_arguments)]
pub fn find_and_smooth_path(
    start_x: f32,
    start_z: f32,
    start_floor: u8,
    goal_x: f32,
    goal_z: f32,
    goal_floor: u8,
    cache: &PassabilityCache,
    max_nodes: usize,
) -> PathResult {
    if start_floor == goal_floor
        && direct_line_ok(start_x, start_z, goal_x, goal_z, start_floor, cache)
    {
        return PathResult {
            waypoints: vec![PathWaypoint {
                x: goal_x,
                z: goal_z,
                floor: goal_floor,
            }],
            found: true,
        };
    }

    let result = find_path(
        start_x,
        start_z,
        start_floor,
        goal_x,
        goal_z,
        goal_floor,
        cache,
        max_nodes,
    );
    if result.waypoints.is_empty() {
        return result;
    }
    // Prepend the player's actual position so smoothing can optimize the
    // entire trajectory (start → goal), not just (first A* cell → goal).
    let mut full_path = Vec::with_capacity(result.waypoints.len() + 1);
    full_path.push(PathWaypoint {
        x: start_x,
        z: start_z,
        floor: start_floor,
    });
    full_path.extend(result.waypoints);
    let smoothed = smooth_path(&full_path, cache);
    PathResult {
        // Remove the start position — the client already knows where the player is
        // and uses the first waypoint as the movement target.
        waypoints: if smoothed.len() > 1 {
            smoothed[1..].to_vec()
        } else {
            smoothed
        },
        found: result.found,
    }
}
