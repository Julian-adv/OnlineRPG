//! Stair-shaft Y model: how high the ground is anywhere on a shaft's run.
//!
//! Every client that moves underground needs this, and they must all agree:
//! the server accepts a claimed dungeon floor only when the reported Y is
//! within [`FLOOR_Y_TOLERANCE`](super::FLOOR_Y_TOLERANCE) of that floor's
//! world Y, so a second copy of the ramp profile that drifts snaps the mover
//! back mid-staircase rather than failing a test.

use super::{dungeon_origin, floor_world_y, FloorLayout, StairShaft, SHAFT_LEN, SHAFT_W};
use crate::world::Position;

/// Flat landing length (in cells) at each end of a stair ramp.
pub const LANDING_CELLS: f32 = 1.0;

/// Run position along `shaft` in `[0, SHAFT_LEN)` measured from the entry
/// (shallow) end, or `None` off the shaft footprint.
pub fn shaft_run_pos(entrance: &Position, shaft: &StairShaft, x: f32, z: f32) -> Option<f32> {
    let (ox, oz) = dungeon_origin(entrance.x, entrance.z);
    let lx = x - ox - shaft.x as f32;
    let lz = z - oz - shaft.z as f32;
    let (lateral, run) = if shaft.along_z { (lx, lz) } else { (lz, lx) };
    if lateral < 0.0 || lateral >= SHAFT_W as f32 || run < 0.0 || run >= SHAFT_LEN as f32 {
        return None;
    }
    Some(if shaft.reversed {
        SHAFT_LEN as f32 - run
    } else {
        run
    })
}

/// Linear stair ramp with flat landings at both ends.
fn ramp_y(high_y: f32, low_y: f32, t: f32) -> f32 {
    let len = SHAFT_LEN as f32;
    if t <= LANDING_CELLS {
        return high_y;
    }
    if t >= len - LANDING_CELLS {
        return low_y;
    }
    let f = (t - LANDING_CELLS) / (len - LANDING_CELLS * 2.0);
    high_y + (low_y - high_y) * f
}

/// Y at the top of the shaft arriving at `depth` (the surface for depth 1).
fn shaft_high_y(entrance_y: f32, depth: u8) -> f32 {
    if depth <= 1 {
        entrance_y
    } else {
        floor_world_y(entrance_y, depth - 1)
    }
}

/// Ground height on `depth`, stair ramps included. `None` when the dungeon
/// has no such floor.
pub fn floor_height_at(
    entrance: &Position,
    layouts: &[FloorLayout],
    depth: u8,
    x: f32,
    z: f32,
) -> Option<f32> {
    let layout = layouts.get(depth.checked_sub(1)? as usize)?;
    let floor_y = floor_world_y(entrance.y, depth);
    if let Some(t) = shaft_run_pos(entrance, &layout.up_shaft, x, z) {
        return Some(ramp_y(shaft_high_y(entrance.y, depth), floor_y, t));
    }
    if let Some(down) = layout.down_shaft.as_ref() {
        if let Some(t) = shaft_run_pos(entrance, down, x, z) {
            return Some(ramp_y(floor_y, floor_world_y(entrance.y, depth + 1), t));
        }
    }
    Some(floor_y)
}

/// Cells of slack around a shaft within which a floor change is accepted:
/// the server sim trails the client that flipped its floor.
pub const SHAFT_CHANGE_MARGIN: i32 = 1;
/// Longest leg that may carry a floor change. Y is interpolated along the
/// whole leg, so a long one clipping the shaft would reach the other floor's
/// grid far from it.
pub const FLOOR_CHANGE_LEG_MAX: f32 = 2.0 * SHAFT_LEN as f32;

/// Whether the leg `from`→`to` (world XZ) touches `shaft`'s footprint grown
/// by `margin` cells — the only walk that may change floors.
pub fn leg_touches_shaft(
    entrance: &Position,
    shaft: &StairShaft,
    margin: i32,
    from: (f32, f32),
    to: (f32, f32),
) -> bool {
    let (ox, oz) = dungeon_origin(entrance.x, entrance.z);
    let r = shaft.rect().expanded(margin);
    let (min_x, min_z) = (ox + r.x as f32, oz + r.z as f32);
    segment_touches_box(
        (min_x, min_x + r.w as f32),
        (min_z, min_z + r.d as f32),
        from,
        to,
    )
}

