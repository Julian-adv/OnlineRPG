//! Agent action model and conversion to game-server commands.
//!
//! Splits responsibility into three layers: the JSON-shaped `AgentResponse`
//! the LLM is expected to emit, parsing helpers that tolerate the various
//! markdown wrappers an LLM might add, and `action_to_command` which lifts
//! a parsed `AgentAction` into a `ClientMessage` for the server.

use onlinerpg_shared::ClientMessage;
use serde::Deserialize;
use tracing::warn;

/// Parsed agent response.
#[derive(Debug, Deserialize)]
pub(super) struct AgentResponse {
    #[allow(dead_code)]
    pub thought: Option<String>,
    pub actions: Vec<AgentAction>,
    /// Optional memory update: appended to the NPC's memory file for future sessions.
    #[allow(dead_code)]
    pub memory_update: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(super) enum AgentAction {
    #[serde(rename = "say", alias = "chat")]
    Say { message: String },
    #[serde(rename = "attack")]
    Attack {
        #[serde(
            alias = "targetId",
            alias = "target_id",
            alias = "target",
            alias = "id"
        )]
        monster_id: String,
    },
    #[serde(rename = "move")]
    Move {
        // Character name: approach them and stop a polite distance short
        // (preferred when walking up to a player or NPC)
        #[serde(alias = "player", alias = "name", alias = "character")]
        target: Option<String>,
        // Absolute coordinates (preferred for places)
        x: Option<f32>,
        #[allow(dead_code)]
        y: Option<f32>,
        z: Option<f32>,
        // Direction + distance fallback (LLMs sometimes use this)
        direction: Option<String>,
        distance: Option<f32>,
        // Dungeon floor to end up on: 1..N counted downward, 0 = surface.
        // Without coordinates the walk targets that floor's stair landing,
        // which is how the agent enters and descends a dungeon.
        #[serde(alias = "dungeon_depth", alias = "floor", alias = "floor_level")]
        depth: Option<i32>,
    },
    /// Keep following a character: re-approach whenever they move, until the
    /// LLM issues anything else that walks us somewhere, or the target is lost.
    #[serde(rename = "follow", alias = "follow_player")]
    Follow {
        #[serde(alias = "player", alias = "name", alias = "character")]
        target: String,
    },
    #[serde(rename = "respawn")]
    Respawn,
    /// Cast the rod at (x, z), or 4 m south when omitted; server validates.
    /// Reflexes (state.rs) fight the fish — this is only the decision to fish.
    #[serde(rename = "fish")]
    Fish { x: Option<f32>, z: Option<f32> },
    /// Reel in and stop fishing.
    #[serde(rename = "stop_fishing")]
    StopFishing,
    /// Haggling (merchants only): offer a price modifier on one item to a
    /// nearby player. The server clamps/validates; see `doc/ECONOMY.md`.
    #[serde(rename = "offer_deal")]
    OfferDeal {
        #[serde(alias = "target", alias = "player_name", alias = "target_player")]
        player: String,
        #[serde(alias = "item_def_id", alias = "item_id")]
        item: String,
        /// "buy" (player buys from you, default) or "sell" (player sells to you).
        #[serde(default)]
        kind: Option<String>,
        #[serde(alias = "modifier", alias = "modifier_percent", alias = "discount_pct")]
        modifier_pct: i32,
        #[serde(default)]
        reason: Option<String>,
    },
    /// Open your trade window on a nearby player's screen (traders only) —
    /// the conversational entry point for trading.
    #[serde(rename = "open_trade", alias = "trade")]
    OpenTrade {
        #[serde(alias = "target", alias = "player_name", alias = "target_player")]
        player: String,
    },
    /// Invite a player to your party by name. Works at any distance, like a
    /// whisper.
    #[serde(rename = "party_invite", alias = "invite_party", alias = "invite")]
    PartyInvite {
        #[serde(alias = "target", alias = "player_name", alias = "target_player")]
        player: String,
    },
    /// Accept a pending party invite — the oldest, or the named inviter's.
    #[serde(rename = "party_accept", alias = "accept_party", alias = "join_party")]
    PartyAccept {
        #[serde(default, alias = "target", alias = "player_name", alias = "inviter")]
        player: Option<String>,
    },
    /// Decline a pending party invite — the oldest, or the named inviter's.
    #[serde(rename = "party_decline", alias = "decline_party")]
    PartyDecline {
        #[serde(default, alias = "target", alias = "player_name", alias = "inviter")]
        player: Option<String>,
    },
    /// Accept a pending party summon — the oldest, or the named caster's.
    #[serde(rename = "summon_accept", alias = "accept_summon")]
    SummonAccept {
        #[serde(default, alias = "target", alias = "player_name", alias = "caster")]
        player: Option<String>,
    },
    /// Decline a pending party summon — the oldest, or the named caster's.
    #[serde(rename = "summon_decline", alias = "decline_summon")]
    SummonDecline {
        #[serde(default, alias = "target", alias = "player_name", alias = "caster")]
        player: Option<String>,
    },
    /// Leave your current party.
    #[serde(rename = "party_leave", alias = "leave_party")]
    PartyLeave,
    /// Say something to your party, wherever its members are.
    #[serde(rename = "party_say", alias = "party_chat")]
    PartySay {
        #[serde(alias = "text")]
        message: String,
    },
    /// Use an item from the bag: gear is equipped (or taken off if already
    /// worn), consumables are drunk, eaten or read. Mirrors the web quickslot.
    #[serde(rename = "use", alias = "use_item", alias = "equip", alias = "eat")]
    Use {
        #[serde(
            alias = "item_def_id",
            alias = "item_id",
            alias = "name",
            alias = "target"
        )]
        item: String,
    },
    /// Walk to an item on the ground and pick it up into the bag. Mirrors
    /// the web client's click-to-pick-up. `item` is the instance id shown
    /// in the world state, or an item name (nearest match).
    #[serde(rename = "pickup", alias = "pick_up", alias = "loot", alias = "take")]
    Pickup {
        #[serde(
            alias = "item_def_id",
            alias = "item_id",
            alias = "instance_id",
            alias = "id",
            alias = "name",
            alias = "target"
        )]
        item: PickupRef,
    },
    /// Sell one or more units of a bag item to a nearby merchant, walking up
    /// to them first. The server owns pricing, proximity and wallet checks.
    #[serde(rename = "sell", alias = "sell_item")]
    Sell {
        #[serde(alias = "item_def_id", alias = "item_id", alias = "name")]
        item: String,
        #[serde(
            default,
            alias = "npc",
            alias = "to",
            alias = "merchant_name",
            alias = "target"
        )]
        merchant: Option<String>,
        /// How many units to sell: a positive count, or "all". Defaults to 1
        /// when omitted.
        #[serde(default, alias = "amount", alias = "count")]
        qty: Option<Qty>,
    },
    /// Buy one catalog item from a nearby merchant, walking up to them
    /// first. The server owns catalog, pricing and gold checks.
    #[serde(rename = "buy", alias = "buy_item", alias = "purchase")]
    Buy {
        #[serde(alias = "item_def_id", alias = "item_id", alias = "name")]
        item: String,
        #[serde(
            default,
            alias = "npc",
            alias = "from",
            alias = "merchant_name",
            alias = "target"
        )]
        merchant: Option<String>,
    },
    /// Drop one or more units of a bag item on the ground where you stand.
    /// Stricter than the web client: worn gear must be taken off first.
    #[serde(rename = "drop", alias = "drop_item", alias = "discard")]
    Drop {
        #[serde(alias = "item_def_id", alias = "item_id", alias = "name")]
        item: String,
        /// How many units to drop: a positive count, or "all". Defaults to 1
        /// when omitted.
        #[serde(default, alias = "amount", alias = "count")]
        qty: Option<Qty>,
    },
    /// Repurchase an item sold to this merchant this session, at the exact
    /// payout price. The server owns the entry list and gold checks.
    #[serde(rename = "buyback", alias = "buy_back", alias = "repurchase")]
    Buyback {
        #[serde(alias = "item_def_id", alias = "item_id", alias = "name")]
        item: String,
        #[serde(
            default,
            alias = "npc",
            alias = "from",
            alias = "merchant_name",
            alias = "target"
        )]
        merchant: Option<String>,
    },
    /// Smash a breakable dungeon prop (barrel/crate) on the current floor,
    /// walking up to it first. The server validates floor and proximity.
    #[serde(rename = "break_prop", alias = "smash", alias = "break")]
    BreakProp {
        #[serde(alias = "id", alias = "prop", alias = "target")]
        prop_id: u32,
    },
    /// Open a chest standing in the agent's own room: the nearest one, or the
    /// great chest when `chest` asks for it. The server validates floor,
    /// proximity, prop kind, boss state and the per-player cooldown, and
    /// answers with loot or a rejection explaining why.
    #[serde(rename = "open_chest", alias = "open_dungeon_chest")]
    OpenChest {
        #[serde(default, alias = "target", alias = "which", alias = "name")]
        chest: Option<String>,
    },
    /// Reroll starting stats. Only meaningful during character creation,
    /// where it is the agent's version of the web client's reroll button.
    #[serde(rename = "reroll", alias = "reroll_stats", alias = "roll_again")]
    Reroll,
    #[serde(rename = "wait", alias = "idle", alias = "observe", alias = "none")]
    Wait,
}

