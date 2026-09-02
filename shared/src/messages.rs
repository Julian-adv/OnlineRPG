//! WebSocket protocol envelopes between client and server. `ClientMessage`
//! is everything a client can ask for (move, attack, place house, equip
//! item …); `ServerMessage` is everything the server pushes back (world
//! snapshots, combat results, inventory deltas, kicks). Both serialize
//! via MessagePack — convenience helpers at the bottom of the file
//! centralise the `rmp_serde::to_vec` / `from_slice` calls so callers
//! don't have to know the wire format.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::character::{Character, CharacterAttributes, CharacterClass, Gender};
use crate::entity::{Monster, MonsterState, Player};
use crate::world::{GameDateTime, Position};
use crate::{fishing, housing, inventory, skills};

/// Which side of a merchant trade a haggled deal applies to.
/// `Buy` = the player buys from the merchant, `Sell` = the player sells to
/// the merchant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DealKind {
    Buy,
    Sell,
}

/// Why a `PlayerAttack` request was dropped. Deliberately coarse: a stale id
/// must not reveal hidden monster state such as its floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttackRejectReason {
    InvalidTarget,
    OutOfRange,
    AttackerDead,
    NotInGame,
}

impl std::fmt::Display for AttackRejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidTarget => "invalid_target",
            Self::OutOfRange => "out_of_range",
            Self::AttackerDead => "attacker_dead",
            Self::NotInGame => "not_in_game",
        })
    }
}

/// A haggled price modifier on one item, as included in `ShopState`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveDeal {
    pub item_def_id: String,
    pub kind: DealKind,
    /// Percentage points added to the normal price (negative = discount on
    /// buys, positive = bonus on sells).
    pub modifier_pct: i32,
    pub expires_in_secs: u32,
}

/// One purchasable item in a non-merchant trader's real inventory, as
/// included in `ShopState`. Merchants use `catalog` (unlimited stock)
/// instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockEntry {
    pub item_def_id: String,
    pub quantity: u32,
}

/// One unit the player recently sold to a merchant, repurchasable at the
/// exact payout the player received. Sold units normally vanish (merchants
/// keep no stock), so this is the only way to undo a mis-sell.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuybackEntry {
    /// Server-issued id the client references in `BuybackItem`; becomes the
    /// restored item's instance id on repurchase.
    pub entry_id: u64,
    pub item_def_id: String,
    pub enchant: i32,
    /// Dye the sold cape carried, so buying it back returns it dyed.
    #[serde(default)]
    pub cape_color: Option<String>,
    /// Same for its texture hash.
    #[serde(default)]
    pub cape_texture: Option<String>,
    /// Gold the player was paid for the unit (smallest unit) — buying it
    /// back costs exactly this, so the round trip is gold-neutral.
    pub price: i64,
}

/// One line of a batched `BuyItems` request: buy `qty` units of one item def.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeLineItem {
    pub item_def_id: String,
    pub qty: u32,
}

/// One line of a batched `SellItems` or `DropItems` request: act on `qty`
/// units of one bag stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BagLineItem {
    pub instance_id: u64,
    pub qty: u32,
}

/// One party member as listed in `PartyState`. `hp`/`max_hp` are the
/// roster-time snapshot; steady-state updates ride `PartyVitals`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartyMember {
    pub id: PlayerId,
    pub name: String,
    pub hp: u32,
    pub max_hp: u32,
    pub class: crate::character::CharacterClass,
}

/// One member's health as listed in `PartyVitals`. No name or class: the
/// roster from `PartyState` already carries them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartyMemberVitals {
    pub id: PlayerId,
    pub hp: u32,
    pub max_hp: u32,
}

/// How long a party invite stays acceptable. Shared so the server's
/// enforcement and the agent-client's pruning are guaranteed equal; the web
/// client mirrors it (`INVITE_TTL_MS` in `partyStore.ts`).
pub const PARTY_INVITE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// How long a party summon stays acceptable; `PARTY_INVITE_TTL`'s twin. The
/// web client mirrors it (`SUMMON_TTL_MS` in `partyStore.ts`).
pub const PARTY_SUMMON_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// One member's location as listed in `PartyPositions`. No name: the roster
/// from `PartyState` already carries it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartyMemberPosition {
    pub id: PlayerId,
    pub x: f32,
    pub z: f32,
    pub floor_level: i8,
}

/// One friend as listed in `FriendList`. Keyed by character id, not the
/// per-session `PlayerId`: a friendship outlives both sessions, and offline
/// friends have no player id at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FriendEntry {
    pub character_id: i64,
    pub name: String,
    pub level: u32,
    pub class: crate::character::CharacterClass,
}

/// One online friend as listed in `FriendsOnline`. No name — `FriendList`
/// already carries it — but the level rides along, so a friend's level-ups
/// show without re-sending the whole roster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnlineFriend {
    pub character_id: i64,
    pub level: u32,
}

/// How long a friend request stays answerable. Four times
/// `PARTY_INVITE_TTL`: a party invite is an offer to play *now*, a friend
/// request can wait out the fight the target is in. The web client mirrors it
/// (`FRIEND_REQUEST_TTL_MS` in `friendStore.ts`).
pub const FRIEND_REQUEST_TTL: std::time::Duration = std::time::Duration::from_secs(120);

/// How long a player-trade request stays answerable. `PARTY_INVITE_TTL`'s
/// length for the same reason: trading is an offer to meet *now*.
pub const PLAYER_TRADE_REQUEST_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// How long an open trade session survives without either side touching it.
/// Long, because haggling has real pauses — but bounded, because an offered
/// item is reserved out of its owner's other actions (doc/TRADE.md).
pub const PLAYER_TRADE_IDLE_TTL: std::time::Duration = std::time::Duration::from_secs(180);

/// One item in a player-trade offer. Carries `enchant` because +0 and +7 are
/// otherwise indistinguishable, which is the cleanest scam in the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerTradeItem {
    pub instance_id: u64,
    pub item_def_id: String,
    pub quantity: u32,
    pub enchant: i32,
    #[serde(default)]
    pub cape_color: Option<String>,
    #[serde(default)]
    pub cape_texture: Option<String>,
}

/// What a client asks to put on the table: whole-offer, never a delta, so a
/// dropped packet cannot desync the two windows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerTradeSlot {
    pub instance_id: u64,
    pub quantity: u32,
}

