//! Runtime passability cache + A* pathfinding for housing interiors.
//!
//! A house's `PassabilityGrid` (offline-computed walls, doors, room
//! boundaries) is converted at load-time into a `RuntimePassability`
//! cache entry. The cache is the single source of truth for both client
//! and server traversal queries: where the player can step, where a
//! cardinal A* expansion is allowed, what floor a Y coordinate maps to.
//!
//! Submodule layout:
//! - `cache`: build a cache entry from `HouseData`; mutate it as doors
//!   open and close.
//! - `query`: read-only collision and floor-lookup helpers used by both
//!   the search loop and continuous movement validation.
//! - `astar` / `stair`: A* search over a virtual 1m grid. Stairwell
//!   intermediate cells are encoded as floor-key values *between* two
//!   regular floors so the same machinery walks them.
//! - `smooth`: greedy line-of-sight smoothing applied on top of A* paths.

mod astar;
mod cache;
mod query;
mod smooth;
mod stair;

pub use astar::{find_path, DEFAULT_MAX_NODES};
pub use cache::{
    apply_door_overlays, build_furniture_passability, build_runtime_passability, door_cells,
    update_door_edge, FurniturePiece,
};
pub use query::{
    attack_line_blocked, blocking_entry_for_mover, get_floor_at_position, get_floor_y_base,
    in_stairwell_span, is_cardinal_move_blocked, is_cell_sealed, is_circle_blocked_on_floor,
    is_movement_blocked, is_movement_blocked_for_mover, leg_touches_stairwell, start_floor_at,
    storey_ground_y, supporting_floor_y, BlockInfo,
};
pub(crate) use query::{ramp_fraction, segment_touches_box};
pub use smooth::find_and_smooth_path;

use std::collections::HashMap;

/// The four cardinal neighbours.
pub const DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

// Edge bitmask constants (matches TypeScript EDGE_N/E/S/W)
pub(super) const EDGE_N: u8 = 1; // -Z edge
pub(super) const EDGE_E: u8 = 2; // +X edge
pub(super) const EDGE_S: u8 = 4; // +Z edge
pub(super) const EDGE_W: u8 = 8; // -X edge