/// One block of the action reference: the prompt text agents read, tied to
/// the serde tag(s) it documents. `action_reference()` renders the table into
/// the `{{ACTIONS}}` slot of the system prompt, so this table IS the format
/// documentation — there is no hand-maintained copy anywhere else.
///
/// Tests lock the table to the enum: every action the parser accepts must be
/// documented here (and nothing extra), and every example line must parse.
/// Adding an `AgentAction` variant without a spec entry fails `cargo test`.
pub(super) struct ActionSpec {
    /// serde tag(s) this block documents — the enum `rename` values.
    /// Read by the lock tests below.
    #[allow(dead_code)]
    pub(super) names: &'static [&'static str],
    /// The enum's `alias` values for these tags. Never rendered — the docs
    /// teach only canonical names — but registered so the enum/table
    /// equality test covers every spelling the parser accepts.
    #[allow(dead_code)]
    pub(super) aliases: &'static [&'static str],
    /// Verbatim prompt text: usage prose with example JSON lines.
    pub(super) doc: &'static str,
}

pub(super) const ACTION_SPECS: &[ActionSpec] = &[
    ActionSpec {
        names: &["say"],
        aliases: &["chat"],
        doc: r#"- Say in chat:
  {"type": "say", "message": "hello"}
  Nearby characters hear it. To whisper privately to one player at any
  distance, prefix "/w " and their name (a [Whisper] event means someone
  whispered to you; answer the same way):
  {"type": "say", "message": "/w PlayerName hello"}
  Players speak many languages; answer each message in the language it
  was written in. Remember each player's language (a memory_update
  note), and use it when speaking to them first."#,
    },
    ActionSpec {
        names: &["attack"],
        aliases: &[],
        doc: r#"- Attack a monster (use the monster's ID from the world state, field name MUST be "monster_id"):
  {"type": "attack", "monster_id": "m2_1"}"#,
    },
    ActionSpec {
        names: &["follow"],
        aliases: &["follow_player"],
        doc: r#"- Follow a character and keep up as they move (use this when someone says
  "follow me" — a plain move stops once you arrive, follow keeps tracking):
  {"type": "follow", "target": "PlayerName"}
  Any other action that takes your body over — a move, attack, pickup, chest,
  trade or fishing — stops the follow. Losing them ends it with a
  [FollowEnded] event."#,
    },
    ActionSpec {
        names: &["move"],
        aliases: &[],
        doc: r#"- Move. To walk up to a character (player or NPC), give their name — you
  approach them and automatically stop at a comfortable talking distance:
  {"type": "move", "target": "PlayerName"}
  To go to a place, use exact coordinates from world state:
  {"type": "move", "x": 10.0, "y": 0.0, "z": -5.0}
  Or move by direction and distance:
  {"type": "move", "direction": "north", "distance": 10.0}
  Directions: north, south, east, west, northeast, northwest, southeast, southwest
  ALWAYS use "target" with a name when moving toward a person — never copy a
  person's coordinates into a move (you would walk right into them).
  To go into a dungeon, name the floor you want with "depth" — 1 is the first
  floor below ground, and you walk to the entrance and down the stairs on your
  own, opening any doors in the way:
  {"type": "move", "depth": 1}
  Deeper floors hold stronger monsters, so descend one floor at a time and only
  while you can still win your fights. To come back up to the surface:
  {"type": "move", "depth": 0}"#,
    },
    ActionSpec {
        names: &["respawn"],
        aliases: &[],
        doc: r#"- Respawn when dead:
  {"type": "respawn"}"#,
    },
    ActionSpec {
        names: &["wait"],
        aliases: &["idle", "observe", "none"],
        doc: r#"- Do nothing (idle/observe/skip turn):
  {"type": "wait"}"#,
    },
    ActionSpec {
        names: &["fish"],
        aliases: &[],
        doc: r#"- Fish (needs a fishing rod worn in your main hand — use it from your bag
  first). Cast at water coordinates from the world state, or omit x/z to
  cast at the water just south of you. Hooking and fighting the fish is
  automatic; you will get a [Fishing] event with the outcome. Moving or
  attacking cancels fishing:
  {"type": "fish", "x": 10.0, "z": -5.0}
  {"type": "fish"}"#,
    },
    ActionSpec {
        names: &["stop_fishing"],
        aliases: &[],
        doc: r#"- Stop fishing (reel in without waiting for a catch):
  {"type": "stop_fishing"}"#,
    },
    ActionSpec {
        names: &["use"],
        aliases: &["use_item", "equip", "eat"],
        doc: r#"- Use an item you are carrying — wear a piece of gear, drink a potion, read
  a scroll, eat food. Name it as it appears in your bag. Using gear you
  already wear takes it off; wearing a torch is how you light your way at
  night:
  {"type": "use", "item": "worn_torch"}"#,
    },
    ActionSpec {
        names: &["pickup"],
        aliases: &["pick_up", "loot", "take"],
        doc: r#"- Pick up an item lying on the ground into your bag. You walk over to it
  first, like a web player clicking it. Give the id shown in the world
  state ("Item on ground: ... [id 6043]"), or its name:
  {"type": "pickup", "item": 6043}
  The world state marks who put an item down ("dropped by Mira"); a
  [GroundItem] event tells you when someone collects such an item, and an
  item that is no longer listed is gone — don't reach for bare ground."#,
    },
    ActionSpec {
        names: &["open_chest"],
        aliases: &["open_dungeon_chest"],
        doc: r#"- Open a chest in a dungeon. You have to be in the room it stands in — the
  world state names the chests there and what each looks like. You walk over
  first, then the server decides; a rejection tells you why it stayed shut.
  Plain form opens the nearest one:
  {"type": "open_chest"}
  To cross the room to the great chest instead of the small one at your feet:
  {"type": "open_chest", "chest": "great"}"#,
    },
    ActionSpec {
        names: &["sell"],
        aliases: &["sell_item"],
        doc: r#"- Sell one or more units of a bag item to a nearby merchant. You walk to
  them first; the merchant pays their rate and your gold updates. Naming
  the merchant is optional — without one you sell to the nearest merchant.
  Defaults to 1 unit; add "qty" for more, or "qty": "all" to sell every
  unit you have. Selling more than you actually own fails outright —
  nothing is sold. Equipped gear must be taken off before selling:
  {"type": "sell", "item": "goblin_sword"}
  {"type": "sell", "item": "healing_potion", "merchant": "Rica", "qty": 5}
  {"type": "sell", "item": "healing_potion", "merchant": "Rica", "qty": "all"}"#,
    },
    ActionSpec {
        names: &["buy"],
        aliases: &["buy_item", "purchase"],
        doc: r#"- Buy one item from a merchant's catalog at base price. You walk to them
  first; the item lands in your bag if your gold covers it. What each
  nearby merchant sells is listed under their name in the world state.
  Naming the merchant is optional — without one you buy from the nearest.
  One unit per action — repeat for more:
  {"type": "buy", "item": "healing_potion", "merchant": "Rica"}"#,
    },
    ActionSpec {
        names: &["drop"],
        aliases: &["drop_item", "discard"],
        doc: r#"- Drop one or more units of a bag item on the ground where you stand (e.g.
  to shed weight for better loot), and anyone can pick it up afterwards.
  Defaults to 1 unit; add "qty" for more, or "qty": "all" to drop every
  unit you have. Dropping more than you actually own fails outright —
  nothing is dropped:
  {"type": "drop", "item": "goblin_sword"}
  {"type": "drop", "item": "old_boot", "qty": 2}
  {"type": "drop", "item": "old_boot", "qty": "all"}"#,
    },
    ActionSpec {
        names: &["buyback"],
        aliases: &["buy_back", "repurchase"],
        doc: r#"- Buy back one item you sold to that merchant this session, for exactly
  what they paid you (undo a mis-sell). The [Buyback] event after each
  sale lists what they still hold:
  {"type": "buyback", "item": "iron_sword", "merchant": "Rica"}"#,
    },
    ActionSpec {
        names: &["offer_deal"],
        aliases: &[],
        doc: r#"- (merchants only) Offer a nearby player a private price on one item — a
  haggle. "kind" is "buy" (they buy from you, the default) or "sell" (they
  sell to you); "modifier_pct" moves the price by that many percent, so a
  negative number is a discount. The server clamps and validates the offer:
  {"type": "offer_deal", "player": "darkcocoa", "item": "healing_potion", "kind": "buy", "modifier_pct": -10}"#,
    },
    ActionSpec {
        names: &["open_trade"],
        aliases: &["trade"],
        doc: r#"- (merchants only) Open your trade window on a nearby player's screen —
  how you move from talk to an actual trade:
  {"type": "open_trade", "player": "darkcocoa"}"#,
    },
    ActionSpec {
        names: &["break_prop"],
        aliases: &["smash", "break"],
        doc: r#"- Smash a breakable dungeon prop (barrel or crate). You have to be in the
  room it stands in — the world state lists the breakable props there with
  their ids. You walk to it first, and smashing opens its cell for movement:
  {"type": "break_prop", "prop_id": 3}"#,
    },
    ActionSpec {
        names: &["party_invite"],
        aliases: &["invite_party", "invite"],
        doc: r#"- Invite a player to your party by name. Works at any distance, like a
  whisper; they get 30 seconds to answer. Your roster shows in the world
  state ("Your party: ..."):
  {"type": "party_invite", "player": "darkcocoa"}"#,
    },
    ActionSpec {
        names: &["party_accept", "party_decline"],
        aliases: &["accept_party", "join_party", "decline_party"],
        doc: r#"- Answer a party invite (the [PartyInvite] event). Accepting puts you in
  their party; a party hunts and travels together. With several invites
  pending, name the inviter — bare answers the oldest:
  {"type": "party_accept"}
  {"type": "party_decline", "player": "darkcocoa"}"#,
    },
    ActionSpec {
        names: &["summon_accept", "summon_decline"],
        aliases: &["accept_summon", "decline_summon"],
        doc: r#"- Answer a party summon (the [PartySummon] event): a party member read a
  summoning scroll asking everyone to teleport to their side. Accepting
  moves you there instantly; you cannot accept mid-combat. With several
  pending, name the caster — bare answers the oldest:
  {"type": "summon_accept"}
  {"type": "summon_decline", "player": "darkcocoa"}"#,
    },
    ActionSpec {
        names: &["party_say"],
        aliases: &["party_chat"],
        doc: r#"- Say something to your whole party (arrives as [Party] lines). Reaches
  every member at any distance, unlike local say:
  {"type": "party_say", "message": "On my way."}"#,
    },
    ActionSpec {
        names: &["party_leave"],
        aliases: &["leave_party"],
        doc: r#"- Leave your current party:
  {"type": "party_leave"}"#,
    },
    ActionSpec {
        names: &["reroll"],
        aliases: &["reroll_stats", "roll_again"],
        doc: r#"- Reroll your starting stats (character creation only — does nothing once
  you are in the world):
  {"type": "reroll"}"#,
    },
];