/// One side of a live trade as the server sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerTradeSide {
    pub player_id: PlayerId,
    pub name: String,
    pub items: Vec<PlayerTradeItem>,
    pub copper: i64,
    pub locked: bool,
    pub confirmed: bool,
}

/// The whole session, re-sent on every change. `you` is always the recipient's
/// own side, so each client gets its own view of the same revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerTradeState {
    /// Bumped on every offer change. A `PlayerTradeConfirm` naming a stale
    /// revision is refused — this is what defeats the last-second swap.
    pub revision: u32,
    pub you: PlayerTradeSide,
    pub them: PlayerTradeSide,
}

/// Web client's rendering environment, so performance complaints can be
/// matched against actual hardware. The client sends it after entering the
/// game, but only when the environment changed since the last report. Field
/// names mirror the client's `ClientEnvReport` interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientEnvReport {
    pub quality: String,
    pub render_budget: String,
    pub antialias: bool,
    pub pixel_ratio: f32,
    pub device_pixel_ratio: f32,
    pub viewport_w: u32,
    pub viewport_h: u32,
    pub screen_w: u32,
    pub screen_h: u32,
    pub backend: String,
    pub gpu_vendor: String,
    pub gpu_architecture: String,
    pub gpu_device: String,
    pub gpu_description: String,
    pub user_agent: String,
}

/// Interaction the server stores for a `/play_music` performance. A wire
/// contract, not a private constant: the server sets it, and both clients
/// compare `PlayerInteractionChanged` against it to know a tune is over.
pub const MUSIC_EMOTE: &str = "guitar_playing";

/// One-shot clips `/emote <name>` may store as the interaction. Same wire
/// contract as [`MUSIC_EMOTE`]: the server validates against this list, and
/// clients start the clip off the broadcast and send `StopInteraction` when
/// it ends. Clip names live in `social.glb`.
pub const ONE_SHOT_EMOTES: &[&str] = &["excited", "clap", "yawn"];

/// Clips `/emote <name>` loops instead of playing once: the dancer's client
/// repeats the clip until the player moves or presses Escape, then sends
/// `StopInteraction` — the held-pose contract of [`MUSIC_EMOTE`], minus the
/// music. Clip names live in `social.glb`.
pub const LOOPING_EMOTES: &[&str] = &[
    "twist",
    "macarena",
    "chicken",
    "stand_pose2",
    "stand_pose3",
    "stand_pose4",
    "weight_shift",
];

/// `message` is `prefix` as a whole slash-command word; returns the trimmed
/// remainder. Shared because the agent-client types the commands this parses —
/// the two sides must agree on what counts as the command word.
pub fn strip_command<'a>(message: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = message.trim().strip_prefix(prefix)?;
    (rest.is_empty() || rest.starts_with(' ')).then(|| rest.trim())
}