#[derive(Debug, Clone)]
pub struct RuntimeFloorGrid {
    pub floor_level: u8,
    pub origin_x: i32,
    pub origin_z: i32,
    pub width: u8,
    pub depth: u8,
    pub y_base: f32,
    pub wall_height: f32,
    pub cells: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct StairwellInfo {
    pub local_min_x: i32,
    pub local_min_z: i32,
    pub local_max_x: i32,
    pub local_max_z: i32,
    pub lower_floor: u8,
    pub upper_floor: u8,
    pub along_z: bool,
    pub reversed: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimePassability {
    pub house_origin_x: f32,
    pub house_origin_z: f32,
    pub min_x: f32,
    pub max_x: f32,
    pub min_z: f32,
    pub max_z: f32,
    pub floors: Vec<RuntimeFloorGrid>,
    pub stairwells: Vec<StairwellInfo>,
    /// Whether this obstacle lets a mover it has sealed in step back out. True
    /// only for furniture, the one kind that can land on a standing player.
    /// See `query::blocking_entry_for_mover`.
    pub yields_to_trapped_mover: bool,
    /// Whether its grids are storeys a mover stands on (a house). False for
    /// furniture and for a dungeon, whose surface shell is one flat grid over
    /// the whole footprint — a collision hull, not ground.
    pub is_ground: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PathWaypoint {
    pub x: f32,
    pub z: f32,
    pub floor: u8,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PathResult {
    pub waypoints: Vec<PathWaypoint>,
    pub found: bool,
}

/// Type alias for the passability cache used throughout the API.
///
/// Deliberately unindexed: every query walks all entries and AABB-rejects.
/// The entry count is content-authored (houses, furniture regions, dungeons),
/// not player-driven, and sits in the single digits — see
/// `doc/RUNTIME_PERFORMANCE.md` for the measurements and for the conditions
/// under which a spatial index becomes worth building.
pub type PassabilityCache = HashMap<String, RuntimePassability>;

#[cfg(test)]
mod tests {
    use super::query::is_cell_sealed;
    use super::smooth::is_line_passable;
    use super::*;

    /// Edge-bitmask cells for a `width`×`depth` grid walled on its outer rim.
    fn perimeter_walls(width: u8, depth: u8) -> Vec<u8> {
        let w = width as usize;
        let d = depth as usize;
        let mut cells = vec![0u8; w * d];
        for x in 0..w {
            cells[x] |= EDGE_N;
            cells[x + (d - 1) * w] |= EDGE_S;
        }
        for z in 0..d {
            cells[z * w] |= EDGE_W;
            cells[z * w + w - 1] |= EDGE_E;
        }
        cells
    }

    fn make_rect_room(width: u8, depth: u8) -> (String, RuntimePassability) {
        let cells = perimeter_walls(width, depth);
        let rp = RuntimePassability {
            house_origin_x: 10.0,
            house_origin_z: 10.0,
            min_x: 10.0,
            max_x: 10.0 + width as f32,
            min_z: 10.0,
            max_z: 10.0 + depth as f32,
            floors: vec![RuntimeFloorGrid {
                floor_level: 0,
                origin_x: 0,
                origin_z: 0,
                width,
                depth,
                y_base: 0.0,
                wall_height: 3.0,
                cells,
            }],
            stairwells: vec![],
            yields_to_trapped_mover: false,
            is_ground: true,
        };
        ("house".to_string(), rp)
    }

    fn make_simple_house() -> (String, RuntimePassability) {
        make_rect_room(3, 3)
    }

    /// 8×5 room with a short interior wall stub jutting from the interior: an
    /// `EDGE_S` segment on the two cells at local (3,1) and (4,1), i.e. a wall
    /// along the z=12 line for world x∈[13,15]. Its tips are convex corners a
    /// body must give a wide berth even though a point-sized line can hug them.
    fn make_room_with_wall_stub() -> (String, RuntimePassability) {
        let (id, mut rp) = make_rect_room(8, 5);
        let cells = &mut rp.floors[0].cells;
        let w = 8usize;
        cells[3 + w] |= EDGE_S;
        cells[4 + w] |= EDGE_S;
        (id, rp)
    }

    #[test]
    fn line_alongside_wall_stub_blocked_by_body_radius() {
        let (id, rp) = make_room_with_wall_stub();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        // A line skimming z=12.2 runs 0.2 alongside the stub (z=12) — the
        // point-sized cell check clears it, but the 0.3 body radius clips the
        // wall, so smoothing must reject it (both endpoints are well clear).
        let clip_from = PathWaypoint {
            x: 11.5,
            z: 12.2,
            floor: 0,
        };
        let clip_to = PathWaypoint {
            x: 16.5,
            z: 12.2,
            floor: 0,
        };
        assert!(
            !is_line_passable(&clip_from, &clip_to, &cache),
            "line grazing the stub within body radius must not be passable"
        );

        // Same line pulled back to z=12.35 (0.35 > radius) clears the stub —
        // isolates the radius as the sole reason the first line is rejected.
        let clear_from = PathWaypoint {
            x: 11.5,
            z: 12.35,
            floor: 0,
        };
        let clear_to = PathWaypoint {
            x: 16.5,
            z: 12.35,
            floor: 0,
        };
        assert!(
            is_line_passable(&clear_from, &clear_to, &cache),
            "line clearing the stub by more than the body radius stays passable"
        );

        // Endpoint sitting against the stub stays passable — a near-wall goal is
        // expected and the mover just stops short, so smoothing shouldn't refuse it.
        let end_from = PathWaypoint {
            x: 13.5,
            z: 12.2,
            floor: 0,
        };
        let end_to = PathWaypoint {
            x: 16.5,
            z: 12.2,
            floor: 0,
        };
        assert!(
            is_line_passable(&end_from, &end_to, &cache),
            "a near-wall endpoint must not block smoothing"
        );
    }

    #[test]
    fn attacks_refuse_to_cross_a_wall() {
        let (id, rp) = make_room_with_wall_stub();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        // Either side of the stub, a metre apart: well inside melee reach and
        // still unreachable.
        assert!(attack_line_blocked(&cache, 13.5, 11.5, 13.5, 12.5, 0));
        // Same metre clear of the stub lands.
        assert!(!attack_line_blocked(&cache, 11.5, 11.5, 11.5, 12.5, 0));
        // Reach is a point, not a body: grazing the stub inside body radius is
        // no wall to a blade.
        assert!(!attack_line_blocked(&cache, 11.5, 12.2, 16.5, 12.2, 0));
        // Walls belong to their own floor, as everywhere else in the cache.
        assert!(!attack_line_blocked(&cache, 13.5, 11.5, 13.5, 12.5, 1));
    }

    #[test]
    fn furniture_cell_blocks_movement_and_pathing() {
        // A single solid furniture cell at world (5,5) on floor 0, blocking the
        // realistic low-obstacle Y band (furniture::FURNITURE_BLOCK_HEIGHT = 1.0).
        let cache = furniture_cache(vec![(5, 5)]);

        // Walking into the sealed cell from the north is blocked...
        assert!(is_movement_blocked(&cache, 5.5, 4.5, 5.5, 5.5, 0, None));
        // ...and from the west, too.
        assert!(is_movement_blocked(&cache, 4.5, 5.5, 5.5, 5.5, 0, None));
        // A parallel move that never enters the cell is allowed.
        assert!(!is_movement_blocked(&cache, 0.5, 0.5, 1.5, 0.5, 0, None));

        // Body radius keeps the character from hugging the furniture.
        assert!(is_circle_blocked_on_floor(&cache, 5.5, 4.85, 0.3, 0, None));

        // Blocking is confined to the furniture's own floor: someone a storey
        // up walks over it freely.
        assert!(!is_movement_blocked(&cache, 5.5, 4.5, 5.5, 5.5, 1, None));

        // Height matters as well as floor. A staircase runs above the floor it
        // stands on, so a climber is keyed to floor 0 while several metres up —
        // a 1 m table must not block them, nor snag their body radius.
        let above = Some(3.69);
        assert!(!is_movement_blocked(&cache, 5.5, 4.5, 5.5, 5.5, 0, above));
        assert!(!is_circle_blocked_on_floor(
            &cache, 5.5, 4.85, 0.3, 0, above
        ));
        // Standing on the floor itself, it blocks as before.
        assert!(is_movement_blocked(
            &cache,
            5.5,
            4.5,
            5.5,
            5.5,
            0,
            Some(0.5)
        ));

        // A* routes around the sealed cell instead of through it.
        assert!(is_cardinal_move_blocked(&cache, 5, 4, 0, 1, 0));
        let path = find_path(5.5, 3.5, 0, 5.5, 7.5, 0, &cache, 500);
        assert!(path.found, "a path around the furniture should exist");
        assert!(
            !path
                .waypoints
                .iter()
                .any(|w| w.x.floor() as i32 == 5 && w.z.floor() as i32 == 5 && w.floor == 0),
            "path must not pass through the sealed cell: {:?}",
            path.waypoints
        );
    }

    /// Cache entry for solid furniture on the given cells, keyed like a real
    /// region so `blocking_entry_for_mover` recognises it as furniture.
    fn furniture_cache(cells: Vec<(i32, i32)>) -> PassabilityCache {
        let rp = build_furniture_passability(&[FurniturePiece {
            cells,
            floor_level: 0,
            y_base: 0.0,
            wall_height: crate::furniture::FURNITURE_BLOCK_HEIGHT,
        }])
        .expect("cells should yield a passability entry");
        let mut cache = PassabilityCache::new();
        cache.insert(crate::furniture::region_cache_key(0, 0), rp);
        cache
    }

    #[test]
    fn furniture_sealed_player_can_step_out() {
        // A bed's footprint seals every edge of the cells it covers, so a player
        // standing where one is placed has no legal step in any direction.
        let cache = furniture_cache(vec![(5, 5)]);
        let y = Some(0.5);
        assert!(is_cell_sealed(&cache, 5.5, 5.5, 0, y));

        // Every way out is refused by the plain check — that is the trap.
        for (tx, tz) in [(6.5, 5.5), (4.5, 5.5), (5.5, 6.5), (5.5, 4.5)] {
            assert!(is_movement_blocked(&cache, 5.5, 5.5, tx, tz, 0, y));
            assert!(
                !is_movement_blocked_for_mover(&cache, 5.5, 5.5, tx, tz, 0, y),
                "a sealed-in mover must be let out towards ({tx}, {tz})"
            );
        }

        // The waiver is only for getting out. Walking back in still blocks, so
        // the cell doesn't become freely passable for everyone else.
        assert!(is_movement_blocked_for_mover(
            &cache, 5.5, 4.5, 5.5, 5.5, 0, y
        ));

        // And it ends at the neighbouring cell: one long sweep can't ride the
        // waiver across a second piece of furniture.
        let cache = furniture_cache(vec![(5, 5), (5, 7)]);
        assert!(is_cell_sealed(&cache, 5.5, 5.5, 0, y));
        assert!(is_movement_blocked_for_mover(
            &cache, 5.5, 5.5, 5.5, 7.5, 0, y
        ));
    }

    #[test]
    fn sealed_mover_cannot_walk_through_walls() {
        // Furniture filling a 3×3 room's centre leaves the player sealed by a
        // mix of furniture and the room's own walls. The furniture sides yield;
        // the house shell must not, or this becomes a wall hack.
        let (id, rp) = make_rect_room(3, 3);
        let mut cache = furniture_cache(vec![(11, 10), (10, 11), (11, 12), (12, 11)]);
        cache.insert(id, rp);
        let y = Some(0.5);

        // Player at the room's centre cell (11,11): furniture on all four sides.
        assert!(is_cell_sealed(&cache, 11.5, 11.5, 0, y));
        assert!(!is_movement_blocked_for_mover(
            &cache, 11.5, 11.5, 12.5, 11.5, 0, y
        ));

        // From a corner furniture cell the outward side is the room's wall.
        // Sealed or not, that wall keeps refusing.
        assert!(is_cell_sealed(&cache, 10.5, 11.5, 0, y));
        assert!(
            is_movement_blocked_for_mover(&cache, 10.5, 11.5, 9.5, 11.5, 0, y),
            "the house wall must refuse even a sealed-in mover"
        );
        // Its furniture-blocked side, back towards the centre, does yield.
        assert!(!is_movement_blocked_for_mover(
            &cache, 10.5, 11.5, 10.5, 10.5, 0, y
        ));
    }

    #[test]
    fn open_floor_is_not_sealed() {
        // Sanity: the waiver must never fire for an ordinary player standing
        // next to a wall, or walls stop working entirely.
        let (id, rp) = make_rect_room(3, 3);
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);
        assert!(!is_cell_sealed(&cache, 11.5, 11.5, 0, Some(0.5)));
        assert!(!is_cell_sealed(&cache, 10.5, 10.5, 0, Some(0.5)));
        assert!(is_movement_blocked_for_mover(
            &cache,
            10.5,
            10.5,
            9.5,
            10.5,
            0,
            Some(0.5)
        ));
    }

    #[test]
    fn cardinal_move_blocked_by_wall() {
        let (id, rp) = make_simple_house();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        // Trying to move west from cell (10, 10) — blocked by west wall
        assert!(is_cardinal_move_blocked(&cache, 10, 10, -1, 0, 0));
        // Moving east from (10, 10) within the house — not blocked
        assert!(!is_cardinal_move_blocked(&cache, 10, 10, 1, 0, 0));
        // Moving east from (12, 10) — blocked by east wall
        assert!(is_cardinal_move_blocked(&cache, 12, 10, 1, 0, 0));
    }

    #[test]
    fn find_path_around_house() {
        let (id, rp) = make_simple_house();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        // Path from west of house to east of house
        let result = find_path(9.5, 11.5, 0, 13.5, 11.5, 0, &cache, 200);
        assert!(result.found);
        assert!(!result.waypoints.is_empty());
        // Path should go around the house, not through it
        assert!(result.waypoints.len() > 1);
    }

    #[test]
    fn path_in_open_terrain() {
        let cache = PassabilityCache::new(); // No houses
        let result = find_path(0.0, 0.0, 0, 5.0, 5.0, 0, &cache, 200);
        assert!(result.found);
    }

    #[test]
    fn smooth_path_does_not_cross_walls() {
        let (id, rp) = make_simple_house();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        // Diagonal line from NW corner to SE corner of house would cross walls
        let from = PathWaypoint {
            x: 9.5,
            z: 9.5,
            floor: 0,
        };
        let to = PathWaypoint {
            x: 13.5,
            z: 13.5,
            floor: 0,
        };
        assert!(!is_line_passable(&from, &to, &cache));

        // Line along the north side outside the house — should be passable
        let from2 = PathWaypoint {
            x: 9.5,
            z: 9.5,
            floor: 0,
        };
        let to2 = PathWaypoint {
            x: 13.5,
            z: 9.5,
            floor: 0,
        };
        assert!(is_line_passable(&from2, &to2, &cache));
    }

    #[test]
    fn smooth_path_preserves_endpoints() {
        let (id, rp) = make_simple_house();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        // Path around the house should be smoothed but still start and end correctly
        let result = find_and_smooth_path(9.5, 11.5, 0, 13.5, 11.5, 0, &cache, 200);
        assert!(result.found);
        // The straight line pierces the house, so the direct shortcut must
        // decline and A* must produce a detour.
        assert!(result.waypoints.len() > 1);
        let first = &result.waypoints[0];
        let last = result.waypoints.last().unwrap();
        // First waypoint should be near start, last near goal
        assert!((first.x - 9.5).abs() < 1.0 || (first.x - 10.5).abs() < 1.0);
        assert!((last.x - 13.5).abs() < 0.01);
    }

    #[test]
    fn smooth_diagonal_inside_room() {
        let (id, rp) = make_rect_room(5, 5);
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        // Diagonal across the room interior (cell centers) — must be passable
        let from = PathWaypoint {
            x: 10.5,
            z: 10.5,
            floor: 0,
        };
        let to = PathWaypoint {
            x: 14.5,
            z: 14.5,
            floor: 0,
        };
        assert!(is_line_passable(&from, &to, &cache));

        // Walk parallel to north wall at z=10.2 — should be passable
        // (directional check: not approaching, just moving parallel)
        let from2 = PathWaypoint {
            x: 10.5,
            z: 10.2,
            floor: 0,
        };
        let to2 = PathWaypoint {
            x: 14.5,
            z: 10.2,
            floor: 0,
        };
        assert!(is_line_passable(&from2, &to2, &cache));

        // Goal near a wall corner — endpoint proximity shouldn't block smoothing
        let from3 = PathWaypoint {
            x: 10.5,
            z: 10.5,
            floor: 0,
        };
        let to3 = PathWaypoint {
            x: 14.8,
            z: 14.8,
            floor: 0,
        };
        assert!(is_line_passable(&from3, &to3, &cache));

        // Full find_and_smooth: diagonal should produce ≤2 waypoints (direct line)
        let result = find_and_smooth_path(10.5, 10.5, 0, 14.5, 14.5, 0, &cache, 500);
        assert!(result.found);
        assert!(
            result.waypoints.len() <= 2,
            "Expected smooth diagonal (≤2 waypoints), got {}",
            result.waypoints.len()
        );
    }

    #[test]
    fn smooth_diagonal_inside_rectangular_room() {
        // Wide rectangle: 8x3
        let (id, rp) = make_rect_room(8, 3);
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        let result = find_and_smooth_path(10.5, 10.5, 0, 17.5, 12.5, 0, &cache, 500);
        assert!(result.found);
        assert!(
            result.waypoints.len() == 1,
            "8x3 room: expected single goal waypoint (direct diagonal), got {} waypoints: {:?}",
            result.waypoints.len(),
            result
                .waypoints
                .iter()
                .map(|w| (w.x, w.z))
                .collect::<Vec<_>>()
        );

        // Tall rectangle: 3x8
        let (id2, rp2) = make_rect_room(3, 8);
        let mut cache2 = PassabilityCache::new();
        cache2.insert(id2, rp2);

        let result2 = find_and_smooth_path(10.5, 10.5, 0, 12.5, 17.5, 0, &cache2, 500);
        assert!(result2.found);
        assert!(
            result2.waypoints.len() == 1,
            "3x8 room: expected single goal waypoint (direct diagonal), got {} waypoints: {:?}",
            result2.waypoints.len(),
            result2
                .waypoints
                .iter()
                .map(|w| (w.x, w.z))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn direct_shortcut_on_open_ground() {
        let cache = PassabilityCache::new();
        let result = find_and_smooth_path(100.5, 200.5, 0, 180.5, 260.5, 0, &cache, 2000);
        assert!(result.found);
        assert_eq!(result.waypoints.len(), 1);
        let wp = &result.waypoints[0];
        assert!((wp.x - 180.5).abs() < 0.01 && (wp.z - 260.5).abs() < 0.01);
    }

    /// Two single-row floors joined by one stairwell column:
    ///   floor 0 (lower): world cells (0..3, z=0)
    ///   floor 1 (upper): world cells (0..3, z=3)
    ///   stairwell: x=0, z=0..4, connecting lower landing (0,0) to upper (0,3).
    /// House origin is (0,0); both rows are open in X with perimeter walls.
    fn make_two_floor_stairwell() -> (String, RuntimePassability) {
        // 3x1 open row walled on its rim: [W|N|S, N|S, E|N|S].
        let row = || perimeter_walls(3, 1);
        let rp = RuntimePassability {
            house_origin_x: 0.0,
            house_origin_z: 0.0,
            min_x: 0.0,
            max_x: 3.0,
            min_z: 0.0,
            max_z: 4.0,
            floors: vec![
                RuntimeFloorGrid {
                    floor_level: 0,
                    origin_x: 0,
                    origin_z: 0,
                    width: 3,
                    depth: 1,
                    y_base: 0.0,
                    wall_height: 3.0,
                    cells: row(),
                },
                RuntimeFloorGrid {
                    floor_level: 1,
                    origin_x: 0,
                    origin_z: 3,
                    width: 3,
                    depth: 1,
                    y_base: 3.1,
                    wall_height: 3.0,
                    cells: row(),
                },
            ],
            stairwells: vec![StairwellInfo {
                local_min_x: 0,
                local_min_z: 0,
                local_max_x: 1,
                local_max_z: 4,
                lower_floor: 0,
                upper_floor: 1,
                along_z: true,
                reversed: false,
            }],
            yields_to_trapped_mover: false,
            is_ground: true,
        };
        ("two_floor".to_string(), rp)
    }

    /// Pins the span a climber may report a height within.
    #[test]
    fn stairwell_span_covers_the_flight_but_not_a_forged_height() {
        let (id, rp) = make_two_floor_stairwell();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        // Mid-flight between the two y_bases (0.0 and 3.1).
        assert!(query::in_stairwell_span(&cache, 0.5, 1.5, 2.0));
        // Just past either end — the step-height jitter the tolerance absorbs.
        assert!(query::in_stairwell_span(&cache, 0.5, 1.5, -0.5));
        assert!(query::in_stairwell_span(&cache, 0.5, 1.5, 3.6));
        // Above the flight entirely: no flight to be on.
        assert!(!query::in_stairwell_span(&cache, 0.5, 1.5, 10.0));
        // Off the stairwell's footprint, at a height it would have allowed.
        assert!(!query::in_stairwell_span(&cache, 2.5, 1.5, 2.0));
    }

    /// The height a server stores for a mover instead of the reported one:
    /// landings and ramp on the stairs, the storey's `y_base` on its grid,
    /// nothing on open terrain.
    #[test]
    fn storey_ground_y_follows_the_stairs_and_the_grid() {
        let (id, rp) = make_two_floor_stairwell();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);
        // Stairwell x 0..1, z 0..4: entry landing, half-way up the 3m run
        // between the 0.5m landings, exit landing — whichever storey the
        // mover is keyed to, whatever it claims; then each storey's own grid.
        for (floor, x, z, hint, want) in [
            (0, 0.5, 0.25, 1000.0, Some(0.0)),
            (0, 0.5, 2.0, 1000.0, Some(1.55)),
            (1, 0.5, 2.0, -1000.0, Some(1.55)),
            (0, 0.5, 3.75, 1000.0, Some(3.1)),
            (0, 1.5, 0.5, 1000.0, Some(0.0)),
            (1, 1.5, 3.5, -5.0, Some(3.1)),
            (1, 1.5, 0.5, 0.0, None),
            (0, 50.0, 50.0, 7.0, None),
        ] {
            let got = query::storey_ground_y(&cache, floor, x, z, hint);
            let close = match (got, want) {
                (Some(a), Some(b)) => (a - b).abs() < 1e-4,
                (None, None) => true,
                _ => false,
            };
            assert!(
                close,
                "floor {floor} at ({x}, {z}): got {got:?}, want {want:?}"
            );
        }
    }

    /// A dungeon's surface shell (or furniture) is a collision hull, not
    /// ground: it must not flatten the stored Y of everyone walking near it.
    #[test]
    fn storey_ground_y_ignores_non_ground_entries() {
        let (id, mut rp) = make_two_floor_stairwell();
        rp.is_ground = false;
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);
        assert_eq!(query::storey_ground_y(&cache, 0, 1.5, 0.5, 1000.0), None);
        assert!(!query::leg_touches_stairwell(
            &cache,
            0,
            1,
            (0.5, 0.5),
            (0.5, 1.5)
        ));
    }

    /// Only a short leg touching the stairwell (±1 cell) may change storey.
    #[test]
    fn leg_touches_stairwell_bounds_where_a_storey_changes() {
        let (id, rp) = make_two_floor_stairwell();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);
        let touches = |from, to| query::leg_touches_stairwell(&cache, 0, 1, from, to);

        assert!(touches((0.5, 0.5), (0.5, 3.5)));
        // A standalone change from the stairs, and from the margin cell.
        assert!(touches((0.5, 2.5), (0.5, 2.5)));
        assert!(touches((1.5, 2.5), (1.5, 2.5)));
        // Two cells off the flight, and a leg clipping the margin but eight
        // cells long (the run is four).
        assert!(!touches((2.5, 2.5), (2.5, 2.5)));
        assert!(!touches((1.5, -3.0), (1.5, 6.0)));
        // Not the storeys this stairwell joins.
        assert!(!query::leg_touches_stairwell(
            &cache,
            1,
            2,
            (0.5, 0.5),
            (0.5, 3.5)
        ));
    }

    /// A leg crosses obstacles mid-span as often as at its ends, so the
    /// supporting height is looked up over the whole swept box.
    #[test]
    fn supporting_floor_y_reads_the_storey_the_leg_crosses() {
        let (id, rp) = make_two_floor_stairwell();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        // Floor 0's grid spans z 0..1; a leg from z=-2 to z=2 crosses it
        // without either end landing on it.
        assert_eq!(
            query::supporting_floor_y(&cache, 0.5, -2.0, 0.5, 2.0, 0, 9.0),
            Some(0.0)
        );
        // Keyed to floor 1, the box over that storey reports it instead.
        assert_eq!(
            query::supporting_floor_y(&cache, 0.5, 2.5, 0.5, 3.5, 1, 9.0),
            Some(3.1)
        );
        // Nothing at that level under the box.
        assert_eq!(
            query::supporting_floor_y(&cache, 0.5, 50.0, 0.5, 51.0, 0, 0.0),
            None
        );
    }

    /// One leg can cross furniture standing on ground of different heights, so
    /// two grids at the same level are resolved by nearest `y_base` — not by
    /// whichever the cache happens to yield first.
    #[test]
    fn supporting_floor_y_picks_the_nearest_of_two_grids_on_one_level() {
        let (id, rp) = make_two_floor_stairwell();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);
        // A second floor-0 grid over the same cells, ten metres up a hill.
        let mut hill = make_two_floor_stairwell().1;
        hill.floors.retain(|f| f.floor_level == 0);
        hill.floors[0].y_base = 10.0;
        hill.stairwells.clear();
        cache.insert("hill".to_string(), hill);

        let leg = |y| query::supporting_floor_y(&cache, 0.5, -2.0, 0.5, 2.0, 0, y);
        assert_eq!(leg(0.4), Some(0.0));
        assert_eq!(leg(9.6), Some(10.0));
    }

    #[test]
    fn cross_floor_query_descends_the_stairwell() {
        let (id, rp) = make_two_floor_stairwell();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        // Start on the upper floor (1), goal on the lower floor (0): differing
        // floors, so the stairwell is traversed.
        let result = find_path(2.5, 3.5, 1, 2.5, 0.5, 0, &cache, 500);
        assert!(result.found, "cross-floor path should be found");
        assert!(
            result.waypoints.iter().any(|w| w.floor == 0),
            "cross-floor path must reach the lower floor: {:?}",
            result.waypoints
        );
    }

    #[test]
    fn same_floor_query_never_leaves_its_floor() {
        let (id, rp) = make_two_floor_stairwell();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        // Same start, but request the goal on the START floor (1). The target
        // cell only exists on floor 0, so without confinement A* would dive
        // down the stairwell to approach it. Confinement keeps every waypoint
        // on floor 1 — the fix that stops dungeon monsters using stairs.
        let result = find_path(2.5, 3.5, 1, 0.5, 0.5, 1, &cache, 500);
        assert!(
            !result.waypoints.is_empty(),
            "confined partial path should still advance along its own floor"
        );
        assert!(
            result.waypoints.iter().all(|w| w.floor == 1),
            "confined path must never descend the stairwell: {:?}",
            result.waypoints
        );
    }

    #[test]
    fn same_floor_query_can_target_stairwell_interior() {
        let (id, rp) = make_two_floor_stairwell();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        // A player on the floor-0 landing clicks the middle of the stair run.
        // The mid-step is also keyed to floor 0, but it is only reachable via
        // the stair-axis expansion; regular same-floor movement treats it as
        // stairwell interior and blocks entry from the room grid.
        let result = find_path(2.5, 0.5, 0, 0.5, 1.5, 0, &cache, 500);
        assert!(result.found, "landing-to-mid-stair click should path");
        let last = result.waypoints.last().expect("path should have waypoints");
        assert!(
            (last.x - 0.5).abs() < 0.01 && (last.z - 1.5).abs() < 0.01,
            "path should end on the clicked stair step: {:?}",
            result.waypoints
        );
    }

    /// Reproduces the dungeon entrance-shaft bug: a player standing on an
    /// *intermediate* stairwell cell clicks a cell on the connected floor.
    /// The shaft's intermediate cells are keyed to the lower floor (0), so the
    /// query MUST start on floor 0 and end on floor 1 for A* to traverse the
    /// stairs. The (buggy) client override forced start_floor == goal_floor,
    /// which confines the search and strands the player.
    #[test]
    fn mid_stairwell_start_reaches_connected_floor() {
        let (id, rp) = make_two_floor_stairwell();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        // Standing mid-shaft at cell (0,1) — an intermediate stair step keyed to
        // floor 0. Goal is the room on floor 1 at (2,3). Start on the shaft's
        // keyed (lower) floor so the stairwell is traversable.
        let result = find_path(0.5, 1.5, 0, 2.5, 3.5, 1, &cache, 500);
        assert!(result.found, "mid-shaft path to the room should be found");
        assert!(
            result.waypoints.last().map(|w| w.floor) == Some(1),
            "path must arrive on the room floor: {:?}",
            result.waypoints
        );
        // It must NOT detour to the far (z=0) landing before heading to the room
        // at z=3 — every emitted (regular-key) waypoint is on the way up.
        assert!(
            result.waypoints.iter().all(|w| w.z >= 1.0),
            "path must not detour back down to the entry landing: {:?}",
            result.waypoints
        );
    }

    /// Confirms the override is the bug: starting the SAME mid-shaft query on
    /// the goal floor (start_floor == goal_floor) confines A* and fails to
    /// produce a path to the room — the player gets stranded / re-routed.
    #[test]
    fn mid_stairwell_start_on_goal_floor_is_stranded() {
        let (id, rp) = make_two_floor_stairwell();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        let result = find_path(0.5, 1.5, 1, 2.5, 3.5, 1, &cache, 500);
        let reaches_room = result
            .waypoints
            .last()
            .map(|w| (w.x - 2.5).abs() < 0.6 && (w.z - 3.5).abs() < 0.6)
            .unwrap_or(false);
        assert!(
            !reaches_room,
            "confined start_floor==goal_floor must NOT reach the room cleanly: {:?}",
            result.waypoints
        );
    }

    /// Two-floor house with a stairwell in the last column (world x∈[1,2],
    /// z∈[0,2]); its bottom landing is cell (1,1). Each floor's grid can
    /// independently seal the landing's south edge — the exit into the
    /// ground-floor room.
    fn make_stairwell_house(lower_seals: bool, upper_seals: bool) -> (String, RuntimePassability) {
        let (w, d) = (2u8, 3u8);
        let landing = 1 + w as usize;
        let mut lower = vec![0u8; (w * d) as usize];
        let mut upper = vec![0u8; (w * d) as usize];
        if lower_seals {
            lower[landing] |= EDGE_S;
        }
        if upper_seals {
            upper[landing] |= EDGE_S;
        }

        let grid = |floor_level: u8, y_base: f32, cells: Vec<u8>| RuntimeFloorGrid {
            floor_level,
            origin_x: 0,
            origin_z: 0,
            width: w,
            depth: d,
            y_base,
            wall_height: 3.0,
            cells,
        };

        let rp = RuntimePassability {
            house_origin_x: 0.0,
            house_origin_z: 0.0,
            min_x: 0.0,
            max_x: w as f32,
            min_z: 0.0,
            max_z: d as f32,
            floors: vec![grid(0, 0.0, lower), grid(1, 3.1, upper)],
            stairwells: vec![StairwellInfo {
                local_min_x: 1,
                local_min_z: 0,
                local_max_x: 2,
                local_max_z: 2,
                lower_floor: 0,
                upper_floor: 1,
                along_z: true,
                reversed: false,
            }],
            yields_to_trapped_mover: false,
            is_ground: true,
        };
        ("house".to_string(), rp)
    }

    /// A player keyed to the upper floor while standing on the bottom landing —
    /// the grid that seals the end they are on. Without the two-floor rule a
    /// blocked step never moves them, so nothing ever corrects it.
    #[test]
    fn stairwell_exit_allowed_when_the_other_connected_floor_allows() {
        let (id, rp) = make_stairwell_house(false, true);
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        assert!(
            !query::is_movement_blocked(&cache, 1.5, 1.5, 1.5, 2.5, 1, None),
            "stepping off the bottom landing must stay open while the lower floor allows it"
        );
    }

    #[test]
    fn stairwell_exit_blocked_when_both_connected_floors_block() {
        let (id, rp) = make_stairwell_house(true, true);
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        assert!(
            query::is_movement_blocked(&cache, 1.5, 1.5, 1.5, 2.5, 1, None),
            "a genuinely walled stairwell exit must still block"
        );
    }

    /// Bounds the `in_stairwell_span` exemption. A caller that trusts a mover's
    /// reported Y inside a stairwell is not handing it the house: the two-floor
    /// consult is deliberately Y-blind, so no claimed height unseals a walled
    /// exit. The exemption reaches only the low obstacles under the stairs,
    /// which is what it is for.
    #[test]
    fn no_claimed_height_unseals_a_walled_stairwell_exit() {
        let (id, rp) = make_stairwell_house(true, true);
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        // 4.2 clears the lower floor's walls (y_base 0 + wall_height 3.0) and
        // sits inside the flight's span, so the exemption would keep it.
        for y in [Some(2.0), Some(4.2), Some(1000.0)] {
            assert!(
                query::is_movement_blocked(&cache, 1.5, 1.5, 1.5, 2.5, 1, y),
                "a walled stairwell exit must block at y={y:?}"
            );
        }
    }

    /// The relaxation is scoped to stairwell footprints: an ordinary wall one
    /// column over is keyed to the mover's floor alone and still blocks.
    #[test]
    fn non_stairwell_wall_still_blocks_on_the_movers_floor() {
        let (id, mut rp) = make_stairwell_house(false, false);
        rp.floors[1].cells[2] |= EDGE_S; // cell (0,1), outside the stairwell
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        assert!(
            query::is_movement_blocked(&cache, 0.5, 1.5, 0.5, 2.5, 1, None),
            "a normal wall on the mover's own floor must block"
        );
        assert!(
            !query::is_movement_blocked(&cache, 0.5, 1.5, 0.5, 2.5, 0, None),
            "...and must not reach the floor below it"
        );
    }

    /// A bed placed over a standing mover seals its whole footprint. The
    /// movement validator waives one step out of a sealed cell across
    /// furniture (`blocking_entry_for_mover`); A* must plan that same step,
    /// or a mover woken on a bed paths nowhere and callers fall back to
    /// teleporting it through walls.
    #[test]
    fn astar_escapes_a_furniture_sealed_start() {
        let cache = furniture_cache(vec![(5, 5), (5, 6)]);
        let result = find_path(5.5, 5.5, 0, 8.5, 5.5, 0, &cache, 500);
        assert!(
            result.found,
            "a mover sealed onto a bed must still path out"
        );
    }

    /// The waiver is furniture-only: sealed in by something that does not
    /// yield, A* must still plan nothing.
    #[test]
    fn astar_grants_no_escape_from_a_wall_sealed_start() {
        let (id, rp) = make_rect_room(1, 1);
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        let result = find_path(10.5, 10.5, 0, 13.5, 10.5, 0, &cache, 500);
        assert!(
            !result.found,
            "walls never yield, even to a sealed-in mover"
        );
        assert!(result.waypoints.is_empty());
    }

    /// Furniture sealed against a wall: the furniture sides open, the wall
    /// side stays solid, so the escape goes around the wall — never through.
    #[test]
    fn sealed_start_escape_respects_walls() {
        let mut cache = furniture_cache(vec![(5, 5)]);
        // A wall along the east edge of x=5 for z in 4..=6.
        cache.insert(
            "house".to_string(),
            RuntimePassability {
                house_origin_x: 5.0,
                house_origin_z: 4.0,
                min_x: 5.0,
                max_x: 6.0,
                min_z: 4.0,
                max_z: 7.0,
                floors: vec![RuntimeFloorGrid {
                    floor_level: 0,
                    origin_x: 0,
                    origin_z: 0,
                    width: 1,
                    depth: 3,
                    y_base: 0.0,
                    wall_height: 3.0,
                    cells: vec![EDGE_E; 3],
                }],
                stairwells: vec![],
                yields_to_trapped_mover: false,
                is_ground: true,
            },
        );

        let result = find_path(5.5, 5.5, 0, 8.5, 5.5, 0, &cache, 500);
        assert!(result.found, "a detour around the wall exists");
        let first = &result.waypoints[0];
        assert!(
            !(first.x.floor() as i32 == 6 && first.z.floor() as i32 == 5),
            "the escape step must not cross the wall side: {:?}",
            result.waypoints
        );
    }
}

#[cfg(test)]
mod real_house_repro {
    use super::astar::find_path;
    use super::*;