/// The "Available action types" section of the system prompt, rendered from
/// `ACTION_SPECS`. Fills the `{{ACTIONS}}` slot in `load_system_prompt`.
pub(super) fn action_reference() -> String {
    let mut out = String::from("Available action types (use EXACTLY these field names):\n");
    for spec in ACTION_SPECS {
        out.push('\n');
        out.push_str(spec.doc.trim_end());
        out.push('\n');
    }
    out
}

impl AgentAction {
    /// Whether this action takes the body over: it walks the agent somewhere,
    /// or plants it (fishing, respawning). A running follow yields to it, and
    /// an NPC holding position skips it. Exhaustive on purpose — a new variant
    /// must not be able to slip past this.
    pub(super) fn takes_over_movement(&self) -> bool {
        match self {
            Self::Move { .. }
            | Self::Attack { .. }
            | Self::Follow { .. }
            | Self::Pickup { .. }
            | Self::OpenChest { .. }
            | Self::BreakProp { .. }
            | Self::Sell { .. }
            | Self::Buy { .. }
            | Self::Buyback { .. }
            | Self::Fish { .. }
            | Self::Respawn => true,
            Self::Say { .. }
            | Self::StopFishing
            | Self::OfferDeal { .. }
            | Self::OpenTrade { .. }
            | Self::PartyInvite { .. }
            | Self::PartyAccept { .. }
            | Self::PartyDecline { .. }
            | Self::SummonAccept { .. }
            | Self::SummonDecline { .. }
            | Self::PartyLeave
            | Self::PartySay { .. }
            | Self::Use { .. }
            | Self::Drop { .. }
            | Self::Reroll
            | Self::Wait => false,
        }
    }

