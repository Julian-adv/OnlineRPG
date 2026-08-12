use super::*;

impl SharedState {
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

    /// Send one movement step toward (x, z) on passability floor `floor`,
    /// posed and floor-stamped by `step_pose`. The single way a mover puts a
    /// step on the wire, so none of them can forget to update the floor we
    /// declare — which the server checks our height against.
    pub async fn send_step(
        &mut self,
        x: f32,
        z: f32,
        floor: u8,
        rotation: f32,
    ) -> anyhow::Result<()> {
        let current_y = self
            .self_player
            .as_ref()
            .map(|p| p.position.y)
            .unwrap_or(0.0);
        let (position, floor_level) = self.step_pose(x, z, floor, current_y);
        self.adopt_floor_level(floor_level);
        self.send_command(ClientMessage::player_move(position, rotation, floor_level))
            .await
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
        self.adopt_floor_level(floor_level);
        self.position_corrections = self.position_corrections.wrapping_add(1);
        if let Some(id) = self.self_player_id {
            self.latest_player_moves.remove(&id);
        }
    }

    /// Send a position sync to correct Y to terrain height.
    /// Should be called after JoinSuccess or PlayerRespawned to snap to ground.
    pub async fn sync_height(&mut self) -> anyhow::Result<()> {
        let Some(ref p) = self.self_player else {
            return Ok(());
        };
        let pos = p.position;
        let rotation = p.rotation;
        self.send_command(ClientMessage::player_move(pos, rotation, 0))
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
