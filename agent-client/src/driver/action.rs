//! Agent action model and conversion to game-server commands.
//!
//! Splits responsibility into three layers: the `AgentAction` enum the LLM
//! is expected to emit, parsing helpers that tolerate the various markdown
//! wrappers an LLM might add, and `action_to_command` which lifts a parsed
//! `AgentAction` into a `ClientMessage` for the server.

use onlinerpg_shared::ClientMessage;
use serde::Deserialize;

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
        /// Unset = the agent's `always_sprint` default; false walks instead.
        sprint: Option<bool>,
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
        /// Unset = the agent's `always_sprint` default; false walks instead.
        sprint: Option<bool>,
    },
    /// Keep following a character: re-approach whenever they move, until the
    /// LLM issues anything else that walks us somewhere, or the target is lost.
    #[serde(rename = "follow", alias = "follow_player")]
    Follow {
        #[serde(alias = "player", alias = "name", alias = "character")]
        target: String,
        /// Unset walks the catch-ups (the target sets the pace anyway);
        /// true sprints them.
        sprint: Option<bool>,
    },
    #[serde(rename = "respawn")]
    Respawn,
    /// Cast the rod at (x, z), or 4 m south when omitted; server validates.
    /// Reflexes (the state module) fight the fish — this is only the decision to fish.
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
    /// Leader-only: remove a member from the party.
    #[serde(rename = "party_kick", alias = "kick", alias = "kick_from_party")]
    PartyKick {
        #[serde(alias = "target", alias = "player_name", alias = "target_player")]
        player: String,
    },
    /// Leader-only: hand party leadership to a member.
    #[serde(rename = "party_promote", alias = "promote", alias = "promote_leader")]
    PartyPromote {
        #[serde(alias = "target", alias = "player_name", alias = "target_player")]
        player: String,
    },
    /// Accept a pending friend request — the oldest, or the named requester's.
    #[serde(rename = "friend_accept", alias = "accept_friend")]
    FriendAccept {
        #[serde(default, alias = "target", alias = "player_name", alias = "requester")]
        player: Option<String>,
    },
    /// Decline a pending friend request — the oldest, or the named requester's.
    #[serde(
        rename = "friend_decline",
        alias = "decline_friend",
        alias = "reject_friend"
    )]
    FriendDecline {
        #[serde(default, alias = "target", alias = "player_name", alias = "requester")]
        player: Option<String>,
    },
    /// Drop a friendship, both directions. Name-based: the friend may be
    /// offline.
    #[serde(rename = "friend_remove", alias = "remove_friend", alias = "unfriend")]
    FriendRemove {
        #[serde(alias = "target", alias = "player_name", alias = "friend")]
        player: String,
    },
    /// Ask which friends are online right now; the answer arrives as a
    /// [FriendsOnline] event.
    #[serde(
        rename = "friends_online",
        alias = "check_friends",
        alias = "list_friends"
    )]
    FriendsOnline,
    /// Drop copper into a performer's tip hat — the nearest one, or the
    /// named hat id.
    #[serde(rename = "tip_hat", alias = "tip")]
    TipHat {
        #[serde(default, alias = "target", alias = "id", alias = "hat_id")]
        hat: Option<u64>,
        #[serde(alias = "gold", alias = "copper", alias = "coins")]
        amount: i64,
    },
    /// Wave off the trade window an NPC pushed onto our screen ("Not now"
    /// on the web client's offer toast).
    #[serde(
        rename = "decline_trade",
        alias = "wave_off_trade",
        alias = "refuse_trade"
    )]
    DeclineTrade,
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
        /// Unset = the agent's `always_sprint` default; false walks instead.
        sprint: Option<bool>,
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
        /// Unset = the agent's `always_sprint` default; false walks instead.
        sprint: Option<bool>,
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
        /// Unset = the agent's `always_sprint` default; false walks instead.
        sprint: Option<bool>,
    },
    /// Drop one or more units of a bag item on the ground where you stand.
    /// Stricter than the web client: worn gear must be taken off first.
    #[serde(rename = "drop", alias = "drop_item", alias = "discard")]
    Drop {
        #[serde(
            alias = "item_def_id",
            alias = "item_id",
            alias = "name",
            alias = "target"
        )]
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
        /// Unset = the agent's `always_sprint` default; false walks instead.
        sprint: Option<bool>,
    },
    /// Smash a breakable dungeon prop (barrel/crate) on the current floor,
    /// walking up to it first. The server validates floor and proximity.
    #[serde(rename = "break_prop", alias = "smash", alias = "break")]
    BreakProp {
        #[serde(alias = "id", alias = "prop", alias = "target")]
        prop_id: u32,
        /// Unset = the agent's `always_sprint` default; false walks instead.
        sprint: Option<bool>,
    },
    /// Open a chest standing in the agent's own room: the nearest one, or the
    /// great chest when `chest` asks for it. The server validates floor,
    /// proximity, prop kind, boss state and the per-player cooldown, and
    /// answers with loot or a rejection explaining why.
    #[serde(rename = "open_chest", alias = "open_dungeon_chest")]
    OpenChest {
        #[serde(default, alias = "target", alias = "which", alias = "name")]
        chest: Option<String>,
        /// Unset = the agent's `always_sprint` default; false walks instead.
        sprint: Option<bool>,
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
    /// Read by the lock tests below and by `action_is`.
    pub(super) names: &'static [&'static str],
    /// The enum's `alias` values for these tags. Never rendered — the docs
    /// teach only canonical names — but registered so the enum/table
    /// equality test covers every spelling the parser accepts, and
    /// `action_is` normalizes it.
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
  "/title" lists your titles, "/title N" shows one above your name,
  "/title off" hides it.
  Players speak many languages; answer each message in the language it
  was written in. Remember each player's language (a memory_update
  note), and use it when speaking to them first."#,
    },
    ActionSpec {
        names: &["attack"],
        aliases: &[],
        doc: r#"- Attack a monster:
  {"type": "attack", "target": "m2_1"}
  You walk into range first, then strike. Add "sprint": false to walk that
  approach instead of sprinting it:
  {"type": "attack", "target": "m2_1", "sprint": false}"#,
    },
    ActionSpec {
        names: &["follow"],
        aliases: &["follow_player"],
        doc: r#"- Follow a character and keep up as they move (use this when someone says
  "follow me" — a plain move stops once you arrive, follow keeps tracking):
  {"type": "follow", "target": "PlayerName"}
  Any other action that takes your body over — a move, attack, pickup, chest,
  trade or fishing — stops the follow. Losing them ends it with a
  [FollowEnded] event. A follow walks its catch-ups — you are paced by the
  person you follow anyway; add "sprint": true only to close a big gap fast:
  {"type": "follow", "target": "PlayerName", "sprint": true}"#,
    },
    ActionSpec {
        names: &["move"],
        aliases: &[],
        doc: r#"- Move. To a character or a monster, giving their name or id — you approach
  and stop at the right distance, talking distance for a person and striking
  distance for a monster:
  {"type": "move", "target": "PlayerName"}
  {"type": "move", "target": "m2_1"}
  To an item lying on the ground, so you can pick it up:
  {"type": "move", "target": 6043}
  To a place, using exact coordinates from the world state:
  {"type": "move", "x": 10.0, "y": 0.0, "z": -5.0}
  Or by direction and distance:
  {"type": "move", "direction": "north", "distance": 10.0}
  Directions are exactly these eight words: north, south, east, west,
  northeast, northwest, southeast, southwest. Any other word fails and you
  stay where you are.
  Never copy a character's coordinates into a move — use their name, or you
  walk right into them.

  You sprint by default whenever you are well fed, which is half again as
  fast as walking. Sprinting burns satiation about 30 times faster than
  walking does, and stops on its own once you are no longer well fed. On a
  long journey you may prefer to save the food — add "sprint": false to walk.
  Every action that walks you somewhere (move, attack, pickup, open_chest,
  break_prop, sell, buy) takes the same "sprint": false:
  {"type": "move", "x": 10.0, "z": -5.0, "sprint": false}

- Go into a dungeon. The world state names each entrance and how far away it
  is. Name the dungeon and the floor you want with "depth" — 1 is the first
  floor below ground — and you walk to the entrance and down the stairs on
  your own, opening any doors in the way:
  {"type": "move", "target": "Old Crypt", "depth": 1}
  Without a depth you walk to the entrance and stop outside:
  {"type": "move", "target": "Old Crypt"}
  Without a name the nearest dungeon is used:
  {"type": "move", "depth": 1}
  Deeper floors hold stronger monsters, so descend one floor at a time and
  only while you can still win your fights. Every fifth floor's stair room is
  locked: its key drops from monsters and clutter on the floors above it, and
  the deepest key also opens the great chest. A key on the ground is the most
  important thing in the dungeon — pick it up before anything else, anyone
  may take it first. The world state says which key you need and whether you
  have it. To come back up to the surface:
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
  night (it lights on equip, so other players see your light too, and goes
  out when you take it off):
  {"type": "use", "target": "worn_torch"}"#,
    },
    ActionSpec {
        names: &["pickup"],
        aliases: &["pick_up", "loot", "take"],
        doc: r#"- Pick up an item lying on the ground into your bag. You walk over to it
  first, like a web player clicking it. Give the id shown in the world
  state ("Item on ground: ... [id 6043]"), or its name:
  {"type": "pickup", "target": 6043}
  The world state marks who put an item down ("dropped by Mira"); a
  [GroundItem] event tells you when someone collects such an item, and an
  item that is no longer listed is gone — don't reach for bare ground.
  A few steps rarely justify the hunger; add "sprint": false to walk:
  {"type": "pickup", "target": 6043, "sprint": false}"#,
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
  {"type": "open_chest", "target": "great"}"#,
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
  {"type": "sell", "item": "healing_potion", "target": "Rica", "qty": 5}
  {"type": "sell", "item": "healing_potion", "target": "Rica", "qty": "all"}"#,
    },
    ActionSpec {
        names: &["buy"],
        aliases: &["buy_item", "purchase"],
        doc: r#"- Buy one item from a merchant's catalog at base price. You walk to them
  first; the item lands in your bag if your gold covers it. What each
  nearby merchant sells is listed under their name in the world state.
  Naming the merchant is optional — without one you buy from the nearest.
  One unit per action — repeat for more:
  {"type": "buy", "item": "healing_potion", "target": "Rica"}"#,
    },
    ActionSpec {
        names: &["drop"],
        aliases: &["drop_item", "discard"],
        doc: r#"- Drop one or more units of a bag item on the ground where you stand (e.g.
  to shed weight for better loot), and anyone can pick it up afterwards.
  Defaults to 1 unit; add "qty" for more, or "qty": "all" to drop every
  unit you have. Dropping more than you actually own fails outright —
  nothing is dropped:
  {"type": "drop", "target": "goblin_sword"}
  {"type": "drop", "target": "old_boot", "qty": 2}
  {"type": "drop", "target": "old_boot", "qty": "all"}"#,
    },
    ActionSpec {
        names: &["buyback"],
        aliases: &["buy_back", "repurchase"],
        doc: r#"- Buy back one item you sold to that merchant this session, for exactly
  what they paid you (undo a mis-sell). The [Buyback] event after each
  sale lists what they still hold:
  {"type": "buyback", "item": "iron_sword", "target": "Rica"}"#,
    },
    ActionSpec {
        names: &["offer_deal"],
        aliases: &[],
        doc: r#"- (merchants only) Offer a nearby player a private price on one item — a
  haggle. "kind" is "buy" (they buy from you, the default) or "sell" (they
  sell to you). Which sign favors the player depends on "kind": on a "buy",
  negative is their discount; on a "sell", *positive* is the bonus you pay
  them (negative lowballs them). The server clamps and validates the offer:
  {"type": "offer_deal", "target": "darkcocoa", "item": "healing_potion", "kind": "buy", "modifier_pct": -10}"#,
    },
    ActionSpec {
        names: &["open_trade"],
        aliases: &["trade"],
        doc: r#"- (merchants only) Open your trade window on a nearby player's screen —
  how you move from talk to an actual trade:
  {"type": "open_trade", "target": "darkcocoa"}"#,
    },
    ActionSpec {
        names: &["break_prop"],
        aliases: &["smash", "break"],
        doc: r#"- Smash a breakable dungeon prop (barrel or crate). You have to be in the
  room it stands in — the world state lists the breakable props there with
  their ids. You walk to it first, and smashing opens its cell for movement:
  {"type": "break_prop", "target": 3}"#,
    },
    ActionSpec {
        names: &["party_invite"],
        aliases: &["invite_party", "invite"],
        doc: r#"- Invite a player to your party by name. Works at any distance, like a
  whisper; they get 30 seconds to answer. Your roster shows in the world
  state ("Your party: ..."):
  {"type": "party_invite", "target": "darkcocoa"}"#,
    },
    ActionSpec {
        names: &["party_accept", "party_decline"],
        aliases: &["accept_party", "join_party", "decline_party"],
        doc: r#"- Answer a party invite (the [PartyInvite] event). Accepting puts you in
  their party; a party hunts and travels together. With several invites
  pending, name the inviter — bare answers the oldest:
  {"type": "party_accept"}
  {"type": "party_decline", "target": "darkcocoa"}"#,
    },
    ActionSpec {
        names: &["summon_accept", "summon_decline"],
        aliases: &["accept_summon", "decline_summon"],
        doc: r#"- Answer a party summon (the [PartySummon] event): a party member read a
  summoning scroll asking everyone to teleport to their side. Accepting
  moves you there instantly; you cannot accept mid-combat. With several
  pending, name the caster — bare answers the oldest:
  {"type": "summon_accept"}
  {"type": "summon_decline", "target": "darkcocoa"}"#,
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
        names: &["party_kick", "party_promote"],
        aliases: &["kick", "kick_from_party", "promote", "promote_leader"],
        doc: r#"- (party leader only) Remove a member from your party, or hand them the
  lead. Your roster in the world state marks who leads:
  {"type": "party_kick", "target": "darkcocoa"}
  {"type": "party_promote", "target": "darkcocoa"}"#,
    },
    ActionSpec {
        names: &["friend_accept", "friend_decline"],
        aliases: &["accept_friend", "decline_friend", "reject_friend"],
        doc: r#"- Answer a friend request (the world state lists pending ones). Friends
  see each other in their friend panels and can check who is online. With
  several requests pending, name the requester — bare answers the oldest:
  {"type": "friend_accept"}
  {"type": "friend_decline", "target": "darkcocoa"}"#,
    },
    ActionSpec {
        names: &["friend_remove"],
        aliases: &["remove_friend", "unfriend"],
        doc: r#"- Drop a friendship, both directions. Works while they are offline:
  {"type": "friend_remove", "target": "darkcocoa"}"#,
    },
    ActionSpec {
        names: &["friends_online"],
        aliases: &["check_friends", "list_friends"],
        doc: r#"- Ask which of your friends are online right now; a [FriendsOnline]
  event answers:
  {"type": "friends_online"}"#,
    },
    ActionSpec {
        names: &["tip_hat"],
        aliases: &["tip"],
        doc: r#"- Drop copper into a performer's tip hat (the world state lists hats
  near you with their ids). You must stand within 5m of the hat; "amount"
  is in copper (100 copper = 1 silver). Bare form tips the nearest hat
  that is not your own:
  {"type": "tip_hat", "amount": 20}
  {"type": "tip_hat", "target": 7, "amount": 20}"#,
    },
    ActionSpec {
        names: &["decline_trade"],
        aliases: &["wave_off_trade", "refuse_trade"],
        doc: r#"- Wave off the trade window a merchant pushed onto your screen (the
  [TradeOffer] event) without buying anything — they will stop offering
  for a while:
  {"type": "decline_trade"}"#,
    },
    ActionSpec {
        names: &["reroll"],
        aliases: &["reroll_stats", "roll_again"],
        doc: r#"- Reroll your starting stats (character creation only — does nothing once
  you are in the world):
  {"type": "reroll"}"#,
    },
];

