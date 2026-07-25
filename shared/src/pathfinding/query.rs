//! Read-only spatial queries against the runtime passability cache. Two
//! flavours: per-edge collision checks (whether stepping or sliding from
//! one cell to a neighbour crosses a wall) used by both A* expansion and
//! continuous player movement, plus floor-level lookups that translate a
//! `(x, z, y)` world position to the floor it belongs to.

use super::stair::stair_step;
use super::{
    PassabilityCache, RuntimeFloorGrid, RuntimePassability, EDGE_E, EDGE_N, EDGE_S, EDGE_W,
};

/// Check if a cardinal (1-cell) move is blocked on a specific floor level.
/// Matches by `floor_level` only — no Y-range check, no proximity buffer.
///
/// `#[inline]` because this is hit per-neighbour by both A* expansion
/// (`astar::find_path`) and Bresenham line-of-sight (`smooth::is_line_passable`),
/// and lives in a different module than both callers.
#[inline]
pub fn is_cardinal_move_blocked(
    cache: &PassabilityCache,
    cell_x: i32,
    cell_z: i32,
    dx: i32,
    dz: i32,
    floor_level: u8,
) -> bool {
    let nx = cell_x + dx;
    let nz = cell_z + dz;
    let (leave_bit, enter_bit) = match (dx, dz) {
        (1, 0) => (EDGE_E, EDGE_W),
        (-1, 0) => (EDGE_W, EDGE_E),
        (0, 1) => (EDGE_S, EDGE_N),
        (0, -1) => (EDGE_N, EDGE_S),
        _ => return false,
    };

    let cx_f = cell_x as f32;
    let nxf = nx as f32;
    let cz_f = cell_z as f32;
    let nzf = nz as f32;
    for rp in cache.values() {
        if cx_f < rp.min_x && nxf < rp.min_x {
            continue;
        }
        if cx_f > rp.max_x && nxf > rp.max_x {
            continue;
        }
        if cz_f < rp.min_z && nzf < rp.min_z {
            continue;
        }
        if cz_f > rp.max_z && nzf > rp.max_z {
            continue;
        }

        let house_ox = rp.house_origin_x.floor() as i32;
        let house_oz = rp.house_origin_z.floor() as i32;
        for floor in &rp.floors {
            if floor.floor_level != floor_level {
                continue;
            }
            let fx = house_ox + floor.origin_x;
            let fz = house_oz + floor.origin_z;
            let w = floor.width as i32;
            let d = floor.depth as i32;

            let gx = cell_x - fx;
            let gz = cell_z - fz;
            if gx >= 0
                && gx < w
                && gz >= 0
                && gz < d
                && floor.cells[(gx + gz * w) as usize] & leave_bit != 0
            {
                return true;
            }

            let ngx = nx - fx;
            let ngz = nz - fz;
            if ngx >= 0
                && ngx < w
                && ngz >= 0
                && ngz < d
                && floor.cells[(ngx + ngz * w) as usize] & enter_bit != 0
            {
                return true;
            }
        }
    }
    false
}

/// Check if movement from→to crosses any blocked cell edge on `floor_level`.
///
/// Floor indices are globally unique across the cache — housing uses 0..3 and
/// dungeons start at `dungeon::DUNGEON_FLOOR_INDEX_BASE` — so an exact match is
/// all it takes to keep a crypt's walls away from the houses above it.
pub fn is_movement_blocked(
    cache: &PassabilityCache,
    from_x: f32,
    from_z: f32,
    to_x: f32,
    to_z: f32,
    floor_level: u8,
    y: Option<f32>,
) -> bool {
    blocking_entries(cache, from_x, from_z, to_x, to_z, floor_level, y)
        .next()
        .is_some()
}

/// Why a move was refused: which cache entry, and whether the stairwell
/// two-floor consult decided it. A stairwell refusal with `consulted == 1`
/// means the "block only when all refuse" rule came down to a single grid —
/// the one sealing the end the mover stands on — which is how a stairwell
/// traps someone rather than merely blocking them.
#[derive(Debug, Clone, Copy)]
pub struct BlockInfo<'a> {
    pub key: &'a str,
    pub stairwell: bool,
    pub consulted: usize,
}

