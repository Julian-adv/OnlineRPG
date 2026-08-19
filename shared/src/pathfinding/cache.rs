//! Build and mutate the runtime passability cache. The cache is keyed by
//! house id and stores per-floor cell grids (with edge-bitmask occupancy)
//! plus stairwell metadata. Built once per house from `HouseData`, then
//! mutated via `update_door_edge` whenever a door opens or closes.

use crate::housing::{HouseData, RoomData, RoomType, WallDirection, WallVariant};

use super::{
    PassabilityCache, RuntimeFloorGrid, RuntimePassability, StairwellInfo, EDGE_E, EDGE_N, EDGE_S,
    EDGE_W,
};

const FLOOR_THICKNESS: f32 = 0.1;
const DEFAULT_WALL_HEIGHT: f32 = 3.0;

#[inline]
fn floor_y_base(floor_level: u8, wall_height: f32) -> f32 {
    floor_level as f32 * (wall_height + FLOOR_THICKNESS)
}

/// Build runtime passability data from a HouseData.
/// Expects pre-computed PassabilityGrid in house.passability.
/// The caller must ensure passability is computed before calling this.
pub fn build_runtime_passability(house: &HouseData) -> RuntimePassability {
    let grids = &house.passability;

    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_z = f32::NEG_INFINITY;

    let floors: Vec<RuntimeFloorGrid> = grids
        .iter()
        .map(|g| {
            let world_min_x = house.origin.x + g.origin_x as f32;
            let world_min_z = house.origin.z + g.origin_z as f32;
            let world_max_x = world_min_x + g.width as f32;
            let world_max_z = world_min_z + g.depth as f32;
            min_x = min_x.min(world_min_x);
            max_x = max_x.max(world_max_x);
            min_z = min_z.min(world_min_z);
            max_z = max_z.max(world_max_z);

            let mut wall_height = DEFAULT_WALL_HEIGHT;
            let mut y_base = house.origin.y;
            for room in &house.rooms {
                if room.floor_level == g.floor_level {
                    wall_height = room.wall_height;
                    y_base = house.origin.y + floor_y_base(room.floor_level, room.wall_height);
                    break;
                }
                if room.room_type == RoomType::Stairwell && g.floor_level == room.floor_level + 1 {
                    wall_height = room.wall_height;
                    y_base = house.origin.y + floor_y_base(g.floor_level, room.wall_height);
                    break;
                }
            }

            RuntimeFloorGrid {
                floor_level: g.floor_level,
                origin_x: g.origin_x,
                origin_z: g.origin_z,
                width: g.width,
                depth: g.depth,
                y_base,
                wall_height,
                cells: g.cells.clone(),
            }
        })
        .collect();

    let mut stairwells = Vec::new();
    for room in &house.rooms {
        if room.room_type == RoomType::Stairwell {
            stairwells.push(StairwellInfo {
                local_min_x: room.local_x,
                local_min_z: room.local_z,
                local_max_x: room.local_x + room.size_x as i32,
                local_max_z: room.local_z + room.size_z as i32,
                lower_floor: room.floor_level,
                upper_floor: room.floor_level + 1,
                along_z: room.size_z as i32 >= room.size_x as i32,
                reversed: room.stair_reversed,
            });
        }
    }

    RuntimePassability {
        house_origin_x: house.origin.x,
        house_origin_z: house.origin.z,
        min_x,
        max_x,
        min_z,
        max_z,
        floors,
        stairwells,
        yields_to_trapped_mover: false,
        is_ground: true,
    }
}

/// The two cells a wall segment separates, and the edge bit each carries
/// toward the other, in coordinates relative to the room's own origin. Add the
/// room origin for house-local cells, or the house origin for world cells (the
/// floor grid's origin cancels either way).
///
/// Split out because callers that only need to know *where* a door is — walking
/// up to one to open it — must derive the same cells this applies bits to.
pub fn door_cells(
    room: &RoomData,
    wall_dir: WallDirection,
    segment_index: usize,
) -> ((i32, i32, u8), (i32, i32, u8)) {
    let seg = segment_index as i32;
    match wall_dir {
        WallDirection::North => ((seg, 0, EDGE_N), (seg, -1, EDGE_S)),
        WallDirection::South => {
            let cz = room.size_z as i32 - 1;
            ((seg, cz, EDGE_S), (seg, cz + 1, EDGE_N))
        }
        WallDirection::West => ((0, seg, EDGE_W), (-1, seg, EDGE_E)),
        WallDirection::East => {
            let cx = room.size_x as i32 - 1;
            ((cx, seg, EDGE_E), (cx + 1, seg, EDGE_W))
        }
    }
}

