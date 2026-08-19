//! Stairwell traversal model for A*.
//!
//! Stairwell cells are walked along a virtual axis (X or Z) with one
//! intermediate "step" per grid cell. To let A* search through them with
//! the same `(x, z, floor_key)` machinery as regular floors, we encode
//! intermediate steps as floor-key values *between* two regular floors.
//!
//! `FLOOR_SCALE = 16` reserves 16 key slots per floor: real floor N uses
//! key `N * 16`, and intermediate stair steps between floor N and N+1 use
//! `N*16 + 1 ..= N*16 + n-1`. The set is built once per A* call as a
//! `HashMap<AStarKey, StairCellExpansion>` so the search loop's stair
//! expansion is an O(1) lookup.

use std::collections::HashMap;

use super::{PassabilityCache, StairwellInfo};

/// `(world_cell_x, world_cell_z, floor_key)` — node identity in A*.
pub(super) type AStarKey = (i32, i32, u16);

/// Slots per floor in the floor-key encoding. Regular floor N is key
/// `N * FLOOR_SCALE`; intermediate stair-step cells slot into the gap
/// between adjacent regular floors.
pub(super) const FLOOR_SCALE: u16 = 16;

#[inline]
pub(super) fn floor_to_key(f: u8) -> u16 {
    f as u16 * FLOOR_SCALE
}

#[inline]
pub(super) fn key_to_floor(k: u16) -> u8 {
    (k / FLOOR_SCALE) as u8
}

#[inline]
pub(super) fn is_regular_key(k: u16) -> bool {
    k.is_multiple_of(FLOOR_SCALE)
}

/// A stairwell's run length (step count) and lateral width.
#[inline]
pub(super) fn stair_dims(stair: &StairwellInfo) -> (i32, i32) {
    let (dx, dz) = (
        stair.local_max_x - stair.local_min_x,
        stair.local_max_z - stair.local_min_z,
    );
    if stair.along_z {
        (dz, dx)
    } else {
        (dx, dz)
    }
}

/// Step index of a stairwell-local cell measured from the entry (lower-floor)
/// end, plus the step count. `None` off the footprint. Inverse of the
/// `step_pos` closure below, and shares its convention: `reversed` flips the
/// physical position so index 0 is always the entry end. Kept here so the
/// endpoint lookup in `query::start_floor_at` and the A* expansion cannot
/// disagree about which end of a shaft is which.
pub(super) fn stair_step(stair: &StairwellInfo, local_x: i32, local_z: i32) -> Option<(i32, i32)> {
    let (n, width) = stair_dims(stair);
    let (run, lateral) = if stair.along_z {
        (local_z - stair.local_min_z, local_x - stair.local_min_x)
    } else {
        (local_x - stair.local_min_x, local_z - stair.local_min_z)
    };
    if run < 0 || run >= n || lateral < 0 || lateral >= width {
        return None;
    }
    Some((if stair.reversed { n - 1 - run } else { run }, n))
}

/// Precomputed stairwell cell neighbor info for the A* expansion.
pub(super) struct StairNeighbor {
    pub(super) x: i32,
    pub(super) z: i32,
    pub(super) fk: u16,
}

pub(super) struct StairCellExpansion {
    pub(super) prev: Option<StairNeighbor>,
    pub(super) next: Option<StairNeighbor>,
}

/// Build the stairwell cell map for A* pathfinding.
/// Maps (x, z, floor_key) → expansion neighbors along the stair axis.
pub(super) fn build_stair_cells(cache: &PassabilityCache) -> HashMap<AStarKey, StairCellExpansion> {
    let mut map = HashMap::new();

    for rp in cache.values() {
        let ox = rp.house_origin_x.floor() as i32;
        let oz = rp.house_origin_z.floor() as i32;

        for stair in &rp.stairwells {
            let lower_key = floor_to_key(stair.lower_floor);
            let upper_key = floor_to_key(stair.upper_floor);
            let (n, width) = stair_dims(stair);

            // Compute the floor key for step i.
            // step_pos already flips physical positions for reversed stairs,
            // so i=0 is always the entry (lower floor) end and i=n-1 is the
            // exit (upper floor) end regardless of reversed.
            let step_key = |i: i32| -> u16 {
                if i == 0 {
                    lower_key
                } else if i == n - 1 {
                    upper_key
                } else {
                    lower_key + i as u16
                }
            };

            // Compute world (x, z) for step i and lateral offset w
            let step_pos = |i: i32, w: i32| -> (i32, i32) {
                if stair.along_z {
                    let z = if stair.reversed {
                        stair.local_max_z - 1 - i
                    } else {
                        stair.local_min_z + i
                    };
                    (ox + stair.local_min_x + w, oz + z)
                } else {
                    let x = if stair.reversed {
                        stair.local_max_x - 1 - i
                    } else {
                        stair.local_min_x + i
                    };
                    (ox + x, oz + stair.local_min_z + w)
                }
            };

            for i in 0..n {
                let fk = step_key(i);
                let prev_fk = if i > 0 { Some(step_key(i - 1)) } else { None };
                let next_fk = if i < n - 1 {
                    Some(step_key(i + 1))
                } else {
                    None
                };

                for w in 0..width {
                    let (cx, cz) = step_pos(i, w);
                    let prev = prev_fk.map(|pk| {
                        let (px, pz) = step_pos(i - 1, w);
                        StairNeighbor {
                            x: px,
                            z: pz,
                            fk: pk,
                        }
                    });
                    let next = next_fk.map(|nk| {
                        let (nx, nz) = step_pos(i + 1, w);
                        StairNeighbor {
                            x: nx,
                            z: nz,
                            fk: nk,
                        }
                    });
                    map.insert((cx, cz, fk), StairCellExpansion { prev, next });
                }
            }
        }
    }

    map
}
