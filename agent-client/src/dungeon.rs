//! Dungeon navigation for the agent: the entrance registry, the generated
//! layouts, and the stair-shaft geometry the mover needs to walk between
//! floors. Port of the browser's `dungeonManager.ts` — layouts and passability
//! come from the shared crate (the same code the server runs), so only the Y
//! model and the floor bookkeeping live here.
//!
//! The agent sends its own Y, and the server derives the floor it collides
//! against from that Y (`get_floor_at_position`), so every step inside a
//! dungeon must carry the height this module computes. A move that keeps the
//! surface Y is refused the moment it reaches a wall that only exists
//! underground.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use onlinerpg_shared::dungeon::{
    cell_center, closed_door_segs, dungeon_origin, dungeon_passability,
    floor_level_for_passability, floor_passability_cells_full, floor_world_y, generate_dungeon_for,
    interior_doors, passability_floor_for_depth, FloorLayout, InteriorDoorSpec, StairShaft, GRID,
    SHAFT_LEN, SHAFT_W,
};
use onlinerpg_shared::pathfinding::RuntimePassability;
use onlinerpg_shared::Position;
use serde::Deserialize;

/// Flat landing length (in cells) at each end of a stair ramp.
const LANDING_CELLS: f32 = 1.0;

/// One entry of `data/dungeons.json`. Only the placement fields matter here;
/// chest drops are server-side.
#[derive(Debug, Clone, Deserialize)]
struct DungeonEntranceDef {
    id: String,
    name: String,
    x: f32,
    y: f32,
    z: f32,
}

