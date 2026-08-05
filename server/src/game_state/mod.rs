use crate::housing::HousingIO;
use crate::item_defs::ItemDefs;
use crate::monster_defs::MonsterDefs;
use crate::types::{CharacterAttributes, Player, PlayerId, ServerMessage};
use bytes::Bytes;
use onlinerpg_shared::housing::{HouseData, RoomData, WallDirection};
use onlinerpg_shared::inventory::PlayerInventory;
use onlinerpg_shared::messages::BuybackEntry;

/// A buyback entry plus the wall-clock deadline after which it is dropped.
/// The expiry is server-side only — `BuybackEntry` is the wire type.
#[derive(Debug, Clone)]
pub struct StoredBuyback {
    pub entry: BuybackEntry,
    pub expires_at_ms: u64,
}

impl StoredBuyback {
    pub fn is_live(&self, now_ms: u64) -> bool {
        self.expires_at_ms > now_ms
    }
}
use onlinerpg_shared::serialize_server_msg;
use onlinerpg_shared::NoSpawnZone;
use onlinerpg_shared::Position;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, mpsc, Mutex, RwLock};
use tracing::{error, warn};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DoorKey {
    house_id: String,
    room_index: u32,
    wall_dir: WallDirection,
    segment_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SpatialCell {
    x: i32,
    z: i32,
}

const PLAYER_SPATIAL_CELL_SIZE: f32 = EVENT_DELIVERY_RADIUS;

impl SpatialCell {
    fn from_position(position: &Position) -> Self {
        Self {
            x: (position.x / PLAYER_SPATIAL_CELL_SIZE).floor() as i32,
            z: (position.z / PLAYER_SPATIAL_CELL_SIZE).floor() as i32,
        }
    }

    /// Every cell overlapping the given XZ box, by integer cell range —
    /// exact, unlike sampling. Callers split a seam-crossing X range into
    /// canonical segments first; the range itself does not wrap.
    fn covering(
        min_x: f32,
        max_x: f32,
        min_z: f32,
        max_z: f32,
    ) -> impl Iterator<Item = SpatialCell> {
        let cell = |v: f32| (v / PLAYER_SPATIAL_CELL_SIZE).floor() as i32;
        let (x0, x1) = (cell(min_x), cell(max_x));
        let (z0, z1) = (cell(min_z), cell(max_z));
        (x0..=x1).flat_map(move |x| (z0..=z1).map(move |z| SpatialCell { x, z }))
    }
}

#[derive(Debug, Clone)]
pub struct BroadcastMessage {
    pub bytes: Bytes,
}

pub type GameStateSender = broadcast::Sender<BroadcastMessage>;
pub type GameStateReceiver = broadcast::Receiver<BroadcastMessage>;

/// Payload of a player's direct channel. Fanout helpers serialize once and
/// share the bytes across recipients; single-recipient sends stay typed so
/// the connection can still inspect them (e.g. `Kicked`).
#[derive(Debug, Clone)]
pub enum DirectMessage {
    Typed(ServerMessage),
    Shared(Bytes),
}

/// The one wire-encode path every outbound message shares; logs and returns
/// `None` on failure.
pub(crate) fn encode_server_msg(msg: &ServerMessage) -> Option<Bytes> {
    match serialize_server_msg(msg) {
        Ok(bytes) => Some(Bytes::from(bytes)),
        Err(e) => {
            error!("Failed to serialize server message: {}", e);
            None
        }
    }
}

mod chat;
pub(crate) use chat::{parse_admin_command, parse_notice_command};
mod combat;
mod deals;
pub(crate) mod fishing;
pub(crate) use deals::band_invariant_holds;
mod dungeon;
pub(crate) mod hunger;
mod inventory;
mod monster;
mod party;
mod passability;
mod player;
pub(crate) use player::{restored_floor_level, MoveCommand};
mod salary;
mod skill_metrics;
mod skills;
pub(crate) use skills::skills_from_rows;
mod time;
mod trading;
pub use trading::BUYBACK_SWEEP_PERIOD;

// Visible crate-wide so tests outside this module (e.g. the login gate in
// `connection`) can reuse the temp-DB and game-state factories.
#[cfg(test)]
pub(crate) mod tests;

pub(crate) const EVENT_DELIVERY_RADIUS: f32 = onlinerpg_shared::EVENT_DELIVERY_RADIUS;

/// How long after the last hit a player still counts as in combat. Gates health
/// regeneration and `/escape` alike, so escaping can't cut a fight short.
pub(crate) const OUT_OF_COMBAT_MS: u64 = 10_000;

/// Item def id for the loose-coin pickup spilled by an opened dungeon chest
/// prop. It never enters a bag — picking it up credits a few copper straight
/// to the player's wallet (see `pickup_item`).
pub(crate) const COIN_PILE_ITEM_ID: &str = "coin_pile";

#[derive(Default)]
struct IdState {
    next_player_number: u32,
    player_numbers: HashMap<PlayerId, u32>,
    owner_spawn_counts: HashMap<u32, u32>,
}

struct AccountSession {
    id: u64,
    player_id: Option<PlayerId>,
    kick_tx: mpsc::UnboundedSender<ServerMessage>,
}

/// Anchor for the game clock: game time = `start_game_seconds` plus scaled
/// real time elapsed since `start_real`. Behind a std RwLock (not tokio)
/// because it is read from sync contexts; writes only happen on debug
/// time jumps.
pub(crate) struct GameClock {
    pub start_real: Instant,
    pub start_game_seconds: i64,
}

/// Server-side ground item with despawn timestamp.
pub(crate) struct ServerGroundItem {
    pub item: onlinerpg_shared::inventory::GroundItem,
    pub dropped_at_ms: u64,
}

#[derive(Clone)]
pub struct GameState {
    players: Arc<RwLock<HashMap<PlayerId, Player>>>,
    /// Lowercased name → online player id, updated by `add_player`/
    /// `remove_player` right after the roster under its own lock (never held
    /// together with another, so momentarily behind `players`). O(1)
    /// case-insensitive name lookups; callers re-validate the id against
    /// `players`.
    player_ids_by_name: Arc<RwLock<HashMap<String, PlayerId>>>,
    movement_intents: Arc<RwLock<HashMap<PlayerId, player::MoveQueue>>>,
    last_player_attacks: Arc<RwLock<HashMap<PlayerId, u64>>>,
    player_spatial_cells: Arc<RwLock<HashMap<SpatialCell, HashSet<PlayerId>>>>,
    monsters: Arc<RwLock<HashMap<String, crate::types::Monster>>>,
    ambient_spawn_allowances: Arc<RwLock<HashMap<(PlayerId, String), u64>>>,
    broadcast_tx: GameStateSender,
    server_notice: Arc<RwLock<Option<String>>>,
    game_clock: Arc<std::sync::RwLock<GameClock>>,
    monster_defs: MonsterDefs,
    item_defs: ItemDefs,
    /// Global rare bonus-drop table shared by every loot source.
    world_drop_defs: crate::world_drop_defs::WorldDropDefs,
    id_state: Arc<RwLock<IdState>>,
    account_sessions: Arc<RwLock<HashMap<String, AccountSession>>>,
    next_account_session: Arc<std::sync::atomic::AtomicU64>,
    direct_channels: Arc<RwLock<HashMap<PlayerId, mpsc::UnboundedSender<DirectMessage>>>>,
    // player_id → (character_id, current_xp, attributes)
    #[allow(clippy::type_complexity)]
    player_characters: Arc<RwLock<HashMap<PlayerId, (i64, u64, CharacterAttributes)>>>,
    /// player_id → current gold (smallest currency unit). Kept out of the
    /// broadcast `Player` struct: gold is private to its owner.
    player_gold: Arc<RwLock<HashMap<PlayerId, i64>>>,
    /// player_id → trained skills. Private to its owner like gold; delivered
    /// via `SkillsUpdate` on join and `SkillXpGained` on change.
    player_skills: Arc<RwLock<HashMap<PlayerId, onlinerpg_shared::skills::Skills>>>,
    /// Players whose skills changed since the last periodic save.
    dirty_skills: Arc<RwLock<HashSet<PlayerId>>>,
    skill_balance_metrics: Arc<skill_metrics::SkillBalanceMetrics>,
    /// Live fishing sessions, one per player, advanced by `tick_fishing`.
    fishing_sessions: Arc<RwLock<HashMap<PlayerId, fishing::FishingSession>>>,
    /// Session count mirror, so the per-move cancel check costs one atomic
    /// load for the non-fishing majority instead of a lock.
    fishing_active: Arc<std::sync::atomic::AtomicUsize>,
    /// Mints per-cast `session_id`s (fishing.rs re-verifies them in the tick).
    next_fishing_session: Arc<std::sync::atomic::AtomicU64>,
    /// Server-side terrain heights (tile-cached). Fishing's water check is
    /// its first gameplay consumer; sampled only in async handlers, never
    /// in ticks.
    height_sampler: Arc<onlinerpg_terrain::height::HeightSampler>,
    /// Server-side unified water surface (sea + rivers, tile-cached). Paired
    /// with `height_sampler` so fishing's water check covers rivers, whose
    /// beds sit above sea level, not just the ocean.
    water_sampler: Arc<onlinerpg_terrain::water::WaterSampler>,
    housing_io: Arc<HousingIO>,
    /// Players whose state has changed since the last periodic save.
    dirty_players: Arc<RwLock<HashSet<PlayerId>>>,
    /// Players whose inventory has changed since the last periodic save.
    dirty_inventories: Arc<RwLock<HashSet<PlayerId>>>,
    /// Players who relocated (or whose party reshaped) since the last
    /// party-position push; the tick maps them to parties, so entries from
    /// partyless players just drop out there.
    party_position_dirty: Arc<RwLock<HashSet<PlayerId>>>,
    /// Serializes periodic and shutdown flushes against per-player logout saves.
    persistence_lock: Arc<Mutex<()>>,
    /// Serializes account replacement and character deletion with game entry.
    character_session_lock: Arc<Mutex<()>>,
    /// In-memory set of currently open doors.
    open_doors: Arc<RwLock<HashSet<DoorKey>>>,
    /// Shared-crate passability cache mirroring what clients build (houses,
    /// solid furniture, dungeons), used to collision-check simulated player
    /// movement. std RwLock: accesses are sync and short.
    passability: Arc<std::sync::RwLock<onlinerpg_shared::pathfinding::PassabilityCache>>,
    /// When each player was last sent a `PositionCorrected`. Only touched when
    /// a correction is sent, and pruned on the refused-move path, so it needs
    /// no disconnect cleanup and stays empty in the normal case.
    last_position_correction: Arc<RwLock<HashMap<PlayerId, Instant>>>,
    /// No-spawn zones (towns, safe areas) from region zone files.
    no_spawn_zones: Vec<NoSpawnZone>,
    /// Player inventories (bag + equipment), keyed by player_id.
    inventories: Arc<RwLock<HashMap<PlayerId, PlayerInventory>>>,
    /// Items dropped on the ground, keyed by instance_id.
    ground_items: Arc<RwLock<HashMap<u64, ServerGroundItem>>>,
    /// Monotonically increasing counter for item instance IDs.
    next_item_instance_id: Arc<RwLock<u64>>,
    /// Live haggled price modifiers granted by LLM NPCs (economy phase 2).
    deals: Arc<RwLock<HashMap<deals::DealKey, deals::DealEntry>>>,
    /// Daily haggling budgets and cooldowns.
    deal_ledgers: Arc<RwLock<deals::DealLedgers>>,
    /// Last game day NPC salaries were paid for; `None` until the first
    /// salary tick after boot.
    npc_salary_last_day: Arc<RwLock<Option<i64>>>,
    /// Dungeon entrance registry (data/dungeons.json).
    dungeon_defs: crate::dungeon_defs::DungeonDefs,
    /// Live dungeon runtimes, keyed by entrance id. Created lazily.
    dungeons: Arc<RwLock<HashMap<String, dungeon::DungeonRuntime>>>,
    /// monster_id → dungeon spawn slot, for respawn bookkeeping on death.
    dungeon_monsters: Arc<RwLock<HashMap<String, dungeon::DungeonMonsterRef>>>,
    /// merchant_player_id → (customer player_id → ticks of hold remaining). A
    /// trading NPC is held in place (its LLM movement is suppressed) while its
    /// entry is non-empty, so it doesn't wander off mid-trade. Each hold
    /// counts down on `tick_shop_holds` so a player can't pin an NPC forever
    /// by keeping the window open. See `register_shop_open`/`close_shop`.
    open_shops: Arc<RwLock<HashMap<PlayerId, HashMap<PlayerId, u8>>>>,
    /// Live parties and pending invites (in-memory; a disconnect is a leave).
    parties: Arc<RwLock<party::Parties>>,
    /// (character_id, merchant npc name) → units that character sold to
    /// that merchant, repurchasable at the recorded payout. Keyed by
    /// character (not the per-session player id) so the list survives a
    /// reconnect. Capped per pair (oldest dropped) and in-memory only.
    /// Entries expire after `BUYBACK_TTL_MS`; reads filter expiry inline and
    /// `tick_buyback_expiry` drops them along with pairs left empty, so the
    /// map stays bounded on a long uptime — nothing else ever removes a key.
    #[allow(clippy::type_complexity)]
    buybacks: Arc<RwLock<HashMap<(i64, String), Vec<StoredBuyback>>>>,
    /// player_id → character names whose chat/whispers this player never
    /// receives (`/block`). Loaded from the DB at login, dropped on logout.
    blocked_names: Arc<RwLock<HashMap<PlayerId, HashSet<String>>>>,
    /// player_id → the character name `/r` replies to (last whisper sent or
    /// received). In-memory only, dropped on logout.
    whisper_partners: Arc<RwLock<HashMap<PlayerId, String>>>,
    /// Lowercased character name → (canonical name, mute expiry). Keyed by
    /// name, not session, so a relog does not clear it; in-memory only, so a
    /// restart does. Expired entries are pruned on mute/unmute and on lookup.
    muted_until: Arc<RwLock<HashMap<String, (String, Instant)>>>,
    /// (character_id, dungeon entrance id) → world clock seconds at that
    /// character's last chest open. Keyed by character (not the per-session
    /// player id) and DB-backed, so the refill gate survives a reconnect and
    /// a restart. Seeded at login; entries are dropped once the night they
    /// record has passed (`claim_chest_open`), not on logout — a logout hook
    /// would race the session replacement that clears it.
    #[allow(clippy::type_complexity)]
    chest_opens: Arc<RwLock<HashMap<(i64, String), i64>>>,
    /// player_id → dungeon entrance ids this character has discovered
    /// (world-map markers). Seeded from the DB at login, dropped on logout;
    /// new discoveries queue in `pending_discovery_saves`.
    dungeon_discoveries: Arc<RwLock<HashMap<PlayerId, HashSet<String>>>>,
    /// (character_id, entrance_id) discoveries awaiting the next
    /// `save_batch`; drained by the periodic flush and the shutdown
    /// snapshot, re-queued if the batch fails.
    pending_discovery_saves: Arc<RwLock<Vec<(i64, String)>>>,
    /// Cell → entrances whose discovery region overlaps it, built once at
    /// startup. `check_dungeon_discovery` looks up the mover's cell before
    /// taking any lock and then tests only the listed entrances, so the
    /// per-move cost stays O(1) however many entrances the registry grows to.
    #[allow(clippy::type_complexity)]
    dungeon_discovery_cells:
        Arc<HashMap<SpatialCell, Vec<&'static crate::dungeon_defs::DungeonEntranceDef>>>,
    /// player_id → satiation + food poisoning (doc/HUNGER.md). Owner-private
    /// like gold; official NPCs have no entry (the exemption).
    hunger: Arc<RwLock<HashMap<PlayerId, hunger::HungerData>>>,
    food_regeneration: Arc<RwLock<HashMap<PlayerId, hunger::FoodRegeneration>>>,
    /// Regen sweep counter: Hungry players heal on alternate sweeps (×0.5).
    regen_ticks: Arc<std::sync::atomic::AtomicU64>,
    /// Lit campfires keyed by id, expired by `tick_campfires`.
    campfires: Arc<RwLock<HashMap<u64, hunger::CampfireEntry>>>,
    /// One grill cast per player, resolved by `tick_grills`.
    grill_sessions: Arc<RwLock<HashMap<PlayerId, hunger::GrillSession>>>,
}

impl GameState {
    /// The `OUT_OF_COMBAT_MS` clock shared by regen, /escape, and summons.
    pub(crate) fn in_combat(player: &Player) -> bool {
        Self::now_ms().saturating_sub(player.last_combat_at) < OUT_OF_COMBAT_MS
    }

    /// Movement, a landed attack, death and disconnect all break cast-type
    /// concentration (fishing, grilling) at the same chokepoints.
    pub(crate) async fn cancel_concentration_if_active(&self, player_id: &PlayerId) {
        self.cancel_fishing_if_active(player_id).await;
        self.cancel_grill_if_active(player_id).await;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        monster_defs: MonsterDefs,
        item_defs: ItemDefs,
        world_drop_defs: crate::world_drop_defs::WorldDropDefs,
        initial_datetime: crate::types::GameDateTime,
        housing_io: Arc<HousingIO>,
        no_spawn_zones: Vec<NoSpawnZone>,
        dungeon_defs: crate::dungeon_defs::DungeonDefs,
        height_sampler: Arc<onlinerpg_terrain::height::HeightSampler>,
        water_sampler: Arc<onlinerpg_terrain::water::WaterSampler>,
    ) -> Self {
        let (broadcast_tx, _) = broadcast::channel(1000);
        let dungeon_discovery_cells = Arc::new(dungeon::discovery_cells(&dungeon_defs));

        Self {
            players: Arc::new(RwLock::new(HashMap::new())),
            player_ids_by_name: Arc::new(RwLock::new(HashMap::new())),
            movement_intents: Arc::new(RwLock::new(HashMap::new())),
            last_player_attacks: Arc::new(RwLock::new(HashMap::new())),
            player_spatial_cells: Arc::new(RwLock::new(HashMap::new())),
            monsters: Arc::new(RwLock::new(HashMap::new())),
            ambient_spawn_allowances: Arc::new(RwLock::new(HashMap::new())),
            broadcast_tx,
            server_notice: Arc::new(RwLock::new(None)),
            game_clock: Arc::new(std::sync::RwLock::new(GameClock {
                start_real: Instant::now(),
                start_game_seconds: Self::datetime_to_total_game_seconds(&initial_datetime),
            })),
            monster_defs,
            item_defs,
            world_drop_defs,
            id_state: Arc::new(RwLock::new(IdState::default())),
            account_sessions: Arc::new(RwLock::new(HashMap::new())),
            next_account_session: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            direct_channels: Arc::new(RwLock::new(HashMap::new())),
            player_characters: Arc::new(RwLock::new(HashMap::new())),
            player_gold: Arc::new(RwLock::new(HashMap::new())),
            player_skills: Arc::new(RwLock::new(HashMap::new())),
            dirty_skills: Arc::new(RwLock::new(HashSet::new())),
            skill_balance_metrics: Arc::new(skill_metrics::SkillBalanceMetrics::default()),
            fishing_sessions: Arc::new(RwLock::new(HashMap::new())),
            fishing_active: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            next_fishing_session: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            height_sampler,
            water_sampler,
            housing_io,
            dirty_players: Arc::new(RwLock::new(HashSet::new())),
            dirty_inventories: Arc::new(RwLock::new(HashSet::new())),
            party_position_dirty: Arc::new(RwLock::new(HashSet::new())),
            persistence_lock: Arc::new(Mutex::new(())),
            character_session_lock: Arc::new(Mutex::new(())),
            open_doors: Arc::new(RwLock::new(HashSet::new())),
            last_position_correction: Arc::new(RwLock::new(HashMap::new())),
            passability: Arc::new(std::sync::RwLock::new(
                onlinerpg_shared::pathfinding::PassabilityCache::new(),
            )),
            no_spawn_zones,
            inventories: Arc::new(RwLock::new(HashMap::new())),
            ground_items: Arc::new(RwLock::new(HashMap::new())),
            next_item_instance_id: Arc::new(RwLock::new(1)),
            deals: Arc::new(RwLock::new(HashMap::new())),
            deal_ledgers: Arc::new(RwLock::new(deals::DealLedgers::default())),
            npc_salary_last_day: Arc::new(RwLock::new(None)),
            dungeon_defs,
            dungeons: Arc::new(RwLock::new(HashMap::new())),
            dungeon_monsters: Arc::new(RwLock::new(HashMap::new())),
            open_shops: Arc::new(RwLock::new(HashMap::new())),
            parties: Arc::new(RwLock::new(party::Parties::default())),
            buybacks: Arc::new(RwLock::new(HashMap::new())),
            blocked_names: Arc::new(RwLock::new(HashMap::new())),
            whisper_partners: Arc::new(RwLock::new(HashMap::new())),
            muted_until: Arc::new(RwLock::new(HashMap::new())),
            chest_opens: Arc::new(RwLock::new(HashMap::new())),
            dungeon_discoveries: Arc::new(RwLock::new(HashMap::new())),
            pending_discovery_saves: Arc::new(RwLock::new(Vec::new())),
            dungeon_discovery_cells,
            hunger: Arc::new(RwLock::new(HashMap::new())),
            food_regeneration: Arc::new(RwLock::new(HashMap::new())),
            regen_ticks: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            campfires: Arc::new(RwLock::new(HashMap::new())),
            grill_sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the no-spawn zones (for sending to clients on join).
    pub fn no_spawn_zones(&self) -> &[NoSpawnZone] {
        &self.no_spawn_zones
    }

    /// Evict terrain tiles idle since the previous sweep from both samplers.
    pub async fn sweep_terrain_caches(&self) -> usize {
        self.height_sampler.sweep_stale_tiles().await + self.water_sampler.sweep_stale_tiles().await
    }

    pub fn subscribe(&self) -> GameStateReceiver {
        self.broadcast_tx.subscribe()
    }

    pub async fn server_notice(&self) -> Option<String> {
        self.server_notice.read().await.clone()
    }

    pub async fn set_server_notice(&self, message: Option<String>) {
        *self.server_notice.write().await = message.clone();
        self.broadcast(ServerMessage::ServerNotice { message });
    }

    pub(crate) fn broadcast(&self, msg: ServerMessage) {
        if let Some(bytes) = encode_server_msg(&msg) {
            let _ = self.broadcast_tx.send(BroadcastMessage { bytes });
        }
    }

    /// Toggle a door's is_open state (in-memory only, no disk I/O).
    /// Validates that the player is within 1.5m (XZ) and on the same floor.
    pub async fn toggle_door(
        &self,
        player_id: &PlayerId,
        house_id: &str,
        room_index: u32,
        wall_dir: WallDirection,
        segment_index: u32,
    ) -> Option<bool> {
        let (player_pos, player_floor) = {
            let players = self.players.read().await;
            let p = players.get(player_id)?;
            (p.position, p.floor_level)
        };

        let house = match self.housing_io.find_house(house_id).await {
            Ok(Some(h)) => h,
            _ => {
                warn!("toggle_door: house {} not found", house_id);
                return None;
            }
        };

        let room = house.rooms.get(room_index as usize)?;

        // Validate door exists
        let seg = room.wall(wall_dir).get(segment_index as usize)?;
        if !seg.variant.is_openable() {
            return None;
        }

        // Validate distance and floor
        if !is_player_near_door(
            room,
            &house.origin,
            wall_dir,
            segment_index,
            &player_pos,
            player_floor,
        ) {
            return None;
        }

        // Toggle in-memory state
        let key = DoorKey {
            house_id: house_id.to_string(),
            room_index,
            wall_dir,
            segment_index,
        };
        let is_open = {
            let mut open_doors = self.open_doors.write().await;
            if open_doors.contains(&key) {
                open_doors.remove(&key);
                false
            } else {
                open_doors.insert(key);
                true
            }
        };

        {
            let mut cache = self.passability_write();
            onlinerpg_shared::pathfinding::update_door_edge(
                &mut cache,
                house_id,
                room,
                wall_dir,
                segment_index as usize,
                is_open,
            );
        }

        Some(is_open)
    }

    /// Stamp in-memory open-door state onto house data before sending it to a
    /// client, so reconnecting players see doors others left open.
    pub async fn apply_open_door_state(&self, houses: &mut [HouseData]) {
        let open_doors = self.open_doors.read().await;
        if open_doors.is_empty() {
            return;
        }
        let mut keys_by_house: HashMap<&str, Vec<&DoorKey>> = HashMap::new();
        for key in open_doors.iter() {
            keys_by_house
                .entry(key.house_id.as_str())
                .or_default()
                .push(key);
        }
        for house in houses.iter_mut() {
            let Some(keys) = keys_by_house.get(house.id.as_str()) else {
                continue;
            };
            for key in keys {
                let Some(room) = house.rooms.get_mut(key.room_index as usize) else {
                    continue;
                };
                let Some(seg) = room
                    .wall_mut(key.wall_dir)
                    .get_mut(key.segment_index as usize)
                else {
                    continue;
                };
                if seg.variant.is_openable() {
                    seg.is_open = true;
                }
            }
        }
    }

    /// Forget open-door state for a house whose passability entry is being
    /// installed or removed; stale keys must not outlive the segment layout.
    pub(crate) async fn clear_open_doors_for_house(&self, house_id: &str) {
        self.open_doors
            .write()
            .await
            .retain(|k| k.house_id != house_id);
    }
}

/// Run a small auth-DB op off the async runtime (rusqlite blocks).
pub(super) async fn auth_db<T, F>(op: F) -> Result<T, crate::auth::AuthError>
where
    F: FnOnce() -> Result<T, crate::auth::AuthError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(op)
        .await
        .map_err(|e| crate::auth::AuthError::Database(e.to_string()))?
}

const MAX_DOOR_DISTANCE: f32 = 2.0;

/// Check that the player is within range of a door and on the same floor.
fn is_player_near_door(
    room: &RoomData,
    house_origin: &Position,
    wall_dir: WallDirection,
    segment_index: u32,
    player_pos: &Position,
    player_floor: i8,
) -> bool {
    // Exact floor match. Clients report 0 outdoors (entering a ground
    // floor door is floor 0 on both sides); negative floors are dungeon
    // depths and never match house doors.
    if player_floor != room.floor_level as i8 {
        warn!(
            "toggle_door: wrong floor — player floor={} door floor={}",
            player_floor, room.floor_level
        );
        return false;
    }

    let seg_center = segment_index as f32 + 0.5;
    let local_x = room.local_x as f32;
    let local_z = room.local_z as f32;
    let size_x = room.size_x as f32;
    let size_z = room.size_z as f32;

    // Door world position (center of 1m segment along the wall)
    let (door_x, door_z) = match wall_dir {
        WallDirection::North => (local_x + seg_center, local_z),
        WallDirection::South => (local_x + seg_center, local_z + size_z),
        WallDirection::East => (local_x + size_x, local_z + seg_center),
        WallDirection::West => (local_x, local_z + seg_center),
    };
    let world_x = house_origin.x + door_x;
    let world_z = house_origin.z + door_z;

    // XZ distance check
    let dx = onlinerpg_shared::shortest_world_delta_x(world_x, player_pos.x);
    let dz = player_pos.z - world_z;
    let dist_sq = dx * dx + dz * dz;
    if dist_sq > MAX_DOOR_DISTANCE * MAX_DOOR_DISTANCE {
        warn!(
            "toggle_door: too far — player ({:.1},{:.1}) door ({:.1},{:.1}) dist={:.2}",
            player_pos.x,
            player_pos.z,
            world_x,
            world_z,
            dist_sq.sqrt()
        );
        return false;
    }

    true
}