    /// The real Aldermark house r-23_+73_1: two floors, stairwell in the last
    /// column. Grids copied verbatim from its stored passability.
    fn real_house() -> (String, RuntimePassability) {
        let f0: Vec<u8> = vec![
            9, 1, 1, 1, 1, 1, 1, 1, 3, 11, 8, 0, 0, 0, 0, 0, 0, 0, 2, 10, 8, 0, 0, 0, 0, 0, 0, 0,
            2, 10, 12, 4, 4, 4, 4, 4, 0, 0, 0, 2, 1, 1, 1, 1, 1, 3, 8, 0, 0, 2, 0, 0, 0, 0, 0, 2,
            12, 4, 4, 6,
        ];
        let f1: Vec<u8> = vec![
            9, 1, 1, 1, 1, 1, 1, 1, 1, 3, 8, 0, 0, 0, 0, 0, 0, 0, 2, 10, 8, 0, 0, 0, 0, 0, 0, 0, 2,
            10, 12, 4, 4, 4, 4, 4, 0, 0, 2, 14, 1, 1, 1, 1, 1, 3, 8, 0, 0, 3, 0, 0, 0, 0, 0, 2, 12,
            4, 4, 6,
        ];
        let grid = |floor_level: u8, y_base: f32, cells: Vec<u8>| RuntimeFloorGrid {
            floor_level,
            origin_x: -6,
            origin_z: 0,
            width: 10,
            depth: 6,
            y_base,
            wall_height: 3.0,
            cells,
        };
        let rp = RuntimePassability {
            house_origin_x: -1470.0,
            house_origin_z: 4732.0,
            min_x: -1476.0,
            max_x: -1466.0,
            min_z: 4732.0,
            max_z: 4738.0,
            floors: vec![grid(0, 1.0609375, f0), grid(1, 4.1609375, f1)],
            stairwells: vec![StairwellInfo {
                local_min_x: 3,
                local_min_z: 0,
                local_max_x: 4,
                local_max_z: 4,
                lower_floor: 0,
                upper_floor: 1,
                along_z: true,
                reversed: true,
            }],
            yields_to_trapped_mover: false,
            is_ground: true,
        };
        ("r-23_+73_1".to_string(), rp)
    }

