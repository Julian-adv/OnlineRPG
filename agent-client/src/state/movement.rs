use super::*;

/// A resolved `move` target.
#[derive(Debug, Clone, PartialEq)]
pub enum MoveTarget {
    Character { id: PlayerId, name: String },
    Monster { id: String },
    GroundItem { instance_id: u64, name: String },
    Prop { prop_id: u32 },
    Chest { selector: String },
    Dungeon { id: String, name: String },
}

/// Why a `move` target did not resolve.
#[derive(Debug, Clone, PartialEq)]
pub enum MoveTargetError {
    /// A monster species where an id belongs, with the ids that match and
    /// how far away each one is.
    SpeciesNotId {
        species: String,
        candidates: Vec<(String, f32)>,
    },
    /// A well-formed monster id that is no longer in sight.
    MonsterGone { id: String },
    Unknown {
        asked: String,
        addressable: Vec<String>,
    },
}

/// Whether a string has the shape of a monster id (`m2_1`), which is how the
/// ladder tells "the goblin called m2_1" from "a goblin".
fn looks_like_monster_id(s: &str) -> bool {
    let Some(rest) = s.strip_prefix(['m', 'M']) else {
        return false;
    };
    let Some((floor, index)) = rest.split_once('_') else {
        return false;
    };
    !floor.is_empty()
        && !index.is_empty()
        && floor.bytes().all(|b| b.is_ascii_digit())
        && index.bytes().all(|b| b.is_ascii_digit())
}

impl SharedState {
    /// Resolve a visible `move` target by id shape, then by exact name.
    pub fn resolve_move_target(&self, raw: &str) -> Result<MoveTarget, MoveTargetError> {
        let asked = raw.trim();

        if looks_like_monster_id(asked) {
            return match self
                .monsters_on_my_floor()
                .find(|m| m.id.eq_ignore_ascii_case(asked))
            {
                Some(m) => Ok(MoveTarget::Monster { id: m.id.clone() }),
                None => Err(MoveTargetError::MonsterGone {
                    id: asked.to_string(),
                }),
            };
        }

        if let Ok(n) = asked.parse::<u64>() {
            if let Some((_, item)) = self
                .ground_items_in_sight()
                .iter()
                .find(|(_, i)| i.instance_id == n)
            {
                return Ok(MoveTarget::GroundItem {
                    instance_id: item.instance_id,
                    name: item.item_def_id.clone(),
                });
            }
            // Players before props: their id ranges overlap (prop ids are
            // room indexes from 0, player ids count from 1), and arrival
            // events teach the model numeric character ids.
            if let Some((id, p)) = self.players_on_my_floor().find(|(id, _)| id.get() == n) {
                return Ok(MoveTarget::Character {
                    id: *id,
                    name: p.name.clone(),
                });
            }
            if let Some(b) = self
                .breakables_in_sight()
                .iter()
                .find(|b| u64::from(b.prop_id) == n)
            {
                return Ok(MoveTarget::Prop { prop_id: b.prop_id });
            }
            return Err(self.unknown_target(asked));
        }

        if let Some((id, p)) = self
            .players_on_my_floor()
            .find(|(_, p)| p.name.eq_ignore_ascii_case(asked))
        {
            return Ok(MoveTarget::Character {
                id: *id,
                name: p.name.clone(),
            });
        }

        if let Some(d) = self.dungeon_named(asked) {
            return Ok(MoveTarget::Dungeon {
                id: d.id.clone(),
                name: d.name.clone(),
            });
        }

        if asked.to_lowercase().contains("chest") && !self.chests_in_sight().is_empty() {
            return Ok(MoveTarget::Chest {
                selector: asked.to_string(),
            });
        }

        // A species name, not an id. Monsters are only ever addressed by id,
        // so hand back the ids that match instead of guessing which one.
        // Checked before ground items, whose loose matcher below would
        // otherwise swallow "goblin" for a goblin_sword lying nearby.
        let candidates = self.monster_ids_of_species(asked);
        if !candidates.is_empty() {
            return Err(MoveTargetError::SpeciesNotId {
                species: asked.to_string(),
                candidates,
            });
        }

        if let Some((instance_id, name)) = self.ground_item_named(asked) {
            return Ok(MoveTarget::GroundItem { instance_id, name });
        }

        Err(self.unknown_target(asked))
    }

