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
/// Same cap for friend requests. The server caps them per requester, not per
/// target, so without this any number of strangers can grow the prompt.
const MAX_PENDING_FRIEND_REQUESTS: usize = 3;
/// How long an NPC-pushed trade window stays answerable, mirroring the web
/// client's offer toast (`OFFER_TTL_MS` in `TradeOfferToast.svelte`). Both
/// wave the offer off once it lapses.
const TRADE_OFFER_TTL: std::time::Duration = std::time::Duration::from_secs(30);
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

mod commands;
mod dungeon;
mod events;
mod inventory;
mod movement;
mod music;
mod perception;
mod social;
mod terrain_grid;
#[cfg(test)]
pub(crate) mod tests;
mod world_cache;
mod world_state;

pub use commands::ActionProgress;
pub use events::EventUrgency;
pub use inventory::{Carried, CarriedBagCopies};
pub use movement::{MoveTarget, MoveTargetError};
pub use social::{PendingFriendRequest, PendingPartyInvite, PendingPartySummon, PushedTrade};
pub use world_cache::WorldCache;

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
    /// Last stance the fight reflex committed to, so each `FishingFight` beat
    /// only reacts on change.
    fishing_stance: Option<onlinerpg_shared::fishing::FishingAction>,
    /// The in-flight reaction; one answer at a time, so beats arriving while
    /// it runs are missed exactly as a person misses them.
    fishing_reaction: Option<tokio::task::JoinHandle<()>>,
    /// Unanswered party invites, oldest first (capped; a flood can't swap
    /// the invite out from under an in-flight `party_accept`). Expired
    /// invites are pruned on mutation and skipped on read, so a dead invite
    /// stops prompting the model.
    pub pending_party_invites: Vec<PendingPartyInvite>,
    /// Unanswered summons, same queue discipline as invites.
    pub pending_party_summons: Vec<PendingPartySummon>,
    /// Unanswered friend requests, same queue discipline as invites.
    pub pending_friend_requests: Vec<PendingFriendRequest>,
    /// Friend roster from `FriendList`, re-sent by the server on any change.
    /// Maps the character ids in `FriendsOnline` back to names.
    pub friends: Vec<onlinerpg_shared::messages::FriendEntry>,
    /// Tip hats in our AOI, so the agent can drop coins in one.
    pub tip_hats: HashMap<u64, onlinerpg_shared::tip_hat::TipHat>,
    /// An NPC-pushed trade window not yet acted on (`ShopState` arrives
    /// unrequested — the agent never sends `OpenShop`). `decline_trade`
    /// answers it, buying or selling clears it, and an untouched one lapses
    /// after `TRADE_OFFER_TTL` so a new offer still reads as new.
    pub pushed_trade: Option<PushedTrade>,
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
    /// Commands an agent action put on the wire, ever. Read only as a
    /// difference, so background traffic must stay out of it — see
    /// [`Self::send_background_command`].
    action_commands_sent: u64,
    /// Agent events an action's own handler pushed, ever. Read only as a
    /// difference; ambient pushes (clock ticks, sightings, tips) stay out —
    /// see [`Self::push_ambient_event`].
    action_events_pushed: u64,
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
            fishing_reaction: None,
            pending_party_invites: Vec::new(),
            pending_party_summons: Vec::new(),
            pending_friend_requests: Vec::new(),
            friends: Vec::new(),
            tip_hats: HashMap::new(),
            pushed_trade: None,
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
            action_commands_sent: 0,
            action_events_pushed: 0,
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
}