/// The title a `/play_music` argument names: a whole title first, then a
/// fragment of one, ignoring case. Shared for the same reason as
/// `strip_command` — the server resolves the query and the agent-client
/// decides beforehand whether it would resolve at all. An empty query is the
/// server's random pick, which is the caller's business, not this rule's.
pub fn resolve_title<'a>(
    mut titles: impl Iterator<Item = &'a str> + Clone,
    query: &str,
) -> Option<&'a str> {
    let wanted = query.trim().to_lowercase();
    if wanted.is_empty() {
        return None;
    }
    titles
        .clone()
        .find(|t| t.to_lowercase() == wanted)
        .or_else(|| titles.find(|t| t.to_lowercase().contains(&wanted)))
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Mandatory first message: protocol check plus who is connecting. The
    /// server refuses anything else until it arrives, and refuses the
    /// connection outright when `protocol_version` differs from its own.
    ///
    /// This shape is frozen. Together with `ServerMessage::AuthError` it is
    /// the only channel that can tell an out-of-date client why it was
    /// refused, so extra data goes in a new message, never in here.
    ClientInfo {
        protocol_version: u32,
        /// Which client program: "web" (browser) or "cli" (agent-client).
        /// Self-reported, used only for the `/who` breakdown — never for
        /// permissions, or clients would have a reason to lie.
        client_kind: String,
        client_version: String,
    },
    /// Browser login: a Google ID token, verified server-side. The account is
    /// looked up (or created) by the token's `sub` claim.
    Authenticate {
        google_id_token: String,
    },
    /// Headless bot login, gated by the server's shared NPC token. The
    /// account is auto-created on first use.
    AuthenticateNpc {
        account_name: String,
        npc_token: String,
    },
    CreateCharacter {
        character_name: String,
        character_class: CharacterClass,
        gender: Gender,
    },
    RollCharacterStats {
        character_class: CharacterClass,
        gender: Gender,
    },
    DeleteCharacter {
        character_id: i64,
    },
    /// New name for one of the account's characters, sent from character
    /// select after the server refused entry with `CharacterRenameRequired`.
    RenameCharacter {
        character_id: i64,
        new_name: String,
    },
    EnterGame {
        character_id: i64,
    },
    /// The scene has finished compiling, so the player can be hit again. See
    /// `entity::WORLD_LOADING_GRACE_MS`.
    WorldReady,
    PlayerMove {
        position: Position,
        rotation: f32,
        #[serde(default)]
        floor_level: i8,
        /// Append to the server's waypoint queue instead of replacing it.
        /// Path-following sends use this so the server walks the same
        /// client-validated polyline; fresh paths (click, keyboard, combat)
        /// replace.
        #[serde(default)]
        append: bool,
        #[serde(default)]
        sprinting: bool,
    },
    /// Floor change that happens *between* waypoints. `PlayerMove::floor_level`
    /// only lands when its waypoint is reached, and a stairwell is a single leg
    /// (A* omits intermediate stair cells), so without this a player descending
    /// stairs stays in the upper floor's AOI until they hit the bottom landing.
    PlayerFloorChanged {
        floor_level: i8,
    },
    ChatMessage {
        message: String,
    },
    MonsterMove {
        monster_id: String,
        position: Position,
        rotation: f32,
        state: MonsterState,
        /// Where a remote view walks the model until the next sync — a point on
        /// the mover's own path, not its destination. Aiming a viewer's straight
        /// line at the destination walks the model through the walls the path
        /// goes around. See `MonsterBrain::current_leg_target`.
        target_position: Position,
    },
    PlayerAttack {
        monster_id: String,
    },
    MonsterAttack {
        monster_id: String,
        target_player_id: PlayerId,
    },
    RequestRespawn,
    /// Open the treasure chest on a dungeon's final floor. The server
    /// validates proximity, boss state and the per-player cooldown.
    OpenDungeonChest {
        entrance_id: String,
    },
    /// Break a destructible dungeon prop (barrel/crate). The server validates
    /// floor, proximity and prop kind, records the break for the dungeon
    /// instance, opens the cell for movement and broadcasts it nearby.
    BreakDungeonProp {
        entrance_id: String,
        depth: u8,
        prop_id: u32,
    },
    /// Open an interactive dungeon chest prop (plays its lid animation). The
    /// server validates floor, proximity and prop kind, records the open for
    /// the dungeon instance and broadcasts it nearby. The chest stays solid
    /// (no passability change) — only the lid animates.
    OpenDungeonProp {
        entrance_id: String,
        depth: u8,
        prop_id: u32,
    },
    /// Toggle a dungeon door's open state. `depth` 0 is the surface entrance
    /// door; ≥1 is an interior room door. `door_id` is the client's opaque
    /// door key (derived from the door's geometry). The server flips the
    /// stored state for (entrance, depth, door_id) and broadcasts nearby
    /// (surface floor for depth 0, the toggler's floor for interior doors).
    ToggleDungeonDoor {
        entrance_id: String,
        depth: u8,
        door_id: u32,
    },
    /// Ask for the open/closed state of every door in a dungeon (entrance +
    /// interior, all depths). The server replies with DungeonDoorsState. Sent
    /// when the client registers the dungeon and again when crossing into
    /// toggle-delivery range, so doors others left open render correctly.
    RequestDungeonDoors {
        entrance_id: String,
    },
    DebugTeleport {
        position: Position,
    },
    DebugDropItem {
        item_def_id: String,
    },
    DebugSetTime {
        hour: u8,
        minute: u8,
    },
    DebugResetDungeonProps {
        entrance_id: String,
    },
    TorchToggle {
        enabled: bool,
    },
    InteractObject {
        object_type: String,
        object_id: u32,
    },
    StopInteraction,
    Heartbeat,
    PlaceHouse {
        house: housing::HouseData,
    },
    ModifyRoom {
        house_id: String,
        room_index: u32,
        room: housing::RoomData,
    },
    RemoveHouse {
        house_id: String,
    },
    ToggleDoor {
        house_id: String,
        room_index: u32,
        wall_dir: housing::WallDirection,
        segment_index: u32,
    },
    EquipItem {
        instance_id: u64,
    },
    UnequipItem {
        slot: inventory::EquipSlot,
    },
    DropItem {
        instance_id: u64,
    },
    /// Drop multiple bag stacks (partial quantities allowed) in one
    /// all-or-nothing transaction, so a multi-item bag cleanup round-trips
    /// once instead of once per stack.
    DropItems {
        items: Vec<BagLineItem>,
    },
    /// The pickup crouch started. Sent at the clip's first frame, whereas
    /// `PickupItem` waits for the grab moment ~35% in, so nearby players see
    /// the whole animation instead of joining it late.
    PickupStarted,
    PickupItem {
        instance_id: u64,
    },
    /// Consume a usable item from the bag (e.g. drink a healing potion).
    UseItem {
        instance_id: u64,
    },
    /// Dye the worn cape with `color` (`#rrggbb`), spending the dye at
    /// `instance_id`. Answers a `CapeDyePrompt`; the server re-checks
    /// everything (doc/CAPE_CUSTOMIZATION.md).
    DyeCape {
        instance_id: u64,
        color: String,
    },
    /// Put the already-uploaded texture `texture` (a content hash) on the worn
    /// cape, spending the transfer kit at `instance_id`. Answers a
    /// `CapeTexturePrompt`; the server re-checks everything.
    ApplyCapeTexture {
        instance_id: u64,
        texture: String,
    },
    /// Report the cape texture another player is wearing. The server records
    /// the hash, the reporter and the target for an admin to review.
    ReportCapeTexture {
        player_id: PlayerId,
    },
    /// Drop `amount` copper into a nearby tip hat. The server checks the
    /// wallet, the distance and that the hat isn't the sender's own.
    TipHat {
        hat_id: u64,
        amount: i64,
    },
    /// Official NPC only: set `item_def_id` on the table in front of the
    /// occupied chair `chair_object_id`. The server resolves the table top.
    ServeMeal {
        chair_object_id: u32,
        item_def_id: String,
    },
    /// Eat the plate served to the chair the sender is sitting on.
    EatMeal {
        meal_id: u64,
    },
    /// Official NPC only: take an abandoned plate away.
    ClearMeal {
        meal_id: u64,
    },
    /// Show `title` above the name, or nothing. Ignored unless the character
    /// has earned it (doc/TITLES.md).
    SetActiveTitle {
        title: Option<String>,
    },
    /// Ask a merchant NPC to open its shop.
    OpenShop {
        merchant_player_id: PlayerId,
    },
    /// Tell the server the player closed a merchant's trade window. The
    /// server tracks open windows so a trading NPC can be held in place (its
    /// LLM movement is suppressed) while a customer is shopping with it.
    CloseShop {
        merchant_player_id: PlayerId,
    },
    /// Buy one unit of an item from a merchant's catalog at base price.
    BuyItem {
        merchant_player_id: PlayerId,
        item_def_id: String,
    },
    /// Sell one unit of a bag item to a merchant at its sell rate.
    SellItem {
        merchant_player_id: PlayerId,
        instance_id: u64,
    },
    /// Repurchase a unit previously sold to this merchant, at the payout
    /// price recorded in its `BuybackEntry`.
    BuybackItem {
        merchant_player_id: PlayerId,
        entry_id: u64,
    },
    /// Buy multiple units, possibly of different items, in one all-or-nothing
    /// transaction (see `SellItems` for the mirror).
    BuyItems {
        merchant_player_id: PlayerId,
        items: Vec<TradeLineItem>,
    },
    /// Sell multiple bag stacks (partial quantities allowed) in one
    /// all-or-nothing transaction.
    SellItems {
        merchant_player_id: PlayerId,
        items: Vec<BagLineItem>,
    },
    /// Repurchase multiple buyback entries at once, all-or-nothing.
    BuybackItems {
        merchant_player_id: PlayerId,
        entry_ids: Vec<u64>,
    },
    /// NPC-only (LLM haggling): offer a price modifier on one item to a
    /// nearby player. The server clamps the modifier to the player's price
    /// band and enforces budgets/cooldowns; see `doc/ECONOMY.md`.
    OfferDeal {
        target_player_id: PlayerId,
        item_def_id: String,
        kind: DealKind,
        /// Requested percentage points off/on the normal price
        /// (negative = discount on buys, positive = bonus on sells).
        modifier_pct: i32,
        /// LLM's stated reason for the decision (logged server-side).
        reason: String,
    },
    /// NPC-only: push the sender's trade window (`ShopState`) onto a nearby
    /// player's client — the conversational entry point for trading
    /// ("LLM opens the trade window", doc/ECONOMY.md).
    OpenTrade {
        target_player_id: PlayerId,
    },
    /// Wave off an NPC-pushed trade offer ("Not now" on the toast, or the
    /// toast timing out unanswered). Relayed to the NPC as `TradeDeclined`
    /// so its agent stops pushing trade windows at that player for a while.
    DeclineTrade {
        merchant_player_id: PlayerId,
    },
    /// Ask a named online player to trade. Name-based like `PartyInvite`, but
    /// unlike it the target must also be within `MAX_TRADE_DISTANCE`.
    PlayerTradeRequest {
        target_name: String,
    },
    /// Open a trade directly against a laid-out stall. Setting a stall out is
    /// the owner's standing consent, so this skips the request step.
    PlayerTradeAtStall {
        stall_id: u64,
    },
    /// Accept or decline a pending trade request from `requester_id`.
    PlayerTradeRespond {
        requester_id: PlayerId,
        accept: bool,
    },
    /// Replace the sender's whole side of the table. Whole-offer rather than
    /// add/remove so the server never has to reconcile a partial view.
    PlayerTradeSetOffer {
        items: Vec<PlayerTradeSlot>,
        copper: i64,
    },
    /// Freeze the sender's side at `revision`. Refused if the revision moved.
    PlayerTradeLock {
        revision: u32,
    },
    /// Reopen the sender's side for edits, clearing both confirmations.
    PlayerTradeUnlock,
    /// Commit the sender's side. Both sides confirmed at the same revision
    /// executes the swap; a stale revision is refused.
    PlayerTradeConfirm {
        revision: u32,
    },
    /// Abandon the session, releasing both sides' reservations.
    PlayerTradeCancel,
    /// Invite a named player to the sender's party. Name-based like whisper:
    /// the target may be outside the sender's AOI.
    PartyInvite {
        target_name: String,
    },
    /// Accept or decline a pending party invite from `inviter_id`.
    PartyRespond {
        inviter_id: PlayerId,
        accept: bool,
    },
    /// Accept or decline a pending party summon from `caster_id`.
    PartySummonRespond {
        caster_id: PlayerId,
        accept: bool,
    },
    /// Leave the current party. The leader leaving promotes the earliest
    /// remaining member; a party reduced to one member disbands.
    PartyLeave,
    /// Leader-only: remove `target_id` from the sender's party. A party
    /// reduced to one member disbands, like `PartyLeave`.
    PartyKick {
        target_id: PlayerId,
    },
    /// Leader-only: hand party leadership to `target_id`.
    PartyPromote {
        target_id: PlayerId,
    },
    /// Say something to the sender's party. Delivered to every online member
    /// wherever they are (no AOI cut), echoed to the sender included.
    PartyChat {
        message: String,
    },
    /// Accept or decline a pending friend request from `requester_id`.
    FriendRespond {
        requester_id: PlayerId,
        accept: bool,
    },
    /// Drop a friendship, both directions. Name-based like `PartyInvite`: the
    /// friend may be offline, so no player id exists to name them by.
    FriendRemove {
        name: String,
    },
    /// Ask which of the sender's friends are online right now. Polled by the
    /// client (faster while the panel is open); there is no presence push.
    RequestFriendsOnline,
    /// Ask where the sender's party members are right now (map open). A
    /// one-shot snapshot: steady-state updates are pushed by the server's
    /// party-position tick whenever a member relocates.
    RequestPartyPositions,
    /// Cast the equipped fishing rod at a water point. The server validates
    /// rod, range, floor and water (water-field depth at the point) and
    /// answers with a `FishingCasted` broadcast or a direct `FishingError`.
    FishingCast {
        position: Position,
    },
    /// Respond to the fish (currently only `Hook`, on a bite). Timing is
    /// judged server-side against the bite deadline plus latency grace.
    FishingRespond {
        action: fishing::FishingAction,
    },
    /// Reel in deliberately. Also implied by moving, attacking or
    /// disconnecting — any of them ends the session as `Aborted`.
    FishingStop,
    /// Logged server-side only; accepted once per connection.
    EnvReport(ClientEnvReport),
}