/// The "=== ACTIONS ===" section of the system prompt, rendered from
/// `ACTION_SPECS`. Fills the `{{ACTIONS}}` slot in `load_system_prompt`.
pub(super) fn action_reference() -> String {
    let mut out = String::from("=== ACTIONS ===\n");
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
            | Self::PartyKick { .. }
            | Self::PartyPromote { .. }
            | Self::PartySay { .. }
            | Self::FriendAccept { .. }
            | Self::FriendDecline { .. }
            | Self::FriendRemove { .. }
            | Self::FriendsOnline
            | Self::TipHat { .. }
            | Self::DeclineTrade
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

    /// Whether the action assumes the agent stands where the turn's plan put
    /// it: everything that moves the body, plus the acts gated on proximity
    /// (trade pushes) or on the spot underfoot (use near a campfire, drop).
    /// A failed move cancels these, as the system prompt promises; speech
    /// and party admin still run.
    pub(super) fn needs_position(&self) -> bool {
        self.takes_over_movement()
            || matches!(
                self,
                Self::OpenTrade { .. }
                    | Self::OfferDeal { .. }
                    | Self::Use { .. }
                    | Self::Drop { .. }
                    // Tipping is gated on standing within 5m of the hat.
                    | Self::TipHat { .. }
            )
    }

    /// Whether the action needs no additional outcome event.
    pub(super) fn outcome_speaks_for_itself(&self) -> bool {
        match self {
            Self::Say { .. } | Self::PartySay { .. } | Self::Wait => true,
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
            | Self::StopFishing
            | Self::Respawn
            | Self::OfferDeal { .. }
            | Self::OpenTrade { .. }
            | Self::PartyInvite { .. }
            | Self::PartyAccept { .. }
            | Self::PartyDecline { .. }
            | Self::SummonAccept { .. }
            | Self::SummonDecline { .. }
            | Self::PartyLeave
            | Self::PartyKick { .. }
            | Self::PartyPromote { .. }
            | Self::FriendAccept { .. }
            | Self::FriendDecline { .. }
            | Self::FriendRemove { .. }
            | Self::FriendsOnline
            | Self::TipHat { .. }
            | Self::DeclineTrade
            | Self::Use { .. }
            | Self::Drop { .. }
            | Self::Reroll => false,
        }
    }

    /// The action name used in feedback.
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Say { .. } => "say",
            Self::Attack { .. } => "attack",
            Self::Move { .. } => "move",
            Self::Follow { .. } => "follow",
            Self::Respawn => "respawn",
            Self::Fish { .. } => "fish",
            Self::StopFishing => "stop_fishing",
            Self::OfferDeal { .. } => "offer_deal",
            Self::OpenTrade { .. } => "open_trade",
            Self::PartyInvite { .. } => "party_invite",
            Self::PartyAccept { .. } => "party_accept",
            Self::PartyDecline { .. } => "party_decline",
            Self::SummonAccept { .. } => "summon_accept",
            Self::SummonDecline { .. } => "summon_decline",
            Self::PartyLeave => "party_leave",
            Self::PartyKick { .. } => "party_kick",
            Self::PartyPromote { .. } => "party_promote",
            Self::FriendAccept { .. } => "friend_accept",
            Self::FriendDecline { .. } => "friend_decline",
            Self::FriendRemove { .. } => "friend_remove",
            Self::FriendsOnline => "friends_online",
            Self::TipHat { .. } => "tip_hat",
            Self::DeclineTrade => "decline_trade",
            Self::PartySay { .. } => "party_say",
            Self::Use { .. } => "use",
            Self::Pickup { .. } => "pickup",
            Self::Sell { .. } => "sell",
            Self::Buy { .. } => "buy",
            Self::Drop { .. } => "drop",
            Self::Buyback { .. } => "buyback",
            Self::BreakProp { .. } => "break_prop",
            Self::OpenChest { .. } => "open_chest",
            Self::Reroll => "reroll",
            Self::Wait => "wait",
        }
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
    if let Ok(turn) = parse_turn_tolerant(reply) {
        if turn
            .actions
            .iter()
            .any(|a| matches!(a, AgentAction::Reroll))
        {
            return true;
        }
        // Any action that did parse is an answer in itself; only when every
        // action failed does the text heuristic below get a say.
        if !turn.actions.is_empty() || turn.errors.is_empty() {
            return false;
        }
    }
    let reply = reply.to_lowercase();
    match (reply.rfind("reroll"), reply.rfind("accept")) {
        (Some(reroll), Some(accept)) => reroll > accept,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

/// What a tolerant parse yields: the actions that parsed, plus a human-readable
/// complaint for each one that didn't. The complaints are fed back to the LLM
/// so a mistyped action reports its own error instead of vanishing.
pub(super) struct ParsedTurn {
    pub actions: Vec<AgentAction>,
    pub memory_update: Option<String>,
    /// Per-player favor deltas as raw JSON so a malformed shape can never
    /// sink the actions; `favor_deltas` coerces.
    pub favor: Option<serde_json::Value>,
    pub errors: Vec<String>,
}

impl ParsedTurn {
    /// Favor deltas in whatever shape the LLM sent them — an integer, a
    /// float, or a numeric string ("+1"). Unusable entries are dropped.
    /// Pure shape coercion: policy (step size, bounds, who qualifies)
    /// belongs to `SharedState::apply_favor`.
    pub(super) fn favor_deltas(&self) -> Vec<(String, i32)> {
        let Some(map) = self.favor.as_ref().and_then(|v| v.as_object()) else {
            return Vec::new();
        };
        map.iter()
            .filter_map(|(name, v)| {
                let n = v
                    .as_i64()
                    .or_else(|| v.as_f64().map(|f| f.round() as i64))
                    .or_else(|| {
                        v.as_str()
                            .and_then(|s| s.trim().trim_start_matches('+').parse().ok())
                    })?;
                let n = n.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
                Some((name.clone(), n))
            })
            .collect()
    }
}

/// Parse each action independently so one malformed action does not discard
/// the whole turn (the say that rode along with it, the memory_update). Every
/// rejected action becomes an error string naming what was wrong.
pub(super) fn parse_turn_tolerant(text: &str) -> anyhow::Result<ParsedTurn> {
    // The error reaches the [BadResponse] prompt event, so it must carry the
    // parser's complaint only, never the reply itself — a model fed its own
    // malformed output continues in that shape.
    let json_str = extract_json(text);
    let mut value: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| anyhow::anyhow!("{e}"))?;
    normalize_targets(&mut value);

    let memory_update = value
        .get("memory_update")
        .and_then(|t| t.as_str())
        .map(str::to_string);
    let favor = value.get("favor").filter(|v| !v.is_null()).cloned();

    let mut actions = Vec::new();
    let mut errors = Vec::new();
    match value.get("actions") {
        Some(serde_json::Value::Array(arr)) => {
            for elem in arr {
                match serde_json::from_value::<AgentAction>(elem.clone()) {
                    Ok(action) => actions.push(action),
                    Err(e) => {
                        let kind = elem
                            .get("type")
                            .and_then(|t| t.as_str())
                            .unwrap_or("(no type)");
                        errors.push(format!("action \"{kind}\": {e} — that action was skipped"));
                    }
                }
            }
        }
        // A missing or non-array "actions" used to yield a silent empty turn —
        // the exact "agent quietly does nothing" failure this parser exists
        // to eliminate. Complain instead.
        Some(_) => errors
            .push("\"actions\" must be a JSON array of action objects — nothing ran".to_string()),
        None => errors.push(
            "your reply has no \"actions\" array — nothing ran. Send \
             [{\"type\": \"wait\"}] to deliberately do nothing"
                .to_string(),
        ),
    }
    Ok(ParsedTurn {
        actions,
        memory_update,
        favor,
        errors,
    })
}

