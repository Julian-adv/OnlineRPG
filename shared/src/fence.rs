use crate::pathfinding::{PassabilityCache, RuntimeFloorGrid, RuntimePassability};
use crate::{wrap_world_x, Position, WORLD_MAX_X, WORLD_MIN_X};
use serde::{Deserialize, Serialize};

pub const ITEM_ID: &str = "wooden_fence";
pub const HEIGHT: f32 = 0.914_062;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FenceAxis {
    X,
    Z,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FenceEdge {
    pub x: i32,
    pub z: i32,
    pub axis: FenceAxis,
}

impl FenceEdge {
    pub fn valid(self) -> bool {
        (WORLD_MIN_X as i32..WORLD_MAX_X as i32).contains(&self.x)
            && (WORLD_MIN_X as i32..WORLD_MAX_X as i32).contains(&self.z)
    }

    pub fn center(self, y: f32) -> Position {
        Position {
            x: self.x as f32 + if self.axis == FenceAxis::X { 0.5 } else { 0.0 },
            y,
            z: self.z as f32 + if self.axis == FenceAxis::Z { 0.5 } else { 0.0 },
        }
    }

    pub fn adjacent_centers(self) -> [Position; 2] {
        let mut a = self.center(0.0);
        let mut b = a;
        match self.axis {
            FenceAxis::X => {
                a.z -= 0.5;
                b.z += 0.5;
            }
            FenceAxis::Z => {
                a.x = wrap_world_x(a.x - 0.5);
                b.x += 0.5;
            }
        }
        [a, b]
    }

    pub fn cache_key(self) -> String {
        format!("fences:{},{}", self.x.div_euclid(32), self.z.div_euclid(32))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fence {
    pub edge: FenceEdge,
    pub y: f32,
    pub owner_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FencePlot {
    pub x: i32,
    pub z: i32,
}

pub fn sync_passability(cache: &mut PassabilityCache, key: &str, fences: &[Fence]) {
    if fences.is_empty() {
        cache.remove(key);
        return;
    }
    let mut floors = Vec::with_capacity(fences.len());
    let (mut min_x, mut max_x, mut min_z, mut max_z) = (
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
    );
    for fence in fences {
        let edge = fence.edge;
        let (x, z, width, depth, cells) = match edge.axis {
            FenceAxis::X => (edge.x, edge.z - 1, 1, 2, vec![4, 1]),
            FenceAxis::Z => (edge.x - 1, edge.z, 2, 1, vec![2, 8]),
        };
        min_x = min_x.min(x as f32);
        max_x = max_x.max((x + width) as f32);
        min_z = min_z.min(z as f32);
        max_z = max_z.max((z + depth) as f32);
        floors.push(RuntimeFloorGrid {
            floor_level: 0,
            origin_x: x,
            origin_z: z,
            width: width as u8,
            depth: depth as u8,
            y_base: fence.y,
            wall_height: HEIGHT,
            cells,
        });
    }
    cache.insert(
        key.to_string(),
        RuntimePassability {
            house_origin_x: 0.0,
            house_origin_z: 0.0,
            min_x,
            max_x,
            min_z,
            max_z,
            floors,
            stairwells: vec![],
            yields_to_trapped_mover: false,
            is_ground: false,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pathfinding::{is_cardinal_move_blocked, is_movement_blocked};

    #[test]
    fn fence_blocks_only_its_edge_in_both_directions() {
        for axis in [FenceAxis::X, FenceAxis::Z] {
            let edge = FenceEdge { x: -2, z: -3, axis };
            let mut cache = PassabilityCache::new();
            sync_passability(
                &mut cache,
                "fences",
                &[Fence {
                    edge,
                    y: 0.0,
                    owner_id: 1,
                }],
            );
            let [a, b] = edge.adjacent_centers();
            for (from, to) in [(a, b), (b, a)] {
                assert!(is_movement_blocked(
                    &cache,
                    from.x,
                    from.z,
                    to.x,
                    to.z,
                    0,
                    Some(0.05)
                ));
                assert!(!is_movement_blocked(
                    &cache,
                    from.x,
                    from.z,
                    to.x,
                    to.z,
                    1,
                    Some(3.0)
                ));
                assert!(is_cardinal_move_blocked(
                    &cache,
                    from.x.floor() as i32,
                    from.z.floor() as i32,
                    (to.x - from.x) as i32,
                    (to.z - from.z) as i32,
                    0
                ));
            }
            let (dx, dz) = if axis == FenceAxis::X { (1, 0) } else { (0, 1) };
            assert!(!is_cardinal_move_blocked(
                &cache,
                a.x.floor() as i32,
                a.z.floor() as i32,
                dx,
                dz,
                0
            ));
            sync_passability(&mut cache, "fences", &[]);
            assert!(!is_movement_blocked(
                &cache,
                a.x,
                a.z,
                b.x,
                b.z,
                0,
                Some(0.05)
            ));
        }
    }

    #[test]
    fn removing_fence_keeps_other_obstacles() {
        let mut cache = PassabilityCache::new();
        let fence = Fence {
            edge: FenceEdge {
                x: 0,
                z: 0,
                axis: FenceAxis::X,
            },
            y: 0.0,
            owner_id: 1,
        };
        sync_passability(&mut cache, "other", std::slice::from_ref(&fence));
        sync_passability(&mut cache, "fences", &[fence]);
        sync_passability(&mut cache, "fences", &[]);
        assert!(is_movement_blocked(
            &cache,
            0.5,
            -0.5,
            0.5,
            0.5,
            0,
            Some(0.05)
        ));
    }
}
