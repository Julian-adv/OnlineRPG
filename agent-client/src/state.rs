use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::dungeon::Dungeon;
use crate::monster_ai::MonsterAiManager;
use onlinerpg_shared::dungeon::{
    cell_center, dungeon_cache_key, floor_cells, floor_level_for_passability,
    passability_floor_for_level, path_max_nodes, set_floor_cells, world_to_cell,
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

pub(crate) use onlinerpg_shared::messages::MUSIC_EMOTE;

const MAX_EVENTS: usize = 200;
/// Rolling window of conversation lines kept as prompt context. Stateless
/// backends (one `codex exec` per prompt) see only this window, so it is
/// the NPC's entire short-term memory of who said what.
const MAX_CHAT_HISTORY: usize = 30;
/// How many of our own recent song titles the world state lists, so a bard
/// can favor tunes it has not played lately.
const MAX_RECENT_SONGS: usize = 8;
/// Accumulated favor a player can hold with this NPC, in either direction.
const FAVOR_MIN: i32 = -5;
const FAVOR_MAX: i32 = 5;
/// Favor at which a player counts as a regular: resident traders bring up
/// their wishlist, and keepsake offers, only around such players —
/// strangers get small talk, not personal business.
const TRADE_FAVOR_THRESHOLD: i32 = 3;

/// Push onto a capped ring: the oldest entry falls off past `cap`.
fn push_capped(q: &mut VecDeque<String>, item: String, cap: usize) {
    q.push_back(item);
    if q.len() > cap {
        q.pop_front();
    }
}
/// How far we may drift before our own performance counts as abandoned.
const MUSIC_STAY_PUT_RADIUS: f32 = 1.5;
/// Quiet spell between our own songs, so a busker is not one unbroken stream.
/// The web client's playlist rests 0-60s between tracks; this is the same
/// idea with a floor under it, since a performance is something people watch.
const MUSIC_REST_MIN_SECS: u64 = 15;
const MUSIC_REST_MAX_SECS: u64 = 45;
/// How close to us an item has to land to be a tip for the music — forgiving,
/// since a shy listener tosses their coins from the edge of the crowd.
const TIP_RADIUS: f32 = 6.0;
/// Cap on tips noticed per song, so a floor strewn with junk — dropped by
/// someone bored or malicious — can't grow the prompt without bound.
const MAX_TIPS_PER_SONG: usize = 5;
/// Distance threshold for "player appeared nearby" agent events (in game units).
const NEARBY_PLAYER_RADIUS: f32 = 10.0;
/// How many ground items the world state lists before summarising the rest.
const MAX_LISTED_GROUND_ITEMS: usize = 10;
/// Real-time cooldown on the wishlist prompt section after the NPC buys
/// a wishlist item (see `trade_satiated_until`).
const WISHLIST_TRADE_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30 * 60);
/// How long trade pushes (open_trade/offer_deal) at a player stay blocked
/// after they wave off our trade window (`TradeDeclined`).
const TRADE_DECLINE_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// Cap on remembered party invites, matching the web client's toast queue.
const MAX_PENDING_PARTY_INVITES: usize = 3;
/// From the shared crate so the server's invite TTL and the agent's pruning
/// are guaranteed equal.
use onlinerpg_shared::messages::{PARTY_INVITE_TTL, PARTY_SUMMON_TTL};
/// NPC sight distance for deciding which nearby human and monster activity
/// matters. Re-exported from the shared crate so the server's event-delivery
/// radius and the agent's perception radius are guaranteed equal.
pub(crate) use onlinerpg_shared::NPC_SIGHT_RADIUS;

/// Eight-way compass word for an offset from the player. North is -z, east
/// is +x; the diagonal band covers ±22.5° around each diagonal.
fn compass(dx: f32, dz: f32) -> &'static str {
    let ns = if dz < 0.0 { "north" } else { "south" };
    let ew = if dx < 0.0 { "west" } else { "east" };
    let (adx, adz) = (dx.abs(), dz.abs());
    // tan(67.5°) ≈ 2.414: beyond that the offset reads as a straight
    // cardinal, inside it as a diagonal.
    if adz > 2.414 * adx {
        ns
    } else if adx > 2.414 * adz {
        ew
    } else {
        match (dz < 0.0, dx < 0.0) {
            (true, false) => "northeast",
            (true, true) => "northwest",
            (false, false) => "southeast",
            (false, true) => "southwest",
        }
    }
}

/// Terrain-grid glyph for a cell's surface. Sea reads from the heightmap
/// (below sea level 0), rivers from the splat's river-bed palette entry.
fn ground_char(surface: Option<u8>, height: Option<f32>) -> char {
    if height.is_some_and(|h| h < 0.0) {
        return '~';
    }
    match surface {
        Some(crate::splat::PAL_RIVER_BED) => '~',
        Some(crate::splat::PAL_CLIFF) => '^',
        Some(crate::splat::PAL_ROAD | crate::splat::PAL_STONE_PATH | crate::splat::PAL_PAVING) => {
            'R'
        }
        Some(crate::splat::PAL_SAND) => 's',
        Some(crate::splat::PAL_SNOW) => '*',
        _ => '.',
    }
}

/// Surface-map geometry, derived from the sight radius so the grid always
/// spans exactly what the agent can perceive.
const GRID_CELL_M: f32 = 3.0;
const GRID_CELLS: i32 = (NPC_SIGHT_RADIUS / GRID_CELL_M) as i32 * 2 + 1;
const GRID_HALF: i32 = GRID_CELLS / 2;

/// Stamp an entity glyph on the terrain grid if its position falls inside.
fn overlay(grid: &mut [Vec<char>], px: f32, pz: f32, x: f32, z: f32, glyph: char) {
    let c = ((x - px) / GRID_CELL_M).round() as i32 + GRID_HALF;
    let r = ((z - pz) / GRID_CELL_M).round() as i32 + GRID_HALF;
    if (0..GRID_CELLS).contains(&r) && (0..GRID_CELLS).contains(&c) {
        grid[r as usize][c as usize] = glyph;
    }
}

/// A party invite the agent hasn't answered yet.
pub struct PendingPartyInvite {
    pub inviter_id: PlayerId,
    pub inviter_name: String,
    pub expires_at: std::time::Instant,
}

/// A summoning-scroll consent request the agent hasn't answered yet.
pub struct PendingPartySummon {
    pub caster_id: PlayerId,
    pub caster_name: String,
    pub expires_at: std::time::Instant,
}

/// Where a carried item sits.
pub enum Carried {
    Worn(onlinerpg_shared::inventory::EquipSlot),
    InBag(u64),
}

/// Result of looking up every bag copy of a named item, for actions that can
/// request more than one unit (sell/drop with a qty). See
/// `AgentState::find_carried_bag_copies`.
pub enum CarriedBagCopies {
    /// At least one bag copy has unspent quantity this turn. `copies` is
    /// (instance_id, remaining_qty) pairs — several entries when the same
    /// item_def_id is fragmented across separate stacks or individually
    /// picked-up non-stackable copies.
    InBag {
        def_id: String,
        copies: Vec<(u64, u32)>,
    },
    /// Known only as an equipped item — no bag copy to sell/drop.
    WornOnly { def_id: String },
}