/// Rewrite the target spellings LLMs keep reaching for into the shapes serde
/// takes: coordinate targets into x/z, float ids into integers, a lone sell/buy
/// "target" into the item it names.
fn normalize_targets(value: &mut serde_json::Value) {
    let Some(actions) = value.get_mut("actions").and_then(|a| a.as_array_mut()) else {
        return;
    };
    for action in actions {
        let tag = action.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if action_is(tag, "move") {
            normalize_move_target(action);
        } else if action_is(tag, "pickup") {
            // Ids the docs teach as numbers come back as floats after
            // arithmetic (6043.0), which serde's integer fields refuse.
            coerce_id_fields(
                action,
                &["target", "item", "id", "instance_id", "item_id", "name"],
            );
        } else if action_is(tag, "break_prop") {
            coerce_id_fields(action, &["target", "id", "prop", "prop_id"]);
        } else if action_is(tag, "tip_hat") {
            coerce_id_fields(
                action,
                &[
                    "target", "id", "hat_id", "hat", "amount", "gold", "copper", "coins",
                ],
            );
        } else if ["sell", "buy", "buyback"].iter().any(|c| action_is(tag, c)) {
            // "target" aliases the merchant, but a sell/buy naming only one
            // thing means the goods — give the value to the item field the
            // parser requires instead of failing with "missing field item".
            let Some(obj) = action.as_object_mut() else {
                continue;
            };
            let has_item = ["item", "item_def_id", "item_id", "name"]
                .iter()
                .any(|k| obj.contains_key(*k));
            if !has_item {
                if let Some(v) = obj.remove("target") {
                    obj.insert("item".to_string(), v);
                }
            }
        }
    }
}

