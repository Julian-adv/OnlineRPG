use std::collections::{HashMap, HashSet};

use crate::dungeon::Dungeon;
use crate::monster_ai::MonsterAiManager;
use onlinerpg_shared::dungeon::{
    dungeon_cache_key, floor_cells, floor_level_for_passability, passability_floor_for_level,
    path_max_nodes, set_floor_cells,
};
use onlinerpg_shared::furniture::{self, FurniturePlacement};
use onlinerpg_shared::housing::{HouseData, WallDirection};
use onlinerpg_shared::inventory::GroundItem;
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
/// How many ground items the world state lists before summarising the rest.
const MAX_LISTED_GROUND_ITEMS: usize = 10;
/// How long a monster's drop stays hidden, so the agent never reaches loot
/// the players around it cannot see yet. Matches the impact frame a web
/// client holds the death for (`PLAYER_ATTACK_IMPACT_DELAY_MS` in
/// client/src/lib/data/combatTiming.ts).
const DEATH_DROP_REVEAL_DELAY: std::time::Duration = std::time::Duration::from_millis(540);
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

/// Shared world data: passability cache, house state and the generated
/// dungeons. Wrapped in `Arc<RwLock<WorldCache>>` so multiple NPC connections
/// share one copy.
pub struct WorldCache {
    passability_cache: PassabilityCache,
    houses: HashMap<String, HouseData>,
    dungeons: Vec<Arc<Dungeon>>,
    /// Open interior doors and broken props per (entrance id, depth), mirrored
    /// from the server so our A* sees the same walls its movement sim does.
    dungeon_doors: HashMap<(String, u8), HashSet<u32>>,
    dungeon_broken_props: HashMap<(String, u8), Vec<u32>>,
    /// Chest props already opened, per (entrance id, depth). No passability
    /// bearing — an open chest stays solid — but an opened chest is one the
    /// agent should stop seeing, the way its lid stays up for a web player.
    dungeon_opened_props: HashMap<(String, u8), HashSet<u32>>,
}

impl WorldCache {
    pub fn new() -> Self {
        Self {
            passability_cache: PassabilityCache::new(),
            houses: HashMap::new(),
            dungeons: Vec::new(),
            dungeon_doors: HashMap::new(),
            dungeon_broken_props: HashMap::new(),
            dungeon_opened_props: HashMap::new(),
        }
    }

    /// Generate every registry dungeon and register its passability — stair
    /// shafts included, so the shared A* walks from the surface down to the
    /// deepest floor with no extra machinery. Run once at startup, mirroring
    /// the server's own `init_passability`; the entries also give surface
    /// paths the entrance walls the server already collides against.
    pub fn register_dungeons(&mut self) {
        for dungeon in crate::dungeon::build_all() {
            self.passability_cache
                .insert(dungeon_cache_key(&dungeon.id), dungeon.passability());
            self.dungeons.push(Arc::new(dungeon));
        }
    }

    /// Dungeon whose footprint covers (x, z), by the shared registry's
    /// footprint test — the same one the server admits us underground by.
    pub fn dungeon_at(&self, x: f32, z: f32) -> Option<Arc<Dungeon>> {
        let def = onlinerpg_shared::dungeon::entrance_at(x, z)?;
        self.dungeon_by_id(&def.id)
    }

    /// Dungeon with the closest entrance.
    pub fn nearest_dungeon(&self, x: f32, z: f32) -> Option<Arc<Dungeon>> {
        self.dungeons
            .iter()
            .min_by(|a, b| {
                let da = crate::geom::PlanarDelta::xz(x, z, a.entrance.x, a.entrance.z).dist;
                let db = crate::geom::PlanarDelta::xz(x, z, b.entrance.x, b.entrance.z).dist;
                da.total_cmp(&db)
            })
            .map(Arc::clone)
    }

    pub fn dungeon_by_id(&self, id: &str) -> Option<Arc<Dungeon>> {
        self.dungeons.iter().find(|d| d.id == id).map(Arc::clone)
    }

    pub fn open_dungeon_doors(&self, id: &str, depth: u8) -> HashSet<u32> {
        self.dungeon_doors
            .get(&(id.to_string(), depth))
            .cloned()
            .unwrap_or_default()
    }

    /// Replace the open-door set for a dungeon (the `DungeonDoorsState`
    /// snapshot covers every depth at once, so unlisted floors are all shut).
    pub fn set_dungeon_doors(&mut self, id: &str, doors: &[(u8, u32)]) {
        let touched: HashSet<u8> = self
            .dungeon_doors
            .keys()
            .filter(|(k, _)| k == id)
            .map(|(_, depth)| *depth)
            .chain(doors.iter().map(|(depth, _)| *depth))
            .collect();
        for depth in &touched {
            self.dungeon_doors.remove(&(id.to_string(), *depth));
        }
        for (depth, door_id) in doors {
            self.dungeon_doors
                .entry((id.to_string(), *depth))
                .or_default()
                .insert(*door_id);
        }
        for depth in touched {
            self.rebuild_dungeon_floor(id, depth);
        }
    }

    pub fn set_dungeon_door(&mut self, id: &str, depth: u8, door_id: u32, is_open: bool) {
        let set = self
            .dungeon_doors
            .entry((id.to_string(), depth))
            .or_default();
        // Re-broadcasts are common; rebuilding a floor's 6400 cells under the
        // shared write lock for a state we already hold is not worth it.
        let changed = if is_open {
            set.insert(door_id)
        } else {
            set.remove(&door_id)
        };
        if changed {
            self.rebuild_dungeon_floor(id, depth);
        }
    }

    pub fn set_dungeon_broken_props(&mut self, id: &str, depth: u8, broken: Vec<u32>) {
        let key = (id.to_string(), depth);
        if self.dungeon_broken_props.get(&key) == Some(&broken) {
            return;
        }
        self.dungeon_broken_props.insert(key, broken);
        self.rebuild_dungeon_floor(id, depth);
    }