/// Every cache entry that refuses this move, paired with its verdict.
/// Lazy, so a caller wanting only the first pays for only the first.
fn blocking_entries<'a>(
    cache: &'a PassabilityCache,
    from_x: f32,
    from_z: f32,
    to_x: f32,
    to_z: f32,
    floor_level: u8,
    y: Option<f32>,
) -> impl Iterator<Item = (&'a super::RuntimePassability, BlockInfo<'a>)> {
    let min_x = from_x.min(to_x);
    let max_x = from_x.max(to_x);
    let min_z = from_z.min(to_z);
    let max_z = from_z.max(to_z);

    cache.iter().filter_map(move |(key, rp)| {
        if max_x < rp.min_x || min_x > rp.max_x || max_z < rp.min_z || min_z > rp.max_z {
            return None;
        }

        let stair_mask = stairwell_floor_mask(rp, min_x, max_x, min_z, max_z, floor_level);
        if stair_mask != 0 {
            let consulted = stairwell_consult(rp, stair_mask, |f| {
                move_blocked_on_floor(rp, f, from_x, from_z, to_x, to_z)
            })?;
            let info = BlockInfo {
                key,
                stairwell: true,
                consulted,
            };
            return Some((rp, info));
        }

        rp.floors
            .iter()
            .find(|floor| {
                floor.floor_level == floor_level
                    && obstacle_reaches_y(floor, y)
                    && move_blocked_on_floor(rp, floor, from_x, from_z, to_x, to_z)
            })
            .map(|_| {
                let info = BlockInfo {
                    key,
                    stairwell: false,
                    consulted: 1,
                };
                (rp, info)
            })
    })
}

/// Whether the cell holding `(x, z)` is walled in on all four sides, leaving a
/// mover inside it no legal step out.
///
/// Each direction is asked through the full [`is_movement_blocked`] path rather
/// than read off one grid's bits, because a seal is usually a mix of entries —
/// a bed's own cells on three sides and the room's wall on the fourth.
pub(crate) fn is_cell_sealed(
    cache: &PassabilityCache,
    x: f32,
    z: f32,
    floor_level: u8,
    y: Option<f32>,
) -> bool {
    let cx = x.floor() + 0.5;
    let cz = z.floor() + 0.5;
    super::DIRS.iter().all(|&(dx, dz)| {
        is_movement_blocked(
            cache,
            cx,
            cz,
            cx + dx as f32,
            cz + dz as f32,
            floor_level,
            y,
        )
    })
}

/// [`is_movement_blocked`], minus refusals that would trap a mover for good,
/// reporting what refused the move.
///
/// Furniture seals the cells it covers, and nothing stops a piece from being
/// placed over a standing player — a bed swallows its whole footprint this way.
/// When every side of the mover's cell is blocked, one step across an obstacle
/// flagged `yields_to_trapped_mover` is waived so they can walk out.
///
/// Anything that does not yield is settled first, so this cannot sink a mover
/// through a building's shell: park furniture on yourself against an outer wall
/// and the wall side stays solid while the other three let you out. Stopping at
/// the first hit would not be enough — a corner is refused by both, and whichever
/// entry the cache happened to yield first would decide.
///
/// The waiver stops at the neighbouring cell. Steps are fractions of a metre, so
/// bounding it there costs a trapped mover nothing while keeping a single long
/// sweep from riding it across anything further.
pub fn blocking_entry_for_mover<'a>(
    cache: &'a PassabilityCache,
    from_x: f32,
    from_z: f32,
    to_x: f32,
    to_z: f32,
    floor_level: u8,
    y: Option<f32>,
) -> Option<BlockInfo<'a>> {
    let mut yielding = None;
    for (rp, info) in blocking_entries(cache, from_x, from_z, to_x, to_z, floor_level, y) {
        if !rp.yields_to_trapped_mover {
            return Some(info);
        }
        yielding.get_or_insert(info);
    }

    let info = yielding?;
    let one_cell = (to_x.floor() - from_x.floor()).abs() <= 1.0
        && (to_z.floor() - from_z.floor()).abs() <= 1.0;
    if one_cell && is_cell_sealed(cache, from_x, from_z, floor_level, y) {
        return None;
    }
    Some(info)
}