/// How urgently an event needs LLM attention. Ordered most urgent first, so
/// `min` picks the one that decides a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

    /// Every registered dungeon — the watch panel draws their entrances.
    pub fn all_dungeons(&self) -> &[Arc<Dungeon>] {
        &self.dungeons
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
        if let Some(house) = self.houses.get_mut(house_id) {
            if let Some(room) = house.rooms.get_mut(room_index as usize) {
                // The wall is the source of truth (door hunting reads
                // `is_open` off it); the edge is derived from it.
                if let Some(wall) = room.wall_mut(wall_dir).get_mut(segment_index) {
                    wall.is_open = is_open;
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
}

/// Shared state between WebSocket reader and Claude driver tasks.
/// Our own `/play_music` performance in flight. We have no audio to end it
/// for us, so the track's length from the registry is the clock, and walking
/// off the starting spot abandons it — as it does for a human player.
struct SelfPerformance {
    ends_at: std::time::Instant,
    from: Position,
}

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
    /// Our own hunger (satiation, band, poisoned) from `HungerUpdate`;
    /// stays None for exempt NPCs.
    pub self_hunger: Option<(u32, onlinerpg_shared::hunger::HungerState, bool)>,
    /// Burning campfires in our AOI, for the grill-your-catch decision.
    pub campfires: HashMap<u64, onlinerpg_shared::hunger::Campfire>,
    /// Laid-out stalls in our AOI, so a merchant knows its own is out.
    pub stalls: HashMap<u64, onlinerpg_shared::stall::Stall>,
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
    /// Until when trade pushes at each player stay blocked after they waved
    /// off our trade window (`TradeDeclined` → `TRADE_DECLINE_COOLDOWN`).
    trade_declined_until: HashMap<PlayerId, std::time::Instant>,
    /// True between our own FishingCasted and FishingEnded. Suppresses LLM
    /// movement (like `trade_busy`) and adds a stay-put prompt line;
    /// `stop_fishing` stays the deliberate exit.
    pub self_fishing: bool,
    /// Last stance the fight reflex sent, so each `FishingFight` beat only
    /// resends on change.
    pub fishing_stance: Option<onlinerpg_shared::fishing::FishingAction>,
    /// Unanswered party invites, oldest first (capped; a flood can't swap
    /// the invite out from under an in-flight `party_accept`). Expired
    /// invites are pruned on mutation and skipped on read, so a dead invite
    /// stops prompting the model.
    pub pending_party_invites: Vec<PendingPartyInvite>,
    /// Unanswered summons, same queue discipline as invites.
    pub pending_party_summons: Vec<PendingPartySummon>,
    /// Current party roster from `PartyState`; empty = not in a party.
    pub party_members: Vec<onlinerpg_shared::messages::PartyMember>,
    pub party_leader: Option<PlayerId>,
    /// Known nearby players
    pub nearby_players: HashMap<PlayerId, Player>,
    /// Per-merchant list of units we sold this session, repurchasable at the
    /// recorded payout (fed by BuybackUpdated/ShopState).
    pub merchant_buyback: HashMap<PlayerId, Vec<onlinerpg_shared::messages::BuybackEntry>>,
    /// Known nearby monsters
    pub nearby_monsters: HashMap<String, Monster>,
    /// Items lying on the ground, keyed by instance id (from the join
    /// snapshot plus GroundItemSpawned/Appeared/Removed).
    ground_items: HashMap<u64, GroundItem>,
    /// Whether this agent busks, from `NpcConfig::plays_music` — the same
    /// gate that put the songbook and tip rules into its prompt, so it is
    /// never instructed about tips it will not receive.
    pub plays_music: bool,
    /// Def ids this NPC could offer as keepsakes (`NpcRow::
    /// offerable_keepsake_ids`) — what `take_up_instrument` keeps out of
    /// its hands, since an offer only reaches items in the bag.
    pub keepsake_ids: Vec<String>,
    events: Vec<ServerMessage>,
    /// Conversation lines already shown to (or heard while asleep by) the
    /// LLM, kept as the RECENT CONVERSATION prompt section (`MAX_CHAT_HISTORY`).
    chat_history: VecDeque<String>,
    /// Titles of our own recent performances, oldest first (`MAX_RECENT_SONGS`).
    recent_songs: VecDeque<String>,
    /// Accumulated per-player favor, keyed by canonical display name. Fed by
    /// the LLM's `favor` response field, clamped to FAVOR_MIN..=FAVOR_MAX,
    /// persisted to the NPC's favor file. Gates keepsake offers structurally.
    pub favor: BTreeMap<String, i32>,
    /// Latest position per monster -- deduplicates high-frequency MonsterMoved events
    latest_monster_moves: HashMap<String, ServerMessage>,
    /// Latest position per player -- deduplicates high-frequency PlayerMoved events
    latest_player_moves: HashMap<PlayerId, ServerMessage>,
    /// Latest game time -- only the most recent matters
    latest_time: Option<ServerMessage>,
    /// Players we've already seen within NEARBY_PLAYER_RADIUS -- prevents duplicate events
    seen_nearby_players: HashSet<PlayerId>,
    /// Who is playing what right now, so the end of a tune is an event too.
    music_performers: HashMap<PlayerId, String>,
    /// Our own running performance (`check_music_finished` is its clock).
    self_performance: Option<SelfPerformance>,
    /// Until when the square stays quiet after our own song (`MUSIC_REST_*`).
    self_music_rest_until: Option<std::time::Instant>,
    /// Tips left while we were still playing, as (instance id, event line).
    /// Held until the song ends: the thanks belong in the quiet spell, and
    /// walking over mid-song would abandon the performance.
    pending_tips: Vec<(u64, String)>,
    /// Tips noticed since the current song started (`MAX_TIPS_PER_SONG`).
    tips_noticed: usize,
    /// An invented song title already woke the driver; the next one waits for
    /// the ordinary prompt, so a model that keeps guessing cannot spin.
    bad_song_title_refused: bool,
    /// POIs currently inside NPC_SIGHT_RADIUS (monsters, loot, dungeon
    /// entrances), keyed by a typed id. Entry fires a [Sighted] event so the
    /// LLM reacts mid-walk instead of at the next scheduled turn.
    sighted_pois: HashSet<String>,
    /// Synthetic agent-side events (e.g. "player appeared nearby")
    agent_events: Vec<String>,
    /// Terrain height sampler (shared across NPC connections)
    pub height_sampler: Arc<HeightSampler>,
    pub splat_sampler: Arc<crate::splat::SplatSampler>,
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
    /// Running follow loop: (target name, task handle). Anything that takes
    /// the body over aborts it; losing the target ends it with an event.
    pub follow_task: Option<(String, tokio::task::JoinHandle<()>)>,
    /// Most urgent reason the driver has been woken for since it last looked.
    wake_urgency: EventUrgency,
}

impl SharedState {
    pub fn new(
        characters: Vec<Character>,
        cmd_tx: mpsc::Sender<ClientMessage>,
        height_sampler: Arc<HeightSampler>,
        splat_sampler: Arc<crate::splat::SplatSampler>,
        world_cache: Arc<std::sync::RwLock<WorldCache>>,
        watch: Option<Arc<crate::watch::NpcWatch>>,
    ) -> Self {
        Self {
            characters,
            in_game: false,
            self_player_id: None,
            self_player: None,
            self_gold: None,
            self_hunger: None,
            campfires: HashMap::new(),
            stalls: HashMap::new(),
            self_bag: Vec::new(),
            self_equipped: HashMap::new(),
            trade_satiated_until: None,
            trade_busy: false,
            trade_declined_until: HashMap::new(),
            self_fishing: false,
            fishing_stance: None,
            pending_party_invites: Vec::new(),
            pending_party_summons: Vec::new(),
            party_members: Vec::new(),
            party_leader: None,
            nearby_players: HashMap::new(),
            merchant_buyback: HashMap::new(),
            nearby_monsters: HashMap::new(),
            ground_items: HashMap::new(),
            plays_music: false,
            keepsake_ids: Vec::new(),
            events: Vec::new(),
            chat_history: VecDeque::new(),
            recent_songs: VecDeque::new(),
            favor: BTreeMap::new(),
            latest_monster_moves: HashMap::new(),
            latest_player_moves: HashMap::new(),
            latest_time: None,
            seen_nearby_players: HashSet::new(),
            music_performers: HashMap::new(),
            self_performance: None,
            self_music_rest_until: None,
            pending_tips: Vec::new(),
            tips_noticed: 0,
            bad_song_title_refused: false,
            sighted_pois: HashSet::new(),
            agent_events: Vec::new(),
            height_sampler,
            splat_sampler,
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
            follow_task: None,
            wake_urgency: EventUrgency::Noise,
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

    /// Characters on our own floor. Someone a floor above is a dot straight
    /// overhead, not a neighbour, so nothing the LLM sees or names should
    /// reach them.
    fn players_on_my_floor(&self) -> impl Iterator<Item = (&PlayerId, &Player)> {
        self.nearby_players
            .iter()
            .filter(|(_, p)| p.floor_level == self.self_floor_level)
    }

    /// Monsters on our own floor — cross-floor ones read to the LLM as
    /// phantom respawns.
    fn monsters_on_my_floor(&self) -> impl Iterator<Item = &Monster> {
        self.nearby_monsters
            .values()
            .filter(|m| m.floor_level == self.self_floor_level)
    }

    /// Returns true if any non-NPC (human) player is in `nearby_players`.
    pub fn has_nearby_human_players(&self) -> bool {
        self.nearby_human_players().next().is_some()
    }

    /// Human players on our floor, within sight, excluding ourselves.
    fn nearby_human_players(&self) -> impl Iterator<Item = (&PlayerId, &Player)> {
        let self_pos = self.self_player.as_ref().map(|p| p.position);
        let self_id = self.self_player_id.as_ref();
        let radius_sq = NPC_SIGHT_RADIUS * NPC_SIGHT_RADIUS;
        self.players_on_my_floor().filter(move |(id, p)| {
            self_id != Some(*id)
                && !p.is_official_npc
                && self_pos.is_some_and(|sp| p.position.dist_xz_sq(&sp) <= radius_sq)
        })
    }

    /// Emit an agent event for any player on our floor that just entered
    /// NEARBY_PLAYER_RADIUS for the first time.
    fn check_nearby_player_proximity(&mut self) {
        let self_pos = match self.self_player.as_ref() {
            Some(p) => &p.position,
            None => return,
        };
        let self_id = match self.self_player_id.as_ref() {
            Some(id) => id,
            None => return,
        };

        let arrived: Vec<(PlayerId, String)> = self
            .players_on_my_floor()
            .filter(|(pid, _)| *pid != self_id && !self.seen_nearby_players.contains(pid))
            .filter_map(|(pid, player)| {
                let dist = crate::geom::PlanarDelta::between(&player.position, self_pos).dist;
                (dist <= NEARBY_PLAYER_RADIUS).then(|| {
                    (
                        *pid,
                        format!(
                            "[PlayerNearby] {} Lv.{} appeared {:.1}m away at ({:.1}, {:.1}, {:.1})",
                            player.name,
                            player.level,
                            dist,
                            player.position.x,
                            player.position.y,
                            player.position.z
                        ),
                    )
                })
            })
            .collect();

        for (pid, event) in arrived {
            self.seen_nearby_players.insert(pid);
            self.agent_events.push(event);
            // A person arriving, not our own bookkeeping — urgent lane.
            self.wake(EventUrgency::Urgent);
        }
    }

    /// A tune ended: say so, since the agent heard it start. Silent for
    /// anyone who was not playing.
    fn finish_music(&mut self, player_id: &PlayerId) {
        let Some(track) = self.music_performers.remove(player_id) else {
            return;
        };
        let is_self = self.self_player_id.as_ref() == Some(player_id);
        let who = if is_self {
            self.self_performance = None;
            let rest = rand::thread_rng().gen_range(MUSIC_REST_MIN_SECS..=MUSIC_REST_MAX_SECS);
            self.self_music_rest_until =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(rest));
            // The break gets its own allowance: the last song of the evening
            // must not leave the counter stuck with no next song to reset it.
            self.tips_noticed = 0;
            "You".to_string()
        } else {
            self.player_display_name(player_id)
        };
        let line = format!("[PlayMusic] {who} finished \"{track}\".");
        if is_self {
            // No wake yet: the rest ending is what invites the next song, and
            // waking now would only draw a command we would have to refuse.
            self.agent_events.push(line);
            // Tips are the exception — the quiet spell is when they get
            // thanked and picked up, and nothing else wakes us before it ends.
            for (_, tip) in std::mem::take(&mut self.pending_tips) {
                self.push_agent_event(tip);
            }
        } else {
            self.push_agent_event(line);
        }
    }

    /// A name the agent can say out loud, never a raw id: someone who acted
    /// and then walked out of sight is just "Someone".
    fn visible_name(&self, player_id: &PlayerId) -> String {
        self.nearby_players
            .get(player_id)
            .map_or_else(|| "Someone".to_string(), |p| p.name.clone())
    }

    /// Someone took an item a player had put down — a tip snatched, or a
    /// gift collected again. Ordinary loot churn stays silent: the world
    /// state already lists what lies about, and a hunting ground would file
    /// a line for every corpse otherwise.
    fn note_pickup(&mut self, item: &GroundItem, picker: &PlayerId) {
        let line = format!(
            "[GroundItem] {} picked up {} [id {}].",
            self.visible_name(picker),
            item.item_def_id,
            item.instance_id
        );
        // Mid-song this is not worth an LLM turn; it rides along with the
        // next prompt, which the end of the song brings soon enough.
        if self.self_performance.is_some() {
            self.agent_events.push(line);
        } else {
            self.push_agent_event(line);
        }
    }

    /// Someone left something at the busker's feet. Announced at once when we
    /// are between songs; held for the end of the song while we are playing.
    /// Only a busker is tipped — a guard's kill drops and a merchant's
    /// neighbours stay ordinary loot — and not while the schedule holds us
    /// in a pose: walking over from a bed would drop it with nothing to
    /// restore it until morning.
    fn note_tip(&mut self, item: &GroundItem) {
        let Some(dropper) = item
            .dropped_by
            .filter(|id| self.self_player_id != Some(*id))
        else {
            return;
        };
        let Some(me) = self.self_player.as_ref() else {
            return;
        };
        let posing = me
            .object_type
            .as_deref()
            .is_some_and(|held| held != MUSIC_EMOTE);
        if !self.plays_music
            || posing
            || item.floor_level != self.self_floor_level
            || self.tips_noticed >= MAX_TIPS_PER_SONG
            || item.position.dist_xz_sq(&me.position) > TIP_RADIUS * TIP_RADIUS
        {
            return;
        }
        let note = format!(
            "[Tip] {} left {} at your feet [id {}].",
            self.visible_name(&dropper),
            item.item_def_id,
            item.instance_id
        );
        self.tips_noticed += 1;
        if self.self_performance.is_some() {
            self.pending_tips.push((item.instance_id, note));
        } else {
            self.push_agent_event(note);
        }
    }

    /// Drop a `/play_music` the agent typed for a song that does not exist, or
    /// while its own tune is still running, or during the quiet spell after it:
    /// a second command restarts the music for every listener, and the LLM is
    /// not patient enough to wait on its own. A timing refusal does not wake the
    /// driver — the end of the rest does that, and waking here would only invite
    /// another attempt. A made-up title does wake it, once: the bard has already
    /// announced the song to the square, and nothing else would prompt it to
    /// take that back before the idle interval, an hour later.
    pub fn refuses_play_command(&mut self, message: &str) -> bool {
        // The same parser the server runs on the other end of this command.
        let Some(query) = onlinerpg_shared::messages::strip_command(message, "/play_music") else {
            return false;
        };
        // An empty query is the server's random pick, and always resolves.
        let query = query.trim();
        if !query.is_empty() && !crate::bgm_defs::knows(query) {
            self.push_agent_event(format!(
                "[PlayMusic] Ignored — there is no song called \"{query}\" in your songbook. \
                 Use a title exactly as the songbook writes it. If you already announced this \
                 one, tell them you had the name wrong and offer a song you do know."
            ));
            if !self.bad_song_title_refused {
                self.bad_song_title_refused = true;
                self.wake(EventUrgency::Urgent);
            }
            return true;
        }
        let now = std::time::Instant::now();
        let resting = self
            .self_player
            .as_ref()
            .and_then(|me| me.object_type.as_deref())
            .filter(|held| *held != MUSIC_EMOTE);
        let why = if let Some(held) = resting {
            // Playing would replace the pose the schedule put us in, and
            // nothing would put us back until the next schedule entry.
            format!("you are using the {held} and would have to get up first")
        } else if let Some(perf) = &self.self_performance {
            format!(
                "you are still playing, with about {}s to go",
                perf.ends_at.saturating_duration_since(now).as_secs()
            )
        } else if let Some(rest_until) = self.self_music_rest_until {
            format!(
                "the square is quiet between songs for another {}s",
                rest_until.saturating_duration_since(now).as_secs()
            )
        } else {
            return false;
        };
        self.agent_events.push(format!(
            "[PlayMusic] Ignored — {why}. One song at a time; wait for the note \
             that says you can start another. If you already announced the \
             title, tell them it is coming rather than leaving the promise \
             hanging."
        ));
        true
    }

    /// Stop strumming once the track we started has run its length, and invite
    /// the next song when the quiet spell after it is over. The web client
    /// ends a performance when its audio ends and rests before the next track;
    /// we have no audio, so this tick is our equivalent — without it an NPC
    /// bard plays one tune forever, or one unbroken stream of them.
    pub fn check_music_finished(&mut self) {
        if let Some(rest_until) = self.self_music_rest_until {
            if std::time::Instant::now() >= rest_until {
                self.self_music_rest_until = None;
                self.push_agent_event(
                    "[PlayMusic] The square is quiet again — time for another song.".to_string(),
                );
            }
        }

        let Some(perf) = &self.self_performance else {
            return;
        };
        let walked_off = self.self_player.as_ref().is_some_and(|me| {
            perf.from.dist_xz_sq(&me.position) > MUSIC_STAY_PUT_RADIUS * MUSIC_STAY_PUT_RADIUS
        });
        if !walked_off && std::time::Instant::now() < perf.ends_at {
            return;
        }
        self.self_performance = None;
        self.pending_commands.push(ClientMessage::StopInteraction);
    }

    /// Emit a [Sighted] event when any point of interest — a monster, dropped
    /// loot, a dungeon entrance — enters NPC_SIGHT_RADIUS on our floor, and
    /// forget it once it drifts well past the edge so a re-entry announces
    /// again. Without this the agent walks straight past everything between
    /// scheduled turns. Only an aggressive monster wakes the driver; the rest
    /// ride to the next prompt so a long walk isn't cut every few metres.
    fn check_sightings(&mut self) {
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

    /// A busker plays on a workhorse instrument — never the starter sword,
    /// and never an offerable keepsake: those stay in the bag, the only
    /// place `shop_info::keepsake_section` offers from. On the join
    /// snapshot, anything else in the main hand is swapped for the cheapest
    /// workhorse. Snapshot-only: what to hold mid-session (a fishing rod,
    /// say) stays the agent's own choice.
    fn take_up_instrument(&mut self) {
        if !self.plays_music {
            return;
        }
        let keepsakes = &self.keepsake_ids;
        let workhorse_instr = |i: &onlinerpg_shared::inventory::ItemInstance| {
            crate::item_defs::get(&i.item_def_id).is_some_and(|d| d.is_instrument())
                && !keepsakes.contains(&i.item_def_id)
        };
        let price = |i: &onlinerpg_shared::inventory::ItemInstance| {
            crate::item_defs::get(&i.item_def_id)
                .and_then(|d| d.base_price)
                .unwrap_or(0)
        };
        let Some(workhorse) = self
            .self_bag
            .iter()
            .filter(|i| workhorse_instr(i))
            .min_by_key(|i| price(i))
        else {
            return;
        };
        let held_is_workhorse = self
            .self_equipped
            .get(&onlinerpg_shared::inventory::EquipSlot::MainHand)
            .is_some_and(|i| workhorse_instr(i) && price(i) <= price(workhorse));
        if held_is_workhorse {
            return;
        }
        let instance_id = workhorse.instance_id;
        self.pending_commands
            .push(ClientMessage::EquipItem { instance_id });
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
        self.adopt_floor_level(floor_level);
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
                sprinting,
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
                    sprinting,
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
    fn forget_monster(&mut self, id: &str) {
        self.nearby_monsters.remove(id);
        self.monster_ai.remove_monster(id);
        self.latest_monster_moves.remove(id);
        self.sighted_pois.remove(&format!("m:{id}"));
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
            // Party chat is addressed to our group, so it wakes us like a
            // whisper; the own-echo Noise rule is the same.
            ServerMessage::PartyChatMessage { from, .. } => {
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
            // Urgent: an invite to answer while it is live, or the verdict
            // on our own invite.
            ServerMessage::PartyInviteReceived { .. }
            | ServerMessage::PartyInviteResult { .. }
            | ServerMessage::PartySummonReceived { .. } => EventUrgency::Urgent,
            ServerMessage::PartyState { .. } => EventUrgency::Routine,
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
            | ServerMessage::GroundItemQuantityChanged { .. }
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
            | ServerMessage::PlayerTorchToggled { .. }
            | ServerMessage::PlayerMainHandChanged { .. } => EventUrgency::Routine,

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

            // Fishing: only our own outcome is worth an LLM look — recast, eat
            // the catch, or give up. In-flight events are reflex-handled, and
            // another player's ending renders no prompt line (driver/prompt.rs),
            // so both are noise.
            ServerMessage::FishingEnded { player_id, .. } => {
                if self_id == Some(player_id) {
                    EventUrgency::Urgent
                } else {
                    EventUrgency::Noise
                }
            }
            ServerMessage::FishingError { .. } => EventUrgency::Urgent,
            ServerMessage::FishingCasted { .. }
            | ServerMessage::FishingBite { .. }
            | ServerMessage::FishingFight { .. } => EventUrgency::Noise,

            // Noise: high-frequency, irrelevant, or housing updates
            ServerMessage::PlayerMoved { .. }
            | ServerMessage::MonsterMoved { .. }
            | ServerMessage::PartyPositions { .. }
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

            // Campfire churn and the grill-cast start are world-state, not
            // events; the outcome (`GrillEnded`) rides the Routine catch-all.
            ServerMessage::CampfireSpawned { .. }
            | ServerMessage::CampfireAppeared { .. }
            | ServerMessage::CampfireRemoved { .. }
            | ServerMessage::StallPlaced { .. }
            | ServerMessage::StallAppeared { .. }
            | ServerMessage::StallRemoved { .. }
            | ServerMessage::GrillStarted => EventUrgency::Noise,

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
                self.self_fishing = false;
                // A character saved underground rejoins there (the server
                // rehydrates it), so adopt the floor instead of assuming 0.
                self.adopt_floor_level(player.floor_level);
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
                    // Any teleport settles the pending summons.
                    self.pending_party_summons.clear();
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
                campfires,
                stalls,
            } => {
                self.nearby_players = players.iter().map(|p| (p.id, p.clone())).collect();
                self.nearby_monsters = monsters.clone();
                self.ground_items.clear();
                for item in ground_items {
                    self.remember_ground_item(item.clone());
                }
                self.campfires.clear();
                for campfire in campfires {
                    self.campfires.insert(campfire.id, campfire.clone());
                }
                self.stalls.clear();
                for stall in stalls {
                    self.stalls.insert(stall.id, stall.clone());
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
                // Out of earshot: the tune is gone, and [PlayerLeft] already
                // says why — no second line about it.
                self.music_performers.remove(player_id);
            }
            ServerMessage::PlayerMusicStarted {
                player_id, track, ..
            } => {
                self.music_performers.insert(*player_id, track.clone());
                if self.self_player_id.as_ref() == Some(player_id) {
                    self.bad_song_title_refused = false;
                    self.tips_noticed = 0;
                    push_capped(&mut self.recent_songs, track.clone(), MAX_RECENT_SONGS);
                    self.self_performance = self.self_player.as_ref().map(|me| SelfPerformance {
                        ends_at: std::time::Instant::now() + crate::bgm_defs::duration(track),
                        from: me.position,
                    });
                }
            }
            ServerMessage::TradeDeclined { player_id, .. } => {
                // Prune on insert so the map cannot grow one dead entry per
                // decliner over a long session.
                let now = std::time::Instant::now();
                self.trade_declined_until.retain(|_, until| now < *until);
                self.trade_declined_until
                    .insert(*player_id, now + TRADE_DECLINE_COOLDOWN);
            }
            ServerMessage::PlayerInteractionChanged {
                player_id,
                object_type,
            } => {
                if self.self_player_id.as_ref() == Some(player_id) {
                    if let Some(me) = self.self_player.as_mut() {
                        me.object_type = object_type.clone();
                    }
                }
                if object_type.as_deref() != Some(MUSIC_EMOTE) {
                    self.finish_music(player_id);
                }
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
                self.forget_monster(monster_id);
            }
            // The server just said this monster does not exist: its
            // MonsterDead/MonsterRemoved never reached us. Silently drop the
            // ghost — the [AttackRejected] event already tells the agent the
            // swing failed, and the next CURRENT STATE no longer lists it.
            ServerMessage::PlayerAttackRejected {
                monster_id,
                reason: onlinerpg_shared::AttackRejectReason::InvalidTarget,
            } => {
                self.forget_monster(monster_id);
            }

            ServerMessage::GroundItemSpawned { item } => {
                self.note_tip(item);
                self.remember_ground_item(item.clone());
            }
            // Not a fresh drop, just an item coming into view — never a tip.
            ServerMessage::GroundItemAppeared { item } => {
                self.remember_ground_item(item.clone());
            }
            ServerMessage::GroundItemRemoved {
                instance_id,
                picked_up_by,
            } => {
                let removed = self.ground_items.remove(instance_id);
                self.pending_tips.retain(|(id, _)| id != instance_id);
                // Only player-dropped items are worth a line — see note_pickup.
                if let Some(item) = removed.filter(|item| item.dropped_by.is_some()) {
                    if let Some(picker) = picked_up_by.filter(|id| self.self_player_id != Some(*id))
                    {
                        self.note_pickup(&item, &picker);
                    }
                }
            }
            ServerMessage::GroundItemQuantityChanged {
                instance_id,
                quantity,
                ..
            } => {
                if let Some(item) = self.ground_items.get_mut(instance_id) {
                    item.quantity = *quantity;
                }
            }
            ServerMessage::CharacterCreated { ref character } => {
                self.characters.push(character.clone());
            }
            ServerMessage::GoldUpdate { gold } => {
                self.self_gold = Some(*gold);
            }
            ServerMessage::HungerUpdate {
                satiation,
                state,
                poisoned_ms,
                ..
            } => {
                self.self_hunger = Some((*satiation, *state, *poisoned_ms > 0));
            }
            ServerMessage::CampfireSpawned { ref campfire }
            | ServerMessage::CampfireAppeared { ref campfire } => {
                self.campfires.insert(campfire.id, campfire.clone());
            }
            ServerMessage::CampfireRemoved { campfire_id } => {
                self.campfires.remove(campfire_id);
            }
            ServerMessage::StallPlaced { ref stall }
            | ServerMessage::StallAppeared { ref stall } => {
                self.stalls.insert(stall.id, stall.clone());
            }
            ServerMessage::StallRemoved { stall_id } => {
                self.stalls.remove(stall_id);
            }
            ServerMessage::TradeBusy { busy } => {
                self.trade_busy = *busy;
            }
            ServerMessage::PartyInviteReceived {
                inviter_id,
                ref inviter_name,
            } => {
                self.prune_expired_party_invites();
                let queue = &mut self.pending_party_invites;
                if queue.len() < MAX_PENDING_PARTY_INVITES
                    && !queue.iter().any(|i| i.inviter_id == *inviter_id)
                {
                    queue.push(PendingPartyInvite {
                        inviter_id: *inviter_id,
                        inviter_name: inviter_name.clone(),
                        expires_at: std::time::Instant::now() + PARTY_INVITE_TTL,
                    });
                }
            }
            ServerMessage::PartySummonReceived {
                caster_id,
                ref caster_name,
            } => {
                self.prune_expired_party_summons();
                // Replace any same-caster entry (always stale: the ack-only
                // cast never re-sends for a live one). No cap — distinct
                // casters bound the queue at the party size.
                let queue = &mut self.pending_party_summons;
                queue.retain(|s| s.caster_id != *caster_id);
                queue.push(PendingPartySummon {
                    caster_id: *caster_id,
                    caster_name: caster_name.clone(),
                    expires_at: std::time::Instant::now() + PARTY_SUMMON_TTL,
                });
            }
            ServerMessage::PartyState {
                leader_id,
                ref members,
            } => {
                self.party_leader = (!members.is_empty()).then_some(*leader_id);
                self.party_members = members.clone();
                // Joining a party settles whichever invite led to it.
                if !members.is_empty() {
                    self.pending_party_invites.clear();
                }
                // A summons only lives while its caster shares the roster.
                self.pending_party_summons
                    .retain(|s| members.iter().any(|m| m.id == s.caster_id));
            }
            ServerMessage::InventoryState { ref inventory }
            | ServerMessage::InventoryUpdated { ref inventory } => {
                self.self_bag = inventory.bag.clone();
                self.self_equipped = inventory.equipped.clone();
                // The join snapshot only — mid-session hands are the agent's.
                if matches!(msg, ServerMessage::InventoryState { .. }) {
                    self.take_up_instrument();
                }
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
            // Fishing reflexes: answer bites/rounds mechanically; the LLM only
            // decides whether to fish. Speed inside the window confers no
            // advantage.
            ServerMessage::FishingCasted { player_id, .. }
                if self.self_player_id.as_ref() == Some(player_id) =>
            {
                self.self_fishing = true;
                self.fishing_stance = None;
            }
            ServerMessage::FishingEnded { player_id, .. }
                if self.self_player_id.as_ref() == Some(player_id) =>
            {
                self.self_fishing = false;
                self.fishing_stance = None;
            }
            ServerMessage::FishingBite { player_id }
                if self.self_player_id.as_ref() == Some(player_id) =>
            {
                self.pending_commands.push(ClientMessage::FishingRespond {
                    action: onlinerpg_shared::fishing::FishingAction::Hook,
                });
            }
            ServerMessage::FishingFight {
                player_id,
                fish_state,
                tension_pct,
                ..
            } if self.self_player_id.as_ref() == Some(player_id) => {
                // Same policy a practiced human plays from the gauge; sent
                // only on change — a stance holds until replaced.
                let stance = onlinerpg_shared::fishing::auto_stance(*fish_state, *tension_pct);
                if self.fishing_stance != Some(stance) {
                    self.fishing_stance = Some(stance);
                    self.pending_commands
                        .push(ClientMessage::FishingRespond { action: stance });
                }
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

        // Check if any POI just entered sight. Only our own relocations
        // matter on the player side — walking (echoed as PlayerMoved),
        // teleports, server corrections; other players never affect what
        // we can see.
        match &msg {
            ServerMessage::GameState { .. }
            | ServerMessage::MonsterSpawned { .. }
            | ServerMessage::MonsterAssigned { .. }
            | ServerMessage::MonsterMoved { .. }
            | ServerMessage::GroundItemSpawned { .. }
            | ServerMessage::GroundItemAppeared { .. }
            | ServerMessage::PositionCorrected { .. } => {
                self.check_sightings();
            }
            ServerMessage::PlayerMoved { player_id, .. }
            | ServerMessage::PlayerTeleported { player_id, .. }
                if self.self_player_id.as_ref() == Some(player_id) =>
            {
                self.check_sightings();
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
            ServerMessage::PartyPositions { .. } => return urgency,
            // In-flight fishing beats: the reflex layer above already
            // answered them; the LLM only needs the FishingEnded outcome.
            ServerMessage::FishingCasted { .. }
            | ServerMessage::FishingBite { .. }
            | ServerMessage::FishingFight { .. } => return urgency,
            // Another player's ending renders no prompt line, so buffering it
            // would turn an otherwise-skipped poll into a blank LLM call.
            ServerMessage::FishingEnded { player_id, .. }
                if self.self_player_id.as_ref() != Some(player_id) =>
            {
                return urgency;
            }
            // Ground items churn in and out of the AOI as everyone moves;
            // the world state lists what is nearby each turn instead.
            ServerMessage::GroundItemSpawned { .. }
            | ServerMessage::GroundItemAppeared { .. }
            | ServerMessage::GroundItemRemoved { .. }
            | ServerMessage::GroundItemQuantityChanged { .. } => return urgency,
            // Campfires likewise live in the world state, and the grill start
            // is answered by GrillEnded a few seconds later.
            ServerMessage::CampfireSpawned { .. }
            | ServerMessage::CampfireAppeared { .. }
            | ServerMessage::CampfireRemoved { .. }
            | ServerMessage::GrillStarted => return urgency,
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
                    self.wake(EventUrgency::Routine);
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
            self.wake(EventUrgency::Urgent);
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

    /// Display name for a player id, falling back to the raw id for someone
    /// out of sight. The one statement of that contract — prompt rendering
    /// and synthetic events both go through here.
    pub fn player_display_name(&self, player_id: &PlayerId) -> String {
        if self.self_player_id.as_ref() == Some(player_id) {
            if let Some(p) = &self.self_player {
                return p.name.clone();
            }
        }
        if let Some(p) = self.nearby_players.get(player_id) {
            return p.name.clone();
        }
        player_id.to_string()
    }

    /// Drain synthetic agent-side events (e.g. player proximity alerts).
    pub fn drain_agent_events(&mut self) -> Vec<String> {
        std::mem::take(&mut self.agent_events)
    }

    /// Remember a conversation line for the RECENT CONVERSATION prompt
    /// section, stamped with the game clock so the LLM can judge staleness.
    pub fn push_chat_history(&mut self, line: &str) {
        let stamped = match (self.game_hour, self.game_minute) {
            (Some(h), Some(m)) => format!("[{h:02}:{m:02}] {line}"),
            _ => line.to_string(),
        };
        push_capped(&mut self.chat_history, stamped, MAX_CHAT_HISTORY);
    }

    /// Conversation lines already handled, oldest first.
    pub fn chat_history(&self) -> &VecDeque<String> {
        &self.chat_history
    }

    /// Apply one favor delta from the LLM. Only a nearby human player
    /// counts — unknown names and NPCs are dropped — and both the step
    /// (±1) and the running total are clamped. Returns whether anything
    /// changed, so the caller knows to persist.
    pub fn apply_favor(&mut self, name: &str, delta: i32) -> bool {
        let delta = delta.clamp(-1, 1);
        if delta == 0 {
            return false;
        }
        let Some((id, is_npc)) = self.resolve_nearby_player(name) else {
            return false;
        };
        if is_npc {
            return false;
        }
        let canonical = self.player_display_name(&id);
        let entry = self.favor.entry(canonical).or_insert(0);
        let next = (*entry + delta).clamp(FAVOR_MIN, FAVOR_MAX);
        let changed = next != *entry;
        *entry = next;
        changed
    }

    /// Whether this player waved off our trade window recently, so
    /// open_trade/offer_deal at them are structurally suppressed until the
    /// cooldown runs out.
    pub fn trade_offer_blocked(&self, player_id: &PlayerId) -> bool {
        self.trade_declined_until
            .get(player_id)
            .is_some_and(|until| std::time::Instant::now() < *until)
    }

    /// Nearby human players whose favor has crossed the trade threshold —
    /// the audience the whole Personal Trading section (wishlist pitches
    /// and keepsake offers alike) is written for. A regular inside the
    /// decline cooldown drops out: they just said not now, so not even
    /// the section should court them.
    pub fn trade_worthy_players(&self) -> Vec<String> {
        self.nearby_human_players()
            .filter(|(id, p)| {
                !self.trade_offer_blocked(id)
                    && self.favor.get(&p.name).copied().unwrap_or(0) >= TRADE_FAVOR_THRESHOLD
            })
            .map(|(_, p)| p.name.clone())
            .collect()
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

    /// Push a synthetic agent event visible to the LLM. Synthetic events are
    /// feedback on the agent's own actions (arrival, a failed move, a kill),
    /// so they wake the LLM driver instead of waiting out the idle interval.
    /// They wake it at `Routine` though: an agent's own arrival note must
    /// never outrank a human talking to some other NPC in the LLM queue.
    pub fn push_agent_event(&mut self, event: String) {
        self.push_agent_event_inner(event, true);
    }

    /// Same, but without waking the driver: the event rides along with
    /// whatever prompt happens next (scenery noted in passing, not danger).
    pub fn push_agent_event_quiet(&mut self, event: String) {
        self.push_agent_event_inner(event, false);
    }

    fn push_agent_event_inner(&mut self, event: String, wake: bool) {
        if let Some(watch) = &self.watch {
            watch.push("agent", event.clone());
        }
        self.agent_events.push(event);
        if wake {
            self.wake(EventUrgency::Routine);
        }
    }

    /// Wake the LLM driver, remembering how urgent the reason was. The driver
    /// takes the urgency at wake-up to pick its rate-limit floor and the
    /// prompt's scheduler priority.
    fn wake(&mut self, urgency: EventUrgency) {
        self.wake_urgency = self.wake_urgency.min(urgency);
        self.urgent_notify.notify_one();
    }

    /// Take the urgency accumulated since the last wake-up, resetting it.
    pub fn take_wake_urgency(&mut self) -> EventUrgency {
        std::mem::replace(&mut self.wake_urgency, EventUrgency::Noise)
    }

    /// Resolve a player name (or raw id) among nearby players, as used by
    /// player-targeting LLM actions. Returns `(player_id, is_official_npc)`.
    /// `name_or_id` stays `&str` because it comes straight from LLM output and
    /// may be either form; the resolved handle is what gets typed.
    /// Only same-floor characters resolve: the world state the LLM saw lists
    /// no one else, and a cross-floor chase would burn its A* budget to reach
    /// a name it never should have been offered.
    pub fn resolve_nearby_player(&self, name_or_id: &str) -> Option<(PlayerId, bool)> {
        self.players_on_my_floor()
            .find(|(id, p)| {
                p.name.eq_ignore_ascii_case(name_or_id)
                    || name_or_id.parse::<u64>().is_ok_and(|n| id.get() == n)
            })
            .map(|(id, p)| (*id, p.is_official_npc))
    }

    /// The nearest NPC merchant on our floor, for trade actions that omit a
    /// merchant name. Usually there is exactly one in range, so guessing is
    /// safe and spares the LLM from naming it.
    pub fn nearest_merchant(&self) -> Option<PlayerId> {
        let self_pos = self.self_player.as_ref().map(|p| p.position)?;
        self.players_on_my_floor()
            .filter(|(_, p)| {
                p.is_official_npc && crate::shop_info::shop_line_for(&p.name).is_some()
            })
            // Only merchants the agent can actually see — the server
            // broadcasts players well beyond that, and "nearest" must not
            // start a long blind walk to one outside the CURRENT STATE list.
            .filter(|(_, p)| {
                p.position.dist_xz_sq(&self_pos) <= NPC_SIGHT_RADIUS * NPC_SIGHT_RADIUS
            })
            .min_by(|(_, a), (_, b)| {
                let da = a.position.dist_xz_sq(&self_pos);
                let db = b.position.dist_xz_sq(&self_pos);
                da.total_cmp(&db)
            })
            .map(|(id, _)| *id)
    }

    /// Every bag copy of the resolved item still available this turn.
    /// `spent` counts the units of each instance already given away earlier
    /// this turn, keyed by instance id: the bag snapshot only refreshes when
    /// InventoryUpdated arrives, and a stack survives a sale one unit at a
    /// time, so an instance stays sellable until its whole quantity is gone.
    pub fn find_carried_bag_copies(
        &self,
        asked: &str,
        spent: &HashMap<u64, u32>,
    ) -> Option<CarriedBagCopies> {
        let (id, placed) = self.find_carried(asked)?;
        let copies: Vec<(u64, u32)> = self
            .self_bag
            .iter()
            .filter(|i| i.item_def_id == id)
            .filter_map(|i| {
                let already = spent.get(&i.instance_id).copied().unwrap_or(0);
                let remaining = i.quantity.saturating_sub(already);
                (remaining > 0).then_some((i.instance_id, remaining))
            })
            .collect();
        if !copies.is_empty() {
            return Some(CarriedBagCopies::InBag { def_id: id, copies });
        }
        match placed {
            Carried::Worn(_) => Some(CarriedBagCopies::WornOnly { def_id: id }),
            // Every bag copy was already spent this turn — same outcome as
            // not finding it at all.
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
    /// Drop invites past the server-side TTL; call before mutating or
    /// answering the queue.
    pub fn prune_expired_party_invites(&mut self) {
        let now = std::time::Instant::now();
        self.pending_party_invites.retain(|i| i.expires_at > now);
    }

    /// Invites still answerable right now (`format_world_state` is `&self`,
    /// so expired entries are skipped rather than pruned).
    pub fn live_party_invites(&self) -> impl Iterator<Item = &PendingPartyInvite> {
        let now = std::time::Instant::now();
        self.pending_party_invites
            .iter()
            .filter(move |i| i.expires_at > now)
    }

    pub fn prune_expired_party_summons(&mut self) {
        let now = std::time::Instant::now();
        self.pending_party_summons.retain(|s| s.expires_at > now);
    }

    /// Summons still answerable right now, `live_party_invites`'s twin.
    pub fn live_party_summons(&self) -> impl Iterator<Item = &PendingPartySummon> {
        let now = std::time::Instant::now();
        self.pending_party_summons
            .iter()
            .filter(move |s| s.expires_at > now)
    }

    /// Snapshot everything the surface-terrain grid render needs, so the
    /// expensive tile sampling can run without the state lock. None
    /// underground — the floor layout lines already cover the map.
    pub fn terrain_grid_job(&self) -> Option<TerrainGridJob> {
        let p = self.self_player.as_ref()?;
        if self.self_floor_level != 0 {
            return None;
        }
        Some(TerrainGridJob {
            px: p.position.x,
            pz: p.position.z,
            py: p.position.y,
            height_sampler: Arc::clone(&self.height_sampler),
            splat_sampler: Arc::clone(&self.splat_sampler),
            world_cache: Arc::clone(&self.world_cache),
        })
    }
}

/// A detached surface-terrain grid render: position and shared samplers
/// snapshotted from `SharedState` so the tile loads (HTTP on a cache miss)
/// never run under the state lock.
pub struct TerrainGridJob {
    px: f32,
    pz: f32,
    py: f32,
    height_sampler: Arc<HeightSampler>,
    splat_sampler: Arc<crate::splat::SplatSampler>,
    world_cache: Arc<std::sync::RwLock<WorldCache>>,
}

impl TerrainGridJob {
    pub async fn render(&self) -> String {
        const CELLS: i32 = GRID_CELLS;
        const CELL_M: f32 = GRID_CELL_M;
        const HALF: i32 = GRID_HALF;

        let (px, pz, py) = (self.px, self.pz, self.py);
        // Height and surface type per cell center (async tile loads).
        let mut heights = vec![None; (CELLS * CELLS) as usize];
        let mut surfaces = vec![None; (CELLS * CELLS) as usize];
        for r in 0..CELLS {
            let cz = pz + (r - HALF) as f32 * CELL_M;
            for c in 0..CELLS {
                let cx = px + (c - HALF) as f32 * CELL_M;
                let i = (r * CELLS + c) as usize;
                heights[i] = self.height_sampler.sample_height(cx, cz).await.ok();
                surfaces[i] = self.splat_sampler.primary_at(cx, cz).await.ok();
            }
        }

        let mut grid: Vec<Vec<char>> = (0..CELLS)
            .map(|r| {
                (0..CELLS)
                    .map(|c| {
                        let i = (r * CELLS + c) as usize;
                        ground_char(surfaces[i], heights[i])
                    })
                    .collect()
            })
            .collect();

        // Buildings and furniture from the passability cache (sync).
        {
            let world = self.world_cache.read().unwrap();
            let cache = world.passability_cache();
            for r in 0..CELLS {
                let cz = pz + (r - HALF) as f32 * CELL_M;
                for c in 0..CELLS {
                    let cx = px + (c - HALF) as f32 * CELL_M;
                    if pathfinding::is_circle_blocked_on_floor(cache, cx, cz, 1.0, 0, None) {
                        grid[r as usize][c as usize] = '#';
                    }
                }
            }
            // Dungeon entrances.
            for d in world.all_dungeons() {
                overlay(&mut grid, px, pz, d.entrance.x, d.entrance.z, 'D');
            }
        }

        // Terrain and fixed map objects only — players, monsters and NPCs
        // live in the entity lists and [Sighted] events, with exact
        // coordinates there. Mixing them in would go stale within a turn.
        grid[HALF as usize][HALF as usize] = '@';

        // Row labels carry exact z, the header carries the x span, so the
        // agent can map any cell to world coordinates without arithmetic
        // guesswork.
        let west_x = px - HALF as f32 * CELL_M;
        let east_x = px + HALF as f32 * CELL_M;
        let mut out = format!(
            "Map: surface, you at ({px:.0}, {pz:.0}) — {size}x{size}m, {cell:.0}m per cell, \
             north up. Columns left to right: x={west:.0} to x={east:.0} (+{cell:.0} per \
             column). Row labels are that row's z.\n",
            size = CELLS * CELL_M as i32,
            cell = CELL_M,
            west = west_x,
            east = east_x,
            px = px,
            pz = pz,
        );
        for (r, row) in grid.iter().enumerate() {
            let cz = pz + (r as i32 - HALF) as f32 * CELL_M;
            out.push_str(&format!("z={:<6.0}", cz));
            for ch in row {
                out.push(' ');
                out.push(*ch);
            }
            out.push('\n');
        }
        out.push_str(
            "(. ground  R road  s sand  ~ water  ^ cliff  * snow  # building  \
             D dungeon entrance  @ you; characters and items are in the lists \
             above, not on this map)\n",
        );

        // Gentle slopes don't show in the glyphs; summarize them so climbs
        // are not a surprise. Cliff cells already read as ^.
        let h_at = |r: i32, c: i32| heights[(r * CELLS + c) as usize];
        let mut slopes = Vec::new();
        for (label, r, c) in [
            ("north", 0, HALF),
            ("south", CELLS - 1, HALF),
            ("east", HALF, CELLS - 1),
            ("west", HALF, 0),
        ] {
            if let Some(h) = h_at(r, c) {
                let dh = h - py;
                if dh.abs() >= 2.0 {
                    slopes.push(format!("{label} {dh:+.0}m"));
                }
            }
        }
        if !slopes.is_empty() {
            out.push_str(&format!(
                "Ground height at the map edge vs you: {}.\n",
                slopes.join(", ")
            ));
        }
        out
    }
}

impl SharedState {
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
            if p.health == 0 {
                lines.push(
                    "You are DEFEATED (HP 0). You do NOT recover on your own and most \
                     actions stay blocked. Respawn now with {\"type\": \"respawn\"} — \
                     the death penalty was already paid when you fell; respawning \
                     costs nothing more."
                        .to_string(),
                );
            }
        }
        if let Some(line) = self.format_dungeon_state() {
            lines.push(line);
        }
        if let Some((satiation, state, poisoned)) = self.self_hunger {
            let mut line = format!("Hunger: {state:?} ({satiation}/1000)");
            if poisoned {
                line.push_str(", food poisoned");
            }
            lines.push(line);
        }
        if let Some(p) = &self.self_player {
            let nearest_fire = self
                .campfires
                .values()
                .filter(|c| c.floor_level == p.floor_level)
                .map(|c| c.position.dist_xz_sq(&p.position))
                .min_by(f32::total_cmp);
            if let Some(d2) = nearest_fire {
                lines.push(format!(
                    "Campfire nearby: {:.1}m away (use a raw fish within 3m to grill it)",
                    d2.sqrt()
                ));
            }
            if let Some(own_stall) = self
                .self_player_id
                .and_then(|id| self.stalls.values().find(|s| s.owner == id))
            {
                lines.push(format!(
                    "Your stall is laid out {:.1}m away",
                    own_stall.position.dist_xz_sq(&p.position).sqrt()
                ));
            }
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
        // Data only — what to do with the list is the role template's call
        // (bard.txt: prefer something fresh, unless a listener asks again).
        if self.plays_music && !self.recent_songs.is_empty() {
            let list: Vec<&str> = self.recent_songs.iter().map(String::as_str).collect();
            lines.push(format!(
                "Songs you played recently, oldest first: {}",
                list.join(", ")
            ));
        }

        if !self.party_members.is_empty() {
            let names: Vec<String> = self
                .party_members
                .iter()
                .map(|m| {
                    if Some(m.id) == self.party_leader {
                        format!("{} (leader)", m.name)
                    } else {
                        m.name.clone()
                    }
                })
                .collect();
            lines.push(format!("Your party: {}", names.join(", ")));
        }
        for invite in self.live_party_invites() {
            lines.push(format!(
                "Pending party invite from {} — answer with party_accept or party_decline",
                invite.inviter_name
            ));
        }
        for summon in self.live_party_summons() {
            lines.push(format!(
                "{} calls you to their side (summoning scroll) — answer with summon_accept or summon_decline",
                summon.caster_name
            ));
        }

        // Nearby players (exclude self and humans beyond the sight radius)
        let sp = self.self_player.as_ref();
        let sight_sq = NPC_SIGHT_RADIUS * NPC_SIGHT_RADIUS;
        for (_, p) in self.players_on_my_floor() {
            if self.self_player_id.as_ref() == Some(&p.id) {
                continue;
            }
            if let Some(sp) = sp {
                if p.position.dist_xz_sq(&sp.position) > sight_sq {
                    continue;
                }
            }
            let npc_tag = if p.is_official_npc { " (NPC)" } else { "" };
            let favor_tag = match self.favor.get(&p.name) {
                Some(v) if !p.is_official_npc && *v != 0 => format!(" (favor {v:+})"),
                _ => String::new(),
            };
            lines.push(format!(
                "Player: {}{npc_tag}{favor_tag} Lv.{} HP {}/{} at ({:.1}, {:.1}, {:.1})",
                p.name, p.level, p.health, p.max_health, p.position.x, p.position.y, p.position.z
            ));
            if p.is_official_npc {
                if let Some(shop) = crate::shop_info::shop_line_for(&p.name) {
                    lines.push(shop);
                }
            }
        }

        // Exclude monsters beyond LLM sight radius
        for m in self.monsters_on_my_floor() {
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
            let dropped_by = match i.dropped_by.as_ref() {
                Some(id) if self.self_player_id.as_ref() == Some(id) => {
                    ", dropped by you".to_string()
                }
                Some(id) => format!(", dropped by {}", self.visible_name(id)),
                None => String::new(),
            };
            let amount = if i.quantity > 1 {
                format!(" x{}", i.quantity)
            } else {
                String::new()
            };
            lines.push(format!(
                "Item on ground: {}{amount} ({:.1}m away) [id {}]{dropped_by}",
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

    #[async_trait::async_trait]
    impl crate::splat::SplatTiles for NoTiles {
        async fn read_splat(&self, _tx: i32, _tz: i32) -> std::io::Result<Vec<u8>> {
            Err(std::io::Error::other("no terrain in tests"))
        }
    }

    pub(crate) fn test_state() -> (SharedState, mpsc::Receiver<ClientMessage>) {
        let (tx, rx) = mpsc::channel(8);
        let state = SharedState::new(
            Vec::new(),
            tx,
            Arc::new(HeightSampler::new(NoTiles)),
            Arc::new(crate::splat::SplatSampler::new(NoTiles)),
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
            lifecycle: Default::default(),
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
            quantity: 1,
            enchant: 0,
            dropped_by: None,
        }
    }

    /// A `ground_item` a player put down, as a tip test sees it.
    fn dropped_item(id: u64, def: &str, x: f32, z: f32, by: PlayerId) -> GroundItem {
        GroundItem {
            dropped_by: Some(by),
            ..ground_item(id, def, x, z, 0)
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
            main_hand: None,
            object_id: None,
            last_combat_at: 0,
            client_kind: Default::default(),
        }
    }

    /// Chat history is a capped ring stamped with the game clock — the
    /// short-term memory a stateless backend gets replayed each prompt.
    #[test]
    fn chat_history_stamps_and_caps() {
        let (mut s, _rx) = test_state();
        s.push_chat_history("[Chat] jake1: hello");
        assert_eq!(s.chat_history()[0], "[Chat] jake1: hello");

        s.game_hour = Some(20);
        s.game_minute = Some(26);
        for i in 0..40 {
            s.push_chat_history(&format!("[Chat] jake1: line {i}"));
        }
        assert_eq!(s.chat_history().len(), 30, "capped at MAX_CHAT_HISTORY");
        assert_eq!(s.chat_history()[0], "[20:26] [Chat] jake1: line 10");
        assert_eq!(s.chat_history()[29], "[20:26] [Chat] jake1: line 39");
    }

    /// Our own performances land in the recent-song list, oldest first and
    /// capped; the world state shows the list only to an agent that busks,
    /// and never counts someone else's tune as ours.
    #[test]
    fn recent_songs_render_for_the_busker_only() {
        let (mut s, _rx) = test_state();
        let me = test_player(0.0, 0.0);
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);
        s.plays_music = true;

        s.push_event(ServerMessage::PlayerMusicStarted {
            player_id: PlayerId::from(2),
            track: "Someone Else's Tune".to_string(),
            elapsed_secs: 0.0,
        });
        for i in 0..10 {
            s.push_event(ServerMessage::PlayerMusicStarted {
                player_id: PlayerId::from(1),
                track: format!("Song {i}"),
                elapsed_secs: 0.0,
            });
        }

        let world = s.format_world_state();
        assert!(
            world.contains("Songs you played recently, oldest first: Song 2,"),
            "capped at MAX_RECENT_SONGS, oldest dropped: {world}"
        );
        assert!(world.contains("Song 9"), "{world}");
        assert!(!world.contains("Someone Else's Tune"), "{world}");

        s.plays_music = false;
        assert!(!s.format_world_state().contains("Songs you played recently"));
    }

    /// Favor: nearby human players only, one step per call, clamped in
    /// total. Crossing the threshold makes a player keepsake-worthy and
    /// the world state shows the standing next to their name.
    #[test]
    fn favor_accumulates_and_gates_keepsakes() {
        let (mut s, _rx) = test_state();
        let me = test_player(0.0, 0.0);
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);

        let mut jake = test_player(3.0, 0.0);
        jake.id = PlayerId::from(2);
        jake.name = "jake1".to_string();
        s.nearby_players.insert(jake.id, jake);

        let mut wick = test_player(4.0, 0.0);
        wick.id = PlayerId::from(3);
        wick.name = "Wick".to_string();
        wick.is_official_npc = true;
        s.nearby_players.insert(wick.id, wick);

        assert!(!s.apply_favor("Wick", 1), "NPCs never earn favor");
        assert!(!s.apply_favor("stranger", 1), "unknown names are dropped");
        assert!(
            s.apply_favor("jake1", 5),
            "an oversized delta still steps once"
        );
        assert_eq!(s.favor.get("jake1"), Some(&1));
        assert!(s.trade_worthy_players().is_empty());

        for _ in 0..10 {
            s.apply_favor("jake1", 1);
        }
        assert_eq!(s.favor.get("jake1"), Some(&FAVOR_MAX));
        assert_eq!(s.trade_worthy_players(), ["jake1"]);
        assert!(
            s.format_world_state().contains("jake1 (favor +5)"),
            "{}",
            s.format_world_state()
        );

        // Even a favored regular is not courted while their decline
        // cooldown runs — the keepsake section drops them too.
        s.push_event(ServerMessage::TradeDeclined {
            player_id: PlayerId::from(2),
            player_name: "jake1".to_string(),
        });
        assert!(s.trade_worthy_players().is_empty());
    }

    /// The wishlist pitch needs an audience with any favor at all, and a
    /// waved-off trade window blocks pushes at that player for the
    /// cooldown — dropping them from the audience despite their favor.
    #[test]
    fn a_declined_trade_offer_blocks_further_pushes() {
        let (mut s, _rx) = test_state();
        let me = test_player(0.0, 0.0);
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);

        let mut jake = test_player(3.0, 0.0);
        jake.id = PlayerId::from(2);
        jake.name = "jake1".to_string();
        s.nearby_players.insert(jake.id, jake);

        assert!(!s.trade_offer_blocked(&PlayerId::from(2)));
        assert!(
            s.trade_worthy_players().is_empty(),
            "a stranger does not earn the shopping list"
        );
        s.apply_favor("jake1", 1);
        assert!(
            s.trade_worthy_players().is_empty(),
            "one kindness does not yet make a regular"
        );
        for _ in 0..2 {
            s.apply_favor("jake1", 1);
        }
        assert_eq!(
            s.trade_worthy_players(),
            ["jake1"],
            "favor at the trade threshold earns the pitch"
        );

        s.push_event(ServerMessage::TradeDeclined {
            player_id: PlayerId::from(2),
            player_name: "jake1".to_string(),
        });

        assert!(s.trade_offer_blocked(&PlayerId::from(2)));
        assert!(
            !s.trade_offer_blocked(&PlayerId::from(3)),
            "the block is per player, not global"
        );
        assert!(
            s.trade_worthy_players().is_empty(),
            "the only regular declined: the section vanishes"
        );
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
            s.remember_ground_item(item);
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

    /// An announced item is loot the agent may go for right away — the
    /// server does any withholding.
    #[test]
    fn an_announced_drop_is_actionable_at_once() {
        let (mut s, _rx) = test_state();
        s.self_player = Some(test_player(0.0, 0.0));

        s.push_event(ServerMessage::GroundItemSpawned {
            item: ground_item(1, "goblin_sword", 2.0, 0.0, 0),
        });
        s.push_event(ServerMessage::GroundItemAppeared {
            item: ground_item(2, "small_sword", 3.0, 0.0, 0),
        });

        let ids: Vec<u64> = s
            .ground_items_in_sight()
            .iter()
            .map(|(_, i)| i.instance_id)
            .collect();
        assert_eq!(ids, vec![1, 2]);
        assert!(s.ground_item(1).is_some());
        assert!(s.format_world_state().contains("goblin_sword"));
    }

    /// A field strewn with drops is summarised, not listed line by line.
    #[test]
    fn world_state_caps_the_ground_item_list() {
        let (mut s, _rx) = test_state();
        s.self_player = Some(test_player(0.0, 0.0));
        for id in 1..=(MAX_LISTED_GROUND_ITEMS as u64 + 3) {
            let item = ground_item(id, "small_sword", id as f32 * 0.5, 0.0, 0);
            s.remember_ground_item(item);
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
            Arc::new(crate::splat::SplatSampler::new(NoTiles)),
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

    /// Every coordinate the underground state line hands the LLM has to be a
    /// cell it can actually stand on. A shaft is walkable on this floor only
    /// along one row — its min corner and the cell half a metre over are both
    /// rock — so a wrong end or a rounded centre reads as a wall.
    #[test]
    fn the_floor_map_only_names_cells_the_agent_can_stand_on() {
        let (mut s, _crypt, _rx) = dungeon_state();
        let mut orientations = std::collections::HashSet::new();

        for def in onlinerpg_shared::dungeon::entrances() {
            let dungeon = s
                .world_cache
                .read()
                .unwrap()
                .dungeon_by_id(&def.id)
                .expect("registered dungeon");

            // Every door open: a shut one is a detour the mover handles, so it
            // must not be confused with a cell walled off for good.
            let doors: Vec<(u8, u32)> = (1..=dungeon.max_depth())
                .flat_map(|d| {
                    dungeon
                        .closed_doors(d, &HashSet::new())
                        .into_iter()
                        .map(move |door| (d, door.door_id))
                })
                .collect();
            s.world_cache
                .write()
                .unwrap()
                .set_dungeon_doors(&dungeon.id, &doors);

            for depth in 1..=dungeon.max_depth() {
                let layout = &dungeon.layouts()[depth as usize - 1];
                orientations.insert(layout.up_shaft.reversed);
                let floor = dungeon.passability_floor(depth);
                stand_at(&mut s, &dungeon, depth, layout.rooms[0].center());

                let line = s.format_dungeon_state().expect("underground state line");
                let where_ = format!("{} floor {depth}", dungeon.id);
                let named = coordinates_in(&line);
                assert!(
                    named.len() > layout.rooms.len(),
                    "{where_} should name every room plus the stairs, got {named:?}"
                );
                for (x, z) in named {
                    let p = Position { x, y: 0.0, z };
                    // Printed coordinates must survive the round trip back to
                    // the cell they name. Cell centres sit on .5, so rounding
                    // them to whole metres silently names the cell next door.
                    let cell = world_to_cell(&dungeon.entrance, x, z);
                    let centre = cell_center(&dungeon.entrance, depth, cell);
                    assert_eq!(
                        (centre.x, centre.z),
                        (x, z),
                        "{where_} prints ({x}, {z}), which reads back as the cell \
                         centred on ({}, {})\n{line}",
                        centre.x,
                        centre.z
                    );
                    assert!(
                        s.world_cache.read().unwrap().is_walkable(&p, floor),
                        "{where_} points the agent at ({x}, {z}), which is solid rock\n{line}"
                    );
                    // A shaft's interior is carved but walled off from this
                    // floor, so walkable is not enough — the goal has to be
                    // routable too.
                    assert!(
                        s.find_path_to(x, z, floor).found,
                        "{where_} points the agent at ({x}, {z}), which no route \
                         reaches\n{line}"
                    );
                }
            }
        }

        assert_eq!(
            orientations.len(),
            2,
            "sample covers only one shaft orientation, so it cannot catch an \
             entry/exit mix-up"
        );
    }

    /// Pull every "(x, z)" pair out of a state line.
    fn coordinates_in(line: &str) -> Vec<(f32, f32)> {
        line.split('(')
            .skip(1)
            .filter_map(|rest| rest.split_once(')'))
            .filter_map(|(inner, _)| inner.split_once(','))
            .filter_map(|(x, z)| Some((x.trim().parse().ok()?, z.trim().parse().ok()?)))
            .collect()
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
            let copies = s
                .find_carried_bag_copies("healing_potion", &spent)
                .unwrap_or_else(|| panic!("unit {unit} of 3 should still be in the bag"));
            let CarriedBagCopies::InBag { copies, .. } = copies else {
                panic!("expected InBag");
            };
            assert_eq!(copies, vec![(7, 4 - unit)]);
            *spent.entry(7).or_default() += 1;
        }
        assert!(s
            .find_carried_bag_copies("healing_potion", &spent)
            .is_none());

        let dropped = HashMap::from([(7, u32::MAX)]);
        assert!(s
            .find_carried_bag_copies("healing_potion", &dropped)
            .is_none());
    }

    /// A stack fragmented across two separate bag entries (e.g. two
    /// non-stackable pickups sharing an item_def_id, or a stack that never
    /// merged) is gathered as one pool spanning both instances.
    #[test]
    fn fragmented_stacks_are_gathered_across_every_instance() {
        let (mut s, _rx) = test_state();
        s.self_bag = vec![
            onlinerpg_shared::inventory::ItemInstance {
                instance_id: 1,
                item_def_id: "old_boot".to_string(),
                quantity: 1,
                enchant: 0,
            },
            onlinerpg_shared::inventory::ItemInstance {
                instance_id: 2,
                item_def_id: "old_boot".to_string(),
                quantity: 1,
                enchant: 0,
            },
        ];

        let CarriedBagCopies::InBag { def_id, copies } = s
            .find_carried_bag_copies("old_boot", &HashMap::new())
            .unwrap()
        else {
            panic!("expected InBag");
        };
        assert_eq!(def_id, "old_boot");
        assert_eq!(copies, vec![(1, 1), (2, 1)]);
    }

    /// Worn-only items report `WornOnly`, not `None` — the caller needs to
    /// tell "nothing by that name" apart from "you're wearing it".
    #[test]
    fn worn_only_item_is_not_a_bag_copy() {
        let (mut s, _rx) = test_state();
        s.self_equipped.insert(
            onlinerpg_shared::inventory::EquipSlot::MainHand,
            onlinerpg_shared::inventory::ItemInstance {
                instance_id: 9,
                item_def_id: "iron_sword".to_string(),
                quantity: 1,
                enchant: 0,
            },
        );

        let CarriedBagCopies::WornOnly { def_id } = s
            .find_carried_bag_copies("iron_sword", &HashMap::new())
            .unwrap()
        else {
            panic!("expected WornOnly");
        };
        assert_eq!(def_id, "iron_sword");
    }

    /// Put the agent on `depth`, standing on `cell`.
    fn stand_at(
        s: &mut SharedState,
        dungeon: &crate::dungeon::Dungeon,
        depth: u8,
        cell: (i32, i32),
    ) -> Position {
        let stand = onlinerpg_shared::dungeon::cell_center(&dungeon.entrance, depth, cell);
        s.self_floor_level = -(depth as i8);
        s.self_player = Some(Player {
            position: stand,
            floor_level: -(depth as i8),
            ..test_player(stand.x, stand.z)
        });
        stand
    }

    /// Standing where the chest room is, on the deepest floor.
    fn in_the_chest_room(s: &mut SharedState, dungeon: &crate::dungeon::Dungeon) -> u8 {
        let depth = dungeon.max_depth();
        let layout = dungeon.layouts().last().unwrap();
        let cell = layout.chest.unwrap();
        let room = layout.room_at(cell.0, cell.1).unwrap();
        stand_at(s, dungeon, depth, room.center());
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

    /// Another player's FishingEnded renders no prompt line, so scheduling an
    /// LLM cycle for it would buy a blank prompt; our own stays urgent.
    #[test]
    fn fishing_ended_wakes_llm_only_for_own_outcome() {
        let (mut s, _rx) = test_state();
        s.self_player_id = Some(PlayerId::from(1));
        let ended = |id: u64| ServerMessage::FishingEnded {
            player_id: PlayerId::from(id),
            outcome: onlinerpg_shared::fishing::FishingOutcome::Escaped,
        };
        assert_eq!(s.classify_event(&ended(1)), EventUrgency::Urgent);
        assert_eq!(s.classify_event(&ended(2)), EventUrgency::Noise);
    }

    /// The driver submits a prompt whenever the event buffer is non-empty, so
    /// a spectator ending must skip the buffer entirely, not just rank low.
    #[test]
    fn fishing_ended_buffers_only_own_outcome() {
        let (mut s, _rx) = test_state();
        s.self_player_id = Some(PlayerId::from(1));
        let ended = |id: u64| ServerMessage::FishingEnded {
            player_id: PlayerId::from(id),
            outcome: onlinerpg_shared::fishing::FishingOutcome::Escaped,
        };
        s.push_event(ended(2));
        assert!(s.events.is_empty(), "spectator ending must not buffer");
        s.push_event(ended(1));
        assert_eq!(s.events.len(), 1, "own ending must reach the prompt");
    }

    #[test]
    fn party_positions_do_not_wake_the_llm() {
        let (mut s, _rx) = test_state();
        let positions = ServerMessage::PartyPositions {
            members: Vec::new(),
        };
        assert_eq!(s.classify_event(&positions), EventUrgency::Noise);
        s.push_event(positions);
        assert!(s.events.is_empty());
    }

    /// The LLM will happily call for a new song halfway through the last one,
    /// which restarts the music for everyone listening. The command is dropped
    /// and the model is told why — on its next prompt, not by waking it here.
    #[test]
    fn a_second_tune_is_refused_while_the_first_still_plays() {
        let (mut s, _rx) = test_state();
        let me = test_player(0.0, 0.0);
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);
        s.in_game = true;

        assert!(
            !s.refuses_play_command("/play_music"),
            "nothing playing yet"
        );

        s.push_event(ServerMessage::PlayerMusicStarted {
            player_id: PlayerId::from(1),
            track: "Twilight Fields".to_string(),
            elapsed_secs: 0.0,
        });

        assert!(s.refuses_play_command("/play_music creekside"));
        assert!(s.refuses_play_command("/play_music"));
        assert!(
            !s.refuses_play_command("Any requests?"),
            "ordinary talk goes through"
        );
        assert!(
            !s.refuses_play_command("/play_musical chairs"),
            "only the whole command word counts"
        );

        let events = s.drain_agent_events();
        assert_eq!(events.len(), 2, "one note per dropped command: {events:?}");
        assert!(events[0].contains("still playing"), "{events:?}");
    }

    /// A title the LLM invented never reaches the server: it would answer
    /// "No such song" an hour before the idle prompt showed it, with the
    /// square still waiting on a song the bard announced.
    #[test]
    fn a_song_the_bard_does_not_know_is_refused_before_it_is_sent() {
        let (mut s, _rx) = test_state();
        let me = test_player(0.0, 0.0);
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);
        s.in_game = true;

        assert!(s.refuses_play_command("/play_music Ballad of the Missing Track"));
        assert_eq!(s.take_wake_urgency(), EventUrgency::Urgent);
        let events = s.drain_agent_events();
        assert!(
            events[0].contains("Ballad of the Missing Track") && events[0].contains("songbook"),
            "{events:?}"
        );

        // A second guess is still refused, but only at the routine cadence.
        assert!(s.refuses_play_command("/play_music Another Invention"));
        assert_eq!(s.take_wake_urgency(), EventUrgency::Routine);

        // Songbook titles pass, including one folded from a "(1)" variant.
        assert!(!s.refuses_play_command("/play_music Twilight Fields"));
        assert!(!s.refuses_play_command("/play_music Wanderer of the Old Fields"));
        assert!(!s.refuses_play_command("/play_music creekside"));
        assert!(!s.refuses_play_command("/play_music"), "the random pick");
    }

    /// A busker pauses between songs. The agent gets no invitation to play
    /// until the quiet spell is over, and asking early is refused with what
    /// is left of it.
    #[test]
    fn a_song_is_followed_by_a_quiet_spell_before_the_next() {
        let (mut s, _rx) = test_state();
        let me = test_player(0.0, 0.0);
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);
        s.in_game = true;

        s.push_event(ServerMessage::PlayerMusicStarted {
            player_id: PlayerId::from(1),
            track: "Twilight Fields".to_string(),
            elapsed_secs: 0.0,
        });
        s.push_event(ServerMessage::PlayerInteractionChanged {
            player_id: PlayerId::from(1),
            object_type: None,
        });

        let rest_until = s.self_music_rest_until.expect("a rest was scheduled");
        let rest = rest_until.saturating_duration_since(std::time::Instant::now());
        assert!(
            rest.as_secs() >= MUSIC_REST_MIN_SECS - 1 && rest.as_secs() <= MUSIC_REST_MAX_SECS,
            "{rest:?}"
        );

        assert!(
            s.refuses_play_command("/play_music"),
            "no encore during the rest"
        );
        s.check_music_finished();
        assert!(
            s.self_music_rest_until.is_some(),
            "the square is still resting"
        );

        s.self_music_rest_until =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        s.check_music_finished();
        let events = s.drain_agent_events();
        assert!(
            events.last().is_some_and(|e| e.contains("another song")),
            "{events:?}"
        );
        assert!(!s.refuses_play_command("/play_music"), "the rest is over");

        // In bed on the night schedule: playing would drop the sleeping pose
        // and nothing would put it back until morning.
        s.push_event(ServerMessage::PlayerInteractionChanged {
            player_id: PlayerId::from(1),
            object_type: Some("bed".to_string()),
        });
        assert!(s.refuses_play_command("/play_music"));
        let events = s.drain_agent_events();
        assert!(
            events.last().is_some_and(|e| e.contains("bed")),
            "{events:?}"
        );
    }

    /// The agent hears a tune start and end. Its own performance has no audio
    /// to end it, so the registry's length is the clock — without that an NPC
    /// bard would strum the same song forever.
    #[test]
    fn a_tune_is_announced_at_both_ends_and_our_own_stops_itself() {
        let (mut s, _rx) = test_state();
        let me = test_player(0.0, 0.0);
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);
        s.in_game = true;

        s.push_event(ServerMessage::PlayerMusicStarted {
            player_id: PlayerId::from(1),
            track: "Twilight Fields".to_string(),
            elapsed_secs: 0.0,
        });

        s.check_music_finished();
        assert!(
            s.drain_pending_commands().is_empty(),
            "the song is still playing"
        );

        if let Some(p) = s.self_performance.as_mut() {
            p.ends_at = std::time::Instant::now() - std::time::Duration::from_secs(1);
        }
        s.check_music_finished();
        assert!(matches!(
            s.drain_pending_commands().as_slice(),
            [ClientMessage::StopInteraction]
        ));

        // The server clears the interaction; that is what the LLM reads.
        s.push_event(ServerMessage::PlayerInteractionChanged {
            player_id: PlayerId::from(1),
            object_type: None,
        });
        let events = s.drain_agent_events();
        assert!(
            events
                .iter()
                .any(|e| e.contains("You finished \"Twilight Fields\"")),
            "{events:?}"
        );
    }

    /// Coins thrown at a busker's feet wait for the end of the song — walking
    /// over mid-tune would abandon the performance — and then name who to
    /// thank. Loot that no player dropped is not a tip.
    #[test]
    fn tips_left_during_a_song_are_announced_when_it_ends() {
        let (mut s, _rx) = test_state();
        let me = test_player(0.0, 0.0);
        s.plays_music = true;
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);
        s.in_game = true;
        let mut listener = test_player(1.0, 0.0);
        listener.id = PlayerId::from(2);
        listener.name = "Mira".to_string();
        s.nearby_players.insert(listener.id, listener);
        let tipper = PlayerId::from(2);

        // A tip before the first note of the day counts too: a busker is a
        // busker whether or not it happens to be playing right then.
        s.push_event(ServerMessage::GroundItemSpawned {
            item: dropped_item(1, "old_boot", 1.0, 0.0, tipper),
        });
        let events = s.drain_agent_events();
        assert!(
            events
                .iter()
                .any(|e| e.contains("[Tip] Mira left old_boot")),
            "{events:?}"
        );

        s.push_event(ServerMessage::PlayerMusicStarted {
            player_id: PlayerId::from(1),
            track: "Twilight Fields".to_string(),
            elapsed_secs: 0.0,
        });
        s.push_event(ServerMessage::GroundItemSpawned {
            item: dropped_item(2, "coin_pile", 1.0, 0.0, tipper),
        });
        // A tip thrown from too far off, a monster's loot, and our own drop.
        s.push_event(ServerMessage::GroundItemSpawned {
            item: dropped_item(3, "small_sword", TIP_RADIUS + 2.0, 0.0, tipper),
        });
        s.push_event(ServerMessage::GroundItemSpawned {
            item: ground_item(4, "goblin_sword", 1.0, 0.0, 0),
        });
        s.push_event(ServerMessage::GroundItemSpawned {
            item: dropped_item(5, "mandolin", 1.0, 0.0, PlayerId::from(1)),
        });
        assert_eq!(s.pending_tips.len(), 1, "{:?}", s.pending_tips);
        // Drops still get their [Sighted] line mid-song; only the [Tip]
        // thanks waits for the music to end.
        let mid_song = s.drain_agent_events();
        assert!(
            mid_song.iter().all(|e| !e.contains("[Tip]")),
            "the song comes before the thanks: {mid_song:?}"
        );
        assert!(
            mid_song.iter().all(|e| !e.contains("mandolin")),
            "our own drop must not be sighted: {mid_song:?}"
        );

        s.push_event(ServerMessage::PlayerInteractionChanged {
            player_id: PlayerId::from(1),
            object_type: None,
        });
        let events = s.drain_agent_events();
        assert!(
            events
                .last()
                .is_some_and(|e| e.contains("[Tip] Mira left coin_pile") && e.contains("[id 2]")),
            "{events:?}"
        );
        assert_eq!(s.take_wake_urgency(), EventUrgency::Routine);
        // Still on the ground, and still remembered as Mira's.
        assert!(
            s.format_world_state()
                .contains("Item on ground: coin_pile (1.0m away) [id 2], dropped by Mira"),
            "{}",
            s.format_world_state()
        );

        // Tipped again during the quiet spell: nothing to wait for now.
        s.push_event(ServerMessage::GroundItemSpawned {
            item: dropped_item(6, "gold_ring", 1.0, 0.0, tipper),
        });
        let events = s.drain_agent_events();
        assert!(
            events.last().is_some_and(|e| e.contains("gold_ring")),
            "{events:?}"
        );

        // Once the schedule has put it to bed, a tip is not worth getting up.
        s.push_event(ServerMessage::PlayerInteractionChanged {
            player_id: PlayerId::from(1),
            object_type: Some("bed".to_string()),
        });
        s.push_event(ServerMessage::GroundItemSpawned {
            item: dropped_item(7, "gold_ring", 1.0, 0.0, tipper),
        });
        let events = s.drain_agent_events();
        assert!(!events.iter().any(|e| e.contains("[Tip]")), "{events:?}");
    }

    /// Nobody tips a guard for standing there: a drop in front of an agent
    /// that does not busk is ordinary loot, and stays out of its events.
    #[test]
    fn only_a_busker_reads_a_drop_as_a_tip() {
        let (mut s, _rx) = test_state();
        let me = test_player(0.0, 0.0);
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);
        s.in_game = true;
        let mut passer_by = test_player(1.0, 0.0);
        passer_by.id = PlayerId::from(2);
        s.nearby_players.insert(passer_by.id, passer_by);

        s.push_event(ServerMessage::GroundItemSpawned {
            item: dropped_item(1, "coin_pile", 1.0, 0.0, PlayerId::from(2)),
        });

        let events = s.drain_agent_events();
        assert!(!events.iter().any(|e| e.contains("[Tip]")), "{events:?}");
    }

    /// A tip someone else grabs first is not thanked for at the end of the
    /// song — the bard would be pointing at bare ground. It hears who took
    /// it instead, and only once the song is over.
    #[test]
    fn a_tip_taken_before_the_song_ends_is_forgotten() {
        let (mut s, _rx) = test_state();
        let me = test_player(0.0, 0.0);
        s.plays_music = true;
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);
        s.in_game = true;
        let mut listener = test_player(1.0, 0.0);
        listener.id = PlayerId::from(2);
        s.nearby_players.insert(listener.id, listener);
        let mut thief = test_player(1.0, 1.0);
        thief.id = PlayerId::from(3);
        thief.name = "Bran".to_string();
        s.nearby_players.insert(thief.id, thief);

        s.push_event(ServerMessage::PlayerMusicStarted {
            player_id: PlayerId::from(1),
            track: "Twilight Fields".to_string(),
            elapsed_secs: 0.0,
        });
        s.push_event(ServerMessage::GroundItemSpawned {
            item: dropped_item(1, "coin_pile", 1.0, 0.0, PlayerId::from(2)),
        });
        s.push_event(ServerMessage::GroundItemRemoved {
            instance_id: 1,
            picked_up_by: Some(PlayerId::from(3)),
        });
        assert_eq!(s.take_wake_urgency(), EventUrgency::Noise, "not mid-song");
        s.push_event(ServerMessage::PlayerInteractionChanged {
            player_id: PlayerId::from(1),
            object_type: None,
        });

        let events = s.drain_agent_events();
        assert!(!events.iter().any(|e| e.contains("[Tip]")), "{events:?}");
        assert!(
            events
                .iter()
                .any(|e| e.contains("[GroundItem] Bran picked up coin_pile")),
            "{events:?}"
        );

        // Between songs there is nothing to hold it back.
        s.push_event(ServerMessage::GroundItemSpawned {
            item: dropped_item(2, "gold_ring", 1.0, 0.0, PlayerId::from(2)),
        });
        s.take_wake_urgency();
        s.push_event(ServerMessage::GroundItemRemoved {
            instance_id: 2,
            picked_up_by: Some(PlayerId::from(3)),
        });
        assert_eq!(s.take_wake_urgency(), EventUrgency::Routine);
        assert!(s
            .drain_agent_events()
            .iter()
            .any(|e| e.contains("Bran picked up gold_ring")));
    }

    /// A bard joining with the starter sword in hand swaps to its workhorse
    /// — the cheapest instrument that is not an offerable keepsake — so the
    /// good mandolin stays in the bag where the keepsake offer can reach
    /// it. A bard already holding the workhorse (or a non-bard) is left
    /// alone.
    #[test]
    fn a_joining_bard_takes_up_the_worn_mandolin() {
        use onlinerpg_shared::inventory::{EquipSlot, ItemInstance, PlayerInventory};

        let item = |instance_id: u64, def: &str| ItemInstance {
            instance_id,
            item_def_id: def.to_string(),
            quantity: 1,
            enchant: 0,
        };
        let mut inventory = PlayerInventory {
            bag: vec![item(1, "worn_mandolin"), item(2, "mandolin")],
            equipped: HashMap::from([(EquipSlot::MainHand, item(3, "worn_iron_sword"))]),
        };

        let (mut s, _rx) = test_state();
        s.plays_music = true;
        s.keepsake_ids = vec!["mandolin".to_string()];
        s.push_event(ServerMessage::InventoryState {
            inventory: inventory.clone(),
        });
        let equips: Vec<_> = s
            .drain_pending_commands()
            .into_iter()
            .filter(|c| matches!(c, ClientMessage::EquipItem { instance_id: 1 }))
            .collect();
        assert_eq!(equips.len(), 1, "swap to the worn workhorse, once");

        // With an instrument in hand, the next snapshot changes nothing.
        inventory.bag = vec![item(2, "mandolin"), item(4, "worn_iron_sword")];
        inventory.equipped = HashMap::from([(EquipSlot::MainHand, item(1, "worn_mandolin"))]);
        s.push_event(ServerMessage::InventoryState {
            inventory: inventory.clone(),
        });
        assert!(
            !s.drain_pending_commands()
                .iter()
                .any(|c| matches!(c, ClientMessage::EquipItem { .. })),
            "the workhorse in hand is left alone"
        );

        // Holding the good mandolin steps down to the worn one, freeing the
        // keepsake back into the bag.
        inventory.bag = vec![item(1, "worn_mandolin"), item(4, "worn_iron_sword")];
        inventory.equipped = HashMap::from([(EquipSlot::MainHand, item(2, "mandolin"))]);
        s.push_event(ServerMessage::InventoryState {
            inventory: inventory.clone(),
        });
        assert!(
            s.drain_pending_commands()
                .iter()
                .any(|c| matches!(c, ClientMessage::EquipItem { instance_id: 1 })),
            "the good mandolin in hand gives way to the workhorse"
        );

        // A non-bard keeps whatever it holds.
        let (mut guard, _rx2) = test_state();
        inventory.equipped = HashMap::from([(EquipSlot::MainHand, item(3, "worn_iron_sword"))]);
        guard.push_event(ServerMessage::InventoryState { inventory });
        assert!(
            !guard
                .drain_pending_commands()
                .iter()
                .any(|c| matches!(c, ClientMessage::EquipItem { .. })),
            "only buskers reach for an instrument"
        );
    }

    /// A `DoorToggled` must land on both faces of the door: the passability
    /// edge A* walks and the `HouseData` wall the door hunt reads. With only
    /// the edge updated, `closed_doors_on_our_floor` kept re-listing a door
    /// that was already open and the agent toggled it shut again.
    #[test]
    fn door_toggle_keeps_house_walls_in_step_with_the_edges() {
        use onlinerpg_shared::housing::{
            HouseData, PassabilityGrid, RoomData, WallConfig, WallDirection, WallVariant,
        };

        let wall = |variant| WallConfig {
            variant,
            texture: 0,
            is_open: false,
        };
        let room = RoomData {
            room_type: Default::default(),
            roof_type: Default::default(),
            roof_ridge_dir: Default::default(),
            stair_reversed: false,
            local_x: 0,
            local_z: 0,
            size_x: 1,
            size_z: 1,
            floor_level: 0,
            floor_texture: 0,
            roof_texture: 0,
            wall_height: 3.0,
            wall_north: vec![wall(WallVariant::WithDoor)],
            wall_south: vec![wall(WallVariant::Solid)],
            wall_east: vec![wall(WallVariant::Solid)],
            wall_west: vec![wall(WallVariant::Solid)],
        };

        let house = HouseData {
            id: "h".to_string(),
            owner_id: "test".to_string(),
            origin: onlinerpg_shared::Position {
                x: 10.0,
                y: 0.0,
                z: 10.0,
            },
            rooms: vec![room],
            passability: vec![PassabilityGrid {
                floor_level: 0,
                origin_x: 0,
                origin_z: 0,
                width: 1,
                depth: 1,
                // All four edges walled (N=1, E=2, S=4, W=8), door shut.
                cells: vec![1 | 2 | 4 | 8],
            }],
        };

        let mut world = WorldCache::new();
        world.add_house(house);

        let door_blocked = |world: &WorldCache| {
            pathfinding::is_movement_blocked(
                world.passability_cache(),
                10.5,
                10.5,
                10.5,
                9.5,
                0,
                None,
            )
        };
        assert!(door_blocked(&world), "the north door starts shut");

        world.update_door("h", 0, WallDirection::North, 0, true);
        assert!(
            world.houses()["h"].rooms[0].wall_north[0].is_open,
            "HouseData must track the open"
        );
        assert!(!door_blocked(&world), "the edge must open with the door");

        world.update_door("h", 0, WallDirection::North, 0, false);
        assert!(!world.houses()["h"].rooms[0].wall_north[0].is_open);
        assert!(door_blocked(&world), "the edge must seal again");
    }

    /// Splat tiles that paint one road cell and one river cell near origin,
    /// so the grid test can assert glyph placement against known world
    /// coordinates.
    struct PaintedSplat;

    #[async_trait::async_trait]
    impl crate::splat::SplatTiles for PaintedSplat {
        async fn read_splat(&self, tx: i32, tz: i32) -> std::io::Result<Vec<u8>> {
            let mut data = vec![0u8; onlinerpg_terrain::defaults::SPLATMAP_SIZE];
            if (tx, tz) == (0, 0) {
                let mut paint = |wx: f32, wz: f32, pal: u8| {
                    let cx = (wx + 32.0).floor() as usize;
                    let cz = (wz + 32.0).floor() as usize;
                    data[(cz * 64 + cx) * 4] = pal << 4;
                };
                paint(6.0, 0.0, crate::splat::PAL_ROAD);
                paint(-6.0, -6.0, crate::splat::PAL_RIVER_BED);
            }
            Ok(data)
        }
    }

    #[tokio::test]
    async fn terrain_grid_labels_world_coordinates_and_paints_surfaces() {
        let (mut s, _rx) = test_state();
        s.splat_sampler = Arc::new(crate::splat::SplatSampler::new(PaintedSplat));
        s.self_player = Some(test_player(0.0, 0.0));
        let grid = s.terrain_grid_job().expect("on the surface").render().await;

        assert!(
            grid.contains("x=-27 to x=27"),
            "header must carry the exact west/east span:\n{grid}"
        );
        assert!(grid.contains("Map: surface, you at (0, 0)"));

        let cells_of = |prefix: &str| -> Vec<String> {
            grid.lines()
                .find(|l| l.starts_with(prefix))
                .unwrap_or_else(|| panic!("no row {prefix} in:\n{grid}"))
                .split_whitespace()
                .skip(1)
                .map(str::to_string)
                .collect()
        };
        // Row z=0: self at column 9, the road cell (6, 0) at column 11.
        let mid = cells_of("z=0 ");
        assert_eq!(mid[9], "@");
        assert_eq!(mid[11], "R");
        // Row z=-6: the river cell (-6, -6) at column 7.
        let north = cells_of("z=-6 ");
        assert_eq!(north[7], "~");
    }

    #[test]
    fn terrain_grid_is_absent_underground() {
        let (mut s, _rx) = test_state();
        s.self_player = Some(test_player(0.0, 0.0));
        s.self_floor_level = -1;
        assert!(s.terrain_grid_job().is_none());
    }

    /// An invalid_target rejection is the server saying the monster does not
    /// exist; the stale entry must leave the list instead of being offered
    /// to the LLM again next turn.
    #[test]
    fn invalid_target_rejection_drops_the_ghost_monster() {
        let (mut s, _rx) = test_state();
        s.nearby_monsters
            .insert("m_ghost".into(), monster("m_ghost"));
        s.push_event(ServerMessage::PlayerAttackRejected {
            monster_id: "m_ghost".into(),
            reason: onlinerpg_shared::AttackRejectReason::InvalidTarget,
        });
        assert!(!s.nearby_monsters.contains_key("m_ghost"));
        // Out-of-range rejections say nothing about existence.
        s.nearby_monsters.insert("m_far".into(), monster("m_far"));
        s.push_event(ServerMessage::PlayerAttackRejected {
            monster_id: "m_far".into(),
            reason: onlinerpg_shared::AttackRejectReason::OutOfRange,
        });
        assert!(s.nearby_monsters.contains_key("m_far"));
    }

    #[test]
    fn quiet_grazers_get_no_sighting_event_but_hunters_do() {
        let (mut s, _rx) = test_state();
        s.self_player = Some(test_player(0.0, 0.0));
        let mut wolf = monster("m_wolf");
        wolf.aggressive = true;
        wolf.position = p(10.0, 0.0, 0.0);
        let mut slime = monster("m_slime");
        slime.position = p(-10.0, 0.0, 0.0);
        s.nearby_monsters.insert("m_wolf".into(), wolf);
        s.nearby_monsters.insert("m_slime".into(), slime);

        s.check_sightings();

        let sighted: Vec<&String> = s
            .agent_events
            .iter()
            .filter(|e| e.starts_with("[Sighted]"))
            .collect();
        assert_eq!(sighted.len(), 1, "only the aggressive monster: {sighted:?}");
        assert!(sighted[0].contains("m_wolf"));
        assert!(
            sighted[0].contains("at (10, 0), 10m east"),
            "sighting must carry coordinates and bearing: {}",
            sighted[0]
        );
    }

    /// A fresh drop (monster loot, chest ejection) arrives as
    /// GroundItemSpawned; it must fire its sighting right away, not wait for
    /// the next move to trigger a re-check.
    #[test]
    fn a_fresh_drop_is_sighted_immediately() {
        let (mut s, _rx) = test_state();
        s.self_player = Some(test_player(0.0, 0.0));
        s.push_event(ServerMessage::GroundItemSpawned {
            item: ground_item(77, "goblin_sword", 8.0, 0.0, 0),
        });
        assert!(
            s.agent_events
                .iter()
                .any(|e| e.starts_with("[Sighted]") && e.contains("goblin_sword")),
            "events: {:?}",
            s.agent_events
        );
    }
}