    #[test]
    fn repro_second_floor_walk_west() {
        let (id, rp) = real_house();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        // Player stuck at world (-1468.0, 4733.07) on floor 1, clicking west
        // into the far room (world x -1472 is local -2, inside room 2).
        let r = find_path(-1468.0, 4733.07, 1, -1472.0, 4733.5, 1, &cache, 20000);
        assert!(
            r.found,
            "a straight walk west across floor 1 must find a path"
        );
        for w in &r.waypoints {
            assert!(
                w.x < -1466.5,
                "path must not detour east into the stairwell: {:?}",
                r.waypoints
            );
        }
    }

    /// Descending 2F→1F, a player stalled on the stairwell at y=4.2 and every
    /// retry re-blocked, so B saw them frozen on the landing while A's own
    /// client walked on. floor 0's walls top out at y_base+wall_height=4.06,
    /// so the height filter dropped it from the two-floor consult and left
    /// floor 1 — the grid sealing this very exit — to decide alone.
    #[test]
    fn stairwell_exit_near_the_top_of_the_flight_is_not_trapped() {
        let (id, rp) = real_house();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        assert!(
            !query::is_movement_blocked(&cache, -1466.5, 4735.5, -1467.2, 4735.5, 1, Some(4.2)),
            "stepping west off the stairwell must stay open even where the \
             lower floor's walls no longer reach the mover"
        );
    }