    /// Ids and distances of the monsters in sight of a given type, nearest
    /// first — what a species-instead-of-id mistake gets told to use.
    fn monster_ids_of_species(&self, species: &str) -> Vec<(String, f32)> {
        let Some(sp) = self.self_player.as_ref() else {
            return Vec::new();
        };
        let sight_sq = NPC_SIGHT_RADIUS * NPC_SIGHT_RADIUS;
        let mut found: Vec<(String, f32)> = self
            .monsters_on_my_floor()
            .filter(|m| m.monster_type.eq_ignore_ascii_case(species))
            .filter_map(|m| {
                let d_sq = m.position.dist_xz_sq(&sp.position);
                (d_sq <= sight_sq).then(|| (m.id.clone(), d_sq.sqrt()))
            })
            .collect();
        found.sort_by(|a, b| a.1.total_cmp(&b.1));
        found
    }

    /// A target that matched nothing, carrying a sample of what would have.
    fn unknown_target(&self, asked: &str) -> MoveTargetError {
        let mut addressable: Vec<String> = self
            .players_on_my_floor()
            .filter(|(_, p)| self.self_player_id.as_ref() != Some(&p.id))
            .map(|(_, p)| p.name.clone())
            .take(4)
            .collect();
        addressable.extend(self.monsters_on_my_floor().map(|m| m.id.clone()).take(4));
        addressable.extend(
            self.ground_items_in_sight()
                .iter()
                .take(3)
                .map(|(_, i)| format!("{} [id {}]", i.item_def_id, i.instance_id)),
        );
        addressable.extend(
            self.world_cache
                .read()
                .unwrap()
                .all_dungeons()
                .iter()
                .map(|d| d.name.clone()),
        );
        MoveTargetError::Unknown {
            asked: asked.to_string(),
            addressable,
        }
    }

    /// Abort a running follow loop, if any. Returns the name that was being
    /// followed. A loop that already ended left its own note, so it does not
    /// count as cancelled.
    pub fn cancel_follow(&mut self) -> Option<String> {
        let (name, handle) = self.follow_task.take()?;
        if handle.is_finished() {
            return None;
        }
        handle.abort();
        Some(name)
    }

    /// This agent's own tip hat, if one is set down.
    pub fn own_tip_hat(&self) -> Option<&onlinerpg_shared::tip_hat::TipHat> {
        self.self_player_id
            .and_then(|id| self.tip_hats.values().find(|h| h.owner == id))
    }

    /// Fold our stall and pick up our tip hat before walking off — the net
    /// for a departure the LLM did not wrap up itself.
    pub async fn pack_up_placeables(&mut self, label: &str) {
        if self.own_stall().is_some() {
            tracing::info!("[{label}] Stall still out — packing it up");
            let pack = ClientMessage::ChatMessage {
                message: "/pack_stall".to_string(),
            };
            if let Err(e) = self.send_command(pack).await {
                tracing::error!("[{label}] Failed to send /pack_stall: {e}");
            }
        }
        if self.own_tip_hat().is_some() {
            let hat = self
                .self_bag
                .iter()
                .find(|i| crate::item_defs::get(&i.item_def_id).is_some_and(|d| d.is_tip_hat()))
                .map(|i| i.instance_id);
            if let Some(instance_id) = hat {
                tracing::info!("[{label}] Tip hat still out — picking it up");
                let cmd = ClientMessage::UseItem { instance_id };
                if let Err(e) = self.send_command(cmd).await {
                    tracing::error!("[{label}] Failed to pick up the tip hat: {e}");
                }
            }
        }
    }

    /// Our floor as a passability cache index, for path queries. Standing on a
    /// stair shaft this is the floor the shaft's cells are keyed to, which is
    /// not always the floor we are nearest — see `pathfinding::start_floor_at`.
    pub fn passability_floor(&self) -> u8 {
        let floor = passability_floor_for_level(self.self_floor_level);
        if self.self_floor_level >= 0 {
            return floor;
        }
        let Some(position) = self.self_player.as_ref().map(|p| p.position) else {
            return floor;
        };
        if onlinerpg_shared::dungeon::entrance_at(position.x, position.z).is_none() {
            return floor;
        }
        let world = self.world_cache.read().unwrap();
        pathfinding::start_floor_at(
            world.passability_cache(),
            position.x,
            position.z,
            position.y,
        )
    }

    /// Ground height at (x, z) for something standing on passability floor
    /// `floor` — a dungeon floor, or the entrance ramp when `floor` is the
    /// surface. `None` means the dungeons have no say and terrain height wins.
    /// The single answer to "how high is the ground here", so the send path,
    /// the mover and the monster relay cannot drift apart.
    pub(super) fn dungeon_ground_y(&self, x: f32, z: f32, floor: u8) -> Option<f32> {
        self.world_cache
            .read()
            .unwrap()
            .dungeon_at(x, z)?
            .ground_y(floor, x, z)
    }