/// Bool wrapper over [`blocking_entry_for_mover`] for continuous movement.
pub fn is_movement_blocked_for_mover(
    cache: &PassabilityCache,
    from_x: f32,
    from_z: f32,
    to_x: f32,
    to_z: f32,
    floor_level: u8,
    y: Option<f32>,
) -> bool {
    blocking_entry_for_mover(cache, from_x, from_z, to_x, to_z, floor_level, y).is_some()
}

#[inline]
fn floor_bit(floor_level: u8) -> u32 {
    1u32 << floor_level.min(31)
}

/// Whether an obstacle on `floor` is tall enough to reach a mover at `y`.
///
/// Floor level says *which storey*; this says whether the thing on it actually
/// reaches you. Walls span the storey, but furniture is only
/// `FURNITURE_BLOCK_HEIGHT` tall, and a staircase runs above the floor it
/// stands on — so someone mid-stairs must clear the tables below them.
///
/// Deliberately one-sided: an equivalent lower bound is what used to trap
/// players at the foot of a stairwell, because a blocked step never moves them
/// and so never corrects the Y that blocked it. `None` means "no Y known"
/// (path smoothing) and conservatively applies every obstacle on the floor.
#[inline]
fn obstacle_reaches_y(floor: &RuntimeFloorGrid, y: Option<f32>) -> bool {
    match y {
        Some(y) => y < floor.y_base + floor.wall_height,
        None => true,
    }
}

/// Run the stairwell two-floor consult: block only when every connected floor
/// refuses. Returns how many floors were consulted, or `None` if any allowed
/// the move. Both the edge check and the body-radius check route through here
/// so the rule cannot be fixed in one and left stale in the other.
///
/// Deliberately no `obstacle_reaches_y` filter: it is a *height* test, and
/// dropping either partner leaves the survivor — the grid sealing the end
/// underfoot — deciding alone, which traps the mover for good. Near the top of
/// a flight the lower floor's walls already fall below the mover, so the filter
/// would drop exactly the partner that grants passage. Furniture lives in its
/// own cache entry with no stairwells, so it still gets the height test on the
/// non-stairwell path.
fn stairwell_consult(
    rp: &super::RuntimePassability,
    stair_mask: u32,
    mut blocked_on: impl FnMut(&RuntimeFloorGrid) -> bool,
) -> Option<usize> {
    let mut consulted = 0usize;
    for f in rp
        .floors
        .iter()
        .filter(|f| stair_mask & floor_bit(f.floor_level) != 0)
    {
        consulted += 1;
        if !blocked_on(f) {
            return None;
        }
    }
    (consulted > 0).then_some(consulted)
}

/// What the direct-path shortcut is up against along a same-floor segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SegmentObstacles {
    /// No cache entry comes within `pad` of the segment's bounding box.
    None,
    /// Obstacles nearby; line-of-sight must decide.
    Walls,
    /// A stairwell connected to this floor touches the bounding box.
    Stairwell,
}

/// Classify a segment's padded bounding box against the cache. The stairwell
/// test mirrors A*'s positional stair fencing: stair interiors are excluded by
/// position, not by cell-edge masks, so LOS alone cannot see them and any
/// footprint contact must fall back to A*.
pub(super) fn segment_obstacles(
    cache: &PassabilityCache,
    min_x: f32,
    max_x: f32,
    min_z: f32,
    max_z: f32,
    floor_level: u8,
    pad: f32,
) -> SegmentObstacles {
    let mut near = false;
    for rp in cache.values() {
        if max_x + pad < rp.min_x
            || min_x - pad > rp.max_x
            || max_z + pad < rp.min_z
            || min_z - pad > rp.max_z
        {
            continue;
        }
        if stairwell_floor_mask(rp, min_x, max_x, min_z, max_z, floor_level) != 0 {
            return SegmentObstacles::Stairwell;
        }
        near = true;
    }
    if near {
        SegmentObstacles::Walls
    } else {
        SegmentObstacles::None
    }
}

