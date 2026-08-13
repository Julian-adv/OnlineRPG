use super::*;

/// Fold a place name for comparison: case and inner spacing carry no meaning
/// ("orc warrens", "Orc Warrens" and "orc_warrens" are one place).
fn normalize_place(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

impl SharedState {
    /// Ask for the door state of the dungeon we stand in. Doors default shut
    /// locally, so without this we would path around one another player left
    /// open — and, worse, believe a route is sealed when it is not.
    pub fn request_dungeon_doors_here(&mut self) {
        let Some(dungeon) = self.dungeon_here() else {
            return;
        };
        self.pending_commands
            .push(ClientMessage::RequestDungeonDoors {
                entrance_id: dungeon.id.clone(),
            });
    }

    /// Dungeon whose footprint covers our position, if any.
    pub fn dungeon_here(&self) -> Option<Arc<Dungeon>> {
        let p = self.self_player.as_ref()?;
        self.world_cache
            .read()
            .unwrap()
            .dungeon_at(p.position.x, p.position.z)
    }

    /// Record an open we are about to send. A clutter prop is marked opened
    /// right away — the server answers a second open on one with silence, and
    /// an agent that cannot see the silence would retarget it forever. The
    /// mark is undone if the server rejects us.
    pub fn chest_open_sent(
        &mut self,
        entrance_id: &str,
        depth: u8,
        kind: crate::dungeon::ChestKind,
    ) {
        if let crate::dungeon::ChestKind::Prop(prop_id) = kind {
            self.world_cache
                .write()
                .unwrap()
                .add_dungeon_opened_prop(entrance_id, depth, prop_id);
        }
        self.pending_chest_open = Some((entrance_id.to_string(), depth, kind));
    }

    /// Whether we have already emptied this dungeon's treasure chest.
    pub fn treasure_chest_spent(&self, entrance_id: &str) -> bool {
        self.treasure_chests_spent.contains(entrance_id)
    }

    /// Chests standing in the room we occupy, nearest first — the treasure
    /// chest and the clutter chests together. Empty above ground, in a
    /// corridor, and once a chest has been opened.
    pub fn chests_in_sight(&self) -> Vec<crate::dungeon::ChestSighting> {
        let Some((pos, depth)) = self.underground_at() else {
            return Vec::new();
        };
        let world = self.world_cache.read().unwrap();
        let Some(dungeon) = world.dungeon_at(pos.x, pos.z) else {
            return Vec::new();
        };
        let empty = HashSet::new();
        let opened = world
            .opened_dungeon_props(&dungeon.id, depth)
            .unwrap_or(&empty);
        let floor = dungeon.passability_floor(depth);
        dungeon.chests_in_room_of(depth, &pos, opened, |c| world.is_walkable(c, floor))
    }

    /// Where we stand when we are underground in a dungeon, and how deep.
    /// `None` above ground — both in-room sighting queries start here.
    fn underground_at(&self) -> Option<(Position, u8)> {
        let p = self.self_player.as_ref()?;
        (self.self_floor_level < 0).then(|| (p.position, self.self_floor_level.unsigned_abs()))
    }

    /// Where we stand relative to the dungeons: the floor we are on when
    /// underground, or the nearest entrance when we are not. Monsters get
    /// stronger with depth, so the LLM needs both to decide whether to dive.
    pub(super) fn format_dungeon_state(&self) -> Option<String> {
        let p = self.self_player.as_ref()?;
        if self.self_floor_level < 0 {
            let depth = self.self_floor_level.unsigned_abs();
            let dungeon = self.dungeon_here();
            let name = dungeon
                .as_ref()
                .map(|d| format!("{} ", d.name))
                .unwrap_or_default();
            let mut line = format!(
                "You are underground: {name}floor {depth} (deeper floors hold stronger \
                 monsters; move with \"depth\" to change floors, 0 to leave)"
            );
            // Chests in our own room, described the way they render so the
            // agent can go for the one it wants. No coordinates — walking
            // over is the action's job.
            let spent = dungeon
                .as_ref()
                .is_some_and(|d| self.treasure_chest_spent(&d.id));
            for chest in self.chests_in_sight() {
                let (looks, note) = match chest.kind {
                    crate::dungeon::ChestKind::Treasure if spent => (
                        "a great chest standing alone",
                        " — you emptied it; it refills at nightfall",
                    ),
                    crate::dungeon::ChestKind::Treasure => ("a great chest standing alone", ""),
                    crate::dungeon::ChestKind::Prop(_) => ("a small chest among the clutter", ""),
                };
                let dist = crate::geom::PlanarDelta::between(&p.position, &chest.position).dist;
                line.push_str(&format!(
                    "\nChest in this room: {looks} ({dist:.0}m away){note}"
                ));
            }
            line.push_str(&self.format_room_props(p));
            // Floor map in world coordinates: without it the LLM aims moves
            // into solid rock and collects [MoveFailed] walls. Cell centres
            // sit on .5 — rounding them to whole metres names the next cell.
            if let Some(d) = dungeon.as_ref() {
                if let Some(layout) = d.layouts().get(depth as usize - 1) {
                    let me = world_to_cell(&d.entrance, p.position.x, p.position.z);
                    let rooms = layout
                        .rooms
                        .iter()
                        .enumerate()
                        .map(|(i, room)| {
                            let c = cell_center(&d.entrance, depth, room.center());
                            format!(
                                "room {} center ({:.1}, {:.1}) {}x{}m{}",
                                i + 1,
                                c.x,
                                c.z,
                                room.w,
                                room.d,
                                if room.contains(me.0, me.1) {
                                    " (you are here)"
                                } else {
                                    ""
                                }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" · ");
                    line.push_str(&format!("\nRooms on this floor: {rooms}"));
                    // Only the up shaft's exit row and the down shaft's entry
                    // row are landings on this floor; the rest of a shaft is
                    // blocked here, so its min corner is not a reachable goal.
                    let up = cell_center(&d.entrance, depth, layout.up_shaft.exit_cell());
                    line.push_str(&format!("\nStairs up at ({:.1}, {:.1})", up.x, up.z));
                    if let Some(down) = &layout.down_shaft {
                        let dn = cell_center(&d.entrance, depth, down.entry_cell());
                        line.push_str(&format!("; stairs down at ({:.1}, {:.1})", dn.x, dn.z));
                    }
                    line.push_str(
                        "\nEverything outside rooms and corridors is solid rock — aim \
                         moves at room centers or stairs.",
                    );
                }
            }
            return Some(line);
        }
        // Every entrance, nearest first, whatever the distance: a dungeon the
        // agent never sees named is one it can never ask to move to.
        let world = self.world_cache.read().unwrap();
        let mut named: Vec<(f32, String)> = world
            .all_dungeons()
            .iter()
            .map(|d| {
                let dist = crate::geom::PlanarDelta::between(&p.position, &d.entrance).dist;
                (
                    dist,
                    format!(
                        "Dungeon: {} entrance at ({:.0}, {:.0}), {dist:.0}m away, {} floors deep",
                        d.name,
                        d.entrance.x,
                        d.entrance.z,
                        d.max_depth()
                    ),
                )
            })
            .collect();
        if named.is_empty() {
            return None;
        }
        named.sort_by(|a, b| a.0.total_cmp(&b.0));
        Some(
            named
                .into_iter()
                .map(|(_, line)| line)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    /// Registered dungeon matching a name (or its registry id), however the
    /// LLM cased and spaced it.
    pub fn dungeon_named(&self, asked: &str) -> Option<Arc<Dungeon>> {
        let key = normalize_place(asked);
        self.world_cache
            .read()
            .unwrap()
            .all_dungeons()
            .iter()
            .find(|d| normalize_place(&d.name) == key || normalize_place(&d.id) == key)
            .map(Arc::clone)
    }

    /// Whether a dungeon prop has already been smashed.
    pub fn is_prop_broken(&self, id: &str, depth: u8, prop_id: u32) -> bool {
        self.world_cache
            .read()
            .unwrap()
            .dungeon_broken_props(id, depth)
            .contains(&prop_id)
    }

    /// Barrels and crates standing in the room we occupy, nearest first.
    /// Empty above ground, in a corridor, and once a prop has been smashed.
    /// Chest props are left out — they reach the agent as chests instead.
    pub fn breakables_in_sight(&self) -> Vec<crate::dungeon::BreakableSighting> {
        let Some((pos, depth)) = self.underground_at() else {
            return Vec::new();
        };
        let world = self.world_cache.read().unwrap();
        let Some(dungeon) = world.dungeon_at(pos.x, pos.z) else {
            return Vec::new();
        };
        let broken = world.dungeon_broken_props(&dungeon.id, depth);
        let floor = dungeon.passability_floor(depth);
        dungeon.breakables_in_room_of(depth, &pos, broken, |c| world.is_walkable(c, floor))
    }

    /// The breakable clutter in the agent's room, for the world state.
    fn format_room_props(&self, p: &Player) -> String {
        use onlinerpg_shared::dungeon::PropKind;
        let props = self.breakables_in_sight();
        if props.is_empty() {
            return String::new();
        }
        let list: Vec<String> = props
            .iter()
            .take(6)
            .map(|b| {
                let kind = match b.kind {
                    PropKind::Crate => "crate",
                    PropKind::Barrel | PropKind::Chest | PropKind::TorchWall => "barrel",
                };
                let dist = crate::geom::PlanarDelta::between(&p.position, &b.position).dist;
                format!("{kind} [prop {}] {dist:.0}m away", b.prop_id)
            })
            .collect();
        format!(
            "\nBreakable props in this room: {} — {{\"type\": \"break_prop\", \"prop_id\": N}} \
             smashes one open.",
            list.join("; ")
        )
    }
}