    pub fn add_dungeon_broken_prop(&mut self, id: &str, depth: u8, prop_id: u32) {
        let broken = self
            .dungeon_broken_props
            .entry((id.to_string(), depth))
            .or_default();
        if broken.contains(&prop_id) {
            return;
        }
        broken.push(prop_id);
        self.rebuild_dungeon_floor(id, depth);
    }

    pub fn set_dungeon_opened_props(&mut self, id: &str, depth: u8, opened: Vec<u32>) {
        self.dungeon_opened_props
            .insert((id.to_string(), depth), opened.into_iter().collect());
    }

    pub fn add_dungeon_opened_prop(&mut self, id: &str, depth: u8, prop_id: u32) {
        self.dungeon_opened_props
            .entry((id.to_string(), depth))
            .or_default()
            .insert(prop_id);
    }

    pub fn remove_dungeon_opened_prop(&mut self, id: &str, depth: u8, prop_id: u32) {
        if let Some(opened) = self.dungeon_opened_props.get_mut(&(id.to_string(), depth)) {
            opened.remove(&prop_id);
        }
    }

    pub fn opened_dungeon_props(&self, id: &str, depth: u8) -> Option<&HashSet<u32>> {
        self.dungeon_opened_props.get(&(id.to_string(), depth))
    }

    /// Broken prop ids for one dungeon floor — `break_prop` checks this before
    /// walking out to a barrel someone already smashed.
    pub fn dungeon_broken_props(&self, id: &str, depth: u8) -> &[u32] {
        self.dungeon_broken_props
            .get(&(id.to_string(), depth))
            .map_or(&[], Vec::as_slice)
    }

    /// Whether a mover can stand in the cell holding `pos` on `floor`. What the
    /// in-room sighting queries use to decide where a prop can be opened from.
    pub fn is_walkable(&self, pos: &Position, floor: u8) -> bool {
        !pathfinding::is_cell_sealed(self.passability_cache(), pos.x, pos.z, floor, None)
    }

