//! Pathfinding abstraction — lets the brain run against WASM's passability data
//! or a native [`PassabilityCache`](pathfinding::PassabilityCache).

use crate::pathfinding::{self, PathResult};

pub trait PathProvider {
    fn find_path(
        &self,
        start_x: f32,
        start_z: f32,
        start_floor: u8,
        goal_x: f32,
        goal_z: f32,
        goal_floor: u8,
    ) -> PathResult;

    /// Whether a blow from `(from_x, from_z)` to `(to_x, to_z)` on `floor`
    /// crosses a wall — a shut door, a stair wall. No default: a provider that
    /// forgot to answer would let its monsters swing through walls, which is
    /// the one thing this gate exists to stop.
    fn attack_line_blocked(
        &self,
        from_x: f32,
        from_z: f32,
        to_x: f32,
        to_z: f32,
        floor: u8,
    ) -> bool;

    /// Cheap pre-check that the cell holding `(x, z)` can be stood in at all.
    /// Standing-cell candidates go through this before `find_path`: A* has no
    /// early goal reject, so a wall-interior candidate would otherwise flood
    /// the whole reachable region every try. Default true — a provider without
    /// the data just pays the path query.
    fn cell_passable(&self, _x: f32, _z: f32, _floor: u8) -> bool {
        true
    }
}

/// PathProvider backed by a reference to PassabilityCache (for native Rust).
pub struct CachePathProvider<'a> {
    pub cache: &'a pathfinding::PassabilityCache,
}

impl<'a> PathProvider for CachePathProvider<'a> {
    fn find_path(
        &self,
        start_x: f32,
        start_z: f32,
        start_floor: u8,
        goal_x: f32,
        goal_z: f32,
        goal_floor: u8,
    ) -> PathResult {
        pathfinding::find_and_smooth_path(
            start_x,
            start_z,
            start_floor,
            goal_x,
            goal_z,
            goal_floor,
            self.cache,
            crate::dungeon::path_max_nodes(start_floor, goal_floor),
        )
    }

    fn attack_line_blocked(
        &self,
        from_x: f32,
        from_z: f32,
        to_x: f32,
        to_z: f32,
        floor: u8,
    ) -> bool {
        pathfinding::attack_line_blocked(self.cache, from_x, from_z, to_x, to_z, floor)
    }

    fn cell_passable(&self, x: f32, z: f32, floor: u8) -> bool {
        !pathfinding::is_cell_sealed(self.cache, x, z, floor, None)
    }
}
