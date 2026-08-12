use super::*;

impl SharedState {
    /// Characters on our own floor. Someone a floor above is a dot straight
    /// overhead, not a neighbour, so nothing the LLM sees or names should
    /// reach them.
    pub(super) fn players_on_my_floor(&self) -> impl Iterator<Item = (&PlayerId, &Player)> {
        self.nearby_players
            .iter()
            .filter(|(_, p)| p.floor_level == self.self_floor_level)
    }

    /// Monsters on our own floor — cross-floor ones read to the LLM as
    /// phantom respawns.
    pub(super) fn monsters_on_my_floor(&self) -> impl Iterator<Item = &Monster> {
        self.nearby_monsters
            .values()
            .filter(|m| m.floor_level == self.self_floor_level)
    }

    /// Emit a [Sighted] event when any point of interest — a monster, dropped
    /// loot, a dungeon entrance — enters NPC_SIGHT_RADIUS on our floor, and
    /// forget it once it drifts well past the edge so a re-entry announces
    /// again. Without this the agent walks straight past everything between
    /// scheduled turns. Only an aggressive monster wakes the driver; the rest
    /// ride to the next prompt so a long walk isn't cut every few metres.
    pub(super) fn check_sightings(&mut self) {
        let (self_pos, self_floor) = match self.self_player.as_ref() {
            Some(p) => (p.position, self.self_floor_level),
            None => return,
        };

        // (typed key, description, wakes_driver)
        let mut newly: Vec<(String, String, bool)> = Vec::new();
        // Keys still close enough to stay "seen" — a wider ring than the entry
        // radius so a POI hovering at the edge doesn't announce every tick.
        let mut nearby: HashSet<String> = HashSet::new();
        let forget_radius = NPC_SIGHT_RADIUS + 5.0;
        let sighted = &self.sighted_pois;
        // Ring bookkeeping shared by every POI kind; hands the key back only
        // when the POI just entered sight.
        let mut track = |key: String, dist: f32| -> Option<String> {
            let new = dist <= NPC_SIGHT_RADIUS && !sighted.contains(&key);
            if dist <= forget_radius {
                nearby.insert(key.clone());
            }
            new.then_some(key)
        };

        // Only aggressive monsters get a sighting event: they are the ones
        // worth waking the driver for, and CURRENT STATE already lists every
        // monster in sight — one event line per grazing mob would flood the
        // prompt in a dense spawn field.
        for (id, m) in &self.nearby_monsters {
            if m.floor_level != self_floor || m.state == MonsterState::Dead || !m.aggressive {
                continue;
            }
            let d = crate::geom::PlanarDelta::to_xz(&self_pos, m.position.x, m.position.z);
            if let Some(key) = track(format!("m:{id}"), d.dist) {
                newly.push((
                    key,
                    format!(
                        "[Sighted] {} [{id}] HP {}/{} — at ({:.0}, {:.0}), {:.0}m {}.",
                        m.monster_type,
                        m.health,
                        m.max_health,
                        m.position.x,
                        m.position.z,
                        d.dist,
                        compass(d.dx, d.dz),
                    ),
                    true,
                ));
            }
        }

        for (iid, item) in &self.ground_items {
            if item.floor_level != self_floor {
                continue;
            }
            // Our own drop: we put it there, announcing it is pure noise.
            if self.self_player_id.is_some() && item.dropped_by == self.self_player_id {
                continue;
            }
            let d = crate::geom::PlanarDelta::to_xz(&self_pos, item.position.x, item.position.z);
            if let Some(key) = track(format!("i:{iid}"), d.dist) {
                newly.push((
                    key,
                    format!(
                        "[Sighted] loot on the ground: {} [id {iid}] — at ({:.0}, {:.0}), {:.0}m {}.",
                        item.item_def_id,
                        item.position.x,
                        item.position.z,
                        d.dist,
                        compass(d.dx, d.dz),
                    ),
                    false,
                ));
            }
        }

        // Dungeon entrances only matter above ground.
        if self_floor >= 0 {
            let wc = self.world_cache.read().unwrap();
            for dg in wc.all_dungeons() {
                let d = crate::geom::PlanarDelta::to_xz(&self_pos, dg.entrance.x, dg.entrance.z);
                if let Some(key) = track(format!("d:{}", dg.name), d.dist) {
                    newly.push((
                        key,
                        format!(
                            "[Sighted] {} entrance ({} floors) — at ({:.0}, {:.0}), {:.0}m {}.",
                            dg.name,
                            dg.max_depth(),
                            dg.entrance.x,
                            dg.entrance.z,
                            d.dist,
                            compass(d.dx, d.dz)
                        ),
                        false,
                    ));
                }
            }
        }

        // Drop anything now well outside sight, so a re-entry announces again.
        self.sighted_pois.retain(|k| nearby.contains(k));

        for (key, note, wake) in newly {
            self.sighted_pois.insert(key);
            if wake {
                self.push_agent_event(note);
            } else {
                self.push_agent_event_quiet(note);
            }
        }
    }

    /// Remember an item on the ground.
    pub(crate) fn remember_ground_item(&mut self, item: GroundItem) {
        self.ground_items.insert(item.instance_id, item);
    }

    /// One ground item by instance id.
    pub fn ground_item(&self, instance_id: u64) -> Option<&GroundItem> {
        self.ground_items.get(&instance_id)
    }

    /// The ground items the agent can act on: on its floor, inside
    /// the sight radius, closest first. The known-item map reaches out to
    /// the server's event radius, so this is what "nearby" means everywhere
    /// downstream — the world state listing and pickup alike.
    pub fn ground_items_in_sight(&self) -> Vec<(f32, &GroundItem)> {
        let Some(sp) = self.self_player.as_ref() else {
            return Vec::new();
        };
        let sight_sq = NPC_SIGHT_RADIUS * NPC_SIGHT_RADIUS;
        let mut in_sight: Vec<_> = self
            .ground_items
            .values()
            .filter(|item| item.floor_level == self.self_floor_level)
            .filter_map(|item| {
                let d_sq = item.position.dist_xz_sq(&sp.position);
                (d_sq <= sight_sq).then_some((d_sq, item))
            })
            .collect();
        in_sight.sort_by(|a, b| a.0.total_cmp(&b.0));
        in_sight
    }
}