/// Bitmask of floor levels connected by a stairwell this move touches that the
/// mover is actually keyed to, or 0 otherwise. Touching (rather than
/// containment) is what matters: stepping off a landing into the adjoining room
/// is exactly the move the other floor's grid seals.
fn stairwell_floor_mask(
    rp: &super::RuntimePassability,
    min_x: f32,
    max_x: f32,
    min_z: f32,
    max_z: f32,
    floor_level: u8,
) -> u32 {
    if rp.stairwells.is_empty() {
        return 0;
    }
    let mut mask = 0;
    for stair in &rp.stairwells {
        if stair.lower_floor != floor_level && stair.upper_floor != floor_level {
            continue;
        }
        let sx0 = rp.house_origin_x + stair.local_min_x as f32;
        let sx1 = rp.house_origin_x + stair.local_max_x as f32;
        let sz0 = rp.house_origin_z + stair.local_min_z as f32;
        let sz1 = rp.house_origin_z + stair.local_max_z as f32;
        if max_x >= sx0 && min_x <= sx1 && max_z >= sz0 && min_z <= sz1 {
            mask |= floor_bit(stair.lower_floor) | floor_bit(stair.upper_floor);
        }
    }
    mask
}

/// Whether from→to crosses a blocked cell edge on one specific floor grid.
fn move_blocked_on_floor(
    rp: &super::RuntimePassability,
    floor: &RuntimeFloorGrid,
    from_x: f32,
    from_z: f32,
    to_x: f32,
    to_z: f32,
) -> bool {
    let local_from_x = from_x - rp.house_origin_x - floor.origin_x as f32;
    let local_from_z = from_z - rp.house_origin_z - floor.origin_z as f32;
    let local_to_x = to_x - rp.house_origin_x - floor.origin_x as f32;
    let local_to_z = to_z - rp.house_origin_z - floor.origin_z as f32;

    edge_blocks_axis(
        local_from_x,
        local_to_x,
        local_from_z,
        local_to_z,
        floor,
        true,
    ) || edge_blocks_axis(
        local_from_z,
        local_to_z,
        local_from_x,
        local_to_x,
        floor,
        false,
    )
}

/// Check if a circle of radius `r` at `(x, z)` overlaps any blocking wall edge
/// on `floor_level`. Enforces player thickness so the character stops short of
/// walls instead of embedding into them, and lets path smoothing reject
/// diagonals whose interior would clip a corner the body radius can't clear.
pub fn is_circle_blocked_on_floor(
    cache: &PassabilityCache,
    x: f32,
    z: f32,
    r: f32,
    floor_level: u8,
    y: Option<f32>,
) -> bool {
    for rp in cache.values() {
        if x + r < rp.min_x || x - r > rp.max_x || z + r < rp.min_z || z - r > rp.max_z {
            continue;
        }

        // Same two-floor rule the edge check uses. Each floor's grid seals the
        // stairwell end it does not own, and the body radius sits right on that
        // seal whenever the mover reaches that end — so a landing would wall
        // itself off from the floor keyed to the far end.
        let stair_mask = stairwell_floor_mask(rp, x - r, x + r, z - r, z + r, floor_level);
        if stair_mask != 0 {
            if stairwell_consult(rp, stair_mask, |f| circle_blocked_on_grid(rp, f, x, z, r))
                .is_some()
            {
                return true;
            }
            continue;
        }

        for floor in &rp.floors {
            if floor.floor_level == floor_level
                && obstacle_reaches_y(floor, y)
                && circle_blocked_on_grid(rp, floor, x, z, r)
            {
                return true;
            }
        }
    }
    false
}

