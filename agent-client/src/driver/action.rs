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
    #[serde(rename = "respawn")]
    Respawn,
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
    /// Use an item from the bag: gear is equipped (or taken off if already
    /// worn), consumables are drunk or read. Mirrors the web quickslot.
    #[serde(rename = "use", alias = "use_item", alias = "equip")]
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
    /// Sell one bag item to a nearby merchant, walking up to them first.
    /// The server owns pricing, proximity and wallet checks.
    #[serde(rename = "sell", alias = "sell_item")]
    Sell {
        #[serde(alias = "item_def_id", alias = "item_id", alias = "name")]
        item: String,
        #[serde(alias = "npc", alias = "to", alias = "merchant_name", alias = "target")]
        merchant: String,
    },
    /// Buy one catalog item from a nearby merchant, walking up to them
    /// first. The server owns catalog, pricing and gold checks.
    #[serde(rename = "buy", alias = "buy_item", alias = "purchase")]
    Buy {
        #[serde(alias = "item_def_id", alias = "item_id", alias = "name")]
        item: String,
        #[serde(
            alias = "npc",
            alias = "from",
            alias = "merchant_name",
            alias = "target"
        )]
        merchant: String,
    },
    /// Drop one bag item on the ground where you stand. Stricter than the
    /// web client: worn gear must be taken off first.
    #[serde(rename = "drop", alias = "drop_item", alias = "discard")]
    Drop {
        #[serde(alias = "item_def_id", alias = "item_id", alias = "name")]
        item: String,
    },
    /// Repurchase an item sold to this merchant this session, at the exact
    /// payout price. The server owns the entry list and gold checks.
    #[serde(rename = "buyback", alias = "buy_back", alias = "repurchase")]
    Buyback {
        #[serde(alias = "item_def_id", alias = "item_id", alias = "name")]
        item: String,
        #[serde(
            alias = "npc",
            alias = "from",
            alias = "merchant_name",
            alias = "target"
        )]
        merchant: String,
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
    serde_json::from_str(json_str)
        .map_err(|e| anyhow::anyhow!("Failed to parse agent response: {e}\nRaw: {text}"))
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
}