    /// Recompute one dungeon floor's cells from the live door/prop state
    /// (shared `dungeon::floor_cells`).
    fn rebuild_dungeon_floor(&mut self, id: &str, depth: u8) {
        let Some(dungeon) = self.dungeon_by_id(id) else {
            return;
        };
        let open = self.open_dungeon_doors(id, depth);
        let broken = self
            .dungeon_broken_props
            .get(&(id.to_string(), depth))
            .cloned()
            .unwrap_or_default();
        let Some(cells) = floor_cells(dungeon.layouts(), depth, &broken, Some(&open)) else {
            return;
        };
        set_floor_cells(&mut self.passability_cache, id, depth, cells);
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

/// A ground item the client knows about, with the moment it may be acted
/// on. Everything is visible on arrival except a monster's drop, which
/// waits out `DEATH_DROP_REVEAL_DELAY`.
struct KnownGroundItem {
    item: GroundItem,
    visible_at: std::time::Instant,
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
    /// Per-merchant list of units we sold this session, repurchasable at the
    /// recorded payout (fed by BuybackUpdated/ShopState).
    pub merchant_buyback: HashMap<PlayerId, Vec<onlinerpg_shared::messages::BuybackEntry>>,
    /// Known nearby monsters
    pub nearby_monsters: HashMap<String, Monster>,
    /// Known items lying on the ground, keyed by instance id (from the join
    /// snapshot plus GroundItemSpawned/Appeared/Removed). Read through
    /// `visible_ground_item(s)` — a monster's drop is known before it is
    /// visible.
    ground_items: HashMap<u64, KnownGroundItem>,
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
    /// Our own wire `floor_level`: 0 = overworld, 1..3 housing floors,
    /// negative = dungeon depth. Kept in the protocol's encoding rather than
    /// the passability cache's so it can be put straight into move packets;
    /// `passability_floor()` converts for path queries.
    pub self_floor_level: i8,
    /// Bumped every time the server snaps us back with `PositionCorrected`.
    /// A path that produced a refused step will produce it again, so movers
    /// watch this and abandon the path instead of grinding the same wall.
    pub position_corrections: u32,
    /// The chest we last asked the server to open, until it answers. Opening a
    /// clutter prop is recorded before the answer arrives (an already-claimed
    /// prop is a silent no-op, and without the record we would target it
    /// forever), so a rejection has to take that record back.
    pending_chest_open: Option<(String, u8, crate::dungeon::ChestKind)>,
    /// Dungeons whose treasure chest we have already emptied. The server
    /// refuses the next open until nightfall, so the world state says so
    /// rather than sending us back to a chest that has nothing for us.
    treasure_chests_spent: HashSet<String>,
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
            merchant_buyback: HashMap::new(),
            nearby_monsters: HashMap::new(),
            ground_items: HashMap::new(),
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
            position_corrections: 0,
            pending_chest_open: None,
            treasure_chests_spent: HashSet::new(),
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

    /// Ground height at (x, z) for something standing on passability floor
    /// `floor` — a dungeon floor, or the entrance ramp when `floor` is the
    /// surface. `None` means the dungeons have no say and terrain height wins.
    /// The single answer to "how high is the ground here", so the send path,
    /// the mover and the monster relay cannot drift apart.
    fn dungeon_ground_y(&self, x: f32, z: f32, floor: u8) -> Option<f32> {
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
        self.self_floor_level = floor_level;
        self.send_command(ClientMessage::player_move(position, rotation, floor_level))
            .await
    }

    /// Put an entity on the ground of dungeon floor `floor`, leaving it where
    /// it is when no dungeon covers the spot.
    fn on_dungeon_floor(&self, position: Position, floor: u8) -> Position {
        match self.dungeon_ground_y(position.x, position.z, floor) {
            Some(y) => Position { y, ..position },
            None => position,
        }
    }

    /// The wire `floor_level` to declare while standing at (x, z, y): whichever
    /// floor's grid sits nearest that Y. Deliberately the shared query the
    /// server itself collides against (`authoritative_floor`), so our
    /// declaration and its collision can never resolve differently.
    fn wire_floor_at(&self, x: f32, z: f32, y: f32) -> i8 {
        let world = self.world_cache.read().unwrap();
        floor_level_for_passability(pathfinding::get_floor_at_position(
            world.passability_cache(),
            x,
            z,
            y,
        ))
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
                // On the entrance stairs the wire floor is still 0 while the Y
                // already follows the ramp, so terrain height must not win there.
                let position = if self.self_floor_level == 0
                    && self.dungeon_ground_y(position.x, position.z, 0).is_none()
                {
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
                    floor_level: self.self_floor_level,
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
                // A dungeon monster stands on its floor, not on the terrain
                // above it — snapping those to heightmap Y would haul the whole
                // floor's monsters up to the surface.
                let floor_level = self
                    .nearby_monsters
                    .get(&monster_id)
                    .map(|m| m.floor_level)
                    .unwrap_or(0);
                let (position, target_position) = if floor_level < 0 {
                    let floor = passability_floor_for_level(floor_level);
                    (
                        self.on_dungeon_floor(position, floor),
                        self.on_dungeon_floor(target_position, floor),
                    )
                } else {
                    // position and target_position are independent coordinates, so
                    // sample both terrain heights concurrently rather than serially.
                    tokio::join!(
                        self.snap_position_to_ground(position, "MonsterMove"),
                        self.snap_position_to_ground(target_position, "MonsterMove target"),
                    )
                };
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

    /// Apply an authoritative player pose. Supersedes whatever move that player
    /// had buffered, which `drain_events` would otherwise replay after us.
    fn apply_player_pose(
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

    /// The server put us somewhere we did not walk to — a refused step, a
    /// return scroll, a respawn. Adopting the pose is not enough: the mover
    /// watches `position_corrections` to drop the path it was walking.
    fn relocate_self(&mut self, position: Position, rotation: f32, floor_level: i8) {
        if let Some(ref mut p) = self.self_player {
            p.position = position;
            p.rotation = rotation;
            p.floor_level = floor_level;
        }
        self.self_floor_level = floor_level;
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

    /// Remember an item on the ground, visible from `visible_at` on.
    pub(crate) fn remember_ground_item(
        &mut self,
        item: GroundItem,
        visible_at: std::time::Instant,
    ) {
        self.ground_items
            .insert(item.instance_id, KnownGroundItem { item, visible_at });
    }

    /// One ground item by instance id, if it is visible yet.
    pub fn visible_ground_item(&self, instance_id: u64) -> Option<&GroundItem> {
        self.ground_items
            .get(&instance_id)
            .filter(|k| k.visible_at <= std::time::Instant::now())
            .map(|k| &k.item)
    }

    /// The ground items the agent can act on: visible, on its floor, inside
    /// the sight radius, closest first. The known-item map reaches out to
    /// the server's event radius, so this is what "nearby" means everywhere
    /// downstream — the world state listing and pickup alike.
    pub fn ground_items_in_sight(&self) -> Vec<(f32, &GroundItem)> {
        let Some(sp) = self.self_player.as_ref() else {
            return Vec::new();
        };
        let now = std::time::Instant::now();
        let sight_sq = NPC_SIGHT_RADIUS * NPC_SIGHT_RADIUS;
        let mut in_sight: Vec<_> = self
            .ground_items
            .values()
            .filter(|k| k.visible_at <= now && k.item.floor_level == self.self_floor_level)
            .filter_map(|k| {
                let d_sq = k.item.position.dist_xz_sq(&sp.position);
                (d_sq <= sight_sq).then_some((d_sq, &k.item))
            })
            .collect();
        in_sight.sort_by(|a, b| a.0.total_cmp(&b.0));
        in_sight
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
            | ServerMessage::GroundItemSpawned { .. }
            | ServerMessage::GroundItemAppeared { .. }
            | ServerMessage::GroundItemRemoved { .. }
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
            | ServerMessage::PlayerHealthUpdate { .. }
            | ServerMessage::PlayerTorchToggled { .. } => EventUrgency::Routine,

            // Being relocated invalidates our walk targets and floor
            // assumptions; someone else being relocated does not.
            ServerMessage::PlayerTeleported { player_id, .. } => {
                if self_id == Some(player_id) {
                    EventUrgency::Urgent
                } else {
                    EventUrgency::Noise
                }
            }
            ServerMessage::PlayerRespawned { player } => {
                if self_id == Some(&player.id) {
                    EventUrgency::Urgent
                } else {
                    EventUrgency::Routine
                }
            }

            // Noise: high-frequency, irrelevant, or housing updates
            ServerMessage::PlayerMoved { .. }
            | ServerMessage::MonsterMoved { .. }
            | ServerMessage::GameTimeSync { .. }
            | ServerMessage::HouseSpawned { .. }
            | ServerMessage::HousesInArea { .. }
            | ServerMessage::HouseUpdated { .. }
            | ServerMessage::HouseRemoved { .. }
            | ServerMessage::DoorToggled { .. } => EventUrgency::Noise,

            // A refused interaction should reach the LLM at poll priority, not
            // sink to the idle queue behind everything else.
            ServerMessage::InteractionRejected { .. }
            | ServerMessage::PlayerAttackRejected { .. } => EventUrgency::Routine,

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
                // A character saved underground rejoins there (the server
                // rehydrates it), so adopt the floor instead of assuming 0.
                self.self_floor_level = player.floor_level;
                self.request_dungeon_doors_here();
            }
            ServerMessage::PositionCorrected {
                position,
                rotation,
                floor_level,
            } => {
                self.relocate_self(*position, *rotation, *floor_level);
            }
            ServerMessage::PlayerTeleported {
                player_id,
                position,
                rotation,
                floor_level,
            } => {
                if self.self_player_id.as_ref() == Some(player_id) {
                    self.relocate_self(*position, *rotation, *floor_level);
                }
                self.apply_player_pose(player_id, *position, *rotation, *floor_level);
            }
            ServerMessage::PlayerRespawned { player } => {
                if self.self_player_id.as_ref() == Some(&player.id) {
                    self.self_player = Some(player.clone());
                    self.relocate_self(player.position, player.rotation, player.floor_level);
                }
                if let Some(p) = self.nearby_players.get_mut(&player.id) {
                    *p = player.clone();
                }
                self.latest_player_moves.remove(&player.id);
            }
            ServerMessage::DungeonDoorsState {
                ref entrance_id,
                ref doors,
            } => {
                self.world_cache
                    .write()
                    .unwrap()
                    .set_dungeon_doors(entrance_id, doors);
            }
            ServerMessage::DungeonDoorToggled {
                ref entrance_id,
                depth,
                door_id,
                is_open,
            } => {
                self.world_cache.write().unwrap().set_dungeon_door(
                    entrance_id,
                    *depth,
                    *door_id,
                    *is_open,
                );
            }
            ServerMessage::DungeonPropsState {
                ref entrance_id,
                depth,
                ref broken,
                ref opened,
            } => {
                let mut cache = self.world_cache.write().unwrap();
                cache.set_dungeon_broken_props(entrance_id, *depth, broken.clone());
                cache.set_dungeon_opened_props(entrance_id, *depth, opened.clone());
            }
            ServerMessage::DungeonPropOpened {
                ref entrance_id,
                depth,
                prop_id,
            } => {
                self.world_cache.write().unwrap().add_dungeon_opened_prop(
                    entrance_id,
                    *depth,
                    *prop_id,
                );
                self.pending_chest_open = None;
            }
            // Our own open landed: the chest owes us nothing until nightfall.
            ServerMessage::DungeonChestOpened {
                ref entrance_id,
                player_id,
                ..
            } if self.self_player_id.as_ref() == Some(player_id) => {
                self.treasure_chests_spent.insert(entrance_id.clone());
                self.pending_chest_open = None;
            }
            // A rejection means the open we recorded never happened. The agent
            // sends no other interaction, so a pending open owns this reason.
            ServerMessage::InteractionRejected { ref reason } => {
                if let Some((entrance_id, depth, kind)) = self.pending_chest_open.take() {
                    match kind {
                        crate::dungeon::ChestKind::Prop(prop_id) => {
                            self.world_cache
                                .write()
                                .unwrap()
                                .remove_dungeon_opened_prop(&entrance_id, depth, prop_id);
                        }
                        // "The chest is empty (it refills at nightfall)" — the
                        // other refusals (boss alive, too far) are ours to fix.
                        crate::dungeon::ChestKind::Treasure if reason.contains("empty") => {
                            self.treasure_chests_spent.insert(entrance_id);
                        }
                        crate::dungeon::ChestKind::Treasure => {}
                    }
                }
            }
            ServerMessage::DungeonPropBroken {
                ref entrance_id,
                depth,
                prop_id,
                ..
            } => {
                self.world_cache.write().unwrap().add_dungeon_broken_prop(
                    entrance_id,
                    *depth,
                    *prop_id,
                );
            }
            ServerMessage::BuybackUpdated {
                merchant_player_id,
                ref buyback,
            } => {
                self.merchant_buyback
                    .insert(*merchant_player_id, buyback.clone());
            }
            ServerMessage::ShopState {
                merchant_player_id,
                ref buyback,
                ..
            } => {
                self.merchant_buyback
                    .insert(*merchant_player_id, buyback.clone());
            }
            ServerMessage::GameState {
                players,
                monsters,
                ground_items,
            } => {
                self.nearby_players = players.iter().map(|p| (p.id, p.clone())).collect();
                self.nearby_monsters = monsters.clone();
                // Snapshot items are already lying there for everyone.
                self.ground_items.clear();
                let now = std::time::Instant::now();
                for item in ground_items {
                    self.remember_ground_item(item.clone(), now);
                }
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
            ServerMessage::GroundItemSpawned {
                item,
                source_monster_id,
            } => {
                // A kill's drop only lands on a web client's screen when the
                // killing blow's impact frame plays; hold it the same span.
                let delay = if source_monster_id.is_some() {
                    DEATH_DROP_REVEAL_DELAY
                } else {
                    std::time::Duration::ZERO
                };
                self.remember_ground_item(item.clone(), std::time::Instant::now() + delay);
            }
            ServerMessage::GroundItemAppeared { item } => {
                self.remember_ground_item(item.clone(), std::time::Instant::now());
            }
            ServerMessage::GroundItemRemoved { instance_id } => {
                self.ground_items.remove(instance_id);
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
                self.monster_ai
                    .apply_authoritative_position(monster_id, *position);
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
            // Ground items churn in and out of the AOI as everyone moves;
            // the world state lists what is nearby each turn instead.
            ServerMessage::GroundItemSpawned { .. }
            | ServerMessage::GroundItemAppeared { .. }
            | ServerMessage::GroundItemRemoved { .. } => return urgency,
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
    fn format_dungeon_state(&self) -> Option<String> {
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
            return Some(line);
        }
        let dungeon = self
            .world_cache
            .read()
            .unwrap()
            .nearest_dungeon(p.position.x, p.position.z)?;
        let dist = crate::geom::PlanarDelta::between(&p.position, &dungeon.entrance).dist;
        Some(format!(
            "Dungeon: {} entrance at ({:.0}, {:.0}), {dist:.0}m away, {} floors deep",
            dungeon.name,
            dungeon.entrance.x,
            dungeon.entrance.z,
            dungeon.max_depth()
        ))
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

    /// Like [`Self::find_carried`], but bag items win over worn ones —
    /// selling should reach the spare in the bag, not the equipped copy.
    ///
    /// `spent` counts the units of each instance already given away earlier
    /// this turn, keyed by instance id: the bag snapshot only refreshes when
    /// InventoryUpdated arrives, and a stack survives a sale one unit at a
    /// time, so an instance stays sellable until its whole quantity is gone.
    pub fn find_carried_bag_first(
        &self,
        asked: &str,
        spent: &HashMap<u64, u32>,
    ) -> Option<(String, Carried)> {
        let (id, placed) = self.find_carried(asked)?;
        if let Some(item) = self.self_bag.iter().find(|i| {
            i.item_def_id == id && spent.get(&i.instance_id).copied().unwrap_or(0) < i.quantity
        }) {
            return Some((id, Carried::InBag(item.instance_id)));
        }
        match placed {
            Carried::Worn(_) => Some((id, placed)),
            // Every bag copy was already spent this turn.
            Carried::InBag(_) => None,
        }
    }

    /// Find the item the agent named among the ones we carry, and where it
    /// sits. Matching is forgiving about the exact id (see
    /// `item_defs::resolve_named`) but never reaches past what we hold.
    pub fn find_carried(&self, asked: &str) -> Option<(String, Carried)> {
        let ids: Vec<&str> = self
            .self_bag
            .iter()
            .chain(self.self_equipped.values())
            .map(|i| i.item_def_id.as_str())
            .collect();
        let id = crate::item_defs::resolve_named(&ids, asked)?;
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
        if let Some(line) = self.format_dungeon_state() {
            lines.push(line);
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
            if p.is_official_npc {
                if let Some(shop) = crate::shop_info::shop_line_for(&p.name) {
                    lines.push(shop);
                }
            }
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

        // Items on the ground. Drops linger for minutes, so a busy hunting
        // ground would otherwise stack dozens of lines into every prompt.
        let ground = self.ground_items_in_sight();
        let hidden = ground.len().saturating_sub(MAX_LISTED_GROUND_ITEMS);
        for (d_sq, i) in ground.into_iter().take(MAX_LISTED_GROUND_ITEMS) {
            lines.push(format!(
                "Item on ground: {} ({:.1}m away) [id {}]",
                i.item_def_id,
                d_sq.sqrt(),
                i.instance_id
            ));
        }
        if hidden > 0 {
            lines.push(format!("(and {hidden} more items further away)"));
        }

        if lines.is_empty() {
            "No state available yet.".to_string()
        } else {
            lines.join("\n")
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    struct NoTiles;

    #[async_trait::async_trait]
    impl onlinerpg_terrain::height::HeightTiles for NoTiles {
        async fn read_heightmap(&self, _tx: i32, _tz: i32) -> std::io::Result<Vec<u8>> {
            Err(std::io::Error::other("no terrain in tests"))
        }
    }

    pub(crate) fn test_state() -> (SharedState, mpsc::Receiver<ClientMessage>) {
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

    pub(crate) fn p(x: f32, y: f32, z: f32) -> Position {
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

    pub(crate) fn ground_item(id: u64, def: &str, x: f32, z: f32, floor: i8) -> GroundItem {
        GroundItem {
            instance_id: id,
            item_def_id: def.to_string(),
            position: p(x, 0.0, z),
            floor_level: floor,
            enchant: 0,
        }
    }

    pub(crate) fn test_player(x: f32, z: f32) -> Player {
        Player {
            id: PlayerId::from(1),
            name: "Me".to_string(),
            position: p(x, 0.0, z),
            rotation: 0.0,
            level: 1,
            health: 10,
            max_health: 10,
            class: onlinerpg_shared::CharacterClass::Rogue,
            gender: Default::default(),
            is_official_npc: false,
            torch_on: false,
            floor_level: 0,
            object_type: None,
            object_id: None,
            last_combat_at: 0,
            client_kind: Default::default(),
        }
    }

    /// The world state lists reachable ground items closest first, and
    /// leaves out other floors and anything out of sight.
    #[test]
    fn world_state_lists_nearby_ground_items() {
        let (mut s, _rx) = test_state();
        s.self_player = Some(test_player(0.0, 0.0));
        for item in [
            ground_item(1, "small_sword", 5.0, 0.0, 0),
            ground_item(2, "wooden_shield", 2.0, 0.0, 0),
            ground_item(3, "coin_pile", 1.0, 0.0, 0),
            ground_item(4, "iron_sword", 0.0, NPC_SIGHT_RADIUS + 5.0, 0),
            ground_item(5, "healing_potion", 3.0, 0.0, 1),
        ] {
            s.remember_ground_item(item, std::time::Instant::now());
        }

        let lines: Vec<String> = s
            .format_world_state()
            .lines()
            .filter(|l| l.starts_with("Item on ground:"))
            .map(str::to_string)
            .collect();

        assert_eq!(
            lines,
            vec![
                "Item on ground: coin_pile (1.0m away) [id 3]",
                "Item on ground: wooden_shield (2.0m away) [id 2]",
                "Item on ground: small_sword (5.0m away) [id 1]",
            ]
        );
    }

    /// A kill's drop is held back for as long as a web client holds the
    /// death animation, so the agent can't loot what nobody can see yet.
    /// Items that were already lying there show up at once.
    #[test]
    fn a_monster_drop_waits_for_the_death_impact() {
        let (mut s, _rx) = test_state();
        s.self_player = Some(test_player(0.0, 0.0));

        s.push_event(ServerMessage::GroundItemSpawned {
            item: ground_item(1, "goblin_sword", 2.0, 0.0, 0),
            source_monster_id: Some("m208_129".to_string()),
        });
        s.push_event(ServerMessage::GroundItemAppeared {
            item: ground_item(2, "small_sword", 2.0, 0.0, 0),
        });

        let ids: Vec<u64> = s
            .ground_items_in_sight()
            .iter()
            .map(|(_, i)| i.instance_id)
            .collect();
        assert_eq!(ids, vec![2]);
        assert!(s.visible_ground_item(1).is_none());
        assert!(!s.format_world_state().contains("goblin_sword"));

        // Once the impact has passed it is loot like any other.
        s.remember_ground_item(
            ground_item(1, "goblin_sword", 2.0, 0.0, 0),
            std::time::Instant::now(),
        );
        assert!(s.visible_ground_item(1).is_some());
    }

    /// A field strewn with drops is summarised, not listed line by line.
    #[test]
    fn world_state_caps_the_ground_item_list() {
        let (mut s, _rx) = test_state();
        s.self_player = Some(test_player(0.0, 0.0));
        for id in 1..=(MAX_LISTED_GROUND_ITEMS as u64 + 3) {
            let item = ground_item(id, "small_sword", id as f32 * 0.5, 0.0, 0);
            s.remember_ground_item(item, std::time::Instant::now());
        }

        let world = s.format_world_state();
        let listed = world
            .lines()
            .filter(|l| l.starts_with("Item on ground:"))
            .count();

        assert_eq!(listed, MAX_LISTED_GROUND_ITEMS);
        assert!(world.contains("(and 3 more items further away)"), "{world}");
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

    /// The server's `FLOOR_Y_TOLERANCE`: how far a declared dungeon floor may
    /// sit from the Y we send before `validated_dungeon_floor` refuses it.
    const SERVER_FLOOR_Y_TOLERANCE: f32 = 2.5;

    fn dungeon_state() -> (
        SharedState,
        Arc<crate::dungeon::Dungeon>,
        mpsc::Receiver<ClientMessage>,
    ) {
        let mut cache = WorldCache::new();
        cache.register_dungeons();
        let world = Arc::new(std::sync::RwLock::new(cache));
        let dungeon = world.read().unwrap().dungeon_at(-1450.0, 4720.0).unwrap();
        let (tx, rx) = mpsc::channel(64);
        let mut state = SharedState::new(
            Vec::new(),
            tx,
            Arc::new(HeightSampler::new(NoTiles)),
            world,
            None,
        );
        state.self_player = Some(Player {
            position: dungeon.entrance,
            ..test_player(0.0, 0.0)
        });
        (state, dungeon, rx)
    }

    /// The floor we declare must follow our height: the server derives the
    /// floor it collides against from the Y we send and validates the
    /// declaration against it, so the two have to resolve identically.
    #[test]
    fn declared_floor_tracks_height() {
        let (s, dungeon, _rx) = dungeon_state();
        let (x, z) = (dungeon.entrance.x, dungeon.entrance.z);

        assert_eq!(s.wire_floor_at(x, z, dungeon.entrance.y), 0);
        assert_eq!(s.wire_floor_at(x, z, dungeon.floor_y(1)), -1);
        assert_eq!(s.wire_floor_at(x, z, dungeon.floor_y(3)), -3);
        // Mid-ramp resolves to whichever floor is nearer, never past the last.
        assert_eq!(s.wire_floor_at(x, z, dungeon.entrance.y - 1.0), 0);
        assert_eq!(s.wire_floor_at(x, z, dungeon.entrance.y - 3.0), -1);
        let deepest = dungeon.max_depth();
        assert_eq!(
            s.wire_floor_at(x, z, dungeon.floor_y(deepest) - 50.0),
            -(deepest as i8)
        );
    }

    /// Chest sightings run off the live passability, so the cell they tell the
    /// mover to stand on must be one A* can actually route to — a clutter prop
    /// is a sealed pillar, and aiming at it strands the agent every time.
    #[test]
    fn a_sighted_chest_is_approached_from_a_cell_a_path_can_reach() {
        let (mut s, dungeon, _rx) = dungeon_state();
        let depth = dungeon.max_depth();
        let layout = dungeon.layouts().last().unwrap();
        let cell = layout.chest.unwrap();
        let room = layout.room_at(cell.0, cell.1).unwrap();
        let stand = onlinerpg_shared::dungeon::cell_center(&dungeon.entrance, depth, room.center());
        let floor = dungeon.passability_floor(depth);

        s.self_floor_level = -(depth as i8);
        s.self_player = Some(Player {
            position: stand,
            floor_level: -(depth as i8),
            ..test_player(stand.x, stand.z)
        });

        let chests = s.chests_in_sight();
        assert!(
            chests
                .iter()
                .any(|c| c.kind == crate::dungeon::ChestKind::Treasure),
            "the chest room should show its treasure chest"
        );
        assert!(
            chests.len() > 1,
            "old_crypt's chest room also holds a clutter chest"
        );
        for chest in chests {
            let a = chest.approach;
            assert!(
                s.world_cache.read().unwrap().is_walkable(&a, floor),
                "{:?} is approached from a sealed cell",
                chest.kind
            );
            assert!(
                s.find_path_to(a.x, a.z, floor).found,
                "{:?} has no route to its approach cell",
                chest.kind
            );
        }
    }

    /// Breakables are offered off the live passability the same way chests
    /// are, and a smashed one drops out of the listing.
    #[tokio::test]
    async fn a_smashed_prop_stops_being_offered() {
        let (mut s, dungeon, _rx) = dungeon_state();
        let depth = in_the_chest_room(&mut s, &dungeon);
        let floor = dungeon.passability_floor(depth);
        let prop = s
            .breakables_in_sight()
            .first()
            .copied()
            .expect("the chest room holds breakable clutter");
        assert!(
            s.find_path_to(prop.approach.x, prop.approach.z, floor)
                .found,
            "a prop is offered with a cell we can route to"
        );

        s.push_event(ServerMessage::DungeonPropBroken {
            entrance_id: dungeon.id.clone(),
            depth,
            prop_id: prop.prop_id,
        });
        assert!(!s
            .breakables_in_sight()
            .iter()
            .any(|b| b.prop_id == prop.prop_id));
    }

    /// A stack survives a sale one unit at a time, so the units left in it
    /// have to stay sellable for the rest of the turn. A drop takes the lot.
    #[test]
    fn selling_off_a_stack_leaves_the_rest_of_it_reachable() {
        let (mut s, _rx) = test_state();
        s.self_bag = vec![onlinerpg_shared::inventory::ItemInstance {
            instance_id: 7,
            item_def_id: "healing_potion".to_string(),
            quantity: 3,
            enchant: 0,
        }];

        let mut spent: HashMap<u64, u32> = HashMap::new();
        for unit in 1..=3 {
            let placed = s
                .find_carried_bag_first("healing_potion", &spent)
                .unwrap_or_else(|| panic!("unit {unit} of 3 should still be in the bag"))
                .1;
            assert!(matches!(placed, Carried::InBag(7)));
            *spent.entry(7).or_default() += 1;
        }
        assert!(s.find_carried_bag_first("healing_potion", &spent).is_none());

        let dropped = HashMap::from([(7, u32::MAX)]);
        assert!(s
            .find_carried_bag_first("healing_potion", &dropped)
            .is_none());
    }

    /// Standing where the chest room is, on the deepest floor.
    fn in_the_chest_room(s: &mut SharedState, dungeon: &crate::dungeon::Dungeon) -> u8 {
        let depth = dungeon.max_depth();
        let layout = dungeon.layouts().last().unwrap();
        let cell = layout.chest.unwrap();
        let room = layout.room_at(cell.0, cell.1).unwrap();
        let stand = onlinerpg_shared::dungeon::cell_center(&dungeon.entrance, depth, room.center());
        s.self_floor_level = -(depth as i8);
        s.self_player = Some(Player {
            position: stand,
            floor_level: -(depth as i8),
            ..test_player(stand.x, stand.z)
        });
        depth
    }

    /// A clutter prop is marked opened before the server answers, because an
    /// already-claimed one answers with silence. A rejection says it never
    /// opened, so the mark has to come back off — otherwise a chest the agent
    /// merely stood too far from is invisible for the rest of the floor.
    #[tokio::test]
    async fn a_rejected_prop_open_becomes_visible_again() {
        let (mut s, dungeon, _rx) = dungeon_state();
        let depth = in_the_chest_room(&mut s, &dungeon);
        let prop = match s
            .chests_in_sight()
            .into_iter()
            .find(|c| matches!(c.kind, crate::dungeon::ChestKind::Prop(_)))
            .expect("the chest room holds a clutter chest")
            .kind
        {
            crate::dungeon::ChestKind::Prop(id) => id,
            _ => unreachable!(),
        };

        s.chest_open_sent(&dungeon.id, depth, crate::dungeon::ChestKind::Prop(prop));
        assert!(
            !s.chests_in_sight()
                .iter()
                .any(|c| c.kind == crate::dungeon::ChestKind::Prop(prop)),
            "a sent open hides the chest so we stop targeting it"
        );

        s.push_event(ServerMessage::InteractionRejected {
            reason: "Too far from the chest".to_string(),
        });
        assert!(
            s.chests_in_sight()
                .iter()
                .any(|c| c.kind == crate::dungeon::ChestKind::Prop(prop)),
            "a refused open leaves the chest there to try again"
        );
    }

    /// An emptied treasure chest still stands there, so it keeps showing — but
    /// the line says it has nothing left, or the agent walks back to it all
    /// night for the same refusal.
    #[tokio::test]
    async fn an_emptied_treasure_chest_says_so() {
        let (mut s, dungeon, _rx) = dungeon_state();
        let depth = in_the_chest_room(&mut s, &dungeon);
        assert!(!s.format_world_state().contains("refills at nightfall"));

        s.chest_open_sent(&dungeon.id, depth, crate::dungeon::ChestKind::Treasure);
        s.push_event(ServerMessage::InteractionRejected {
            reason: "The chest is empty (it refills at nightfall)".to_string(),
        });

        let world = s.format_world_state();
        assert!(
            world.contains("a great chest standing alone")
                && world.contains("you emptied it; it refills at nightfall"),
            "{world}"
        );
    }

    /// Registering the dungeon is all the shared A* needs to walk the entrance
    /// stairwell: a path from the surface to floor 1 must exist and end there.
    #[test]
    fn a_path_leads_from_the_entrance_down_to_the_first_floor() {
        let (s, dungeon, _rx) = dungeon_state();
        let landing = dungeon.arrival_position(1).unwrap();
        let floor = dungeon.passability_floor(1);

        let path = s.find_path_to(landing.x, landing.z, floor);

        assert!(path.found, "no route from the entrance down to floor 1");
        assert_eq!(path.waypoints.last().map(|w| w.floor), Some(floor));
    }

    /// Every step of that descent must declare a floor the server accepts and
    /// collides against identically — it derives the floor from the Y we send,
    /// so a step whose declaration and height disagree gets snapped back.
    #[test]
    fn descending_steps_declare_a_floor_the_server_accepts() {
        let (mut s, dungeon, _rx) = dungeon_state();
        let landing = dungeon.arrival_position(1).unwrap();
        let path = s.find_path_to(landing.x, landing.z, dungeon.passability_floor(1));
        assert!(path.found);

        let mut seen_underground = false;
        for wp in &path.waypoints {
            // Mirror the mover: subdivide the leg and pose each step.
            loop {
                let position = s.self_player.as_ref().unwrap().position;
                let to_wp = crate::geom::PlanarDelta::to_xz(&position, wp.x, wp.z);
                if to_wp.dist < 0.1 {
                    break;
                }
                let (sx, sz) = if to_wp.dist <= 3.0 {
                    (wp.x, wp.z)
                } else {
                    let r = 3.0 / to_wp.dist;
                    (position.x + to_wp.dx * r, position.z + to_wp.dz * r)
                };
                let (pose, floor_level) = s.step_pose(sx, sz, wp.floor, position.y);
                if floor_level < 0 {
                    seen_underground = true;
                    let expected = dungeon.floor_y(floor_level.unsigned_abs());
                    assert!(
                        (pose.y - expected).abs() <= SERVER_FLOOR_Y_TOLERANCE,
                        "floor {floor_level} declared at y={} (floor sits at {expected})",
                        pose.y
                    );
                }
                s.self_player.as_mut().unwrap().position = pose;
                s.self_floor_level = floor_level;
            }
        }

        assert!(seen_underground, "the walk never went underground");
        assert_eq!(s.self_floor_level, -1);
    }

    /// A point partway down the entrance ramp, low enough to read as floor 1.
    fn mid_shaft_point(dungeon: &crate::dungeon::Dungeon) -> (f32, f32, f32) {
        let e = dungeon.entrance;
        // Past the ramp's midpoint (so the nearest floor is the one below) but
        // short of the bottom landing.
        let low = dungeon.floor_y(1) + 0.5;
        let high = (e.y + dungeon.floor_y(1)) / 2.0 - 0.2;
        let mut step = 0;
        while step < 80 * 80 {
            let x = e.x - 20.0 + (step % 80) as f32 * 0.5;
            let z = e.z - 20.0 + (step / 80) as f32 * 0.5;
            step += 1;
            if let Some(y) = dungeon.ground_y(0, x, z) {
                if y > low && y < high {
                    return (x, z, y);
                }
            }
        }
        panic!("no mid-ramp point found on the entrance shaft");
    }

    /// Re-pathing from halfway down the stairs (after a fight or a correction)
    /// must still work: those cells are keyed to the floor above, so searching
    /// under the floor we are nearest would strand the agent on the steps.
    #[test]
    fn a_path_still_leads_on_from_halfway_down_the_stairs() {
        let (mut s, dungeon, _rx) = dungeon_state();
        let (x, z, y) = mid_shaft_point(&dungeon);
        s.self_player.as_mut().unwrap().position = Position { x, y, z };
        s.self_floor_level = s.wire_floor_at(x, z, y);

        assert_eq!(s.self_floor_level, -1, "mid-ramp should read as floor 1");
        assert_eq!(
            s.passability_floor(),
            0,
            "stair cells are keyed one floor up"
        );

        let landing = dungeon.arrival_position(1).unwrap();
        let path = s.find_path_to(landing.x, landing.z, dungeon.passability_floor(1));
        assert!(path.found, "no route on from the middle of the stairs");
    }

    /// The stairs down sit behind shut doors on most floors, so opening one has
    /// to reopen the cells A* walks — otherwise the agent never gets past
    /// floor 1 no matter how many doors it toggles.
    #[test]
    fn opening_a_door_reopens_the_route_behind_it() {
        let (mut s, dungeon, _rx) = dungeon_state();
        let landing = dungeon.arrival_position(1).unwrap();
        s.self_player.as_mut().unwrap().position = landing;
        s.self_floor_level = -1;

        let below = dungeon.arrival_position(2).unwrap();
        let goal_floor = dungeon.passability_floor(2);
        assert!(
            !s.find_path_to(below.x, below.z, goal_floor).found,
            "floor 1's stairs down are supposed to start sealed"
        );

        let doors: Vec<(u8, u32)> = dungeon
            .closed_doors(1, &HashSet::new())
            .iter()
            .map(|d| (1u8, d.door_id))
            .collect();
        assert!(!doors.is_empty());
        s.world_cache
            .write()
            .unwrap()
            .set_dungeon_doors(&dungeon.id, &doors);

        assert!(
            s.find_path_to(below.x, below.z, goal_floor).found,
            "the way down stayed sealed after opening floor 1's doors"
        );
    }

    /// A self-teleport must resync position, rotation AND floor, or the client
    /// keeps walking from the stale spot and drags the character back.
    #[test]
    fn a_self_teleport_resyncs_position_and_floor() {
        let (mut s, _rx) = test_state();
        let me = test_player(-1464.5, 4690.5);
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);
        s.self_floor_level = -5;

        s.push_event(ServerMessage::PlayerTeleported {
            player_id: PlayerId::from(1),
            position: p(-1456.0, 1.2, 4735.0),
            rotation: 2.5,
            floor_level: 0,
        });

        assert_eq!(
            s.self_floor_level, 0,
            "teleport must clear the stale dungeon floor"
        );
        let now = s.self_player.as_ref().unwrap();
        assert_eq!(now.position.x, -1456.0);
        assert_eq!(now.position.z, 4735.0);
        assert_eq!(now.rotation, 2.5);
        assert_eq!(now.floor_level, 0);
        assert_eq!(
            s.position_corrections, 1,
            "teleport must abandon any in-flight walk, like PositionCorrected"
        );
    }

    /// A respawn relocates us the same way, just via its own message.
    #[test]
    fn a_self_respawn_resyncs_position_and_floor() {
        let (mut s, _rx) = test_state();
        let me = test_player(-1464.5, 4690.5);
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);
        s.self_floor_level = -5;

        let mut revived = test_player(-1475.0, 4742.0);
        revived.floor_level = 0;
        s.push_event(ServerMessage::PlayerRespawned { player: revived });

        assert_eq!(s.self_floor_level, 0);
        let now = s.self_player.as_ref().unwrap();
        assert_eq!(now.position.x, -1475.0);
        assert_eq!(now.floor_level, 0);
        assert_eq!(s.position_corrections, 1);
    }

    /// Someone else's teleport moves their tracked entry, not ours: mixing the
    /// two would have us chase a neighbour's destination.
    #[test]
    fn a_neighbours_teleport_only_moves_their_entry() {
        let (mut s, _rx) = test_state();
        let me = test_player(-1464.5, 4690.5);
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);

        let mut them = test_player(-1460.0, 4695.0);
        them.id = PlayerId::from(2);
        s.nearby_players.insert(them.id, them);

        s.push_event(ServerMessage::PlayerTeleported {
            player_id: PlayerId::from(2),
            position: p(-1456.0, 1.2, 4735.0),
            rotation: 0.0,
            floor_level: -3,
        });

        let them = &s.nearby_players[&PlayerId::from(2)];
        assert_eq!(them.position.x, -1456.0);
        assert_eq!(them.floor_level, -3);
        assert_eq!(s.self_player.as_ref().unwrap().position.x, -1464.5);
        assert_eq!(s.self_floor_level, 0);
        assert_eq!(
            s.position_corrections, 0,
            "a neighbour's teleport must not abandon our path"
        );
    }

    /// A dungeon monster's moves must keep its floor's height. Terrain snapping
    /// would haul the whole floor's monsters up to the surface.
    #[tokio::test]
    async fn dungeon_monster_moves_keep_their_floor_height() {
        let (mut s, dungeon, _rx) = dungeon_state();
        let landing = dungeon.arrival_position(2).unwrap();
        let mut m = monster("m1");
        m.floor_level = -2;
        m.position = landing;
        s.nearby_monsters.insert("m1".to_string(), m);

        s.send_command(ClientMessage::MonsterMove {
            monster_id: "m1".to_string(),
            position: p(landing.x, 999.0, landing.z),
            rotation: 0.0,
            state: MonsterState::Run,
            target_position: p(landing.x, 999.0, landing.z),
        })
        .await
        .unwrap();

        let y = s.nearby_monsters["m1"].position.y;
        assert!(
            (y - dungeon.floor_y(2)).abs() < 0.01,
            "monster ended at y={y}, floor 2 sits at {}",
            dungeon.floor_y(2)
        );
    }
}