/// Whether a circle at `(x, z)` clips a blocking edge on one specific grid.
fn circle_blocked_on_grid(
    rp: &super::RuntimePassability,
    floor: &RuntimeFloorGrid,
    x: f32,
    z: f32,
    r: f32,
) -> bool {
    let r2 = r * r;
    let local_x = x - rp.house_origin_x - floor.origin_x as f32;
    let local_z = z - rp.house_origin_z - floor.origin_z as f32;
    let w = floor.width as i32;
    let d = floor.depth as i32;
    let min_cx = ((local_x - r).floor() as i32).max(0);
    let max_cx = ((local_x + r).floor() as i32).min(w - 1);
    let min_cz = ((local_z - r).floor() as i32).max(0);
    let max_cz = ((local_z + r).floor() as i32).min(d - 1);
    for cz in min_cz..=max_cz {
        for cx in min_cx..=max_cx {
            let cell = floor.cells[(cx + cz * w) as usize];
            if cell == 0 {
                continue;
            }
            let cx_f = cx as f32;
            let cz_f = cz as f32;
            if cell & EDGE_N != 0 && unit_segment_dist_sq(local_x, local_z, cx_f, cz_f, true) < r2 {
                return true;
            }
            if cell & EDGE_S != 0
                && unit_segment_dist_sq(local_x, local_z, cx_f, cz_f + 1.0, true) < r2
            {
                return true;
            }
            if cell & EDGE_W != 0 && unit_segment_dist_sq(local_x, local_z, cx_f, cz_f, false) < r2
            {
                return true;
            }
            if cell & EDGE_E != 0
                && unit_segment_dist_sq(local_x, local_z, cx_f + 1.0, cz_f, false) < r2
            {
                return true;
            }
        }
    }
    false
}

/// Squared distance from point `(px, pz)` to a unit-length axis-aligned
/// segment starting at `(sx, sz)`. `along_x` selects whether the segment
/// extends in +X (a north/south wall) or +Z (a west/east wall).
#[inline]
fn unit_segment_dist_sq(px: f32, pz: f32, sx: f32, sz: f32, along_x: bool) -> f32 {
    if along_x {
        let cx = px.clamp(sx, sx + 1.0);
        let dx = px - cx;
        let dz = pz - sz;
        dx * dx + dz * dz
    } else {
        let cz = pz.clamp(sz, sz + 1.0);
        let dx = px - sx;
        let dz = pz - cz;
        dx * dx + dz * dz
    }
}

/// Check if any cell boundary crossing along one axis is blocked.
fn edge_blocks_axis(
    from_a: f32,
    to_a: f32,
    from_b: f32,
    to_b: f32,
    floor: &RuntimeFloorGrid,
    x_axis: bool,
) -> bool {
    let from_cell = from_a.floor() as i32;
    let to_cell = to_a.floor() as i32;
    if from_cell == to_cell {
        return false;
    }

    let size_a = if x_axis { floor.width } else { floor.depth } as i32;
    let size_b = if x_axis { floor.depth } else { floor.width } as i32;
    let w = floor.width as i32;
    let idx = |a: i32, b: i32| -> usize {
        if x_axis {
            (a + b * w) as usize
        } else {
            (b + a * w) as usize
        }
    };

    let step: i32 = if to_cell > from_cell { 1 } else { -1 };
    let (leave_bit, enter_bit) = match (x_axis, step > 0) {
        (true, true) => (EDGE_E, EDGE_W),
        (true, false) => (EDGE_W, EDGE_E),
        (false, true) => (EDGE_S, EDGE_N),
        (false, false) => (EDGE_N, EDGE_S),
    };

    // Loop-invariant: skip the whole sweep if the parametric denominator is
    // numerically zero (would otherwise produce NaN `t` values inside).
    let denom = to_a - from_a;
    if denom.abs() <= f32::EPSILON {
        return false;
    }
    let mut cell = from_cell;
    while cell != to_cell {
        let edge_coord = if step > 0 { cell + 1 } else { cell };
        let next_cell = cell + step;
        let t = (edge_coord as f32 - from_a) / denom;
        let cell_b = (from_b + t * (to_b - from_b)).floor() as i32;
        if cell_b >= 0 && cell_b < size_b {
            if cell >= 0 && cell < size_a && floor.cells[idx(cell, cell_b)] & leave_bit != 0 {
                return true;
            }
            if next_cell >= 0
                && next_cell < size_a
                && floor.cells[idx(next_cell, cell_b)] & enter_bit != 0
            {
                return true;
            }
        }
        cell += step;
    }
    false
}