    /// Of those, the ones an NPC pinned in place must not run at all. Fishing
    /// and respawning keep it where it is, so they stay allowed.
    pub(super) fn blocked_while_holding_position(&self) -> bool {
        self.takes_over_movement() && !matches!(self, Self::Fish { .. } | Self::Respawn)
    }
}

/// Whether an `open_chest` selector asks for the great chest rather than the
/// nearest one. The word the prompts teach, plus what an LLM reaches for.
pub(super) fn asks_for_great_chest(chest: Option<&str>) -> bool {
    chest.is_some_and(|c| {
        let c = c.to_lowercase();
        ["great", "treasure", "big", "large"]
            .iter()
            .any(|k| c.contains(k))
    })
}

/// How a sell/drop action names an amount: an explicit count, or the
/// literal "all" of whatever is currently owned — resolved against the live
/// bag at dispatch time (`Self::resolve`), not fixed when the LLM composed
/// the action.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum Qty {
    Count(u32),
    Named(String),
}

impl Qty {
    /// The concrete count this resolves to given `available` units actually
    /// owned, or `None` if it isn't a positive count or the word "all".
    pub(super) fn resolve(&self, available: u32) -> Option<u32> {
        match self {
            Self::Count(n) if *n > 0 => Some(*n),
            Self::Named(s) if s.trim().eq_ignore_ascii_case("all") => Some(available),
            _ => None,
        }
    }
}

/// How a pickup names its target: the instance id from the world state
/// ("[id 6043]"), or an item name resolved to the nearest match. LLMs send
/// either, and the id may arrive as a number or a numeric string.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum PickupRef {
    Id(u64),
    Name(String),
}

impl PickupRef {
    /// The instance id the agent meant, however it was spelled.
    pub(super) fn as_id(&self) -> Option<u64> {
        match self {
            Self::Id(id) => Some(*id),
            Self::Name(name) => name.trim().parse().ok(),
        }
    }
}

impl std::fmt::Display for PickupRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Id(id) => write!(f, "id {id}"),
            Self::Name(name) => f.write_str(name),
        }
    }
}

/// Whether the agent asked to roll its starting stats again. Read from the
/// ordinary action envelope; a reply we cannot parse counts as acceptance, so
/// a confused agent cannot spin the roll loop.
pub(crate) fn wants_reroll(reply: &str) -> bool {
    if let Ok(parsed) = parse_agent_response(reply) {
        return parsed
            .actions
            .iter()
            .any(|a| matches!(a, AgentAction::Reroll));
    }
    let reply = reply.to_lowercase();
    match (reply.rfind("reroll"), reply.rfind("accept")) {
        (Some(reroll), Some(accept)) => reroll > accept,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

/// Parse a raw text response from an LLM into structured actions.
pub(super) fn parse_agent_response(text: &str) -> anyhow::Result<AgentResponse> {
    let json_str = extract_json(text);
    let mut value: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| anyhow::anyhow!("Failed to parse agent response: {e}\nRaw: {text}"))?;
    normalize_move_targets(&mut value);
    serde_json::from_value(value)
        .map_err(|e| anyhow::anyhow!("Failed to parse agent response: {e}\nRaw: {text}"))
}

/// What a tolerant parse yields: the actions that parsed, plus a human-readable
/// complaint for each one that didn't. The complaints are fed back to the LLM
/// so a mistyped action reports its own error instead of vanishing.
pub(super) struct ParsedTurn {
    #[allow(dead_code)]
    pub thought: Option<String>,
    pub actions: Vec<AgentAction>,
    pub memory_update: Option<String>,
    pub errors: Vec<String>,
}

/// Parse each action independently so one malformed action does not discard
/// the whole turn (the say that rode along with it, the memory_update). Every
/// rejected action becomes an error string naming what was wrong.
pub(super) fn parse_turn_tolerant(text: &str) -> anyhow::Result<ParsedTurn> {
    let json_str = extract_json(text);
    let mut value: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| anyhow::anyhow!("Failed to parse agent response: {e}\nRaw: {text}"))?;
    normalize_move_targets(&mut value);

    let thought = value
        .get("thought")
        .and_then(|t| t.as_str())
        .map(str::to_string);
    let memory_update = value
        .get("memory_update")
        .and_then(|t| t.as_str())
        .map(str::to_string);

    let mut actions = Vec::new();
    let mut errors = Vec::new();
    if let Some(arr) = value.get("actions").and_then(|a| a.as_array()) {
        for elem in arr {
            match serde_json::from_value::<AgentAction>(elem.clone()) {
                Ok(action) => actions.push(action),
                Err(e) => {
                    let kind = elem
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("(no type)");
                    errors.push(format!("action \"{kind}\": {e}"));
                }
            }
        }
    }
    Ok(ParsedTurn {
        thought,
        actions,
        memory_update,
        errors,
    })
}

