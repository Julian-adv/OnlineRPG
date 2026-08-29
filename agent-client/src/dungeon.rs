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
    ground_y_for_floor, interior_doors, passability_floor_for_depth, world_to_cell,
    DungeonEntranceDef, FloorLayout, InteriorDoorSpec, PropKind, Room,
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

/// Which chest a sighting is. The two render differently, so the agent is
/// told which it is looking at, the same as a player can see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChestKind {
    /// The boss-guarded chest on the deepest floor (`OpenDungeonChest`).
    Treasure,
    /// A clutter chest, by its index into the floor's props
    /// (`OpenDungeonProp`).
    Prop(u32),
}

/// A chest standing in the same room as the agent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChestSighting {
    pub kind: ChestKind,
    /// The chest itself — what the distance shown to the agent measures to.
    pub position: Position,
    /// Where to stand to open it. The treasure chest carries no collision, so
    /// it is the chest's own cell; a clutter prop is a 1×1 pillar A* can never
    /// route onto, so it is the walkable cell beside it.
    pub approach: Position,
}

/// A breakable barrel or crate standing in the same room as the agent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BreakableSighting {
    /// Index into the floor's props — what `BreakDungeonProp` names.
    pub prop_id: u32,
    pub kind: PropKind,
    /// The prop itself; the server measures its interact range from here.
    pub position: Position,
    /// The walkable cell beside it to stand in.
    pub approach: Position,
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

    /// Every chest someone at `pos` on `depth` can see: the ones sharing their
    /// room, nearest first. Empty from a corridor or a stair landing outside
    /// every room, and empty from the next room over however close the chest
    /// is — walls hide it, as they hide it from a player at the web client.
    ///
    /// `opened` holds the prop indices already claimed on this floor.
    /// `walkable` decides where the agent can stand to open a prop; pass the
    /// live passability, not the layout, so broken props and shut doors count.
    pub fn chests_in_room_of(
        &self,
        depth: u8,
        pos: &Position,
        opened: &HashSet<u32>,
        walkable: impl Fn(&Position) -> bool,
    ) -> Vec<ChestSighting> {
        let Some((layout, room)) = self.room_of(depth, pos) else {
            return Vec::new();
        };
        let mut seen: Vec<ChestSighting> = layout
            .chest
            .filter(|c| room.contains(c.0, c.1))
            .map(|c| (ChestKind::Treasure, c))
            .into_iter()
            .chain(
                layout
                    .props
                    .iter()
                    .enumerate()
                    .filter(|(i, p)| {
                        matches!(p.kind, PropKind::Chest)
                            && !opened.contains(&(*i as u32))
                            && room.contains(p.x, p.z)
                    })
                    .map(|(i, p)| (ChestKind::Prop(i as u32), (p.x, p.z))),
            )
            .filter_map(|(kind, cell)| {
                let position = cell_center(&self.entrance, depth, cell);
                let approach = self.approach_cell(layout, depth, cell, pos, &walkable)?;
                Some(ChestSighting {
                    kind,
                    position,
                    approach,
                })
            })
            .collect();
        let dist = |c: &ChestSighting| crate::geom::PlanarDelta::between(pos, &c.position).dist;
        seen.sort_by(|a, b| dist(a).total_cmp(&dist(b)));
        seen
    }

    /// Every barrel or crate someone at `pos` on `depth` can see, nearest
    /// first — the same room rule as [`Self::chests_in_room_of`]. Walls hide
    /// clutter from the agent as they do from a web player, and the walk-to
    /// loop's leash assumes a target inside our own room.
    ///
    /// `broken` holds the prop indices already smashed on this floor.
    pub fn breakables_in_room_of(
        &self,
        depth: u8,
        pos: &Position,
        broken: &[u32],
        walkable: impl Fn(&Position) -> bool,
    ) -> Vec<BreakableSighting> {
        let Some((layout, room)) = self.room_of(depth, pos) else {
            return Vec::new();
        };
        let mut seen: Vec<BreakableSighting> = layout
            .props
            .iter()
            .enumerate()
            .filter(|(i, p)| {
                matches!(p.kind, PropKind::Barrel | PropKind::Crate)
                    && !broken.contains(&(*i as u32))
                    && room.contains(p.x, p.z)
            })
            .filter_map(|(i, p)| {
                Some(BreakableSighting {
                    prop_id: i as u32,
                    kind: p.kind,
                    position: cell_center(&self.entrance, depth, (p.x, p.z)),
                    approach: self.approach_cell(layout, depth, (p.x, p.z), pos, &walkable)?,
                })
            })
            .collect();
        let dist = |b: &BreakableSighting| crate::geom::PlanarDelta::between(pos, &b.position).dist;
        seen.sort_by(|a, b| dist(a).total_cmp(&dist(b)));
        seen
    }

    /// The floor layout and the room `pos` stands in — where every in-room
    /// sighting starts. `None` from a corridor or a landing outside any room.
    fn room_of(&self, depth: u8, pos: &Position) -> Option<(&FloorLayout, &Room)> {
        let layout = self.layout_at(depth)?;
        let (px, pz) = world_to_cell(&self.entrance, pos.x, pos.z);
        Some((layout, layout.room_at(px, pz)?))
    }

    /// Chests and props are 1×1 collision pillars, so A* can never route onto
    /// one: stand in the open cell beside it, the one nearest `pos`.
    fn approach_cell(
        &self,
        layout: &FloorLayout,
        depth: u8,
        cell: (i32, i32),
        pos: &Position,
        walkable: &impl Fn(&Position) -> bool,
    ) -> Option<Position> {
        layout
            .approach_cells(cell)
            .map(|c| cell_center(&self.entrance, depth, c))
            .filter(|p| walkable(p))
            .min_by(|a, b| {
                let d = |p: &Position| crate::geom::PlanarDelta::between(pos, p).dist;
                d(a).total_cmp(&d(b))
            })
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
pub(crate) mod tests {
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

    /// The deepest floor of old_crypt: its treasure chest cell, the room that
    /// holds it, and a spot at that room's center to look from.
    /// A dungeon whose chest room holds a clutter chest and a breakable.
    /// Which one that is depends on the generated layouts; the tests that
    /// need both look it up rather than assuming old_crypt has them.
    pub(crate) fn cluttered_chest_room_dungeon() -> Dungeon {
        build_all()
            .into_iter()
            .find(|d| {
                let last = d.layouts().last().unwrap();
                let cell = last.chest.unwrap();
                let room = last.room_at(cell.0, cell.1).unwrap();
                let kinds: Vec<PropKind> = last
                    .props
                    .iter()
                    .filter(|p| room.contains(p.x, p.z))
                    .map(|p| p.kind)
                    .collect();
                kinds.contains(&PropKind::Chest)
                    && kinds
                        .iter()
                        .any(|k| matches!(k, PropKind::Barrel | PropKind::Crate))
            })
            .expect("a registry dungeon with clutter in its chest room")
    }

    fn chest_room(d: &Dungeon) -> (u8, (i32, i32), Room, Position) {
        let depth = d.max_depth();
        let layout = d.layouts().last().expect("old_crypt has floors");
        let cell = layout.chest.expect("final floor has a chest");
        let room = *layout
            .room_at(cell.0, cell.1)
            .expect("the chest is in a room");
        (
            depth,
            cell,
            room,
            cell_center(&d.entrance, depth, room.center()),
        )
    }

    /// The live grid seals props and shut doors; tests that only care about
    /// sighting take the layout's word for what is floor.
    fn carved_only(d: &Dungeon, depth: u8) -> impl Fn(&Position) -> bool + '_ {
        move |p: &Position| {
            let (x, z) = world_to_cell(&d.entrance, p.x, p.z);
            let props_here = d
                .layout_at(depth)
                .map(|l| l.props.iter().any(|pr| (pr.x, pr.z) == (x, z)))
                .unwrap_or(false);
            d.layout_at(depth).is_some_and(|l| l.is_carved(x, z)) && !props_here
        }
    }

    /// Breakable clutter follows the same room rule as the chests: the walk-to
    /// loop's leash only stretches across one room, so listing a barrel two
    /// rooms away would only ever hand the agent an action it cannot finish.
    #[test]
    fn breakables_are_only_visible_from_inside_their_room() {
        let d = crypt();
        let depth = d.max_depth();
        let layout = d.layouts().last().expect("old_crypt has floors");
        let (id, prop) = layout
            .props
            .iter()
            .enumerate()
            .find(|(_, p)| matches!(p.kind, PropKind::Barrel | PropKind::Crate))
            .expect("the final floor has breakable clutter");
        let room = *layout
            .room_at(prop.x, prop.z)
            .expect("clutter is scattered through the rooms");
        let inside = cell_center(&d.entrance, depth, room.center());

        let here = d.breakables_in_room_of(depth, &inside, &[], carved_only(&d, depth));
        assert!(here.iter().any(|b| b.prop_id == id as u32));
        let dists: Vec<f32> = here
            .iter()
            .map(|b| crate::geom::PlanarDelta::between(&inside, &b.position).dist)
            .collect();
        assert!(
            dists.windows(2).all(|w| w[0] <= w[1]),
            "sightings come back nearest first"
        );

        let smashed = d.breakables_in_room_of(depth, &inside, &[id as u32], carved_only(&d, depth));
        assert!(!smashed.iter().any(|b| b.prop_id == id as u32));

        let elsewhere = layout
            .rooms
            .iter()
            .find(|r| r.x != room.x || r.z != room.z)
            .expect("the final floor has more than one room");
        let other_room = cell_center(&d.entrance, depth, elsewhere.center());
        assert!(!d
            .breakables_in_room_of(depth, &other_room, &[], carved_only(&d, depth))
            .iter()
            .any(|b| b.prop_id == id as u32));
    }

    /// A chest shows itself to someone in its own room and to nobody else —
    /// the agent has to walk the floor to find it, like a web player does.
    #[test]
    fn chests_are_only_visible_from_inside_their_room() {
        let d = crypt();
        let (depth, cell, room, _) = chest_room(&d);
        let chest_pos = cell_center(&d.entrance, depth, cell);
        let none = HashSet::new();

        let here = d.chests_in_room_of(depth, &chest_pos, &none, carved_only(&d, depth));
        assert_eq!(here.first().map(|c| c.kind), Some(ChestKind::Treasure));
        assert_eq!(here[0].position, chest_pos);

        let layout = d.layouts().last().unwrap();
        let elsewhere = layout
            .rooms
            .iter()
            .find(|r| r.x != room.x || r.z != room.z)
            .expect("the final floor has more than one room");
        let other_room = cell_center(&d.entrance, depth, elsewhere.center());
        assert!(!d
            .chests_in_room_of(depth, &other_room, &none, carved_only(&d, depth))
            .iter()
            .any(|c| c.kind == ChestKind::Treasure));
    }

    /// Clutter chests come back alongside the treasure chest, nearest first,
    /// and drop out of sight once opened.
    #[test]
    fn clutter_chests_are_sighted_like_the_treasure_and_hidden_once_opened() {
        let d = cluttered_chest_room_dungeon();
        let (depth, _, room, stand) = chest_room(&d);
        let prop = d
            .layouts()
            .last()
            .unwrap()
            .props
            .iter()
            .enumerate()
            .find(|(_, p)| matches!(p.kind, PropKind::Chest) && room.contains(p.x, p.z))
            .map(|(i, _)| i as u32)
            .expect("the chest room holds a clutter chest");

        let sighted = d.chests_in_room_of(depth, &stand, &HashSet::new(), carved_only(&d, depth));
        assert!(sighted.iter().any(|c| c.kind == ChestKind::Prop(prop)));
        assert!(sighted.iter().any(|c| c.kind == ChestKind::Treasure));
        let mut dists = sighted
            .iter()
            .map(|c| crate::geom::PlanarDelta::between(&stand, &c.position).dist);
        let first = dists.next().unwrap();
        assert!(dists.all(|d| d >= first), "sightings are nearest first");

        let opened = HashSet::from([prop]);
        assert!(!d
            .chests_in_room_of(depth, &stand, &opened, carved_only(&d, depth))
            .iter()
            .any(|c| c.kind == ChestKind::Prop(prop)));
    }

    /// Every chest is a 1×1 collision pillar, so it is opened from a cell
    /// beside it — pathing at the chest's own cell never arrives.
    #[test]
    fn chests_are_approached_from_a_walkable_neighbour() {
        let d = crypt();
        let (depth, _, _, stand) = chest_room(&d);
        let walkable = carved_only(&d, depth);

        for sighting in d.chests_in_room_of(depth, &stand, &HashSet::new(), carved_only(&d, depth))
        {
            assert!(
                walkable(&sighting.approach),
                "{:?} is approached from an unwalkable cell",
                sighting.kind
            );
            let gap =
                crate::geom::PlanarDelta::between(&sighting.approach, &sighting.position).dist;
            assert!(
                gap > 0.0 && gap <= 1.5,
                "{:?} is approached from {gap}m away",
                sighting.kind
            );
        }
    }
}