/// Whether `tag` spells the action canonically named `canon`, in any alias
/// the parser accepts. Read off `ACTION_SPECS`, which the lock tests tie to
/// the enum — so a new alias is normalized without another list to update.
fn action_is(tag: &str, canon: &str) -> bool {
    ACTION_SPECS.iter().any(|spec| {
        spec.names.contains(&canon) && (spec.names.contains(&tag) || spec.aliases.contains(&tag))
    })
}

/// LLMs keep writing `{"type":"move","target":[x,z]}` or `"target":{"x":..}`
/// despite the schema saying `target` is a name. Rewrite those into the x/z
/// fields instead of discarding the whole response.
fn normalize_move_target(action: &mut serde_json::Value) {
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
        return;
    }
    // Ids print as numbers, so LLMs write them as numbers (6043.0 after
    // arithmetic); the resolver reads shapes off the string.
    if let Some(n) = action.get("target").and_then(as_integer) {
        action.as_object_mut().unwrap()["target"] = n.to_string().into();
    }
}

/// Put the named fields back into integer shape: a float that is really an
/// id (3.0), or an id quoted as a string ("3").
fn coerce_id_fields(action: &mut serde_json::Value, fields: &[&str]) {
    let Some(obj) = action.as_object_mut() else {
        return;
    };
    for field in fields {
        let Some(v) = obj.get(*field) else {
            continue;
        };
        let n = as_integer(v).or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()));
        if let Some(n) = n {
            obj[*field] = n.into();
        }
    }
}

