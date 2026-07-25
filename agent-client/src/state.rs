use std::collections::{HashMap, HashSet};

use crate::monster_ai::MonsterAiManager;
use onlinerpg_shared::furniture::{self, FurniturePlacement};
use onlinerpg_shared::housing::{HouseData, WallDirection};
use onlinerpg_shared::pathfinding::{self, PassabilityCache, PathResult};
use onlinerpg_shared::{
    Character, ClientMessage, Monster, MonsterState, Player, PlayerId, ServerMessage,
};
use onlinerpg_shared::{NoSpawnZone, Position};
use onlinerpg_terrain::height::HeightSampler;
use rand::Rng;
use std::sync::Arc;
use tokio::sync::{mpsc, Notify};

const MAX_EVENTS: usize = 200;
/// Distance threshold for "player appeared nearby" agent events (in game units).
const NEARBY_PLAYER_RADIUS: f32 = 10.0;
/// Real-time cooldown on the wishlist prompt section after the NPC buys
/// a wishlist item (see `trade_satiated_until`).
const WISHLIST_TRADE_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30 * 60);
/// NPC sight distance for deciding which nearby human and monster activity
/// matters. Re-exported from the shared crate so the server's event-delivery
/// radius and the agent's perception radius are guaranteed equal.
pub(crate) use onlinerpg_shared::NPC_SIGHT_RADIUS;

/// Where a carried item sits.
pub enum Carried {
    Worn(onlinerpg_shared::inventory::EquipSlot),
    InBag(u64),
}

/// How urgently an event needs LLM attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventUrgency {
    /// Must be processed immediately (combat damage to self, death, direct chat, kicked)
    Urgent,
    /// Can wait and be batched with next prompt (world state changes, xp, spawns)
    Routine,
    /// Don't send to LLM at all (high-frequency movement, time sync)
    Noise,
}

/// Shared world data: passability cache and house state.
/// Wrapped in `Arc<RwLock<WorldCache>>` so multiple NPC connections can share it.
pub struct WorldCache {
    passability_cache: PassabilityCache,
    houses: HashMap<String, HouseData>,
}

impl WorldCache {
    pub fn new() -> Self {
        Self {
            passability_cache: PassabilityCache::new(),
            houses: HashMap::new(),
        }
    }

    pub fn passability_cache(&self) -> &PassabilityCache {
        &self.passability_cache
    }

    pub fn houses(&self) -> &HashMap<String, HouseData> {
        &self.houses
    }

    pub fn add_house(&mut self, house: HouseData) {
        let rp = pathfinding::build_runtime_passability(&house);
        self.passability_cache.insert(house.id.clone(), rp);
        pathfinding::apply_door_overlays(&mut self.passability_cache, &house);
        self.houses.insert(house.id.clone(), house);
    }

    pub fn remove_house(&mut self, house_id: &str) {
        self.houses.remove(house_id);
        self.passability_cache.remove(house_id);
    }

    /// Register (or replace) a region's solid furniture in the passability cache
    /// so the bot paths around it, mirroring the browser's
    /// `passability_set_furniture` (same `furniture:rx,rz` key + shared
    /// `furniture` resolution). Empty/non-solid regions clear the entry.
    pub fn sync_furniture(&mut self, rx: i32, rz: i32, placements: &[FurniturePlacement]) {
        let key = furniture::region_cache_key(rx, rz);
        match furniture::build_furniture_passability_for_placements(placements) {
            Some(rp) => {
                self.passability_cache.insert(key, rp);
            }
            None => {
                self.passability_cache.remove(&key);
            }
        }
    }

    pub fn update_door(
        &mut self,
        house_id: &str,
        room_index: u32,
        wall_dir: WallDirection,
        segment_index: usize,
        is_open: bool,
    ) {
        if let Some(house) = self.houses.get(house_id) {
            if let Some(room) = house.rooms.get(room_index as usize) {
                pathfinding::update_door_edge(
                    &mut self.passability_cache,
                    house_id,
                    room,
                    wall_dir,
                    segment_index,
                    is_open,
                );
            }
        }
    }
}