/// LLMs keep writing `{"type":"move","target":[x,z]}` or `"target":{"x":..}`
/// despite the schema saying `target` is a name. Rewrite those into the x/z
/// fields instead of discarding the whole response.
fn normalize_move_targets(value: &mut serde_json::Value) {
    let Some(actions) = value.get_mut("actions").and_then(|a| a.as_array_mut()) else {
        return;
    };
    for action in actions {
        if action.get("type").and_then(|t| t.as_str()) != Some("move") {
            continue;
        }
        let coords: Option<(f64, Option<f64>, f64)> = match action.get("target") {
            Some(serde_json::Value::Array(arr)) => {
                let nums: Vec<f64> = arr.iter().filter_map(|n| n.as_f64()).collect();
                match nums.len() {
                    2 => Some((nums[0], None, nums[1])),
                    3 => Some((nums[0], Some(nums[1]), nums[2])),
                    _ => None,
                }
            }
            Some(serde_json::Value::Object(obj)) => {
                match (
                    obj.get("x").and_then(|n| n.as_f64()),
                    obj.get("z").and_then(|n| n.as_f64()),
                ) {
                    (Some(x), Some(z)) => Some((x, obj.get("y").and_then(|n| n.as_f64()), z)),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some((x, y, z)) = coords {
            let obj = action.as_object_mut().unwrap();
            obj.remove("target");
            obj.entry("x").or_insert_with(|| x.into());
            if let Some(y) = y {
                obj.entry("y").or_insert_with(|| y.into());
            }
            obj.entry("z").or_insert_with(|| z.into());
        }
    }
}

/// Extract JSON object from text that might contain markdown code blocks.
fn extract_json(text: &str) -> &str {
    let trimmed = text.trim();

    // Try to find ```json ... ``` block
    if let Some(start) = trimmed.find("```json") {
        let after_marker = &trimmed[start + 7..];
        if let Some(end) = after_marker.find("```") {
            return after_marker[..end].trim();
        }
    }

    // Try to find ``` ... ``` block
    if let Some(start) = trimmed.find("```") {
        let after_marker = &trimmed[start + 3..];
        if let Some(end) = after_marker.find("```") {
            return after_marker[..end].trim();
        }
    }

    // Try to find raw JSON object
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            return &trimmed[start..=end];
        }
    }

    trimmed
}

/// Resolve move goal coordinates from an AgentAction::Move. Supports both
/// absolute `(x, z)` and the `direction + distance` fallback some LLMs
/// prefer; the latter requires a known player position.
pub(super) fn resolve_move_goal(
    x: &Option<f32>,
    z: &Option<f32>,
    direction: &Option<String>,
    distance: &Option<f32>,
    player_pos: Option<&onlinerpg_shared::Position>,
) -> Option<(f32, f32)> {
    if let (Some(x), Some(z)) = (x, z) {
        Some((*x, *z))
    } else if let (Some(dir), Some(dist), Some(pp)) = (direction.as_deref(), distance, player_pos) {
        let (dx, dz) = direction_to_offset(dir);
        Some((pp.x + dx * dist, pp.z + dz * dist))
    } else {
        None
    }
}

/// Convert an AgentAction into a ClientMessage for the game server.
/// `player_pos` is needed to resolve relative move directions and to compute rotation.
pub(super) fn action_to_command(
    action: &AgentAction,
    player_pos: Option<&onlinerpg_shared::Position>,
) -> Option<ClientMessage> {
    match action {
        // Handled in `execute::handle_response` (needs name resolution and a
        // background chase task).
        AgentAction::Follow { .. } => None,
        AgentAction::Say { message } => Some(ClientMessage::ChatMessage {
            message: message.clone(),
        }),
        AgentAction::Attack { monster_id } => Some(ClientMessage::PlayerAttack {
            monster_id: monster_id.clone(),
        }),
        AgentAction::Move {
            target,
            x,
            y: _,
            z,
            direction,
            distance,
            depth,
        } => {
            // Name-targeted and dungeon-floor moves need SharedState (name
            // resolution, layouts); handled in `execute::handle_response`.
            if target.is_some() || depth.is_some() {
                return None;
            }
            let (gx, gz) = resolve_move_goal(x, z, direction, distance, player_pos)?;
            let rotation = if let Some(pp) = player_pos {
                (gx - pp.x).atan2(gz - pp.z)
            } else {
                0.0
            };
            Some(ClientMessage::player_move(
                onlinerpg_shared::Position {
                    x: gx,
                    y: player_pos.map(|p| p.y).unwrap_or(0.0),
                    z: gz,
                },
                rotation,
                0,
            ))
        }
        AgentAction::Respawn => Some(ClientMessage::RequestRespawn),
        AgentAction::Fish { x, z } => {
            // Explicit coordinates, or a fixed short cast south of the agent.
            // The server is the judge of whether that spot is water.
            let (cx, cz) = match (x, z, player_pos) {
                (Some(x), Some(z), _) => (*x, *z),
                (_, _, Some(pp)) => (pp.x, pp.z + 4.0),
                _ => return None,
            };
            Some(ClientMessage::FishingCast {
                position: onlinerpg_shared::Position {
                    x: cx,
                    y: 0.0,
                    z: cz,
                },
            })
        }
        AgentAction::StopFishing => Some(ClientMessage::FishingStop),
        AgentAction::PartyInvite { player } => Some(ClientMessage::PartyInvite {
            target_name: player.clone(),
        }),
        AgentAction::PartyLeave => Some(ClientMessage::PartyLeave),
        AgentAction::PartySay { message } => Some(ClientMessage::PartyChat {
            message: message.clone(),
        }),
        // Answering needs the stored invite; handled in
        // `execute::handle_response`.
        AgentAction::PartyAccept { .. }
        | AgentAction::PartyDecline { .. }
        | AgentAction::SummonAccept { .. }
        | AgentAction::SummonDecline { .. } => None,
        // Need player-name → id resolution from SharedState; handled in
        // `execute::handle_response`, not here.
        AgentAction::OfferDeal { .. } => None,
        AgentAction::OpenTrade { .. } => None,
        // Needs the bag and worn gear from SharedState; likewise handled there.
        AgentAction::Use { .. } => None,
        AgentAction::Sell { .. } => None,
        AgentAction::Buy { .. } => None,
        AgentAction::Drop { .. } => None,
        AgentAction::Buyback { .. } => None,
        AgentAction::BreakProp { .. } => None,
        AgentAction::OpenChest { .. } => None,
        // Needs ground-item resolution and the walk-to loop; handled there too.
        AgentAction::Pickup { .. } => None,
        // Only reaches the server as a pre-creation RollCharacterStats; in
        // game there is nothing left to reroll.
        AgentAction::Reroll => None,
        AgentAction::Wait => None,
    }
}

/// Convert a cardinal/ordinal direction string to a (dx, dz) unit offset.
fn direction_to_offset(dir: &str) -> (f32, f32) {
    match dir.to_lowercase().as_str() {
        "north" | "n" => (0.0, -1.0),
        "south" | "s" => (0.0, 1.0),
        "east" | "e" => (1.0, 0.0),
        "west" | "w" => (-1.0, 0.0),
        "northeast" | "ne" => (0.707, -0.707),
        "northwest" | "nw" => (-0.707, -0.707),
        "southeast" | "se" => (0.707, 0.707),
        "southwest" | "sw" => (-0.707, 0.707),
        _ => {
            warn!("Unknown direction '{dir}', defaulting to north");
            (0.0, -1.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_single_action(json: &str) -> AgentAction {
        let resp = parse_agent_response(json).unwrap();
        resp.actions.into_iter().next().unwrap()
    }

    #[test]
    fn move_parses_character_target() {
        let action = parse_single_action(r#"{"actions": [{"type": "move", "target": "Karl"}]}"#);
        let AgentAction::Move { target, .. } = action else {
            panic!("expected Move");
        };
        assert_eq!(target.as_deref(), Some("Karl"));
    }

    #[test]
    fn move_target_accepts_player_alias() {
        let action = parse_single_action(r#"{"actions": [{"type": "move", "player": "Karl"}]}"#);
        let AgentAction::Move { target, .. } = action else {
            panic!("expected Move");
        };
        assert_eq!(target.as_deref(), Some("Karl"));
    }

    #[test]
    fn move_still_parses_coordinates() {
        let action = parse_single_action(
            r#"{"actions": [{"type": "move", "x": 10.0, "y": 0.0, "z": -5.0}]}"#,
        );
        let AgentAction::Move { target, x, z, .. } = action else {
            panic!("expected Move");
        };
        assert_eq!(target, None);
        assert_eq!(x, Some(10.0));
        assert_eq!(z, Some(-5.0));
    }

    #[test]
    fn move_target_coordinate_array_becomes_xz() {
        let action =
            parse_single_action(r#"{"actions": [{"type": "move", "target": [-1460, 4730]}]}"#);
        let AgentAction::Move { target, x, z, .. } = action else {
            panic!("expected Move");
        };
        assert_eq!(target, None);
        assert_eq!(x, Some(-1460.0));
        assert_eq!(z, Some(4730.0));
    }

    #[test]
    fn move_target_xyz_array_becomes_xyz() {
        let action =
            parse_single_action(r#"{"actions": [{"type": "move", "target": [-1460, 1.1, 4730]}]}"#);
        let AgentAction::Move {
            target, x, y, z, ..
        } = action
        else {
            panic!("expected Move");
        };
        assert_eq!(target, None);
        assert_eq!(x, Some(-1460.0));
        assert_eq!(y, Some(1.1));
        assert_eq!(z, Some(4730.0));
    }

    #[test]
    fn move_target_coordinate_object_becomes_xz() {
        let action = parse_single_action(
            r#"{"actions": [{"type": "move", "target": {"x": 10.0, "z": -5.0}}]}"#,
        );
        let AgentAction::Move { target, x, z, .. } = action else {
            panic!("expected Move");
        };
        assert_eq!(target, None);
        assert_eq!(x, Some(10.0));
        assert_eq!(z, Some(-5.0));
    }

    #[test]
    fn move_ignores_extra_reason_field() {
        let action = parse_single_action(
            r#"{"actions": [{"type": "move", "x": -1485.0, "z": 4720.0,
                "reason": "남서쪽 슬라임 탐색"}]}"#,
        );
        let AgentAction::Move { x, z, .. } = action else {
            panic!("expected Move");
        };
        assert_eq!(x, Some(-1485.0));
        assert_eq!(z, Some(4720.0));
    }

    #[test]
    fn bad_action_is_reported_but_others_survive() {
        // An invented action type and a valid say in the same turn: the say
        // and the memory_update survive, and the bad one becomes an error
        // that names the offending type.
        let turn = parse_turn_tolerant(
            r#"{"actions": [{"type": "look", "reason": "scan"}, {"type": "say", "message": "hi"}],
                "memory_update": "kept"}"#,
        )
        .unwrap();
        assert_eq!(turn.actions.len(), 1);
        let AgentAction::Say { ref message } = turn.actions[0] else {
            panic!("expected Say to survive");
        };
        assert_eq!(message, "hi");
        assert_eq!(turn.memory_update.as_deref(), Some("kept"));
        assert_eq!(turn.errors.len(), 1);
        assert!(turn.errors[0].contains("look"));
    }

    #[test]
    fn missing_required_field_is_reported_not_silent() {
        // attack with no monster_id: the whole turn used to die. Now the say
        // survives and the attack becomes a named error.
        let turn = parse_turn_tolerant(
            r#"{"actions": [{"type": "attack"}, {"type": "say", "message": "hi"}]}"#,
        )
        .unwrap();
        assert_eq!(turn.actions.len(), 1);
        assert!(matches!(turn.actions[0], AgentAction::Say { .. }));
        assert_eq!(turn.errors.len(), 1);
        assert!(turn.errors[0].contains("attack"));
    }

    #[test]
    fn party_actions_parse_with_aliases() {
        let action = parse_single_action(
            r#"{"actions": [{"type": "party_invite", "player": "darkcocoa"}]}"#,
        );
        let AgentAction::PartyInvite { player } = action else {
            panic!("expected PartyInvite");
        };
        assert_eq!(player, "darkcocoa");

        for (json, expected) in [
            (
                r#"{"actions": [{"type": "party_accept"}]}"#,
                AgentAction::PartyAccept { player: None },
            ),
            (
                r#"{"actions": [{"type": "join_party"}]}"#,
                AgentAction::PartyAccept { player: None },
            ),
            (
                r#"{"actions": [{"type": "party_decline"}]}"#,
                AgentAction::PartyDecline { player: None },
            ),
            (
                r#"{"actions": [{"type": "leave_party"}]}"#,
                AgentAction::PartyLeave,
            ),
        ] {
            assert!(
                matches!((parse_single_action(json), &expected), (a, e) if std::mem::discriminant(&a) == std::mem::discriminant(e)),
                "{json}"
            );
        }

        let action =
            parse_single_action(r#"{"actions": [{"type": "party_decline", "player": "Mallory"}]}"#);
        let AgentAction::PartyDecline { player } = action else {
            panic!("expected PartyDecline");
        };
        assert_eq!(player.as_deref(), Some("Mallory"));
    }

    #[test]
    fn use_parses_item_and_its_aliases() {
        for json in [
            r#"{"actions": [{"type": "use", "item": "torch"}]}"#,
            r#"{"actions": [{"type": "use_item", "item_def_id": "torch"}]}"#,
            r#"{"actions": [{"type": "equip", "name": "torch"}]}"#,
        ] {
            let AgentAction::Use { item } = parse_single_action(json) else {
                panic!("expected Use for {json}");
            };
            assert_eq!(item, "torch");
        }
    }

    #[test]
    fn sell_and_drop_qty_defaults_to_none() {
        let AgentAction::Sell { qty, .. } = parse_single_action(
            r#"{"actions": [{"type": "sell", "item": "torch", "merchant": "Rica"}]}"#,
        ) else {
            panic!("expected Sell");
        };
        assert!(qty.is_none());

        let AgentAction::Drop { qty, .. } =
            parse_single_action(r#"{"actions": [{"type": "drop", "item": "torch"}]}"#)
        else {
            panic!("expected Drop");
        };
        assert!(qty.is_none());
    }

    #[test]
    fn sell_and_drop_parse_a_numeric_qty() {
        let AgentAction::Sell { qty, .. } = parse_single_action(
            r#"{"actions": [{"type": "sell", "item": "torch", "merchant": "Rica", "qty": 5}]}"#,
        ) else {
            panic!("expected Sell");
        };
        assert_eq!(qty.unwrap().resolve(100), Some(5));

        let AgentAction::Drop { qty, .. } =
            parse_single_action(r#"{"actions": [{"type": "drop", "item": "torch", "amount": 3}]}"#)
        else {
            panic!("expected Drop");
        };
        assert_eq!(qty.unwrap().resolve(100), Some(3));
    }

    #[test]
    fn sell_qty_accepts_the_word_all() {
        let AgentAction::Sell { qty, .. } = parse_single_action(
            r#"{"actions": [{"type": "sell", "item": "torch", "merchant": "Rica", "qty": "all"}]}"#,
        ) else {
            panic!("expected Sell");
        };
        assert_eq!(qty.unwrap().resolve(7), Some(7));
    }

    #[test]
    fn qty_resolve_rejects_zero_and_unknown_words() {
        assert_eq!(Qty::Count(0).resolve(10), None);
        assert_eq!(Qty::Named("some".to_string()).resolve(10), None);
        // Case-insensitive, tolerates surrounding whitespace.
        assert_eq!(Qty::Named(" ALL ".to_string()).resolve(4), Some(4));
    }

    #[test]
    fn pickup_parses_instance_id_as_number() {
        for json in [
            r#"{"actions": [{"type": "pickup", "item": 6043}]}"#,
            r#"{"actions": [{"type": "loot", "instance_id": 6043}]}"#,
            r#"{"actions": [{"type": "pick_up", "id": 6043}]}"#,
        ] {
            let AgentAction::Pickup { item } = parse_single_action(json) else {
                panic!("expected Pickup for {json}");
            };
            assert!(matches!(item, PickupRef::Id(6043)), "for {json}");
        }
    }

    #[test]
    fn open_chest_parses_its_aliases_and_target() {
        for (json, want) in [
            (r#"{"actions": [{"type": "open_chest"}]}"#, None),
            (r#"{"actions": [{"type": "open_dungeon_chest"}]}"#, None),
            (
                r#"{"actions": [{"type": "open_chest", "chest": "great"}]}"#,
                Some("great"),
            ),
            (
                r#"{"actions": [{"type": "open_chest", "which": "the big one"}]}"#,
                Some("the big one"),
            ),
        ] {
            let AgentAction::OpenChest { chest } = parse_single_action(json) else {
                panic!("expected OpenChest for {json}");
            };
            assert_eq!(chest.as_deref(), want, "for {json}");
        }
    }

    #[test]
    fn sell_parses_its_aliases_for_item_and_merchant() {
        for json in [
            r#"{"actions": [{"type": "sell", "item": "goblin_sword", "merchant": "Rica"}]}"#,
            r#"{"actions": [{"type": "sell_item", "item_id": "goblin_sword", "npc": "Rica"}]}"#,
            r#"{"actions": [{"type": "sell", "name": "goblin_sword", "to": "Rica"}]}"#,
        ] {
            let AgentAction::Sell { item, merchant, .. } = parse_single_action(json) else {
                panic!("expected Sell for {json}");
            };
            assert_eq!(
                (item.as_str(), merchant.as_deref()),
                ("goblin_sword", Some("Rica"))
            );
        }
    }

    #[test]
    fn buy_parses_its_aliases_for_item_and_merchant() {
        for json in [
            r#"{"actions": [{"type": "buy", "item": "healing_potion", "merchant": "Rica"}]}"#,
            r#"{"actions": [{"type": "purchase", "item_def_id": "healing_potion", "from": "Rica"}]}"#,
            r#"{"actions": [{"type": "buy_item", "name": "healing_potion", "target": "Rica"}]}"#,
        ] {
            let AgentAction::Buy { item, merchant } = parse_single_action(json) else {
                panic!("expected Buy for {json}");
            };
            assert_eq!(
                (item.as_str(), merchant.as_deref()),
                ("healing_potion", Some("Rica"))
            );
        }
    }

    #[test]
    fn buyback_parses_its_aliases_and_stays_distinct_from_buy() {
        for json in [
            r#"{"actions": [{"type": "buyback", "item": "iron_sword", "merchant": "Rica"}]}"#,
            r#"{"actions": [{"type": "buy_back", "item_id": "iron_sword", "npc": "Rica"}]}"#,
            r#"{"actions": [{"type": "repurchase", "name": "iron_sword", "from": "Rica"}]}"#,
        ] {
            let AgentAction::Buyback { item, merchant } = parse_single_action(json) else {
                panic!("expected Buyback for {json}");
            };
            assert_eq!(
                (item.as_str(), merchant.as_deref()),
                ("iron_sword", Some("Rica"))
            );
        }
    }

    #[test]
    fn trade_actions_parse_without_a_merchant() {
        let AgentAction::Sell { item, merchant, .. } =
            parse_single_action(r#"{"actions": [{"type": "sell", "item": "worn_iron_sword"}]}"#)
        else {
            panic!("expected Sell");
        };
        assert_eq!(item.as_str(), "worn_iron_sword");
        assert_eq!(merchant, None);

        let AgentAction::Buy { merchant, .. } =
            parse_single_action(r#"{"actions": [{"type": "buy", "item": "healing_potion"}]}"#)
        else {
            panic!("expected Buy");
        };
        assert_eq!(merchant, None);
    }

    #[test]
    fn drop_parses_its_aliases() {
        for json in [
            r#"{"actions": [{"type": "drop", "item": "torch"}]}"#,
            r#"{"actions": [{"type": "drop_item", "item_def_id": "torch"}]}"#,
            r#"{"actions": [{"type": "discard", "name": "torch"}]}"#,
        ] {
            let AgentAction::Drop { item, .. } = parse_single_action(json) else {
                panic!("expected Drop for {json}");
            };
            assert_eq!(item, "torch");
        }
    }

    #[test]
    fn break_prop_parses_its_aliases_and_id() {
        for json in [
            r#"{"actions": [{"type": "break_prop", "prop_id": 3}]}"#,
            r#"{"actions": [{"type": "smash", "id": 3}]}"#,
            r#"{"actions": [{"type": "break", "target": 3}]}"#,
        ] {
            let AgentAction::BreakProp { prop_id } = parse_single_action(json) else {
                panic!("expected BreakProp for {json}");
            };
            assert_eq!(prop_id, 3, "for {json}");
        }
    }

    /// The wording the prompts teach picks the great chest; anything else
    /// (including no target at all) leaves the nearest one winning.
    #[test]
    fn great_chest_selector_covers_what_the_prompts_teach() {
        for want in ["great", "the great chest", "Treasure", "big one", "large"] {
            assert!(asks_for_great_chest(Some(want)), "{want} should select it");
        }
        for other in ["small", "nearest", "clutter", ""] {
            assert!(!asks_for_great_chest(Some(other)), "{other} should not");
        }
        assert!(!asks_for_great_chest(None));
    }

    #[test]
    fn pickup_parses_item_name() {
        for json in [
            r#"{"actions": [{"type": "pickup", "item": "small_sword"}]}"#,
            r#"{"actions": [{"type": "take", "name": "small_sword"}]}"#,
        ] {
            let AgentAction::Pickup { item } = parse_single_action(json) else {
                panic!("expected Pickup for {json}");
            };
            let PickupRef::Name(name) = item else {
                panic!("expected a name ref for {json}");
            };
            assert_eq!(name, "small_sword");
        }
    }

    /// `use` needs the bag and worn gear, so it resolves in `execute`.
    #[test]
    fn use_produces_no_direct_command() {
        let action = parse_single_action(r#"{"actions": [{"type": "use", "item": "torch"}]}"#);
        assert!(action_to_command(&action, None).is_none());
    }

    #[test]
    fn reroll_is_read_from_the_action_envelope() {
        assert!(wants_reroll(
            r#"{"thought": "too frail", "actions": [{"type": "reroll"}]}"#
        ));
        assert!(wants_reroll(
            "```json\n{\"actions\": [{\"type\": \"roll_again\"}]}\n```"
        ));
        assert!(!wants_reroll(
            r#"{"thought": "good enough", "actions": [{"type": "wait"}]}"#
        ));
    }

    #[test]
    fn reroll_falls_back_to_the_last_word_said() {
        assert!(wants_reroll("Too weak for a knight. Reroll."));
        assert!(!wants_reroll("I could reroll, but I accept this one."));
    }

    /// A reply we cannot read must not keep the roll loop spinning.
    #[test]
    fn unreadable_reply_accepts_the_roll() {
        assert!(!wants_reroll(""));
        assert!(!wants_reroll("Hmm, hard to say."));
        assert!(!wants_reroll(r#"{"actions": []}"#));
    }

    #[test]
    fn fish_action_parses_and_casts() {
        let response =
            parse_agent_response(r#"{"actions": [{"type": "fish", "x": 10.0, "z": -5.0}]}"#)
                .unwrap();
        let cmd = action_to_command(&response.actions[0], None);
        match cmd {
            Some(ClientMessage::FishingCast { position }) => {
                assert_eq!(position.x, 10.0);
                assert_eq!(position.z, -5.0);
            }
            other => panic!("expected FishingCast, got {other:?}"),
        }
    }

    #[test]
    fn fish_without_coords_casts_ahead_of_the_agent() {
        let response = parse_agent_response(r#"{"actions": [{"type": "fish"}]}"#).unwrap();
        let pos = onlinerpg_shared::Position {
            x: 1.0,
            y: 0.0,
            z: 2.0,
        };
        match action_to_command(&response.actions[0], Some(&pos)) {
            Some(ClientMessage::FishingCast { position }) => {
                assert_eq!(position.x, 1.0);
                assert_eq!(position.z, 6.0);
            }
            other => panic!("expected FishingCast, got {other:?}"),
        }
        // No coordinates and no known position: nothing to send.
        assert!(action_to_command(&response.actions[0], None).is_none());
    }

    #[test]
    fn stop_fishing_parses() {
        let response = parse_agent_response(r#"{"actions": [{"type": "stop_fishing"}]}"#).unwrap();
        assert!(matches!(
            action_to_command(&response.actions[0], None),
            Some(ClientMessage::FishingStop)
        ));
    }

    // ---- Guardrail: every JSON action hint embedded in prompt text must
    // parse under the real action schema, so drift (like the party_accept
    // hint once saying "action" instead of "type") breaks here instead of
    // silently confusing the model.

    /// Undo Rust string-literal escaping so hints in .rs sources read as
    /// they will render: join `\`-newline continuations, then `{{`→`{`,
    /// `}}`→`}`, `\"`→`"`.
    fn unescape_rust_literals(src: &str) -> String {
        let mut joined = String::with_capacity(src.len());
        for (i, chunk) in src.split("\\\n").enumerate() {
            joined.push_str(if i == 0 { chunk } else { chunk.trim_start() });
        }
        joined
            .replace("{{", "{")
            .replace("}}", "}")
            .replace("\\\"", "\"")
    }

    /// Fill the value placeholders hints use (`"item": ...`, `"prop_id": N`).
    fn fill_placeholders(text: &str) -> String {
        text.replace(": ...,", r#": "x","#)
            .replace(": ...}", r#": "x"}"#)
            .replace(": N,", ": 1,")
            .replace(": N}", ": 1}")
    }

    /// Action hints in `text`: balanced JSON objects starting at `{"`.
    fn extract_hints(text: &str) -> Vec<&str> {
        let bytes = text.as_bytes();
        let (mut out, mut i) = (Vec::new(), 0);
        while i < bytes.len() {
            if bytes[i] == b'{' && bytes.get(i + 1) == Some(&b'"') {
                let mut stream =
                    serde_json::Deserializer::from_str(&text[i..]).into_iter::<serde_json::Value>();
                if let Some(Ok(_)) = stream.next() {
                    let len = stream.byte_offset();
                    out.push(&text[i..i + len]);
                    i += len;
                    continue;
                }
            }
            i += 1;
        }
        out
    }

    /// Every `.txt` under `dir`, recursively, except LLM-written memory files.
    fn prompt_files_under(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                prompt_files_under(&path, out);
            } else if path.extension() == Some("txt".as_ref())
                && path.file_name() != Some("memory.txt".as_ref())
            {
                out.push(path);
            }
        }
    }

    #[test]
    fn embedded_prompt_hints_match_the_action_schema() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files: Vec<_> = [
            "src/driver/prompt.rs",
            "src/state.rs",
            "data/system_prompt.txt",
        ]
        .iter()
        .map(|f| root.join(f))
        .collect();
        for dir in ["data/templates", "data/user_prompts", "data/npcs"] {
            prompt_files_under(&root.join(dir), &mut files);
        }
        // Floors catch extractor rot (a hint drifting into unparseable JSON
        // silently drops out of extraction); unlisted files default to 0.
        let floors = [
            ("src/driver/prompt.rs", 2),
            ("src/state.rs", 1),
            ("data/system_prompt.txt", 26),
            ("guard.txt", 2),
            ("merchant.txt", 2),
            ("newcomer.txt", 1),
            ("veteran.txt", 1),
        ];
        for path in files {
            let name = path.strip_prefix(root).unwrap().display().to_string();
            let raw = std::fs::read_to_string(&path)
                .unwrap()
                // Check what the model reads: the {{ACTIONS}} slot filled,
                // exactly as load_system_prompt fills it.
                .replace("{{ACTIONS}}", &action_reference());
            let text = if name.ends_with(".rs") {
                unescape_rust_literals(&raw)
            } else {
                raw
            };
            let text = fill_placeholders(&text);
            let hints = extract_hints(&text);
            let floor = floors
                .iter()
                .find(|(f, _)| name.ends_with(f))
                .map_or(0, |&(_, n)| n);
            assert!(
                hints.len() >= floor,
                "{name}: expected at least {floor} action hints, extractor found {}",
                hints.len()
            );
            for hint in hints {
                let wrapped = format!(r#"{{"actions": [{hint}]}}"#);
                if let Err(e) = parse_agent_response(&wrapped) {
                    panic!("{name}: embedded action hint does not parse: {hint}\n{e}");
                }
            }
        }
    }

    // ACTION_SPECS is the single source of the action documentation; these
    // tests weld it to the enum so neither can drift from the other.

    /// Every action the parser accepts is documented, and nothing that no
    /// longer exists stays documented. The parser side comes from serde's own
    /// unknown-variant error, which lists the enum's variant names — so this
    /// needs no hand-maintained second list.
    #[test]
    fn action_docs_cover_exactly_the_parser_actions() {
        let msg = serde_json::from_value::<AgentAction>(serde_json::json!({"type": "__nope__"}))
            .unwrap_err()
            .to_string();
        let listed = msg
            .split("expected one of ")
            .nth(1)
            .expect("serde unknown-variant error should list the variants");
        let parser: std::collections::BTreeSet<&str> = listed
            .split('`')
            .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            .collect();
        let documented: std::collections::BTreeSet<&str> = ACTION_SPECS
            .iter()
            .flat_map(|s| s.names.iter().chain(s.aliases).copied())
            .collect();
        assert_eq!(
            parser, documented,
            "ACTION_SPECS drifted from the AgentAction enum"
        );
    }

    /// Every example line in the docs parses with the real parser and sits
    /// under the block that documents its action type.
    #[test]
    fn every_documented_example_parses() {
        for spec in ACTION_SPECS {
            let examples: Vec<&str> = spec
                .doc
                .lines()
                .map(str::trim)
                .filter(|l| l.starts_with(r#"{"type""#))
                .collect();
            assert!(
                !examples.is_empty(),
                "doc block {:?} shows no example",
                spec.names
            );
            for line in examples {
                let value: serde_json::Value = serde_json::from_str(line)
                    .unwrap_or_else(|e| panic!("doc example is not JSON: {line}\n{e}"));
                let tag = value["type"].as_str().unwrap();
                assert!(
                    spec.names.contains(&tag),
                    "example sits under the wrong doc block: {line}"
                );
                serde_json::from_value::<AgentAction>(value)
                    .unwrap_or_else(|e| panic!("doc example no longer parses: {line}\n{e}"));
            }
        }
    }

    /// The shared prompt keeps its `{{ACTIONS}}` slot — without it the
    /// generated reference silently stops reaching the model.
    #[test]
    fn system_prompt_file_carries_the_actions_slot() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/system_prompt.txt");
        let text = std::fs::read_to_string(path).unwrap();
        assert!(
            text.contains("{{ACTIONS}}"),
            "data/system_prompt.txt lost its {{{{ACTIONS}}}} slot"
        );
    }
}