    /// `old_crypt`, whose AABB is an 80 m square swallowing a whole block of
    /// Aldermark — including the house above. Every cell walls off every side,
    /// and one of its stairwells sits directly under the house. Its floors carry
    /// real dungeon indices, which is what keeps them off housing's 0..3.
    fn dungeon_under_the_house() -> (String, RuntimePassability) {
        let (w, d) = (80usize, 80usize);
        let grid = |floor_level: u8, y_base: f32| RuntimeFloorGrid {
            floor_level,
            origin_x: 0,
            origin_z: 0,
            width: w as u8,
            depth: d as u8,
            y_base,
            wall_height: 3.0,
            cells: vec![EDGE_N | EDGE_E | EDGE_S | EDGE_W; w * d],
        };
        let rp = RuntimePassability {
            house_origin_x: -1490.0,
            house_origin_z: 4680.0,
            min_x: -1490.0,
            max_x: -1410.0,
            min_z: 4680.0,
            max_z: 4760.0,
            floors: vec![
                grid(crate::dungeon::passability_floor_for_depth(1), -30.0),
                grid(crate::dungeon::passability_floor_for_depth(2), -26.9),
            ],
            // World x -1470..-1466, z 4730..4734 — squarely under the house.
            stairwells: vec![StairwellInfo {
                local_min_x: 20,
                local_min_z: 50,
                local_max_x: 24,
                local_max_z: 54,
                lower_floor: crate::dungeon::passability_floor_for_depth(1),
                upper_floor: crate::dungeon::passability_floor_for_depth(2),
                along_z: true,
                reversed: false,
            }],
            yields_to_trapped_mover: false,
            is_ground: true,
        };
        ("dungeon:old_crypt".to_string(), rp)
    }