/// Update passability edge bits when a door is opened or closed.
pub fn update_door_edge(
    cache: &mut PassabilityCache,
    house_id: &str,
    room: &RoomData,
    wall_dir: WallDirection,
    segment_index: usize,
    is_open: bool,
) {
    let rp = match cache.get_mut(house_id) {
        Some(rp) => rp,
        None => return,
    };

    let floor = match rp
        .floors
        .iter_mut()
        .find(|f| f.floor_level == room.floor_level)
    {
        Some(f) => f,
        None => return,
    };

    let rx = room.local_x - floor.origin_x;
    let rz = room.local_z - floor.origin_z;

    let ((dx, dz, edge), (adj_dx, adj_dz, adj_edge)) = door_cells(room, wall_dir, segment_index);
    let (cx, cz) = (rx + dx, rz + dz);
    let (adj_cx, adj_cz) = (rx + adj_dx, rz + adj_dz);

    let w = floor.width as i32;
    let d = floor.depth as i32;

    let set_or_clear = |cells: &mut Vec<u8>, gx: i32, gz: i32, bit: u8| {
        if gx < 0 || gx >= w || gz < 0 || gz >= d {
            return;
        }
        let idx = (gx + gz * w) as usize;
        if is_open {
            cells[idx] &= !bit;
        } else {
            cells[idx] |= bit;
        }
    };

    set_or_clear(&mut floor.cells, cx, cz, edge);
    set_or_clear(&mut floor.cells, adj_cx, adj_cz, adj_edge);
}

/// One solid furniture piece: the world grid cells it occupies plus the floor
/// it sits on. `cells` are integer world cell coordinates (floor of the world
/// XZ position).
pub struct FurniturePiece {
    pub cells: Vec<(i32, i32)>,
    pub floor_level: u8,
    /// World Y of the floor the piece stands on. Used by the Y-gated movement /
    /// circle queries to confine blocking to the piece's own floor.
    pub y_base: f32,
    /// Vertical extent above `y_base` within which the piece blocks — roughly
    /// one storey so furniture on a lower floor never blocks the floor above.
    pub wall_height: f32,
}

/// Build a standalone passability entry that seals each furniture cell on all
/// four edges (`EDGE_ALL`), so both continuous movement collision and A*
/// pathing treat the cell as solid. Each piece becomes its own small
/// `RuntimeFloorGrid` (sized to that piece's footprint), so multi-region sets
/// never overflow the `u8` grid dimensions. World cell coords map through
/// `house_origin = 0` + `floor.origin = grid min cell`. Returns `None` when no
/// piece contributes any cell (caller should drop/skip the cache entry).
pub fn build_furniture_passability(pieces: &[FurniturePiece]) -> Option<RuntimePassability> {
    let edge_all = EDGE_N | EDGE_E | EDGE_S | EDGE_W;
    let mut floors = Vec::new();
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_z = f32::NEG_INFINITY;

    for piece in pieces {
        if piece.cells.is_empty() {
            continue;
        }
        let mut cmin_x = i32::MAX;
        let mut cmax_x = i32::MIN;
        let mut cmin_z = i32::MAX;
        let mut cmax_z = i32::MIN;
        for &(cx, cz) in &piece.cells {
            cmin_x = cmin_x.min(cx);
            cmax_x = cmax_x.max(cx);
            cmin_z = cmin_z.min(cz);
            cmax_z = cmax_z.max(cz);
        }
        let width = (cmax_x - cmin_x + 1) as usize;
        let depth = (cmax_z - cmin_z + 1) as usize;
        // Guard against a pathological footprint overflowing the u8 grid dims.
        if width > u8::MAX as usize || depth > u8::MAX as usize {
            continue;
        }

        let mut cells = vec![0u8; width * depth];
        for &(cx, cz) in &piece.cells {
            let gx = (cx - cmin_x) as usize;
            let gz = (cz - cmin_z) as usize;
            cells[gx + gz * width] = edge_all;
        }

        min_x = min_x.min(cmin_x as f32);
        max_x = max_x.max((cmax_x + 1) as f32);
        min_z = min_z.min(cmin_z as f32);
        max_z = max_z.max((cmax_z + 1) as f32);

        floors.push(RuntimeFloorGrid {
            floor_level: piece.floor_level,
            origin_x: cmin_x,
            origin_z: cmin_z,
            width: width as u8,
            depth: depth as u8,
            y_base: piece.y_base,
            wall_height: piece.wall_height,
            cells,
        });
    }

    if floors.is_empty() {
        return None;
    }

    Some(RuntimePassability {
        house_origin_x: 0.0,
        house_origin_z: 0.0,
        min_x,
        max_x,
        min_z,
        max_z,
        floors,
        stairwells: Vec::new(),
        // The only builder that seals every side of a cell, and the only
        // obstacle that can land on top of a standing player.
        yields_to_trapped_mover: true,
        is_ground: false,
    })
}

/// Apply open-door overlays from a HouseData to its runtime passability cache entry.
/// Should be called after build_runtime_passability to reflect doors that are already open.
pub fn apply_door_overlays(cache: &mut PassabilityCache, house: &HouseData) {
    for room in &house.rooms {
        for (dir, segs) in [
            (WallDirection::North, &room.wall_north),
            (WallDirection::South, &room.wall_south),
            (WallDirection::East, &room.wall_east),
            (WallDirection::West, &room.wall_west),
        ] {
            for (i, seg) in segs.iter().enumerate() {
                if seg.variant == WallVariant::WithDoor && seg.is_open {
                    update_door_edge(cache, &house.id, room, dir, i, true);
                }
            }
        }
    }
}