/// A JSON number that names an id, whether the model wrote it whole or as a
/// float. Anything with a fractional part is a coordinate, not an id.
fn as_integer(value: &serde_json::Value) -> Option<u64> {
    if let Some(n) = value.as_u64() {
        return Some(n);
    }
    let f = value.as_f64()?;
    (f.fract() == 0.0 && f >= 0.0 && f <= u64::MAX as f64).then_some(f as u64)
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
) -> Result<(f32, f32), GoalError> {
    if let (Some(x), Some(z)) = (x, z) {
        Ok((*x, *z))
    } else if let (Some(dir), Some(dist), Some(pp)) = (direction.as_deref(), distance, player_pos) {
        let (dx, dz) =
            direction_to_offset(dir).ok_or_else(|| GoalError::BadDirection(dir.into()))?;
        Ok((pp.x + dx * dist, pp.z + dz * dist))
    } else {
        Err(GoalError::NoGoal)
    }
}

/// Why a move carried no usable destination.
#[derive(Debug, PartialEq)]
pub(super) enum GoalError {
    /// A direction word that names no direction. Not guessed at — see
    /// [`direction_to_offset`].
    BadDirection(String),
    /// Nothing to go on: no coordinate pair, no direction with a distance.
    NoGoal,
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
        AgentAction::Attack { monster_id, .. } => Some(ClientMessage::PlayerAttack {
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
            ..
        } => {
            // Name-targeted and dungeon-floor moves need SharedState (name
            // resolution, layouts); handled in `execute::handle_response`.
            if target.is_some() || depth.is_some() {
                return None;
            }
            let (gx, gz) = resolve_move_goal(x, z, direction, distance, player_pos).ok()?;
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
        AgentAction::FriendsOnline => Some(ClientMessage::RequestFriendsOnline),
        // Answering needs the stored invite; handled in
        // `execute::handle_response`.
        AgentAction::PartyAccept { .. }
        | AgentAction::PartyDecline { .. }
        | AgentAction::SummonAccept { .. }
        | AgentAction::SummonDecline { .. } => None,
        // Need pending-request, roster or nearby-hat state from SharedState;
        // handled in `execute::handle_response`.
        AgentAction::PartyKick { .. }
        | AgentAction::PartyPromote { .. }
        | AgentAction::FriendAccept { .. }
        | AgentAction::FriendDecline { .. }
        | AgentAction::FriendRemove { .. }
        | AgentAction::TipHat { .. }
        | AgentAction::DeclineTrade => None,
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
/// `None` for anything else: guessing north sent the agent walking the wrong
/// way with nothing in the transcript to explain it.
pub(super) fn direction_to_offset(dir: &str) -> Option<(f32, f32)> {
    Some(match dir.trim().to_lowercase().as_str() {
        "north" | "n" => (0.0, -1.0),
        "south" | "s" => (0.0, 1.0),
        "east" | "e" => (1.0, 0.0),
        "west" | "w" => (-1.0, 0.0),
        "northeast" | "ne" => (0.707, -0.707),
        "northwest" | "nw" => (-0.707, -0.707),
        "southeast" | "se" => (0.707, 0.707),
        "southwest" | "sw" => (-0.707, 0.707),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_single_action(json: &str) -> AgentAction {
        let turn = parse_turn_tolerant(json).unwrap();
        assert!(turn.errors.is_empty(), "{:?}", turn.errors);
        turn.actions.into_iter().next().unwrap()
    }

    /// "Unset" must stay distinguishable from `false`: only the former takes
    /// the agent's default.
    #[test]
    fn the_sprint_opt_out_parses_on_every_walking_action() {
        let sprint_of = |json: &str| match parse_single_action(json) {
            AgentAction::Move { sprint, .. }
            | AgentAction::Attack { sprint, .. }
            | AgentAction::Pickup { sprint, .. }
            | AgentAction::Follow { sprint, .. }
            | AgentAction::OpenChest { sprint, .. }
            | AgentAction::BreakProp { sprint, .. }
            | AgentAction::Sell { sprint, .. }
            | AgentAction::Buy { sprint, .. }
            | AgentAction::Buyback { sprint, .. } => sprint,
            other => panic!("expected a walking action for {json}, got {other:?}"),
        };

        for (bare, opted_out) in [
            (
                r#"{"actions": [{"type": "move", "x": 1.0, "z": 2.0}]}"#,
                r#"{"actions": [{"type": "move", "x": 1.0, "z": 2.0, "sprint": false}]}"#,
            ),
            (
                r#"{"actions": [{"type": "attack", "target": "m2_1"}]}"#,
                r#"{"actions": [{"type": "attack", "target": "m2_1", "sprint": false}]}"#,
            ),
            (
                r#"{"actions": [{"type": "pickup", "target": 6043}]}"#,
                r#"{"actions": [{"type": "pickup", "target": 6043, "sprint": false}]}"#,
            ),
            (
                r#"{"actions": [{"type": "follow", "target": "Karl"}]}"#,
                r#"{"actions": [{"type": "follow", "target": "Karl", "sprint": false}]}"#,
            ),
            (
                r#"{"actions": [{"type": "open_chest"}]}"#,
                r#"{"actions": [{"type": "open_chest", "sprint": false}]}"#,
            ),
            (
                r#"{"actions": [{"type": "break_prop", "target": 7}]}"#,
                r#"{"actions": [{"type": "break_prop", "target": 7, "sprint": false}]}"#,
            ),
            (
                r#"{"actions": [{"type": "sell", "item": "apple"}]}"#,
                r#"{"actions": [{"type": "sell", "item": "apple", "sprint": false}]}"#,
            ),
            (
                r#"{"actions": [{"type": "buy", "item": "apple"}]}"#,
                r#"{"actions": [{"type": "buy", "item": "apple", "sprint": false}]}"#,
            ),
            (
                r#"{"actions": [{"type": "buyback", "item": "apple"}]}"#,
                r#"{"actions": [{"type": "buyback", "item": "apple", "sprint": false}]}"#,
            ),
        ] {
            assert_eq!(sprint_of(bare), None, "{bare}");
            assert_eq!(sprint_of(opted_out), Some(false), "{opted_out}");
        }
    }

    /// The prompt must state the default speed, or the LLM cannot weigh travel
    /// time against the food it costs.
    #[test]
    fn the_action_reference_explains_the_default_movement_speed() {
        let reference = action_reference();
        assert!(reference.contains("sprint by default"), "{reference}");
        assert!(reference.contains(r#""sprint": false"#), "{reference}");
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
    fn missing_or_malformed_actions_array_is_reported_not_silent() {
        // No "actions" key at all: an empty turn with no complaint would leave
        // the agent guessing why nothing happened.
        let turn = parse_turn_tolerant(r#"{"thought": "hmm"}"#).unwrap();
        assert!(turn.actions.is_empty());
        assert_eq!(turn.errors.len(), 1);
        assert!(turn.errors[0].contains("\"actions\""));

        // "actions" present but not an array.
        let turn = parse_turn_tolerant(r#"{"actions": "wait"}"#).unwrap();
        assert!(turn.actions.is_empty());
        assert_eq!(turn.errors.len(), 1);
        assert!(turn.errors[0].contains("array"));

        // An explicitly empty array stays a legal deliberate no-op.
        let turn = parse_turn_tolerant(r#"{"actions": []}"#).unwrap();
        assert!(turn.actions.is_empty());
        assert!(turn.errors.is_empty());
    }

    /// Favor arrives in whatever shape the LLM chose — int, string, float —
    /// and a garbage entry (or a garbage favor field altogether) drops out
    /// without sinking the actions. Values pass through unclamped: the
    /// step-size policy lives in `apply_favor` alone.
    #[test]
    fn favor_deltas_tolerate_llm_spellings() {
        let turn = parse_turn_tolerant(
            r#"{"actions": [{"type": "wait"}],
                "favor": {"jake1": 1, "mira": "+1", "tom": 3.7, "bad": "warm"}}"#,
        )
        .unwrap();
        let mut deltas = turn.favor_deltas();
        deltas.sort();
        assert_eq!(
            deltas,
            [
                ("jake1".to_string(), 1),
                ("mira".to_string(), 1),
                ("tom".to_string(), 4)
            ],
            "floats round, strings parse, the unparseable entry is dropped"
        );

        let turn =
            parse_turn_tolerant(r#"{"actions": [{"type": "wait"}], "favor": "jake1 is nice"}"#)
                .unwrap();
        assert!(turn.favor_deltas().is_empty());
        assert_eq!(turn.actions.len(), 1, "actions survive a malformed favor");

        let turn = parse_turn_tolerant(r#"{"actions": [{"type": "wait"}]}"#).unwrap();
        assert!(turn.favor_deltas().is_empty());
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
            let AgentAction::Pickup { item, .. } = parse_single_action(json) else {
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
            let AgentAction::OpenChest { chest, .. } = parse_single_action(json) else {
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
            let AgentAction::Buy { item, merchant, .. } = parse_single_action(json) else {
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
            let AgentAction::Buyback { item, merchant, .. } = parse_single_action(json) else {
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
            r#"{"actions": [{"type": "drop", "target": "torch"}]}"#,
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
            let AgentAction::BreakProp { prop_id, .. } = parse_single_action(json) else {
                panic!("expected BreakProp for {json}");
            };
            assert_eq!(prop_id, 3, "for {json}");
        }
    }

    /// Ids the docs teach as numbers come back as floats after arithmetic,
    /// or quoted — the same tolerance move targets already had.
    #[test]
    fn pickup_and_break_prop_tolerate_float_and_quoted_ids() {
        let AgentAction::Pickup { item, .. } =
            parse_single_action(r#"{"actions": [{"type": "pickup", "target": 6043.0}]}"#)
        else {
            panic!("expected Pickup");
        };
        assert!(matches!(item, PickupRef::Id(6043)));

        for json in [
            r#"{"actions": [{"type": "break_prop", "target": 3.0}]}"#,
            r#"{"actions": [{"type": "break_prop", "target": "3"}]}"#,
        ] {
            let AgentAction::BreakProp { prop_id, .. } = parse_single_action(json) else {
                panic!("expected BreakProp for {json}");
            };
            assert_eq!(prop_id, 3, "for {json}");
        }
    }

    /// A sell/buy that names only one thing means the goods: its "target"
    /// binds to the item, not to the merchant field the alias would pick.
    #[test]
    fn a_lone_trade_target_names_the_goods() {
        let AgentAction::Sell { item, merchant, .. } =
            parse_single_action(r#"{"actions": [{"type": "sell", "target": "healing_potion"}]}"#)
        else {
            panic!("expected Sell");
        };
        assert_eq!((item.as_str(), merchant), ("healing_potion", None));

        let AgentAction::Buy { item, merchant, .. } =
            parse_single_action(r#"{"actions": [{"type": "buy", "target": "healing_potion"}]}"#)
        else {
            panic!("expected Buy");
        };
        assert_eq!((item.as_str(), merchant), ("healing_potion", None));

        // With the goods named, "target" keeps meaning the merchant.
        let AgentAction::Sell { item, merchant, .. } = parse_single_action(
            r#"{"actions": [{"type": "sell", "item": "healing_potion", "target": "Rica"}]}"#,
        ) else {
            panic!("expected Sell");
        };
        assert_eq!(
            (item.as_str(), merchant.as_deref()),
            ("healing_potion", Some("Rica"))
        );
    }

    /// The parse error the [BadResponse] event carries must not quote the
    /// reply: a model fed its own malformed output continues in that shape.
    #[test]
    fn a_parse_error_never_echoes_the_reply() {
        let reply = "Sure! I will move to Karl now and greet him warmly.";
        let Err(err) = parse_turn_tolerant(reply) else {
            panic!("expected a parse error");
        };
        let err = err.to_string();
        assert!(!err.contains("Karl"), "{err}");
        assert!(!err.contains("Sure"), "{err}");
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
            let AgentAction::Pickup { item, .. } = parse_single_action(json) else {
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
    fn party_kick_and_promote_parse_their_member() {
        let AgentAction::PartyKick { player } =
            parse_single_action(r#"{"actions": [{"type": "party_kick", "target": "darkcocoa"}]}"#)
        else {
            panic!("expected PartyKick");
        };
        assert_eq!(player, "darkcocoa");

        let AgentAction::PartyPromote { player } =
            parse_single_action(r#"{"actions": [{"type": "promote", "player": "darkcocoa"}]}"#)
        else {
            panic!("expected PartyPromote via alias");
        };
        assert_eq!(player, "darkcocoa");
    }

    #[test]
    fn friend_actions_parse_with_aliases() {
        let AgentAction::FriendAccept { player } =
            parse_single_action(r#"{"actions": [{"type": "friend_accept"}]}"#)
        else {
            panic!("expected FriendAccept");
        };
        assert_eq!(player, None);

        let AgentAction::FriendDecline { player } = parse_single_action(
            r#"{"actions": [{"type": "decline_friend", "target": "Mallory"}]}"#,
        ) else {
            panic!("expected FriendDecline via alias");
        };
        assert_eq!(player.as_deref(), Some("Mallory"));

        let AgentAction::FriendRemove { player } =
            parse_single_action(r#"{"actions": [{"type": "unfriend", "target": "Mallory"}]}"#)
        else {
            panic!("expected FriendRemove via alias");
        };
        assert_eq!(player, "Mallory");

        assert!(matches!(
            parse_single_action(r#"{"actions": [{"type": "friends_online"}]}"#),
            AgentAction::FriendsOnline
        ));
    }

    #[test]
    fn friends_online_sends_the_request_over_the_socket() {
        let action = parse_single_action(r#"{"actions": [{"type": "friends_online"}]}"#);
        assert!(matches!(
            action_to_command(&action, None),
            Some(ClientMessage::RequestFriendsOnline)
        ));
    }

    #[test]
    fn tip_hat_parses_bare_and_with_a_float_or_string_id() {
        let AgentAction::TipHat { hat, amount } =
            parse_single_action(r#"{"actions": [{"type": "tip_hat", "amount": 20}]}"#)
        else {
            panic!("expected TipHat");
        };
        assert_eq!(hat, None);
        assert_eq!(amount, 20);

        // Ids print as numbers, so LLMs send floats and quoted numbers.
        for json in [
            r#"{"actions": [{"type": "tip_hat", "target": 7.0, "amount": 20}]}"#,
            r#"{"actions": [{"type": "tip", "hat_id": "7", "copper": 20}]}"#,
        ] {
            let AgentAction::TipHat { hat, amount } = parse_single_action(json) else {
                panic!("expected TipHat for {json}");
            };
            assert_eq!(hat, Some(7), "for {json}");
            assert_eq!(amount, 20, "for {json}");
        }
    }

    #[test]
    fn decline_trade_parses_and_stays_off_the_socket() {
        let action = parse_single_action(r#"{"actions": [{"type": "decline_trade"}]}"#);
        assert!(matches!(action, AgentAction::DeclineTrade));
        assert!(action_to_command(&action, None).is_none());
        assert!(!action.takes_over_movement());
    }

    #[test]
    fn fish_action_parses_and_casts() {
        let action =
            parse_single_action(r#"{"actions": [{"type": "fish", "x": 10.0, "z": -5.0}]}"#);
        let cmd = action_to_command(&action, None);
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
        let action = parse_single_action(r#"{"actions": [{"type": "fish"}]}"#);
        let pos = onlinerpg_shared::Position {
            x: 1.0,
            y: 0.0,
            z: 2.0,
        };
        match action_to_command(&action, Some(&pos)) {
            Some(ClientMessage::FishingCast { position }) => {
                assert_eq!(position.x, 1.0);
                assert_eq!(position.z, 6.0);
            }
            other => panic!("expected FishingCast, got {other:?}"),
        }
        // No coordinates and no known position: nothing to send.
        assert!(action_to_command(&action, None).is_none());
    }

    #[test]
    fn stop_fishing_parses() {
        let action = parse_single_action(r#"{"actions": [{"type": "stop_fishing"}]}"#);
        assert!(matches!(
            action_to_command(&action, None),
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

    /// Action hints in `text`: balanced JSON objects starting at `{"` that
    /// carry a "type" key. Objects without one (the favor examples)
    /// document response fields, not actions.
    fn extract_hints(text: &str) -> Vec<&str> {
        let bytes = text.as_bytes();
        let (mut out, mut i) = (Vec::new(), 0);
        while i < bytes.len() {
            if bytes[i] == b'{' && bytes.get(i + 1) == Some(&b'"') {
                let mut stream =
                    serde_json::Deserializer::from_str(&text[i..]).into_iter::<serde_json::Value>();
                if let Some(Ok(_)) = stream.next() {
                    let len = stream.byte_offset();
                    let hint = &text[i..i + len];
                    if hint.contains("\"type\"") {
                        out.push(hint);
                    }
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
        let mut files: Vec<_> = ["src/driver/prompt.rs", "data/system_prompt.txt"]
            .iter()
            .map(|f| root.join(f))
            .collect();
        for entry in std::fs::read_dir(root.join("src/state")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension() == Some("rs".as_ref()) {
                files.push(path);
            }
        }
        for dir in ["data/templates", "data/user_prompts", "data/npcs"] {
            prompt_files_under(&root.join(dir), &mut files);
        }
        // Floors catch extractor rot (a hint drifting into unparseable JSON
        // silently drops out of extraction); unlisted files default to 0.
        let floors = [
            ("src/driver/prompt.rs", 2),
            ("src/state/dungeon.rs", 1),
            ("src/state/world_state.rs", 1),
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
                let turn = parse_turn_tolerant(&wrapped)
                    .unwrap_or_else(|e| panic!("{name}: hint is not JSON: {hint}\n{e}"));
                assert!(
                    turn.errors.is_empty(),
                    "{name}: embedded action hint does not parse: {hint}\n{:?}",
                    turn.errors
                );
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
                // The whole turn parser, not bare serde: normalising numeric
                // ids is part of reading a turn.
                let turn = parse_turn_tolerant(&format!(r#"{{"actions": [{line}]}}"#)).unwrap();
                assert!(
                    turn.errors.is_empty(),
                    "doc example no longer parses: {line}\n{:?}",
                    turn.errors
                );
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

    /// Ground item ids print as numbers, so LLMs write them as numbers. They
    /// reach the resolver as the string it reads shapes off.
    #[test]
    fn a_numeric_move_target_survives_as_a_string() {
        // A model that did arithmetic writes 6043.0, and a number left where a
        // string belongs loses the whole response, not just this action.
        for json in [
            r#"{"actions": [{"type": "move", "target": 6043}]}"#,
            r#"{"actions": [{"type": "move", "target": 6043.0}]}"#,
        ] {
            let AgentAction::Move { target, x, z, .. } = parse_single_action(json) else {
                panic!("expected Move for {json}");
            };
            assert_eq!(target.as_deref(), Some("6043"));
            assert_eq!((x, z), (None, None));
        }
    }

    /// The eight compass words, however the LLM abbreviates or pads them.
    #[test]
    fn the_compass_words_resolve() {
        assert_eq!(direction_to_offset("north"), Some((0.0, -1.0)));
        assert_eq!(direction_to_offset(" EAST "), Some((1.0, 0.0)));
        assert_eq!(direction_to_offset("sw"), Some((-0.707, 0.707)));
    }

    /// Anything else stops the move. Guessing north walked the agent the
    /// wrong way with nothing in the transcript to explain it.
    #[test]
    fn a_word_that_is_not_a_direction_is_not_guessed_at() {
        for junk in ["forward", "up", "left", "왼쪽", ""] {
            assert_eq!(direction_to_offset(junk), None, "{junk}");
        }

        let pos = onlinerpg_shared::Position {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        assert_eq!(
            resolve_move_goal(
                &None,
                &None,
                &Some("forward".to_string()),
                &Some(10.0),
                Some(&pos)
            ),
            Err(GoalError::BadDirection("forward".to_string()))
        );
    }

    /// Only speech and waiting report themselves; everything else owes the
    /// LLM an outcome. A miscategorised action silently loses its feedback.
    #[test]
    fn only_speech_and_waiting_report_themselves() {
        for quiet in [
            AgentAction::Say {
                message: "hi".to_string(),
            },
            AgentAction::PartySay {
                message: "hi".to_string(),
            },
            AgentAction::Wait,
        ] {
            assert!(quiet.outcome_speaks_for_itself(), "{quiet:?}");
        }
        for owes in [
            AgentAction::Move {
                target: None,
                x: Some(1.0),
                y: None,
                z: Some(2.0),
                direction: None,
                distance: None,
                depth: None,
                sprint: None,
            },
            AgentAction::Attack {
                monster_id: "m2_1".to_string(),
                sprint: None,
            },
            AgentAction::Use {
                item: "torch".to_string(),
            },
            AgentAction::PartyLeave,
        ] {
            assert!(!owes.outcome_speaks_for_itself(), "{owes:?}");
        }
    }
}