    /// Collision once inferred the floor from Y, which put a player on the
    /// house's 2F inside the crypt's Y-blind stairwell rule and walled off a
    /// westward walk. Keying on the floor index makes the two spaces disjoint
    /// by construction — housing owns 0..3, dungeons start well above it.
    #[test]
    fn dungeon_below_cannot_block_the_surface_house() {
        let mut cache = PassabilityCache::new();
        let (id, rp) = real_house();
        cache.insert(id, rp);
        let (did, drp) = dungeon_under_the_house();
        cache.insert(did, drp);

        assert!(
            !query::is_movement_blocked(&cache, -1468.0, 4732.45, -1468.05, 4732.45, 1, None),
            "a dungeon 30 m below must not wall off the house above it"
        );
    }

    /// Descending 2F→1F. The stairwell is grid column cx=9; its bottom landing
    /// (cz=3, reversed stairs) is cell 14 = E|S|W on floor 1 — floor 1 seals the
    /// end it does not own. The edge check survives that via the two-floor rule,
    /// but the body radius sits right on the seal, so the circle check needs the
    /// same rule or the player is walled in at the foot of the stairs.
    #[test]
    fn body_radius_clears_the_stairwell_end_the_keyed_floor_seals() {
        let (id, rp) = real_house();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        // Keyed to floor 1, stepping west off the bottom landing.
        assert!(
            !query::is_circle_blocked_on_floor(&cache, -1467.1, 4735.5, 0.3, 1, None),
            "floor 1's seal on the bottom landing must not trap the body radius"
        );
        // The outer east wall is walled on both floors and must still stop it.
        assert!(
            query::is_circle_blocked_on_floor(&cache, -1466.1, 4735.5, 0.3, 1, None),
            "a wall both connected floors agree on must still block the body"
        );
    }