/// Slab clip of a 2D segment against an axis-aligned box.
fn segment_touches_box(
    (min_x, max_x): (f32, f32),
    (min_z, max_z): (f32, f32),
    (x0, z0): (f32, f32),
    (x1, z1): (f32, f32),
) -> bool {
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p0, p1, lo, hi) in [(x0, x1, min_x, max_x), (z0, z1, min_z, max_z)] {
        let d = p1 - p0;
        if d.abs() <= f32::EPSILON {
            if p0 < lo || p0 > hi {
                return false;
            }
            continue;
        }
        let (ta, tb) = ((lo - p0) / d, (hi - p0) / d);
        t0 = t0.max(ta.min(tb));
        t1 = t1.min(ta.max(tb));
        if t0 > t1 {
            return false;
        }
    }
    true
}

/// Surface entrance ramp height, or `None` when (x, z) is off the shaft —
/// out there the terrain sampler owns Y.
pub fn entrance_ramp_height_at(
    entrance: &Position,
    layouts: &[FloorLayout],
    x: f32,
    z: f32,
) -> Option<f32> {
    let first = layouts.first()?;
    let t = shaft_run_pos(entrance, &first.up_shaft, x, z)?;
    Some(ramp_y(entrance.y, floor_world_y(entrance.y, 1), t))
}

/// Ground height for something standing on the passability floor `floor`: a
/// dungeon floor, or the surface entrance ramp when `floor` is the surface.
/// `None` means the dungeon has no say and terrain height wins.
pub fn ground_y_for_floor(
    entrance: &Position,
    layouts: &[FloorLayout],
    floor: u8,
    x: f32,
    z: f32,
) -> Option<f32> {
    match super::floor_level_for_passability(floor) {
        depth if depth < 0 => floor_height_at(entrance, layouts, depth.unsigned_abs(), x, z),
        0 => entrance_ramp_height_at(entrance, layouts, x, z),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dungeon::{generate_dungeon_for, DUNGEON_FLOOR_HEIGHT};

    fn crypt() -> (Position, Vec<FloorLayout>) {
        let def = crate::dungeon::entrance("old_crypt").expect("old_crypt is in dungeons.csv");
        (def.position(), generate_dungeon_for(&def.id))
    }

    #[test]
    fn ramp_descends_one_storey_between_landings() {
        let (e, _) = crypt();
        let top = e.y;
        assert_eq!(ramp_y(top, top - 4.0, 0.0), top);
        assert_eq!(ramp_y(top, top - 4.0, 1.0), top);
        assert_eq!(ramp_y(top, top - 4.0, SHAFT_LEN as f32 - 1.0), top - 4.0);
        assert_eq!(ramp_y(top, top - 4.0, 4.0), top - 2.0);
    }

    #[test]
    fn leg_touches_shaft_within_one_cell_or_across_it() {
        let (e, layouts) = crypt();
        let shaft = layouts[0].up_shaft;
        let r = shaft.rect();
        let (ox, oz) = dungeon_origin(e.x, e.z);
        let at = |cx: i32, cz: i32| (ox + cx as f32 + 0.5, oz + cz as f32 + 0.5);
        let inside = at(r.x, r.z);
        let beside = at(r.x - 1, r.z);
        let far = at(r.x - 3, r.z);
        let across = at(r.x + r.w + 2, r.z);
        assert!(leg_touches_shaft(&e, &shaft, 1, inside, inside));
        assert!(leg_touches_shaft(&e, &shaft, 1, beside, beside));
        assert!(!leg_touches_shaft(&e, &shaft, 1, far, far));
        assert!(leg_touches_shaft(&e, &shaft, 1, far, across));
        assert!(!leg_touches_shaft(&e, &shaft, 0, beside, beside));
    }

    /// Standing anywhere on depth 1 that isn't a shaft must report that
    /// floor's world Y, or the server would collide us against the surface.
    #[test]
    fn floor_height_off_the_shaft_is_the_floor_y() {
        let (e, layouts) = crypt();
        let landing = crate::dungeon::cell_center(&e, 1, layouts[0].up_shaft.exit_cell());
        assert_eq!(landing.y, e.y - DUNGEON_FLOOR_HEIGHT);
        let y = floor_height_at(&e, &layouts, 1, landing.x, landing.z).unwrap();
        assert!((y - landing.y).abs() < 0.01, "{y} vs {}", landing.y);
    }
}
