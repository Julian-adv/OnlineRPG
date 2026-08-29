//! Interior room doors: deterministic placement of double doors at corridor
//! mouths in room walls. Single source of truth shared by the client renderer
//! (via wasm), client collision, and the server's passability cache — all
//! three derive the same door list from the layout, so a rendered door is
//! always the door that blocks, and the opaque `door_id` in toggle packets
//! resolves to the same opening everywhere.

use std::collections::HashSet;

use serde::Serialize;

use super::{gen, is_locked_depth, FloorLayout};

/// Percent chance a qualifying corridor mouth gets a door.
const INTERIOR_DOOR_PCT: u32 = 30;

/// The surface entrance door, the only door id depth 0 has. Disjoint from the
/// interior ids below (`entrance_door_id_is_not_an_interior_door` holds it
/// over the generated floors), so one id space covers both. Frozen like
/// `door_hash`: it is on the wire, and the client mirrors it in
/// `dungeonManager.ts`.
pub const ENTRANCE_DOOR_ID: u32 = 0;

/// Wall side indices, matching the client's `WALL_N/E/S/W`.
const WALL_N: u8 = 0;
const WALL_E: u8 = 1;
const WALL_S: u8 = 2;
const WALL_W: u8 = 3;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InteriorDoorSpec {
    /// Which of the room's walls holds the corridor mouth (0/1/2/3 = N/E/S/W).
    pub wall: u8,
    /// Opening start cell along the wall.
    pub lat0: i32,
    /// Opening width in cells.
    pub len: i32,
    /// Room↔corridor grid line.
    pub wall_line: i32,
    /// Stable id used by toggle packets and the open-door state maps:
    /// `wall * 0x10000 + lat0 * 0x100 + wall_line` (grid < 256, no overlap).
    pub door_id: u32,
    /// Index into `layout.rooms` of the room whose wall this is.
    #[serde(skip)]
    pub room: usize,
    /// Only the floor's key works it, and it shuts itself again.
    pub locked: bool,
}

impl InteriorDoorSpec {
    /// North/south doors span X; east/west doors span Z.
    pub fn spans_x(&self) -> bool {
        self.wall == WALL_N || self.wall == WALL_S
    }

    /// Blocking segment as the floor-local `(ax, az, bx, bz)` quad consumed
    /// by `floor_passability_cells_full`'s `closed_door_segs`.
    pub fn seg(&self) -> [i32; 4] {
        if self.spans_x() {
            [
                self.lat0,
                self.wall_line,
                self.lat0 + self.len,
                self.wall_line,
            ]
        } else {
            [
                self.wall_line,
                self.lat0,
                self.wall_line,
                self.lat0 + self.len,
            ]
        }
    }
}

/// FNV-1a-style [0, 1000) hash of four small ints, bit-identical to the
/// client's original `doorHash` (u32 xor + wrapping mul ≙ JS `Math.imul`).
/// Frozen: changing it moves every existing door and invalidates door ids.
fn door_hash(a: i32, b: i32, c: i32, d: i32) -> u32 {
    let mut h: u32 = 2166136261;
    for v in [a, b, c, d] {
        h = (h ^ (v as u32)).wrapping_mul(16777619);
    }
    h % 1000
}

/// Every corridor mouth on the floor: maximal runs of a room wall whose
/// outward neighbour is corridor, as door candidates.
pub(super) fn wall_openings(layout: &FloorLayout) -> Vec<InteriorDoorSpec> {
    let mut openings = Vec::new();
    let locked_floor = is_locked_depth(layout.depth);
    for (room_idx, room) in layout.rooms.iter().enumerate() {
        for wall in [WALL_N, WALL_E, WALL_S, WALL_W] {
            let spans_x = wall == WALL_N || wall == WALL_S;
            let outer_low = wall == WALL_N || wall == WALL_W;
            let lat_lo = if spans_x { room.x } else { room.z };
            let lat_hi = lat_lo + if spans_x { room.w } else { room.d };
            let wall_line = (if spans_x { room.z } else { room.x })
                + if outer_low {
                    0
                } else if spans_x {
                    room.d
                } else {
                    room.w
                };
            // Interior cell hugging the wall, and the step toward the
            // corridor neighbour just outside it.
            let fixed = if outer_low { wall_line } else { wall_line - 1 };
            let step: i32 = if outer_low { -1 } else { 1 };
            let mut start = -1;
            for lat in lat_lo..=lat_hi {
                let (cx, cz) = if spans_x { (lat, fixed) } else { (fixed, lat) };
                let (nx, nz) = if spans_x {
                    (cx, cz + step)
                } else {
                    (cx + step, cz)
                };
                let open = lat < lat_hi
                    && layout.is_carved(cx, cz)
                    && gen::cell_is_corridor(layout, nx, nz);
                if open && start < 0 {
                    start = lat;
                }
                if !open && start >= 0 {
                    openings.push(InteriorDoorSpec {
                        wall,
                        lat0: start,
                        len: lat - start,
                        wall_line,
                        door_id: (wall as u32) * 0x10000
                            + (start as u32) * 0x100
                            + wall_line as u32,
                        room: room_idx,
                        locked: locked_floor && room_idx == 0,
                    });
                    start = -1;
                }
            }
        }
    }
    openings
}

/// Give each corridor mouth a door with `INTERIOR_DOOR_PCT`% chance, hashed
/// from the opening's coordinates so the list is stable per layout. A locked
/// floor's stair-room exit always gets one.
pub fn interior_doors(layout: &FloorLayout) -> Vec<InteriorDoorSpec> {
    wall_openings(layout)
        .into_iter()
        .filter(|d| {
            d.locked
                || door_hash(layout.depth as i32, d.wall as i32, d.lat0, d.wall_line)
                    < INTERIOR_DOOR_PCT * 10
        })
        .collect()
}

/// Ids of the doors on `layout` that need the floor's key.
pub fn locked_door_ids(layout: &FloorLayout) -> Vec<u32> {
    wall_openings(layout)
        .into_iter()
        .filter(|d| d.locked)
        .map(|d| d.door_id)
        .collect()
}

/// Flat `(ax, az, bx, bz)` quads of every interior door on the floor NOT in
/// `open` — the `closed_door_segs` input to `floor_passability_cells_full`.
/// Doors default shut, so `None` (no state yet) seals them all.
pub fn closed_door_segs(layout: &FloorLayout, open: Option<&HashSet<u32>>) -> Vec<i32> {
    interior_doors(layout)
        .iter()
        .filter(|d| !open.is_some_and(|s| s.contains(&d.door_id)))
        .flat_map(|d| d.seg())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::door_hash;

    /// Golden values computed with the original client JS implementation
    /// (`Math.imul(h ^ (v >>> 0), 16777619)`, then `(h >>> 0) % 1000`). A
    /// mismatch means deployed dungeons' doors would silently move.
    #[test]
    fn door_hash_matches_client_js_golden_values() {
        for (a, b, c, d, expected) in [
            (1, 0, 5, 10, 955),
            (1, 1, 12, 30, 405),
            (3, 2, 40, 7, 909),
            (20, 3, 79, 79, 374),
            (5, 0, 0, 0, 352),
            (7, 2, 33, 64, 901),
            (2, 1, 17, 3, 996),
            (19, 3, 60, 21, 472),
        ] {
            assert_eq!(door_hash(a, b, c, d), expected, "({a}, {b}, {c}, {d})");
        }
    }
}