    /// Position and wire floor for a step to (x, z) on passability floor
    /// `floor`. Inside a dungeon the Y comes from that floor (or the stair
    /// ramp we are walking), and the declared floor follows the Y — the server
    /// derives collision from Y and validates the declaration against it, so
    /// anything else is either refused or silently collided on the wrong
    /// floor. Above ground the caller's Y stands and `send_command` snaps it.
    pub fn step_pose(&self, x: f32, z: f32, floor: u8, current_y: f32) -> (Position, i8) {
        match self.dungeon_ground_y(x, z, floor) {
            Some(y) => (Position { x, y, z }, self.wire_floor_at(x, z, y)),
            None => (
                Position { x, y: current_y, z },
                floor_level_for_passability(floor),
            ),
        }
    }

    /// The action's opt-out over `always_sprint`, then the server's own hunger
    /// gate (`satiation > NORMAL_MIN`) so both sims agree on our speed. With no
    /// hunger data yet, let the server judge.
    pub fn sprint_allowed(&self, requested: Option<bool>) -> bool {
        requested.unwrap_or(self.always_sprint)
            && self
                .self_hunger
                .is_none_or(|(satiation, _)| satiation > onlinerpg_shared::hunger::NORMAL_MIN)
    }

    /// Send one movement step toward (x, z) on passability floor `floor`,
    /// posed and floor-stamped by `step_pose`. The single way a mover puts a
    /// step on the wire, so none of them can forget to update the floor we
    /// declare — which the server checks our height against. `background` is
    /// for steps no current action asked for (the follow task), which must
    /// stay out of the action-progress count. Returns whether the step
    /// actually sprints, which is what the caller paces the walk by.
    pub async fn send_step(
        &mut self,
        x: f32,
        z: f32,
        floor: u8,
        rotation: f32,
        background: bool,
        sprint: Option<bool>,
    ) -> anyhow::Result<bool> {
        let current_y = self
            .self_player
            .as_ref()
            .map(|p| p.position.y)
            .unwrap_or(0.0);
        let (position, floor_level) = self.step_pose(x, z, floor, current_y);
        self.adopt_floor_level(floor_level);
        let sprinting = self.sprint_allowed(sprint);
        let cmd = ClientMessage::PlayerMove {
            position,
            rotation,
            floor_level,
            append: false,
            sprinting,
        };
        self.send_flagged_command(cmd, background).await?;
        Ok(sprinting)
    }

    /// Put an entity on the ground of dungeon floor `floor`, leaving it where
    /// it is when no dungeon covers the spot.
    pub(super) fn on_dungeon_floor(&self, position: Position, floor: u8) -> Position {
        match self.dungeon_ground_y(position.x, position.z, floor) {
            Some(y) => Position { y, ..position },
            None => position,
        }
    }

    /// The wire `floor_level` to declare while standing at (x, z, y): whichever
    /// floor's grid sits nearest that Y. Deliberately the shared query the
    /// server itself collides against (`authoritative_floor`), so our
    /// declaration and its collision can never resolve differently.
    pub(super) fn wire_floor_at(&self, x: f32, z: f32, y: f32) -> i8 {
        let world = self.world_cache.read().unwrap();
        floor_level_for_passability(pathfinding::get_floor_at_position(
            world.passability_cache(),
            x,
            z,
            y,
        ))
    }

