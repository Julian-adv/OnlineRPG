//! Dungeon navigation for the agent: the generated layouts plus the geometry
//! queries the mover asks of them. The entrance registry, the stair-shaft Y
//! model and the passability rebuild all live in the shared crate — the same
//! code the server runs and the browser calls through wasm — so nothing here
//! can drift from them.
//!
//! The agent sends its own Y, and the server derives the floor it collides
//! against from that Y (`get_floor_at_position`), so every step inside a
//! dungeon must carry the height the shared model computes. A move that keeps
//! the surface Y is refused the moment it reaches a wall that only exists
//! underground.

use std::collections::HashSet;

#[cfg(test)]
use onlinerpg_shared::dungeon::floor_world_y;
use onlinerpg_shared::dungeon::{
    cell_center, dungeon_origin, dungeon_passability, entrances, generate_dungeon_for,
    ground_y_for_floor, interior_doors, passability_floor_for_depth, DungeonEntranceDef,
    FloorLayout, InteriorDoorSpec,
};
use onlinerpg_shared::pathfinding::RuntimePassability;
use onlinerpg_shared::Position;

/// A generated dungeon plus the geometry queries the mover asks of it.
pub struct Dungeon {
    pub id: String,
    pub name: String,
    pub entrance: Position,
    layouts: Vec<FloorLayout>,
}

/// A shut interior door standing between the agent and where it wants to go,
/// with the two cell centers straddling it — one of them is on our side.
pub struct DoorApproach {
    pub door_id: u32,
    pub sides: [(f32, f32); 2],
}

impl Dungeon {
    fn new(def: &DungeonEntranceDef) -> Self {
        Self {
            id: def.id.clone(),
            name: def.name.clone(),
            entrance: def.position(),
            layouts: generate_dungeon_for(&def.id),
        }
    }

    /// Deepest floor of this dungeon (1-based).
    pub fn max_depth(&self) -> u8 {
        self.layouts.len() as u8
    }

    pub fn layouts(&self) -> &[FloorLayout] {
        &self.layouts
    }

    /// Whether (x, z) lies inside the dungeon's grid footprint.
    pub fn footprint_contains(&self, x: f32, z: f32) -> bool {
        onlinerpg_shared::dungeon::footprint_contains(self.entrance.x, self.entrance.z, x, z)
    }

    fn layout_at(&self, depth: u8) -> Option<&FloorLayout> {
        self.layouts.get(depth.checked_sub(1)? as usize)
    }

    pub fn passability(&self) -> RuntimePassability {
        dungeon_passability(&self.entrance, &self.layouts)
    }

    /// World Y of a floor. Test-only: movers go through [`Self::ground_y`],
    /// which also covers the stair ramps.
    #[cfg(test)]
    pub fn floor_y(&self, depth: u8) -> f32 {
        floor_world_y(self.entrance.y, depth)
    }

    /// Ground height for a step keyed to the passability floor `floor`.
    pub fn ground_y(&self, floor: u8, x: f32, z: f32) -> Option<f32> {
        ground_y_for_floor(&self.entrance, &self.layouts, floor, x, z)
    }

    /// Where a descent to `depth` lands: the exit landing of the shaft that
    /// arrives there. Always carved, so it is a safe goal for "go to floor N".
    pub fn arrival_position(&self, depth: u8) -> Option<Position> {
        let layout = self.layout_at(depth)?;
        Some(cell_center(
            &self.entrance,
            depth,
            layout.up_shaft.exit_cell(),
        ))
    }

    /// World XZ of the cell pair a door segment separates, at its midpoint.
    fn door_sides(&self, door: &InteriorDoorSpec) -> [(f32, f32); 2] {
        let (ox, oz) = dungeon_origin(self.entrance.x, self.entrance.z);
        let lat = door.lat0 as f32 + door.len as f32 / 2.0;
        let line = door.wall_line as f32;
        if door.spans_x() {
            [(ox + lat, oz + line - 0.5), (ox + lat, oz + line + 0.5)]
        } else {
            [(ox + line - 0.5, oz + lat), (ox + line + 0.5, oz + lat)]
        }
    }

    /// Every shut interior door on `depth`, with the cells on either side.
    /// A dungeon's stairs down usually sit behind one, so a blocked route is
    /// the mover's cue to walk to the nearest of these and open it.
    pub fn closed_doors(&self, depth: u8, open: &HashSet<u32>) -> Vec<DoorApproach> {
        let Some(layout) = self.layout_at(depth) else {
            return Vec::new();
        };
        interior_doors(layout)
            .iter()
            .filter(|d| !open.contains(&d.door_id))
            .map(|d| DoorApproach {
                door_id: d.door_id,
                sides: self.door_sides(d),
            })
            .collect()
    }

    pub fn passability_floor(&self, depth: u8) -> u8 {
        passability_floor_for_depth(depth)
    }
}

/// Generate every registry dungeon. Cheap enough (one 80×80 grid per floor)
/// to build once at startup and share across all agent connections, which is
/// also what the server does with its own copy.
pub fn build_all() -> Vec<Dungeon> {
    entrances().iter().map(Dungeon::new).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crypt() -> Dungeon {
        build_all()
            .into_iter()
            .find(|d| d.id == "old_crypt")
            .expect("old_crypt is in the shared entrance registry")
    }

    /// Every floor above the last has stairs down, and they start shut behind
    /// interior doors on most floors — the mover has to open them to descend.
    #[test]
    fn shafts_and_doors_exist_to_descend() {
        let d = crypt();
        assert!(d.max_depth() >= 5);
        for depth in 1..d.max_depth() {
            assert!(
                d.arrival_position(depth + 1).is_some(),
                "depth {depth} has no shaft down"
            );
        }
        let closed: usize = (1..=d.max_depth())
            .map(|depth| d.closed_doors(depth, &HashSet::new()).len())
            .sum();
        assert!(closed > 0, "expected shut interior doors in old_crypt");
    }
}