/// Shared state between WebSocket reader and Claude driver tasks.
pub struct SharedState {
    pub characters: Vec<Character>,
    pub in_game: bool,
    /// Our own player ID (set on JoinSuccess)
    pub self_player_id: Option<PlayerId>,
    /// Our own player state (updated from JoinSuccess, GameState, health updates, etc.)
    pub self_player: Option<Player>,
    /// Our own gold in the smallest unit (from GoldUpdate). NPC traders'
    /// wallets are real server-side gold (economy phase 3).
    pub self_gold: Option<i64>,
    /// Our own bag (from InventoryState/InventoryUpdated), so a trading
    /// NPC knows what it carries.
    pub self_bag: Vec<onlinerpg_shared::inventory::ItemInstance>,
    /// What we are wearing, so `use` knows whether to equip or take off.
    pub self_equipped:
        HashMap<onlinerpg_shared::inventory::EquipSlot, onlinerpg_shared::inventory::ItemInstance>,
    /// Until when the wishlist prompt section stays suppressed after a
    /// successful purchase — a satisfied shopper stops shopping for a
    /// while even if other wishes remain.
    pub trade_satiated_until: Option<std::time::Instant>,
    /// True while at least one player has our trade window open (server
    /// `TradeBusy`). We stay put and keep serving them — the LLM's movement
    /// actions are suppressed — until the trade ends.
    pub trade_busy: bool,
    /// Known nearby players
    pub nearby_players: HashMap<PlayerId, Player>,
    /// Known nearby monsters
    pub nearby_monsters: HashMap<String, Monster>,
    events: Vec<ServerMessage>,
    /// Latest position per monster -- deduplicates high-frequency MonsterMoved events
    latest_monster_moves: HashMap<String, ServerMessage>,
    /// Latest position per player -- deduplicates high-frequency PlayerMoved events
    latest_player_moves: HashMap<PlayerId, ServerMessage>,
    /// Latest game time -- only the most recent matters
    latest_time: Option<ServerMessage>,
    /// Players we've already seen within NEARBY_PLAYER_RADIUS -- prevents duplicate events
    seen_nearby_players: HashSet<PlayerId>,
    /// Synthetic agent-side events (e.g. "player appeared nearby")
    agent_events: Vec<String>,
    /// Terrain height sampler (shared across NPC connections)
    pub height_sampler: Arc<HeightSampler>,
    /// Shared world cache: passability + houses (shared across NPC connections)
    pub world_cache: Arc<std::sync::RwLock<WorldCache>>,
    /// Current game time: is_night flag from server
    pub is_night: Option<bool>,
    /// Current game hour (0-23)
    pub game_hour: Option<u32>,
    /// Current game minute (0-59)
    pub game_minute: Option<u32>,
    /// Current floor level for the agent
    pub self_floor_level: u8,
    cmd_tx: mpsc::Sender<ClientMessage>,
    /// Notified when an urgent event arrives
    pub urgent_notify: Arc<Notify>,
    /// Monster AI manager for server-assigned monsters
    pub monster_ai: MonsterAiManager,
    /// Pending commands from monster AI and spawn requests
    pending_commands: Vec<ClientMessage>,
    /// No-spawn zones received from server on join
    no_spawn_zones: Vec<NoSpawnZone>,
    /// Spectator panel handle; feeds it chat/combat/system lines
    watch: Option<Arc<crate::watch::NpcWatch>>,
}

impl SharedState {
    pub fn new(
        characters: Vec<Character>,
        cmd_tx: mpsc::Sender<ClientMessage>,
        height_sampler: Arc<HeightSampler>,
        world_cache: Arc<std::sync::RwLock<WorldCache>>,
        watch: Option<Arc<crate::watch::NpcWatch>>,
    ) -> Self {
        Self {
            characters,
            in_game: false,
            self_player_id: None,
            self_player: None,
            self_gold: None,
            self_bag: Vec::new(),
            self_equipped: HashMap::new(),
            trade_satiated_until: None,
            trade_busy: false,
            nearby_players: HashMap::new(),
            nearby_monsters: HashMap::new(),
            events: Vec::new(),
            latest_monster_moves: HashMap::new(),
            latest_player_moves: HashMap::new(),
            latest_time: None,
            seen_nearby_players: HashSet::new(),
            agent_events: Vec::new(),
            height_sampler,
            world_cache,
            is_night: None,
            game_hour: None,
            game_minute: None,
            self_floor_level: 0,
            cmd_tx,
            urgent_notify: Arc::new(Notify::new()),
            monster_ai: MonsterAiManager::new(),
            pending_commands: Vec::new(),
            no_spawn_zones: Vec::new(),
            watch,
        }
    }