    pub(super) async fn snap_position_to_ground(
        &self,
        mut position: Position,
        context: &str,
    ) -> Position {
        let original_y = position.y;
        match self
            .height_sampler
            .sample_height(position.x, position.z)
            .await
        {
            Ok(terrain_y) => {
                tracing::debug!(
                    "{context} height correction: ({:.1}, {:.1}) y: {:.2} -> {:.2}",
                    position.x,
                    position.z,
                    original_y,
                    terrain_y
                );
                position.y = terrain_y;
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to sample terrain height for {context} at ({:.1}, {:.1}): {e}",
                    position.x,
                    position.z
                );
            }
        }
        position
    }

    /// Apply an authoritative monster pose — server fanout, a reject
    /// correction, or the local echo of our own outgoing move.
    pub(super) fn apply_monster_pose(
        &mut self,
        monster_id: &str,
        position: Position,
        rotation: f32,
        state: MonsterState,
    ) {
        if let Some(m) = self.nearby_monsters.get_mut(monster_id) {
            m.position = position;
            m.rotation = rotation;
            m.state = state;
        }
    }

    /// Apply an authoritative player pose. Supersedes whatever move that player
    /// had buffered, which `drain_events` would otherwise replay after us.
    pub(super) fn apply_player_pose(
        &mut self,
        player_id: &PlayerId,
        position: Position,
        rotation: f32,
        floor_level: i8,
    ) {
        if let Some(p) = self.nearby_players.get_mut(player_id) {
            p.position = position;
            p.rotation = rotation;
            p.floor_level = floor_level;
        }
        self.latest_player_moves.remove(player_id);
    }

    /// Adopt a floor change. No local purge: every server-side removal now
    /// reaches this client — watched monsters via the floor-aware AOI diff,
    /// owned ones (the corpse sweep included) via owner-directed messages.
    pub(crate) fn adopt_floor_level(&mut self, floor_level: i8) {
        self.self_floor_level = floor_level;
    }

    /// Whether `self_player.position` currently reflects the real body rather
    /// than a force-move burst the server is still walking.
    pub fn self_pose_settled(&self) -> bool {
        self.self_pose_settles_at
            .is_none_or(|t| std::time::Instant::now() >= t)
    }

    /// Flag `self_player.position` as a promise for the next `walk_secs`
    /// plus slack: a force-move burst sends legs the server has yet to walk.
    /// A sub-second walk (the usual last metre) is not worth hiding.
    pub fn suppress_pose_for(&mut self, walk_secs: f32) {
        const POSE_SETTLE_SLACK: std::time::Duration = std::time::Duration::from_secs(2);
        if walk_secs <= 1.0 {
            return;
        }
        self.self_pose_settles_at = Some(
            std::time::Instant::now()
                + std::time::Duration::from_secs_f32(walk_secs)
                + POSE_SETTLE_SLACK,
        );
    }

    /// Drop every trace of a monster: the entry itself, its AI mirror, its
    /// move-dedup slot, and its sighting so a reappearance announces again.
    /// The single recipe for all removal paths — a new shadow collection
    /// belongs here, not in each caller.
    pub(super) fn forget_monster(&mut self, id: &str) {
        self.nearby_monsters.remove(id);
        self.monster_ai.remove_monster(id);
        self.latest_monster_moves.remove(id);
        self.sighted_pois.remove(&format!("m:{id}"));
    }

    /// The server put us somewhere we did not walk to — a refused step, a
    /// return scroll, a respawn. Adopting the pose is not enough: the mover
    /// watches `position_corrections` to drop the path it was walking.
    pub(super) fn relocate_self(&mut self, position: Position, rotation: f32, floor_level: i8) {
        if let Some(ref mut p) = self.self_player {
            p.position = position;
            p.rotation = rotation;
            p.floor_level = floor_level;
        }
        self.self_pose_settles_at = None;
        self.adopt_floor_level(floor_level);
        self.position_corrections = self.position_corrections.wrapping_add(1);
        if let Some(id) = self.self_player_id {
            self.latest_player_moves.remove(&id);
        }
    }

    /// Send a position sync to correct Y to terrain height.
    /// Should be called after JoinSuccess or PlayerRespawned to snap to
    /// ground. Background: the rx task fires it, not an agent action.
    pub async fn sync_height(&mut self) -> anyhow::Result<()> {
        let Some(ref p) = self.self_player else {
            return Ok(());
        };
        let pos = p.position;
        let rotation = p.rotation;
        self.send_background_command(ClientMessage::player_move(pos, rotation, 0))
            .await
    }

    /// Our own pose mirror. `send_command` writes it optimistically on
    /// InteractObject/StopInteraction; the server echo and rejection
    /// converge it.
    pub(super) fn set_self_pose(&mut self, object_type: Option<String>) {
        if let Some(p) = self.self_player.as_mut() {
            p.object_type = object_type;
        }
    }

    /// A goal for walking toward `(x, z)`: the point itself, or — when its
    /// cell is sealed (furniture swallows the cell a bed pose is authored
    /// on) — the centre of the nearest open neighbouring cell.
    pub fn walkable_near(&self, x: f32, z: f32, floor: u8) -> (f32, f32) {
        let world = self.world_cache.read().unwrap();
        let cache = world.passability_cache();
        if !pathfinding::is_cell_sealed(cache, x, z, floor, None) {
            return (x, z);
        }
        let (cx, cz) = (x.floor() + 0.5, z.floor() + 0.5);
        let d2 = |(nx, nz): (f32, f32)| (nx - x).powi(2) + (nz - z).powi(2);
        (-1..=1i32)
            .flat_map(|dz| (-1..=1i32).map(move |dx| (dx, dz)))
            .filter(|&d| d != (0, 0))
            .map(|(dx, dz)| (cx + dx as f32, cz + dz as f32))
            .filter(|&(nx, nz)| !pathfinding::is_cell_sealed(cache, nx, nz, floor, None))
            .min_by(|&a, &b| d2(a).total_cmp(&d2(b)))
            .unwrap_or((x, z))
    }

    /// Whether a wall stands between us and a point, as the server judges
    /// every blow.
    pub fn attack_line_blocked(&self, to_x: f32, to_z: f32) -> bool {
        let Some(from) = self.self_player.as_ref().map(|p| p.position) else {
            return false;
        };
        let floor = self.passability_floor();
        let world = self.world_cache.read().unwrap();
        pathfinding::attack_line_blocked(
            world.passability_cache(),
            from.x,
            from.z,
            to_x,
            to_z,
            floor,
        )
    }

    /// Find a smoothed path from current position to the goal.
    pub fn find_path_to(&self, goal_x: f32, goal_z: f32, goal_floor: u8) -> PathResult {
        let (start_x, start_z) = match &self.self_player {
            Some(p) => (p.position.x, p.position.z),
            None => {
                return PathResult {
                    waypoints: Vec::new(),
                    found: false,
                }
            }
        };
        let start_floor = self.passability_floor();
        let max_nodes = path_max_nodes(start_floor, goal_floor);
        let world = self.world_cache.read().unwrap();
        pathfinding::find_and_smooth_path(
            start_x,
            start_z,
            start_floor,
            goal_x,
            goal_z,
            goal_floor,
            world.passability_cache(),
            max_nodes,
        )
    }

    /// Build a `PlayerMove` command at the current position rotated to face
    /// the monster. Mirrors the web client's pre-attack position-sync, so
    /// the swing animation orients toward the target. Returns `None` if
    /// either the agent or the monster isn't currently known.
    pub fn face_monster_command(&self, monster_id: &str) -> Option<ClientMessage> {
        let target_pos = self.nearby_monsters.get(monster_id)?.position;
        self.face_position_command(target_pos)
    }

    /// Like `face_monster_command`, but toward another player or NPC — a
    /// position-sync that rotates us to face them, e.g. after walking up
    /// to someone for a conversation.
    pub fn face_player_command(&self, player_id: &PlayerId) -> Option<ClientMessage> {
        let target_pos = self.nearby_players.get(player_id)?.position;
        self.face_position_command(target_pos)
    }

    /// Position-sync at the current location, rotated to face `target_pos`.
    fn face_position_command(&self, target_pos: Position) -> Option<ClientMessage> {
        let self_player = self.self_player.as_ref()?;
        let to_target = crate::geom::PlanarDelta::between(&self_player.position, &target_pos);
        Some(ClientMessage::player_move(
            self_player.position,
            to_target.rotation(),
            self.self_floor_level,
        ))
    }

    /// Pick a spawn position 20–25m around the bot's own player, rejecting
    /// houses and no-spawn zones (+ margin). The async send path snaps Y to
    /// terrain height before the spawn request is sent.
    pub(super) fn find_valid_spawn_position(&self) -> Option<Position> {
        // Mirror the server's NO_SPAWN_MARGIN / client's TOWN_MARGIN so the bot
        // doesn't generate spawn requests the server will reject around towns.
        const TOWN_MARGIN: f32 = 30.0;

        let center = self.self_player.as_ref()?.position;

        // Don't spawn around a bot that is standing in (or near) a town.
        if self
            .no_spawn_zones
            .iter()
            .any(|z| z.contains_with_margin(center.x, center.z, TOWN_MARGIN))
        {
            return None;
        }

        let world = self.world_cache.read().unwrap();
        let mut rng = rand::thread_rng();
        for _ in 0..10 {
            let angle = rng.gen_range(0.0..std::f32::consts::TAU);
            let dist = rng.gen_range(20.0..25.0);
            let x = center.x + angle.cos() * dist;
            let z = center.z + angle.sin() * dist;

            // Reject if inside a house (bots roam the surface only)
            if pathfinding::is_movement_blocked(world.passability_cache(), x, z, x, z, 0, None) {
                continue;
            }

            // Reject if inside a no-spawn zone (+ margin)
            if self
                .no_spawn_zones
                .iter()
                .any(|zone| zone.contains_with_margin(x, z, TOWN_MARGIN))
            {
                continue;
            }

            return Some(Position { x, y: 0.0, z });
        }
        None
    }
}
