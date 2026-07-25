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
}