    /// Returns true if any non-NPC (human) player is in `nearby_players`.
    pub fn has_nearby_human_players(&self) -> bool {
        let Some(self_player) = self.self_player.as_ref() else {
            return false;
        };
        let self_id = self.self_player_id.as_ref();
        let radius_sq = NPC_SIGHT_RADIUS * NPC_SIGHT_RADIUS;

        self.nearby_players.iter().any(|(id, p)| {
            if self_id == Some(id) || p.is_official_npc {
                return false;
            }
            p.position.dist_xz_sq(&self_player.position) <= radius_sq
        })
    }

    /// Check all nearby players and emit an agent event for any player
    /// that just entered NEARBY_PLAYER_RADIUS for the first time.
    fn check_nearby_player_proximity(&mut self) {
        let self_pos = match self.self_player.as_ref() {
            Some(p) => &p.position,
            None => return,
        };
        let self_id = match self.self_player_id.as_ref() {
            Some(id) => id,
            None => return,
        };

        for (pid, player) in &self.nearby_players {
            if pid == self_id {
                continue;
            }
            if self.seen_nearby_players.contains(pid) {
                continue;
            }
            let dx = player.position.x - self_pos.x;
            let dz = player.position.z - self_pos.z;
            let dist = (dx * dx + dz * dz).sqrt();
            if dist <= NEARBY_PLAYER_RADIUS {
                self.seen_nearby_players.insert(*pid);
                self.agent_events.push(format!(
                    "[PlayerNearby] {} Lv.{} appeared {:.1}m away at ({:.1}, {:.1}, {:.1})",
                    player.name,
                    player.level,
                    dist,
                    player.position.x,
                    player.position.y,
                    player.position.z
                ));
                self.urgent_notify.notify_one();
            }
        }
    }