/// Entrance registry, embedded from the same `data/dungeons.json` the server
/// and the browser embed — entrances never travel over the wire.
static ENTRANCES: LazyLock<Vec<DungeonEntranceDef>> = LazyLock::new(|| {
    let defs: HashMap<String, DungeonEntranceDef> =
        serde_json::from_str(include_str!("../../data/dungeons.json"))
            .expect("failed to parse data/dungeons.json");
    let mut defs: Vec<_> = defs.into_values().collect();
    defs.sort_by(|a, b| a.id.cmp(&b.id));
    defs
});

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
            entrance: Position {
                x: def.x,
                y: def.y,
                z: def.z,
            },
            layouts: generate_dungeon_for(&def.id),
        }
    }

    /// Deepest floor of this dungeon (1-based).
    pub fn max_depth(&self) -> u8 {
        self.layouts.len() as u8
    }

    /// Whether (x, z) lies inside the dungeon's grid footprint.
    pub fn footprint_contains(&self, x: f32, z: f32) -> bool {
        let (ox, oz) = dungeon_origin(self.entrance.x, self.entrance.z);
        x >= ox && x < ox + GRID as f32 && z >= oz && z < oz + GRID as f32
    }

    fn layout_at(&self, depth: u8) -> Option<&FloorLayout> {
        self.layouts.get(depth.checked_sub(1)? as usize)
    }

    pub fn passability(&self) -> RuntimePassability {
        dungeon_passability(&self.entrance, &self.layouts)
    }

    /// Rebuilt cells for one floor under the live door/prop state.
    pub fn floor_cells(
        &self,
        depth: u8,
        open_doors: &HashSet<u32>,
        broken: &[u32],
    ) -> Option<Vec<u8>> {
        let layout = self.layout_at(depth)?;
        Some(floor_passability_cells_full(
            layout,
            broken,
            &closed_door_segs(layout, Some(open_doors)),
        ))
    }

    pub fn floor_y(&self, depth: u8) -> f32 {
        floor_world_y(self.entrance.y, depth)
    }

    /// Run position along a shaft in `[0, SHAFT_LEN)` measured from the entry
    /// (shallow) end, or `None` off the shaft footprint.
    fn shaft_run_pos(&self, shaft: &StairShaft, x: f32, z: f32) -> Option<f32> {
        let (ox, oz) = dungeon_origin(self.entrance.x, self.entrance.z);
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
    fn shaft_high_y(&self, depth: u8) -> f32 {
        if depth <= 1 {
            self.entrance.y
        } else {
            self.floor_y(depth - 1)
        }
    }

    /// Ground height on `depth`, stair ramps included.
    pub fn floor_height_at(&self, depth: u8, x: f32, z: f32) -> Option<f32> {
        let layout = self.layout_at(depth)?;
        if let Some(t) = self.shaft_run_pos(&layout.up_shaft, x, z) {
            return Some(Self::ramp_y(
                self.shaft_high_y(depth),
                self.floor_y(depth),
                t,
            ));
        }
        if let Some(down) = layout.down_shaft.as_ref() {
            if let Some(t) = self.shaft_run_pos(down, x, z) {
                return Some(Self::ramp_y(
                    self.floor_y(depth),
                    self.floor_y(depth + 1),
                    t,
                ));
            }
        }
        Some(self.floor_y(depth))
    }

    /// Surface entrance ramp height, or `None` when (x, z) is off the shaft —
    /// out there the terrain sampler owns Y.
    pub fn entrance_ramp_height_at(&self, x: f32, z: f32) -> Option<f32> {
        let first = self.layouts.first()?;
        let t = self.shaft_run_pos(&first.up_shaft, x, z)?;
        Some(Self::ramp_y(self.entrance.y, self.floor_y(1), t))
    }

    /// Ground height for a step keyed to the passability floor `floor`.
    pub fn ground_y(&self, floor: u8, x: f32, z: f32) -> Option<f32> {
        match floor_level_for_passability(floor) {
            depth if depth < 0 => self.floor_height_at(depth.unsigned_abs(), x, z),
            0 => self.entrance_ramp_height_at(x, z),
            _ => None,
        }
    }

    /// A* start floor for a position on `depth`. The stairwell model keys a
    /// shaft's intermediate steps to the *shallower* connected floor, so a
    /// search started mid-shaft under the deeper key can only enter the shaft
    /// through its bottom landing — it walks the agent back down the stairs
    /// before climbing out. The exit landing genuinely belongs to `depth`.
    pub fn pathfinding_floor_at(&self, depth: u8, x: f32, z: f32) -> u8 {
        let default = passability_floor_for_depth(depth);
        let Some(layout) = self.layout_at(depth) else {
            return default;
        };
        match self.shaft_run_pos(&layout.up_shaft, x, z) {
            Some(t) if t < (SHAFT_LEN - 1) as f32 => {
                if depth <= 1 {
                    0
                } else {
                    passability_floor_for_depth(depth - 1)
                }
            }
            _ => default,
        }
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
    ENTRANCES.iter().map(Dungeon::new).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use onlinerpg_shared::dungeon::DUNGEON_FLOOR_HEIGHT;

    fn crypt() -> Dungeon {
        build_all()
            .into_iter()
            .find(|d| d.id == "old_crypt")
            .expect("old_crypt is in data/dungeons.json")
    }

    #[test]
    fn ramp_descends_one_storey_between_landings() {
        let d = crypt();
        let top = d.entrance.y;
        assert_eq!(Dungeon::ramp_y(top, top - 4.0, 0.0), top);
        assert_eq!(Dungeon::ramp_y(top, top - 4.0, 1.0), top);
        assert_eq!(
            Dungeon::ramp_y(top, top - 4.0, SHAFT_LEN as f32 - 1.0),
            top - 4.0
        );
        assert_eq!(Dungeon::ramp_y(top, top - 4.0, 4.0), top - 2.0);
    }

    /// Standing anywhere on depth 1 that isn't a shaft must report that
    /// floor's world Y, or the server would collide us against the surface.
    #[test]
    fn floor_height_off_the_shaft_is_the_floor_y() {
        let d = crypt();
        let landing = d.arrival_position(1).unwrap();
        assert_eq!(landing.y, d.entrance.y - DUNGEON_FLOOR_HEIGHT);
        let y = d.floor_height_at(1, landing.x, landing.z).unwrap();
        assert!((y - landing.y).abs() < 0.01, "{y} vs {}", landing.y);
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