/// Get the floor level at a world position based on Y height.
/// Returns 0 if outside any house.
/// Picks the floor whose y_base is closest to y among all floors whose
/// grid contains the cell — handles mid-stairwell clicks and overlapping
/// floor ranges at stairwell landings.
pub fn get_floor_at_position(cache: &PassabilityCache, x: f32, z: f32, y: f32) -> u8 {
    let cx = x.floor() as i32;
    let cz = z.floor() as i32;
    let mut best_floor: u8 = 0;
    let mut best_dist = f32::INFINITY;
    let mut found = false;

    for rp in cache.values() {
        if x < rp.min_x || x > rp.max_x || z < rp.min_z || z > rp.max_z {
            continue;
        }
        let house_ox = rp.house_origin_x.floor() as i32;
        let house_oz = rp.house_origin_z.floor() as i32;
        for floor in &rp.floors {
            let gx = cx - house_ox - floor.origin_x;
            let gz = cz - house_oz - floor.origin_z;
            if gx < 0 || gx >= floor.width as i32 || gz < 0 || gz >= floor.depth as i32 {
                continue;
            }
            // Cell is inside this floor's grid — pick the closest y_base
            let dist = (y - floor.y_base).abs();
            if dist < best_dist {
                best_dist = dist;
                best_floor = floor.floor_level;
                found = true;
            }
        }
    }

    if found {
        best_floor
    } else {
        0
    }
}

/// Get the yBase for a given floor level at a world position.
pub fn get_floor_y_base(cache: &PassabilityCache, x: f32, z: f32, floor_level: u8) -> Option<f32> {
    let cx = x.floor() as i32;
    let cz = z.floor() as i32;
    for rp in cache.values() {
        if x < rp.min_x || x > rp.max_x || z < rp.min_z || z > rp.max_z {
            continue;
        }
        let house_ox = rp.house_origin_x.floor() as i32;
        let house_oz = rp.house_origin_z.floor() as i32;
        for floor in &rp.floors {
            if floor.floor_level != floor_level {
                continue;
            }
            let gx = cx - house_ox - floor.origin_x;
            let gz = cz - house_oz - floor.origin_z;
            if gx >= 0 && gx < floor.width as i32 && gz >= 0 && gz < floor.depth as i32 {
                return Some(floor.y_base);
            }
        }
    }
    None
}

fn floor_y_base_of(rp: &RuntimePassability, floor_level: u8) -> Option<f32> {
    rp.floors
        .iter()
        .find(|f| f.floor_level == floor_level)
        .map(|f| f.y_base)
}

/// Passability floor an A* endpoint at (x, z, y) must be keyed to.
///
/// [`get_floor_at_position`] answers "which floor is this standing on", which
/// is right everywhere except *inside* a stairwell. A* keys a stairwell's
/// intermediate steps off its lower floor (`stair::build_stair_cells`) and only
/// seeds those keys for a search on that floor, so an endpoint mid-flight keyed
/// to the upper floor can reach the stairs only through the far landing — the
/// mover walks the whole flight the wrong way first. Both landings are genuine
/// floor cells and fall through to the Y lookup.
///
/// Y still decides between stairwells stacked on one footprint: the candidate
/// whose interpolated step height sits nearest `y` wins.
pub fn start_floor_at(cache: &PassabilityCache, x: f32, z: f32, y: f32) -> u8 {
    let cx = x.floor() as i32;
    let cz = z.floor() as i32;
    let mut best: Option<(f32, u8)> = None;

    for rp in cache.values() {
        if x < rp.min_x || x > rp.max_x || z < rp.min_z || z > rp.max_z {
            continue;
        }
        let ox = rp.house_origin_x.floor() as i32;
        let oz = rp.house_origin_z.floor() as i32;
        for stair in &rp.stairwells {
            let Some((step, n)) = stair_step(stair, cx - ox, cz - oz) else {
                continue;
            };
            if step == 0 || step == n - 1 {
                continue;
            }
            let (Some(lower_y), Some(upper_y)) = (
                floor_y_base_of(rp, stair.lower_floor),
                floor_y_base_of(rp, stair.upper_floor),
            ) else {
                continue;
            };
            let f = step as f32 / (n - 1) as f32;
            let dist = (y - (lower_y + (upper_y - lower_y) * f)).abs();
            if best.is_none_or(|(best_dist, _)| dist < best_dist) {
                best = Some((dist, stair.lower_floor));
            }
        }
    }

    match best {
        Some((_, floor)) => floor,
        None => get_floor_at_position(cache, x, z, y),
    }
}