    async fn snap_position_to_ground(&self, mut position: Position, context: &str) -> Position {
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

    pub async fn send_command(&mut self, msg: ClientMessage) -> anyhow::Result<()> {
        let msg = match msg {
            ClientMessage::PlayerMove {
                position,
                rotation,
                append,
                ..
            } => {
                let position = if self.self_floor_level == 0 {
                    self.snap_position_to_ground(position, "PlayerMove").await
                } else {
                    position
                };
                // Update local position immediately so subsequent reads don't use stale data
                if let Some(ref mut p) = self.self_player {
                    p.position = position;
                    p.rotation = rotation;
                }
                ClientMessage::PlayerMove {
                    position,
                    rotation,
                    floor_level: self.self_floor_level as i8,
                    append,
                }
            }
            ClientMessage::RequestSpawnMonster {
                monster_type,
                position,
                rotation,
            } => ClientMessage::RequestSpawnMonster {
                monster_type,
                position: self
                    .snap_position_to_ground(position, "RequestSpawnMonster")
                    .await,
                rotation,
            },
            ClientMessage::MonsterMove {
                monster_id,
                position,
                rotation,
                state,
                target_position,
            } => {
                // position and target_position are independent coordinates, so
                // sample both terrain heights concurrently rather than serially.
                let (position, target_position) = tokio::join!(
                    self.snap_position_to_ground(position, "MonsterMove"),
                    self.snap_position_to_ground(target_position, "MonsterMove target"),
                );
                // The server skips echoing our own monster moves back;
                // mirror them locally or owned monsters freeze at spawn.
                self.apply_monster_pose(&monster_id, position, rotation, state);
                ClientMessage::MonsterMove {
                    monster_id,
                    position,
                    rotation,
                    state,
                    target_position,
                }
            }
            other => other,
        };
        self.cmd_tx
            .send(msg)
            .await
            .map_err(|e| anyhow::anyhow!("Command channel closed: {e}"))
    }

    /// Apply an authoritative monster pose — server fanout, a reject
    /// correction, or the local echo of our own outgoing move.
    fn apply_monster_pose(
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

    /// Classify how urgent a server event is for LLM processing.
    pub fn classify_event(&self, msg: &ServerMessage) -> EventUrgency {
        let self_id = self.self_player_id.as_ref();
        match msg {
            // Urgent: we are being attacked or we died
            ServerMessage::MonsterAttackedPlayer { player_id, .. } => {
                if self_id == Some(player_id) {
                    EventUrgency::Urgent
                } else {
                    EventUrgency::Routine
                }
            }
            ServerMessage::PlayerDead { player_id } => {
                if self_id == Some(player_id) {
                    EventUrgency::Urgent
                } else {
                    EventUrgency::Routine
                }
            }
            // Urgent: a human chats (not ourselves). NPC→NPC chat is only
            // Routine: urgent wakeups on both sides turn any shared topic
            // into an endless conversation loop (and an LLM-cost leak), so
            // NPC replies wait for the next batched prompt instead.
            ServerMessage::ChatMessage { player_id, .. } => {
                if self_id == Some(player_id) {
                    EventUrgency::Noise
                } else if self
                    .nearby_players
                    .get(player_id)
                    .is_some_and(|p| p.is_official_npc)
                {
                    EventUrgency::Routine
                } else {
                    EventUrgency::Urgent
                }
            }
            // Urgent: a whisper is always addressed to us; the echo of our
            // own outgoing whisper is the Noise case.
            ServerMessage::WhisperMessage { from, .. } => {
                let self_name = self.self_player.as_ref().map(|p| p.name.as_str());
                if Some(from.as_str()) == self_name {
                    EventUrgency::Noise
                } else {
                    EventUrgency::Urgent
                }
            }
            // Routine: feedback on our own command (/who output, whisper
            // errors) — worth seeing, not worth an immediate wakeup.
            ServerMessage::SystemMessage { .. } => EventUrgency::Routine,
            // Urgent: kicked
            ServerMessage::Kicked { .. } => EventUrgency::Urgent,

            // Urgent: verdict on our haggling offer — the NPC should follow
            // up in the ongoing conversation (e.g. correct a clamped price).
            ServerMessage::DealResult { .. } => EventUrgency::Urgent,

            // Urgent: a player traded with us, or our trade request failed —
            // both deserve an in-character reaction.
            ServerMessage::TradeNotice { .. } | ServerMessage::TradeError { .. } => {
                EventUrgency::Urgent
            }

            // State-only: tracked on SharedState, shown in the world state.
            ServerMessage::GoldUpdate { .. }
            | ServerMessage::GoldGained { .. }
            | ServerMessage::InventoryState { .. }
            | ServerMessage::InventoryUpdated { .. }
            | ServerMessage::TradeBusy { .. } => EventUrgency::Noise,

            // Urgent: another player attacks a monster (so we can join in)
            ServerMessage::PlayerAttacked { player_id, .. } => {
                if self_id != Some(player_id) {
                    EventUrgency::Urgent
                } else {
                    EventUrgency::Routine
                }
            }
            ServerMessage::MonsterProvoked { .. } => EventUrgency::Routine,

            // Routine: world state changes
            ServerMessage::JoinSuccess { .. }
            | ServerMessage::GameState { .. }
            | ServerMessage::PlayerJoined { .. }
            | ServerMessage::PlayerLeft { .. }
            | ServerMessage::PlayerAppeared { .. }
            | ServerMessage::PlayerDisappeared { .. }
            | ServerMessage::MonsterSpawned { .. }
            | ServerMessage::MonsterAssigned { .. }
            | ServerMessage::SpawnMonsterRequest { .. }
            | ServerMessage::MonsterDead { .. }
            | ServerMessage::MonsterRemoved { .. }
            | ServerMessage::XpGained { .. }
            | ServerMessage::PlayerRespawned { .. }
            | ServerMessage::PlayerHealthUpdate { .. }
            | ServerMessage::PlayerTorchToggled { .. } => EventUrgency::Routine,

            // Noise: high-frequency, irrelevant, or housing updates
            ServerMessage::PlayerMoved { .. }
            | ServerMessage::PlayerTeleported { .. }
            | ServerMessage::MonsterMoved { .. }
            | ServerMessage::GameTimeSync { .. }
            | ServerMessage::HouseSpawned { .. }
            | ServerMessage::HousesInArea { .. }
            | ServerMessage::HouseUpdated { .. }
            | ServerMessage::HouseRemoved { .. }
            | ServerMessage::DoorToggled { .. }
            | ServerMessage::InteractionRejected { .. } => EventUrgency::Noise,

            // Auth/character events: routine (handled before game entry)
            _ => EventUrgency::Routine,
        }
    }

    fn handle_managed_monster_hit(
        &mut self,
        monster_id: &str,
        player_id: &PlayerId,
        hit: bool,
        damage: u32,
    ) {
        if !self.monster_ai.manages(monster_id) {
            return;
        }

        let world = self.world_cache.read().unwrap();
        let commands = self.monster_ai.handle_monster_hit(
            monster_id,
            player_id,
            hit,
            damage,
            world.passability_cache(),
        );
        drop(world);
        self.pending_commands.extend(commands);
    }

    /// Push an event and update tracked state. Returns the urgency of the event.
    pub fn push_event(&mut self, msg: ServerMessage) -> EventUrgency {
        // Feed the spectator panel before mutating, while names still resolve
        if let Some(watch) = self.watch.clone() {
            if let Some(kind) = crate::watch::feed_kind(&msg) {
                let line = crate::watch::feed_fallback(&msg)
                    .or_else(|| crate::driver::format_event(self, &msg));
                if let Some(line) = line {
                    watch.push(kind, line);
                }
            }
        }

        // Update tracked state from certain messages
        match &msg {
            ServerMessage::JoinSuccess { player, .. } => {
                self.in_game = true;
                self.self_player_id = Some(player.id);
                self.self_player = Some(player.clone());
            }
            ServerMessage::GameState {
                players, monsters, ..
            } => {
                self.nearby_players = players.iter().map(|p| (p.id, p.clone())).collect();
                self.nearby_monsters = monsters.clone();
                // Update self_player from game state
                if let Some(self_id) = self.self_player_id {
                    if let Some(p) = self.nearby_players.get(&self_id).cloned() {
                        self.self_player = Some(p);
                    }
                }
            }
            ServerMessage::PlayerHealthUpdate {
                player_id,
                health,
                max_health,
            } if self.self_player_id.as_ref() == Some(player_id) => {
                if let Some(p) = self.self_player.as_mut() {
                    p.health = *health;
                    p.max_health = *max_health;
                }
            }
            // Only ever sent direct to the player who earned (or lost) the XP,
            // so this never describes anyone in `nearby_players`.
            ServerMessage::XpGained {
                player_id,
                new_level,
                max_hp,
                current_hp,
                ..
            } if self.self_player_id.as_ref() == Some(player_id) => {
                if let Some(ref mut p) = self.self_player {
                    p.level = *new_level;
                    p.health = *current_hp;
                    p.max_health = *max_hp;
                }
            }
            ServerMessage::PlayerJoined { player } | ServerMessage::PlayerAppeared { player } => {
                self.nearby_players.insert(player.id, player.clone());
            }
            ServerMessage::PlayerLeft { player_id }
            | ServerMessage::PlayerDisappeared { player_id } => {
                self.nearby_players.remove(player_id);
                self.seen_nearby_players.remove(player_id);
            }
            ServerMessage::MonsterSpawned { monster } => {
                self.nearby_monsters
                    .insert(monster.id.clone(), monster.clone());
            }
            ServerMessage::SpawnMonsterRequest { monster_type } => {
                if let Some(pos) = self.find_valid_spawn_position() {
                    let mut rng = rand::thread_rng();
                    let rotation = rng.gen_range(0.0..std::f32::consts::TAU);
                    self.pending_commands
                        .push(ClientMessage::RequestSpawnMonster {
                            monster_type: monster_type.clone(),
                            position: pos,
                            rotation,
                        });
                }
            }
            ServerMessage::NoSpawnZones { zones } => {
                self.no_spawn_zones = zones.clone();
            }
            ServerMessage::MonsterAssigned { monster } => {
                self.nearby_monsters
                    .insert(monster.id.clone(), monster.clone());
                self.monster_ai.add_monster(monster);
            }
            ServerMessage::MonsterDead { monster_id, .. } => {
                self.nearby_monsters.remove(monster_id);
                self.monster_ai.handle_monster_dead(monster_id);
            }
            ServerMessage::MonsterRemoved { monster_id } => {
                self.nearby_monsters.remove(monster_id);
                self.monster_ai.remove_monster(monster_id);
            }
            ServerMessage::CharacterCreated { ref character } => {
                self.characters.push(character.clone());
            }
            ServerMessage::GoldUpdate { gold } => {
                self.self_gold = Some(*gold);
            }
            ServerMessage::TradeBusy { busy } => {
                self.trade_busy = *busy;
            }
            ServerMessage::InventoryState { ref inventory }
            | ServerMessage::InventoryUpdated { ref inventory } => {
                self.self_bag = inventory.bag.clone();
                self.self_equipped = inventory.equipped.clone();
            }
            // A player sold to us = we bought a wishlist item (the server
            // only lets residents buy their wishlist): shopping mood
            // satisfied for a while.
            ServerMessage::TradeNotice {
                kind: onlinerpg_shared::messages::DealKind::Sell,
                ..
            } => {
                self.trade_satiated_until =
                    Some(std::time::Instant::now() + WISHLIST_TRADE_COOLDOWN);
            }
            ServerMessage::PlayerMoved {
                player_id,
                position,
                ..
            } => {
                // Update tracked position for self and nearby players
                if self.self_player_id.as_ref() == Some(player_id) {
                    if let Some(ref mut p) = self.self_player {
                        p.position = *position;
                    }
                }
                if let Some(p) = self.nearby_players.get_mut(player_id) {
                    p.position = *position;
                }
            }
            ServerMessage::MonsterMoved {
                monster_id,
                position,
                rotation,
                state,
                ..
            } => {
                self.apply_monster_pose(monster_id, *position, *rotation, *state);
            }
            ServerMessage::HouseSpawned { ref house } => {
                self.world_cache.write().unwrap().add_house(house.clone());
            }
            ServerMessage::HousesInArea { ref houses } => {
                let mut world = self.world_cache.write().unwrap();
                for house in houses {
                    world.add_house(house.clone());
                }
            }
            ServerMessage::HouseUpdated { ref house } => {
                self.world_cache.write().unwrap().add_house(house.clone());
            }
            ServerMessage::HouseRemoved { ref house_id } => {
                self.world_cache.write().unwrap().remove_house(house_id);
            }
            ServerMessage::DoorToggled {
                ref house_id,
                room_index,
                ref wall_dir,
                segment_index,
                is_open,
            } => {
                self.world_cache.write().unwrap().update_door(
                    house_id,
                    *room_index,
                    *wall_dir,
                    *segment_index as usize,
                    *is_open,
                );
            }
            // Notify monster AI when a managed monster is attacked
            ServerMessage::PlayerAttacked {
                player_id,
                monster_id,
                hit,
                damage,
                ..
            } => {
                self.handle_managed_monster_hit(monster_id, player_id, *hit, *damage);
            }
            ServerMessage::MonsterProvoked {
                player_id,
                monster_id,
            } => {
                self.handle_managed_monster_hit(monster_id, player_id, false, 0);
            }
            _ => {}
        }

        // Check if any player just entered the nearby radius
        match &msg {
            ServerMessage::GameState { .. }
            | ServerMessage::PlayerJoined { .. }
            | ServerMessage::PlayerAppeared { .. }
            | ServerMessage::PlayerMoved { .. } => {
                self.check_nearby_player_proximity();
            }
            _ => {}
        }

        let urgency = self.classify_event(&msg);

        // Deduplicate high-frequency movement events: keep only latest per entity
        match &msg {
            ServerMessage::MonsterMoved {
                monster_id,
                position,
                ..
            } => {
                // Only forward to LLM if monster is within sight radius
                let dominated_by_distance = self.self_player.as_ref().is_some_and(|sp| {
                    position.dist_xz_sq(&sp.position) > NPC_SIGHT_RADIUS * NPC_SIGHT_RADIUS
                });
                if !dominated_by_distance {
                    self.latest_monster_moves.insert(monster_id.clone(), msg);
                }
                return urgency;
            }
            ServerMessage::PlayerMoved { player_id, .. } => {
                self.latest_player_moves.insert(*player_id, msg);
                return urgency;
            }
            // A pure state flag; it changes movement gating but is not an LLM
            // event in its own right.
            ServerMessage::TradeBusy { .. } => return urgency,
            ServerMessage::GameTimeSync { datetime, is_night } => {
                let prev_night = self.is_night;
                let prev_hour = self.game_hour;
                let hour = datetime.hour as u32;
                let minute = datetime.minute as u32;
                let night = *is_night;
                self.is_night = Some(night);
                self.game_hour = Some(hour);
                self.game_minute = Some(minute);
                self.latest_time = Some(msg);
                // Detect day/night transition or hour change → wake driver
                if (prev_night.is_some() && prev_night != self.is_night)
                    || (prev_hour.is_some() && prev_hour != self.game_hour)
                {
                    self.agent_events.push(format!(
                        "[TimeChange] It is now {hour:02}:{minute:02} ({}).",
                        if night { "night" } else { "day" }
                    ));
                    self.urgent_notify.notify_one();
                }
                return urgency;
            }
            _ => {}
        }

        self.events.push(msg);

        // Cap buffer size: drop oldest events
        if self.events.len() > MAX_EVENTS {
            let overflow = self.events.len() - MAX_EVENTS;
            self.events.drain(..overflow);
        }

        // Notify Claude driver if urgent
        if urgency == EventUrgency::Urgent {
            self.urgent_notify.notify_one();
        }

        urgency
    }

    pub fn drain_events(&mut self) -> Vec<ServerMessage> {
        let mut events = std::mem::take(&mut self.events);

        // Append latest snapshots
        if let Some(time) = self.latest_time.take() {
            events.push(time);
        }
        events.extend(self.latest_monster_moves.drain().map(|(_, v)| v));
        events.extend(self.latest_player_moves.drain().map(|(_, v)| v));

        events
    }

    /// Drain pending commands (from monster AI reactions, spawn requests, etc.)
    pub fn drain_pending_commands(&mut self) -> Vec<ClientMessage> {
        std::mem::take(&mut self.pending_commands)
    }

    /// Drain synthetic agent-side events (e.g. player proximity alerts).
    pub fn drain_agent_events(&mut self) -> Vec<String> {
        std::mem::take(&mut self.agent_events)
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
        let world = self.world_cache.read().unwrap();
        pathfinding::find_and_smooth_path(
            start_x,
            start_z,
            self.self_floor_level,
            goal_x,
            goal_z,
            goal_floor,
            world.passability_cache(),
            pathfinding::DEFAULT_MAX_NODES,
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
            self.self_floor_level as i8,
        ))
    }

    /// Pick a spawn position 20–25m around the bot's own player, rejecting
    /// houses and no-spawn zones (+ margin). The async send path snaps Y to
    /// terrain height before the spawn request is sent.
    fn find_valid_spawn_position(&self) -> Option<Position> {
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

    /// Push a synthetic agent event visible to the LLM.
    pub fn push_agent_event(&mut self, event: String) {
        if let Some(watch) = &self.watch {
            watch.push("agent", event.clone());
        }
        self.agent_events.push(event);
    }

    /// Resolve a player name (or raw id) among nearby players, as used by
    /// player-targeting LLM actions. Returns `(player_id, is_official_npc)`.
    /// `name_or_id` stays `&str` because it comes straight from LLM output and
    /// may be either form; the resolved handle is what gets typed.
    pub fn resolve_nearby_player(&self, name_or_id: &str) -> Option<(PlayerId, bool)> {
        self.nearby_players
            .iter()
            .find(|(id, p)| {
                p.name.eq_ignore_ascii_case(name_or_id)
                    || name_or_id.parse::<u64>().is_ok_and(|n| id.get() == n)
            })
            .map(|(id, p)| (*id, p.is_official_npc))
    }

    /// Find the item the agent named among the ones we carry, and where it
    /// sits. Matching is forgiving about the exact id (see
    /// `item_defs::resolve_carried`) but never reaches past what we hold.
    pub fn find_carried(&self, asked: &str) -> Option<(String, Carried)> {
        let ids: Vec<&str> = self
            .self_bag
            .iter()
            .chain(self.self_equipped.values())
            .map(|i| i.item_def_id.as_str())
            .collect();
        let id = crate::item_defs::resolve_carried(&ids, asked)?;
        let placed = self
            .self_equipped
            .iter()
            .find(|(_, i)| i.item_def_id == id)
            .map(|(slot, _)| Carried::Worn(*slot))
            .or_else(|| {
                self.self_bag
                    .iter()
                    .find(|i| i.item_def_id == id)
                    .map(|i| Carried::InBag(i.instance_id))
            })?;
        Some((id.to_string(), placed))
    }

    /// Current game time snapshot for schedule resolution.
    pub fn time_context(&self) -> (Option<bool>, Option<u32>, Option<u32>) {
        (self.is_night, self.game_hour, self.game_minute)
    }

    /// Build a text summary of current world state for the LLM prompt.
    pub fn format_world_state(&self) -> String {
        let mut lines = Vec::new();

        if let Some(ref p) = self.self_player {
            lines.push(format!(
                "You: {} Lv.{} {:?} HP {}/{} at ({:.1}, {:.1}, {:.1})",
                p.name,
                p.level,
                p.class,
                p.health,
                p.max_health,
                p.position.x,
                p.position.y,
                p.position.z
            ));
        }
        if let Some(gold) = self.self_gold {
            lines.push(format!(
                "Your gold: {}",
                crate::shop_info::format_price(gold)
            ));
        }
        if !self.self_bag.is_empty() {
            let items: Vec<String> = self
                .self_bag
                .iter()
                .map(|i| {
                    if i.quantity > 1 {
                        format!("{} x{}", i.item_def_id, i.quantity)
                    } else {
                        i.item_def_id.clone()
                    }
                })
                .collect();
            lines.push(format!("Your bag: {}", items.join(", ")));
        }
        if !self.self_equipped.is_empty() {
            let mut worn: Vec<String> = self
                .self_equipped
                .iter()
                .map(|(slot, i)| format!("{}: {}", slot.as_str(), i.item_def_id))
                .collect();
            worn.sort();
            lines.push(format!("You are wearing: {}", worn.join(", ")));
        }

        // Nearby players (exclude self and humans beyond the sight radius)
        let sp = self.self_player.as_ref();
        let sight_sq = NPC_SIGHT_RADIUS * NPC_SIGHT_RADIUS;
        for p in self.nearby_players.values() {
            if self.self_player_id.as_ref() == Some(&p.id) {
                continue;
            }
            if let Some(sp) = sp {
                if p.position.dist_xz_sq(&sp.position) > sight_sq {
                    continue;
                }
            }
            lines.push(format!(
                "Player: {} Lv.{} HP {}/{} at ({:.1}, {:.1}, {:.1})",
                p.name, p.level, p.health, p.max_health, p.position.x, p.position.y, p.position.z
            ));
        }

        // Exclude monsters beyond LLM sight radius
        for m in self.nearby_monsters.values() {
            if let Some(sp) = sp {
                if m.position.dist_xz_sq(&sp.position) > sight_sq {
                    continue;
                }
            }
            lines.push(format!(
                "Monster: {} [{}] HP {}/{} state={} at ({:.1}, {:.1}, {:.1})",
                m.monster_type,
                m.id,
                m.health,
                m.max_health,
                m.state,
                m.position.x,
                m.position.y,
                m.position.z
            ));
        }

        if lines.is_empty() {
            "No state available yet.".to_string()
        } else {
            lines.join("\n")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoTiles;

    #[async_trait::async_trait]
    impl onlinerpg_terrain::height::HeightTiles for NoTiles {
        async fn read_heightmap(&self, _tx: i32, _tz: i32) -> std::io::Result<Vec<u8>> {
            Err(std::io::Error::other("no terrain in tests"))
        }
    }

    fn test_state() -> (SharedState, mpsc::Receiver<ClientMessage>) {
        let (tx, rx) = mpsc::channel(8);
        let state = SharedState::new(
            Vec::new(),
            tx,
            Arc::new(HeightSampler::new(NoTiles)),
            Arc::new(std::sync::RwLock::new(WorldCache::new())),
            None,
        );
        (state, rx)
    }

    fn p(x: f32, y: f32, z: f32) -> Position {
        Position { x, y, z }
    }

    fn monster(id: &str) -> Monster {
        Monster {
            id: id.to_string(),
            monster_type: "slime".to_string(),
            position: p(0.0, 0.0, 0.0),
            rotation: 0.0,
            state: MonsterState::Idle,
            owner_id: None,
            health: 10,
            max_health: 10,
            floor_level: 0,
            level_override: None,
            aggressive: false,
            last_attack_at: 0,
            last_move_at: 0,
            move_budget: 0.0,
        }
    }

    /// The server never echoes our own monster moves back (the owner is
    /// skipped in the fanout), so `send_command` must apply them locally.
    #[tokio::test]
    async fn outgoing_monster_move_echoes_into_local_state() {
        let (mut s, mut rx) = test_state();
        s.nearby_monsters.insert("m1".to_string(), monster("m1"));

        s.send_command(ClientMessage::MonsterMove {
            monster_id: "m1".to_string(),
            position: p(3.0, 1.0, 4.0),
            rotation: 1.5,
            state: MonsterState::Run,
            target_position: p(6.0, 1.0, 8.0),
        })
        .await
        .unwrap();

        let m = &s.nearby_monsters["m1"];
        assert_eq!(m.position.x, 3.0);
        assert_eq!(m.position.z, 4.0);
        assert_eq!(m.rotation, 1.5);
        assert_eq!(m.state, MonsterState::Run);

        match rx.try_recv() {
            Ok(ClientMessage::MonsterMove { monster_id, .. }) => assert_eq!(monster_id, "m1"),
            other => panic!("expected MonsterMove on the wire, got {other:?}"),
        }
    }
}