impl ClientMessage {
    /// A queue-replacing PlayerMove (`append: false`), the common case.
    pub fn player_move(position: Position, rotation: f32, floor_level: i8) -> Self {
        Self::PlayerMove {
            position,
            rotation,
            floor_level,
            append: false,
            sprinting: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    AuthSuccess {
        account_name: String,
        characters: Vec<Character>,
        /// Bearer credential for the player's own REST calls (cape texture
        /// upload). Separate from the Google id token, which expires inside
        /// an hour while a session runs all evening.
        #[serde(default)]
        cape_upload_token: String,
    },
    JoinSuccess {
        player: Player,
        /// Unlocks debug/cheat UI; resolved per character at EnterGame.
        is_admin: bool,
    },
    AuthError {
        message: String,
    },
    CharacterCreated {
        character: Character,
    },
    CharacterStatsRolled {
        attributes: CharacterAttributes,
        max_hp: u32,
    },
    CharacterDeleted {
        character_id: i64,
    },
    /// Entry refused: the character's name is on the banned list. NPC
    /// accounts never see this, having no one to answer the prompt.
    CharacterRenameRequired {
        character_id: i64,
    },
    CharacterRenamed {
        character_id: i64,
        name: String,
    },
    CharacterError {
        message: String,
    },
    PlayerJoined {
        player: Player,
    },
    PlayerLeft {
        player_id: PlayerId,
    },
    PlayerAppeared {
        player: Player,
    },
    PlayerDisappeared {
        player_id: PlayerId,
    },
    PlayerMoved {
        player_id: PlayerId,
        position: Position,
        rotation: f32,
        #[serde(default)]
        floor_level: i8,
        #[serde(default)]
        sprinting: bool,
    },
    PlayerTeleported {
        player_id: PlayerId,
        position: Position,
        rotation: f32,
        #[serde(default)]
        floor_level: i8,
    },
    /// A dungeon treasure chest was opened. The rolled items burst out of
    /// the chest as ground drops moments later; the gold goes straight to
    /// the opener's wallet. Broadcast nearby — except when `item_def_ids`
    /// is empty and `gold` is 0: that is a re-open of a chest the opener
    /// already claimed tonight (real opens always pay gold), sent to the
    /// opener alone so their lid swings on an empty box.
    DungeonChestOpened {
        entrance_id: String,
        player_id: PlayerId,
        item_def_ids: Vec<String>,
        gold: i64,
    },
    /// A destructible dungeon prop was broken: swap it to its broken variant
    /// and open its cell for movement. Broadcast to nearby players (the
    /// breaker included).
    DungeonPropBroken {
        entrance_id: String,
        depth: u8,
        prop_id: u32,
    },
    /// An interactive dungeon chest prop was opened: play its lid-open
    /// animation. Broadcast to nearby players (the opener included).
    DungeonPropOpened {
        entrance_id: String,
        depth: u8,
        prop_id: u32,
    },
    /// Snapshot of which props on a floor are already broken or opened, sent
    /// directly to a player as they enter that dungeon floor so late arrivals
    /// render the current state (broken debris + chests in the open pose).
    DungeonPropsState {
        entrance_id: String,
        depth: u8,
        broken: Vec<u32>,
        opened: Vec<u32>,
    },
    /// A dungeon door was toggled (surface entrance at depth 0, or an interior
    /// room door at depth ≥1). Delivered to nearby players on the door's floor
    /// (surface floor for depth 0); the toggler always receives it. Clients
    /// re-pull DungeonDoorsState when crossing into delivery range.
    DungeonDoorToggled {
        entrance_id: String,
        depth: u8,
        door_id: u32,
        is_open: bool,
    },
    /// Snapshot of every open door in a dungeon (entrance + interior), sent in
    /// reply to RequestDungeonDoors and pushed on every dungeon floor entry
    /// (door broadcasts are floor/radius-gated, so an arriving player may
    /// have missed toggles since their registration-time snapshot). Each
    /// entry is (depth, door_id); doors not listed are shut.
    DungeonDoorsState {
        entrance_id: String,
        doors: Vec<(u8, u32)>,
    },
    /// Every dungeon entrance this character has discovered (world-map
    /// markers): full snapshot at join and after each new discovery. Only
    /// ids travel — both sides embed the entrance registry, so the client
    /// resolves names and positions locally.
    DungeonDiscoveries {
        entrance_ids: Vec<String>,
    },
    ChatMessage {
        player_id: PlayerId,
        message: String,
    },
    /// A private message, delivered only to the target and echoed to the
    /// sender. Carries names instead of ids: whisper ignores distance, so
    /// either end may be outside the other's AOI where an id resolves to
    /// nobody.
    WhisperMessage {
        from: String,
        to: String,
        message: String,
    },
    /// Private server reply to the player's own command (`/who`, `/escape`,
    /// whisper errors). Not `ChatMessage`: clients must render it as a system
    /// line, not the player's own speech.
    SystemMessage {
        message: String,
    },
    /// A party-channel line, sent to every online member (the sender's echo
    /// included). Carries the name like `WhisperMessage`: party chat ignores
    /// distance, so the sender may be outside a member's AOI.
    PartyChatMessage {
        from: String,
        message: String,
    },
    /// One verse of a recital, paced over a song. `logged` (`/recite`) puts
    /// it in the chat log as well as the bubble — the first pass of a lyric;
    /// `/recite_quiet` repeats are bubble-only so the loop does not flood
    /// the log. Same reach as speech.
    Recital {
        player_id: PlayerId,
        line: String,
        logged: bool,
    },
    /// Direct to the invitee: a party invite to answer with `PartyRespond`
    /// before it expires server-side.
    PartyInviteReceived {
        inviter_id: PlayerId,
        inviter_name: String,
    },
    /// Direct to the inviter: the invite's outcome. Kept distinct from
    /// `SystemMessage` like `TradeError` so the agent-client reacts to it.
    PartyInviteResult {
        target_name: String,
        accepted: bool,
        message: String,
    },
    /// Direct to the target: a trade request to answer with
    /// `PlayerTradeRespond` before it expires server-side.
    PlayerTradeRequested {
        requester_id: PlayerId,
        requester_name: String,
    },
    /// Direct to the requester: the request's outcome.
    PlayerTradeRequestResult {
        target_name: String,
        accepted: bool,
        message: String,
    },
    /// The full session after any change, to both sides. `you` is the
    /// recipient's own side; the offered items double as the client's
    /// authoritative reservation list for greying out bag slots.
    PlayerTradeUpdate {
        state: PlayerTradeState,
    },
    /// The session is over. `completed` separates a finished swap from a
    /// cancel, an expiry, or a failed commit.
    PlayerTradeEnded {
        completed: bool,
        message: String,
    },
    /// A rejected action inside a live session (stale revision, overweight,
    /// untradeable item). The session survives; the window shows the reason.
    PlayerTradeError {
        message: String,
    },
    /// Direct to each other party member when one reads a summoning scroll:
    /// a consent request to answer with `PartySummonRespond` before it
    /// expires server-side.
    PartySummonReceived {
        caster_id: PlayerId,
        caster_name: String,
    },
    /// Direct to each member after any roster change. Empty `members` means
    /// the receiver is no longer in a party.
    PartyState {
        leader_id: PlayerId,
        members: Vec<PartyMember>,
    },
    /// The whole friend roster, offline friends included. Sent at login and
    /// re-sent after any change, to both sides of it.
    FriendList {
        friends: Vec<FriendEntry>,
    },
    /// Answer to `RequestFriendsOnline`: which friends are online right now.
    /// Absence from the list is the offline signal, so a shrinking list needs
    /// no separate message.
    FriendsOnline {
        friends: Vec<OnlineFriend>,
    },
    /// Direct to the target: a friend request to answer with `FriendRespond`
    /// before it expires server-side.
    FriendRequestReceived {
        requester_id: PlayerId,
        requester_name: String,
    },
    /// Party member locations with no AOI cut — the point is members beyond
    /// it. Pushed to the whole party when a member relocates, and sent
    /// directly as the answer to `RequestPartyPositions`. Includes the
    /// recipient (one payload serves every member; clients filter
    /// themselves); empty when the requester is not in a party.
    PartyPositions {
        members: Vec<PartyMemberPosition>,
    },
    /// Party member health with no AOI cut, `PartyPositions`' twin: pushed to
    /// the whole party when a member's health changes. The roster snapshot in
    /// `PartyState` seeds the panel; this keeps it current.
    PartyVitals {
        members: Vec<PartyMemberVitals>,
    },
    GameState {
        /// A list, not a map keyed by id: `PlayerId` is numeric and
        /// `wasm_api`'s `serialize_maps_as_objects` refuses non-string keys,
        /// which would fail the whole frame. Each `Player` carries its own id
        /// anyway, so the key was redundant.
        players: Vec<Player>,
        monsters: HashMap<String, Monster>,
        #[serde(default)]
        ground_items: Vec<inventory::GroundItem>,
        #[serde(default)]
        campfires: Vec<crate::hunger::Campfire>,
        #[serde(default)]
        stalls: Vec<crate::stall::Stall>,
        #[serde(default)]
        tip_hats: Vec<crate::tip_hat::TipHat>,
        #[serde(default)]
        meals: Vec<crate::meal::Meal>,
    },
    GameTimeSync {
        datetime: GameDateTime,
        is_night: bool,
    },
    /// NPC clients only (doc/PRICING.md).
    PricingNotice(crate::pricing::PricingNotice),
    MonsterSpawned {
        monster: Monster,
    },
    /// Server assigns a monster to this client for AI control.
    MonsterAssigned {
        monster: Monster,
    },
    MonsterMoved {
        monster_id: String,
        position: Position,
        rotation: f32,
        state: MonsterState,
        /// Where a remote view walks the model until the next sync — a point on
        /// the mover's own path, not its destination. Aiming a viewer's straight
        /// line at the destination walks the model through the walls the path
        /// goes around. See `MonsterBrain::current_leg_target`.
        target_position: Position,
        owner_id: Option<PlayerId>,
        /// Set on chase legs; viewers aim the walk at the chased player's
        /// live local position instead of the sync-old `target_position`,
        /// stopping at the carried radius.
        chasing: Option<crate::monster_ai::ChaseAim>,
    },
    MonsterRemoved {
        monster_id: String,
    },
    MonsterDead {
        monster_id: String,
        dropped_weapon_item_def_id: Option<String>,
    },
    PlayerAttacked {
        player_id: PlayerId,
        monster_id: String,
        hit: bool,
        roll: u8,
        damage: u32,
    },
    /// A valid attack attempt made outside melee range. No attack roll or
    /// damage is applied, but the managed monster should acquire the player.
    MonsterProvoked {
        player_id: PlayerId,
        monster_id: String,
    },
    /// Direct ack to the attacker for a dropped `PlayerAttack` request, so a
    /// rejection is distinguishable from packet loss.
    PlayerAttackRejected {
        monster_id: String,
        reason: AttackRejectReason,
    },
    MonsterAttackedPlayer {
        monster_id: String,
        player_id: PlayerId,
        hit: bool,
        roll: u8,
        damage: u32,
        current_health: u32,
    },
    PlayerDead {
        player_id: PlayerId,
    },
    PlayerRespawned {
        player: Player,
    },
    PlayerHealthUpdate {
        player_id: PlayerId,
        health: u32,
        max_health: u32,
    },
    XpGained {
        player_id: PlayerId,
        xp_amount: u32,
        xp_lost: u64,
        total_xp: u64,
        new_level: u32,
        leveled_up: bool,
        max_hp: u32,
        current_hp: u32,
        /// The kill this XP came from, so a client can hold the gain until that
        /// monster starts its death animation. `None` for the death penalty.
        monster_id: Option<String>,
    },
    /// Direct message: the receiving player's full trained-skill map, sent
    /// once on EnterGame. Skills stay out of the broadcast `Player` struct —
    /// like gold, they are private to their owner.
    SkillsUpdate {
        skills: skills::Skills,
    },
    /// Direct message: the receiving player gained skill XP (the trained-skill
    /// mirror of `XpGained`). `xp_amount` is what was actually banked after
    /// the level-cap clamp.
    SkillXpGained {
        skill: skills::SkillId,
        xp_amount: u64,
        total_xp: u64,
        new_level: u32,
        leveled_up: bool,
    },
    /// A player's cast landed: render their bobber at `position`. Broadcast
    /// nearby (the caster included) so fishing is visible to passers-by.
    /// `rotation` is the caster facing the water — carried here because the
    /// caster's own face-turn packet ticks out later and would lose the race.
    FishingCasted {
        player_id: PlayerId,
        position: Position,
        rotation: f32,
    },
    /// The bobber dipped — the angler has the shared bite window (plus
    /// latency grace, judged server-side) to send `Hook`.
    FishingBite {
        player_id: PlayerId,
    },
    /// One 250 ms beat of the hooked fight: where the fish is (`bobber` — the
    /// float tracks it), what it's doing, and the line's tension. Broadcast —
    /// the state is public information by design (agent parity), bystanders
    /// render the moving bobber and splash. `stamina_pct` drives the splash
    /// intensity: a fresh fish thrashes, a spent one barely ripples.
    FishingFight {
        player_id: PlayerId,
        bobber: Position,
        fish_state: fishing::FishState,
        tension_pct: u32,
        stamina_pct: u32,
    },
    /// The session is over: despawn the bobber and, for the angler, show the
    /// outcome. A caught fish also arrives via the normal `InventoryUpdated`
    /// (or `GroundItemSpawned` when the bag couldn't take the weight).
    FishingEnded {
        player_id: PlayerId,
        outcome: fishing::FishingOutcome,
    },
    /// Direct: a fishing request was refused (no rod, not water, too far…).
    /// Mirrors `InventoryError`.
    FishingError {
        message: String,
    },
    Kicked {
        player_id: PlayerId,
        reason: String,
    },
    ServerNotice {
        message: Option<String>,
    },
    PlayerTorchToggled {
        player_id: PlayerId,
        enabled: bool,
    },
    /// The `wet` soaking went up or came off this player (doc/DEBUFF.md).
    /// Cosmetic — only the footprint trail reads it.
    PlayerWetToggled {
        player_id: PlayerId,
        wet: bool,
    },
    /// A nearby player's shown title changed (doc/TITLES.md).
    PlayerTitleChanged {
        player_id: PlayerId,
        title: Option<String>,
    },
    /// The recipient earned a title; sent to them alone.
    TitleEarned {
        title: String,
    },
    /// The recipient's own title list and active pick, on entry and after
    /// every change.
    PlayerTitles {
        titles: Vec<String>,
        active: Option<String>,
    },
    /// The client asked to use a cape dye and may open its colour picker:
    /// there is a cape on to dye and the dye is in the bag. The server keeps
    /// no pending state — `DyeCape` re-checks everything.
    CapeDyePrompt {
        instance_id: u64,
    },
    /// Same for a cape transfer kit: a cape is on and the kit is in the bag,
    /// so the client may open its image picker. Nothing is spent until
    /// `ApplyCapeTexture`.
    CapeTexturePrompt {
        instance_id: u64,
    },
    /// A player's equipped main-hand item changed; `None` reverts remote
    /// rendering to the class default weapon.
    PlayerMainHandChanged {
        player_id: PlayerId,
        item_def_id: Option<String>,
    },
    /// A player's equipped back item changed; `None` removes the cape from
    /// remote rendering. `cape_color` is the dye on that instance, if any —
    /// re-dyeing sends this with an unchanged `item_def_id`.
    PlayerBackChanged {
        player_id: PlayerId,
        item_def_id: Option<String>,
        #[serde(default)]
        cape_color: Option<String>,
        #[serde(default)]
        cape_texture: Option<String>,
    },
    PlayerInteractionChanged {
        player_id: PlayerId,
        object_type: Option<String>,
        /// Furniture placement id of the occupied object (None for emotes).
        /// Lets consumers key on the exact chair/bed, not its coordinates.
        #[serde(default)]
        object_id: Option<u32>,
    },
    /// A player started a `/play_music` performance; nearby clients play the
    /// named BGM track. `track` is the title the server resolved from its
    /// registry — receivers play it only if their own BGM list has it.
    /// The performance ends with the emote (`PlayerInteractionChanged` /
    /// [`MUSIC_EMOTE`] giving way to anything else). Also sent to a player
    /// who comes into earshot mid-performance, with `elapsed_secs` saying
    /// how far in the tune already is.
    PlayerMusicStarted {
        player_id: PlayerId,
        track: String,
        #[serde(default)]
        elapsed_secs: f32,
    },
    InteractionRejected {
        reason: String,
    },
    HouseSpawned {
        house: housing::HouseData,
    },
    HouseUpdated {
        house: housing::HouseData,
    },
    TreeTilesInvalidated {
        tiles: Vec<(i32, i32)>,
    },
    HouseRemoved {
        house_id: String,
    },
    HousesInArea {
        houses: Vec<housing::HouseData>,
    },
    DoorToggled {
        house_id: String,
        room_index: u32,
        wall_dir: housing::WallDirection,
        segment_index: u32,
        is_open: bool,
    },
    /// Sent once on join: full inventory state.
    InventoryState {
        inventory: inventory::PlayerInventory,
    },
    /// Sent after any inventory mutation.
    InventoryUpdated {
        inventory: inventory::PlayerInventory,
    },
    /// A new item was created on the ground. Sent when the item becomes real,
    /// so a client spawns it on arrival: a dying monster's loot is held back
    /// server-side until the killing blow lands.
    GroundItemSpawned {
        item: inventory::GroundItem,
    },
    /// An existing ground item became visible to the client.
    GroundItemAppeared {
        item: inventory::GroundItem,
    },
    /// A ground item was picked up, despawned, or left the client's view.
    GroundItemRemoved {
        instance_id: u64,
        /// Who picked it up, when someone did — `None` for a despawn or an
        /// item that merely dropped out of range.
        picked_up_by: Option<PlayerId>,
    },
    /// A pile shrank without emptying: someone took part of it, having been
    /// able to carry only some of the units.
    GroundItemQuantityChanged {
        instance_id: u64,
        quantity: u32,
        /// Who took the units, for the loot line in chat; clients derive the
        /// taken count from the quantity they had cached.
        picked_up_by: Option<PlayerId>,
    },
    /// Response to OpenShop (or pushed by an NPC's OpenTrade): the trader's
    /// goods. Display prices come from item definitions; the server
    /// re-validates them on Buy/Sell.
    ShopState {
        merchant_player_id: PlayerId,
        merchant_name: String,
        /// Merchant catalog (unlimited stock). Empty for non-merchants.
        catalog: Vec<String>,
        /// Percentage of base price paid when the player sells. For
        /// non-merchants this is the wishlist premium rate (can exceed 100).
        sell_rate_percent: u32,
        /// Haggled price modifiers this player currently holds with this
        /// merchant.
        #[serde(default)]
        active_deals: Vec<ActiveDeal>,
        /// Non-merchants only buy these item defs (their wishlist). Empty
        /// for merchants, who buy anything with a base price.
        #[serde(default)]
        wishlist: Vec<String>,
        /// Non-merchant real-inventory stock the player can buy (at base
        /// price). Empty for merchants, who use `catalog`.
        #[serde(default)]
        stock: Vec<StockEntry>,
        /// Units this player recently sold to this merchant, repurchasable
        /// at the recorded payout. Empty for non-merchants, whose bought
        /// units stay visible in `stock`.
        #[serde(default)]
        buyback: Vec<BuybackEntry>,
        /// Consumable buy-price index, 100 = base; residents send 100.
        #[serde(default = "default_price_index_percent")]
        price_index_percent: u32,
    },
    /// Direct message: the receiving player's current gold (smallest unit).
    GoldUpdate {
        gold: i64,
    },
    /// Direct message: the receiving player's effective stats (base attribute
    /// plus equipped-gear bonuses) — the exact numbers combat and haggling use.
    /// Sent on join and after any equipment change so the client never
    /// duplicates the server formula.
    EffectiveStatsUpdated {
        guard: i32,
        cha: i32,
    },
    /// Direct message: the receiving player gained loose currency from a
    /// pickup. `amount` is in the smallest unit (copper).
    GoldGained {
        amount: i64,
    },
    /// A shop request failed. Kept distinct from `SystemMessage`: the
    /// agent-client reacts urgently to a failed trade.
    TradeError {
        message: String,
    },
    /// Direct to a player: a haggled price modifier changed on one item.
    /// `modifier_pct == 0` means the deal was consumed or cleared.
    DealUpdated {
        merchant_player_id: PlayerId,
        item_def_id: String,
        kind: DealKind,
        modifier_pct: i32,
        expires_in_secs: u32,
    },
    /// Direct to a player: its buyback list with one merchant changed (a
    /// sell added an entry, or a buyback consumed one).
    BuybackUpdated {
        merchant_player_id: PlayerId,
        buyback: Vec<BuybackEntry>,
    },
    /// Direct to a trading NPC: whether at least one player currently has its
    /// trade window open. While `busy` is true the NPC's LLM keeps its place
    /// (movement is suppressed) so it doesn't wander off mid-trade; it can
    /// still talk and haggle.
    TradeBusy {
        busy: bool,
    },
    /// Direct to a trading NPC: a player completed a buy/sell against it,
    /// so its LLM can react in conversation. `kind` is from the player's
    /// perspective (Buy = the player bought from the NPC).
    TradeNotice {
        player_name: String,
        item_def_id: String,
        kind: DealKind,
        /// Gold that changed hands (smallest unit).
        price: i64,
        /// The NPC's wallet after the trade.
        npc_gold: i64,
    },
    /// Direct to a merchant NPC: the named player waved off its pushed
    /// trade window ("Not now", or the offer toast expired). The agent
    /// suppresses trade pushes at them for a cooldown.
    TradeDeclined {
        player_id: PlayerId,
        player_name: String,
    },
    /// Direct to the offering NPC: the server's verdict on its `OfferDeal`.
    DealResult {
        target_player_id: PlayerId,
        target_player_name: String,
        item_def_id: String,
        kind: DealKind,
        accepted: bool,
        /// The modifier actually in effect (after band clamping); 0 when
        /// rejected.
        applied_modifier_pct: i32,
        message: String,
    },
    /// Direct to one player: the movement sim refused a step, so the client has
    /// walked somewhere the server cannot follow. Snap back to the server's copy
    /// and drop the path that led there — keeping it would just walk into the
    /// same refusal again. Carries no `player_id`: it only ever goes to the
    /// player it corrects. Not a relocation, so no camera reset or dungeon
    /// resync the way `PlayerTeleported` does.
    PositionCorrected {
        position: Position,
        rotation: f32,
        #[serde(default)]
        floor_level: i8,
    },
    /// Direct to the owner only (exact satiation is private, doc/HUNGER.md).
    /// Sent on band transitions, eating and debuff changes — not on every
    /// decay tick. Carries the effective multipliers (hunger × debuffs) so
    /// the client never re-derives them.
    HungerUpdate {
        satiation: u32,
        state: crate::hunger::HungerState,
        move_mult: f32,
        attack_mult: f32,
        carry_mult: f32,
    },
    /// Direct to the owner only: the full list of active debuffs, sent when
    /// one is applied, refreshed or expires (doc/DEBUFF.md).
    DebuffUpdate {
        debuffs: Vec<crate::debuff::ActiveDebuffState>,
    },
    /// A campfire was just lit nearby (play the ignition, not just appear).
    CampfireSpawned {
        campfire: crate::hunger::Campfire,
    },
    /// An already-burning campfire entered the receiver's AOI.
    CampfireAppeared {
        campfire: crate::hunger::Campfire,
    },
    /// Burned out or left the receiver's AOI.
    CampfireRemoved {
        campfire_id: u64,
    },
    /// A merchant just laid out a stall nearby.
    StallPlaced {
        stall: crate::stall::Stall,
    },
    /// An already-laid stall entered the receiver's AOI.
    StallAppeared {
        stall: crate::stall::Stall,
    },
    /// Packed up or left the receiver's AOI.
    StallRemoved {
        stall_id: u64,
    },
    /// A performer just set a tip hat down nearby.
    TipHatPlaced {
        tip_hat: crate::tip_hat::TipHat,
    },
    /// An already-placed tip hat entered the receiver's AOI.
    TipHatAppeared {
        tip_hat: crate::tip_hat::TipHat,
    },
    /// Picked up, left behind by its owner, or left the receiver's AOI.
    TipHatRemoved {
        tip_hat_id: u64,
    },
    /// A maid just set a dish down on a table nearby.
    MealPlaced {
        meal: crate::meal::Meal,
    },
    /// An already-served dish entered the receiver's AOI.
    MealAppeared {
        meal: crate::meal::Meal,
    },
    /// The guest finished it; the empty plate stays until cleared.
    MealEaten {
        meal_id: u64,
    },
    /// Cleared, expired, or left the receiver's AOI.
    MealRemoved {
        meal_id: u64,
    },
    /// Direct to the griller: the 3s grill cast began.
    GrillStarted,
    /// Direct to the griller. `grilled_item_def_id` is None when the cast was
    /// cancelled (movement, combat, the fire burning out). The grilled item
    /// itself arrives through the normal `InventoryUpdated`.
    GrillEnded {
        grilled_item_def_id: Option<String>,
    },
    /// Direct to each dungeon occupant before the sunset reset puts them out.
    DungeonReset,
}

pub use crate::entity::PlayerId;

// Serialization helpers (used by both server and wasm). `#[inline]` so the
// rmp_serde call lands directly at the call site even though the protocol
// types live in their own crate from the consumers' perspective.
#[inline]
pub fn serialize_client_msg(msg: &ClientMessage) -> Result<Vec<u8>, rmp_serde::encode::Error> {
    rmp_serde::to_vec(msg)
}

#[inline]
pub fn deserialize_client_msg(bytes: &[u8]) -> Result<ClientMessage, rmp_serde::decode::Error> {
    rmp_serde::from_slice(bytes)
}

#[inline]
pub fn serialize_server_msg(msg: &ServerMessage) -> Result<Vec<u8>, rmp_serde::encode::Error> {
    rmp_serde::to_vec(msg)
}

#[inline]
pub fn deserialize_server_msg(bytes: &[u8]) -> Result<ServerMessage, rmp_serde::decode::Error> {
    rmp_serde::from_slice(bytes)
}

fn default_price_index_percent() -> u32 {
    100
}