    /// The body-radius check runs the same two-floor consult as the edge check
    /// and must not trap the mover either: near the top of the flight floor 0's
    /// walls fall below y=4.2, and dropping that partner leaves floor 1 — which
    /// seals this end — deciding alone.
    #[test]
    fn body_radius_near_the_top_of_the_flight_is_not_trapped() {
        let (id, rp) = real_house();
        let mut cache = PassabilityCache::new();
        cache.insert(id, rp);

        assert!(
            !query::is_circle_blocked_on_floor(&cache, -1467.1, 4735.5, 0.3, 1, Some(4.2)),
            "the body radius must clear the stairwell where the lower floor's \
             walls no longer reach the mover"
        );
    }

    /// The disjointness must not be bought by making the dungeon toothless:
    /// down in the crypt, on its own floor, every wall still blocks.
    #[test]
    fn dungeon_still_blocks_on_its_own_floor() {
        let mut cache = PassabilityCache::new();
        let (did, drp) = dungeon_under_the_house();
        cache.insert(did, drp);

        assert!(
            query::is_movement_blocked(
                &cache,
                -1468.0,
                4732.45,
                -1468.05,
                4732.45,
                crate::dungeon::passability_floor_for_depth(1),
                None,
            ),
            "every cell is walled, so the move must still block at crypt depth"
        );
    }
}
