//! Live in-game entity records: the per-frame state the server broadcasts
//! for every player and monster. `Player` and `Monster` are the snapshot
//! types embedded in `ServerMessage::GameState`; `MonsterState` is the
//! enum the client renders as an animation pose.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::character::{CharacterClass, Gender};
use crate::world::Position;

/// A live player's session handle. Minted fresh on every login and dropped on
/// disconnect — it is never persisted, so it is not a durable identity (the
/// unique character name is). It is a newtype rather than a bare `String` so
/// the compiler can keep it apart from the other id-shaped strings it travels
/// with, above all monster ids: `Monster::is_controllable_by` is an
/// authorization gate, and before this the two were the same type there.
///
/// `#[serde(transparent)]` puts a bare integer on the wire. Two invariants
/// follow from that, and this is the one place they are stated:
///
/// 1. Ids are minted from a counter starting at 1 (the server's
///    `next_player_id`), so they stay far below 2^53 and survive the JS number
///    boundary exactly. `0` is therefore free to mean "no player".
/// 2. Never use this as a map key in a type that crosses `wasm_api::to_js`.
///    `serialize_maps_as_objects` rejects non-string keys outright and the
///    client swallows the resulting error, so the whole frame silently
///    disappears. `ServerMessage::GameState` carries a `Vec<Player>` for this
///    reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlayerId(u64);

impl PlayerId {
    pub fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for PlayerId {
    fn from(n: u64) -> Self {
        Self(n)
    }
}

impl fmt::Display for PlayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    pub position: Position,
    pub rotation: f32,
    pub level: u32,
    pub health: u32,
    pub max_health: u32,
    pub class: CharacterClass,
    #[serde(default)]
    pub gender: Gender,
    #[serde(default)]
    pub is_official_npc: bool,
    #[serde(default)]
    pub torch_on: bool,
    #[serde(default)]
    pub floor_level: i8,
    // NEVER add `skip_serializing_if` here: rmp_serde::to_vec encodes
    // structs as positional arrays, so skipping a mid-struct field shifts
    // every later field into the wrong slot on the wire.
    #[serde(default)]
    pub object_type: Option<String>,
    /// Equipped main-hand item def id; `None` renders the class default.
    #[serde(default)]
    pub main_hand: Option<String>,
    /// Equipped back item def id; `None` renders no cape.
    #[serde(default)]
    pub back: Option<String>,
    #[serde(skip)]
    pub object_id: Option<u32>,
    #[serde(skip)]
    pub last_combat_at: u64,
    /// Which program drives this player, from the `ClientInfo` handshake.
    /// Deliberately never serialized: it feeds the `/who` totals only, and
    /// broadcasting it would let clients label individual players.
    #[serde(skip)]
    pub client_kind: ClientKind,
}

/// Client program on the other end of a connection. Self-reported, so it may
/// only ever inform counts — never permissions, or clients would have a
/// reason to lie (`doc/REMOTE_AGENT_CLIENT.md`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientKind {
    /// Never sent a handshake (only reachable for players created in tests).
    #[default]
    Unknown,
    /// Browser client.
    Web,
    /// agent-client.
    Cli,
    Other,
}

impl ClientKind {
    /// Map the handshake string onto the known set, so a hostile client
    /// cannot invent labels.
    pub fn from_reported(reported: &str) -> Self {
        match reported {
            "web" => ClientKind::Web,
            "cli" => ClientKind::Cli,
            _ => ClientKind::Other,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ClientKind::Unknown => "unknown",
            ClientKind::Web => "web",
            ClientKind::Cli => "cli",
            ClientKind::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MonsterState {
    #[serde(rename = "idle")]
    Idle,
    #[serde(rename = "walk")]
    Walk,
    #[serde(rename = "run")]
    Run,
    #[serde(rename = "attack")]
    Attack,
    #[serde(rename = "hit")]
    Hit,
    #[serde(rename = "dead")]
    Dead,
}

impl fmt::Display for MonsterState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Walk => write!(f, "walk"),
            Self::Run => write!(f, "run"),
            Self::Attack => write!(f, "attack"),
            Self::Hit => write!(f, "hit"),
            Self::Dead => write!(f, "dead"),
        }
    }
}

/// Who owns a monster's removal, decided at spawn time. New spawn kinds
/// (events, bosses, quests) add variants here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MonsterLifecycle {
    /// Ambient spawn: the ownership system (AOI events + sweep) despawns it
    /// once no player is near.
    #[default]
    Ambient,
    /// A dungeon floor slot owns its removal (floor exit, slot respawn).
    DungeonSlot,
}

impl MonsterLifecycle {
    /// Whether abandonment despawns this monster when nobody is inside its
    /// AOI, or leaves it for its own lifecycle to remove.
    pub fn despawns_when_unattended(self) -> bool {
        matches!(self, Self::Ambient)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Monster {
    pub id: String,
    pub monster_type: String,
    pub position: Position,
    pub rotation: f32,
    pub state: MonsterState,
    pub owner_id: Option<PlayerId>,
    pub health: u32,
    pub max_health: u32,
    /// 0 = overworld, 1..3 housing floors, negative = dungeon depth.
    #[serde(default)]
    pub floor_level: i8,
    /// Depth-scaled combat level for dungeon monsters; `None` uses the
    /// monster definition's level.
    // `skip_serializing_if` here dropped the field from rmp_serde's
    // positional array for every overworld monster, shifting `aggressive`
    // into this slot on the wire — the client then read the bool where it
    // expected this u8 and rejected the whole message.
    #[serde(default)]
    pub level_override: Option<u8>,
    /// Proactive (선공형) monster: attacks players on sight rather than only
    /// retaliating when hit. Drives behavior-tree selection on the agent-client.
    #[serde(default)]
    pub aggressive: bool,
    /// Server-only; never on the wire.
    #[serde(skip)]
    pub lifecycle: MonsterLifecycle,
    #[serde(skip)]
    pub last_attack_at: u64,
    /// Server timestamp (ms) the movement budget was last refilled. Paired with
    /// `move_budget` to rate-limit client-driven moves so an owned monster can't
    /// be teleported onto a distant victim.
    #[serde(skip)]
    pub last_move_at: u64,
    /// Remaining movement allowance (meters) in the monster's move token bucket,
    /// refilled at its run speed. A move costing more than this is refused.
    #[serde(skip)]
    pub move_budget: f32,
}

impl Monster {
    /// Gate for client-driven mutations (move/attack): alive and owned by the requester.
    pub fn is_controllable_by(&self, player_id: &PlayerId) -> bool {
        self.state != MonsterState::Dead && self.owner_id.as_ref() == Some(player_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// rmp_serde encodes structs as positional arrays, so every field must
    /// serialize unconditionally — a `skip_serializing_if` that fires shifts
    /// all later fields into the wrong slots. These round-trip the exact
    /// case that broke: an overworld monster with `level_override: None`
    /// followed by a populated `aggressive` flag.
    #[test]
    fn monster_roundtrips_with_none_level_override() {
        let monster = Monster {
            id: "m1".into(),
            monster_type: "slime".into(),
            position: Position {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            rotation: 0.5,
            state: MonsterState::Walk,
            owner_id: None,
            health: 10,
            max_health: 12,
            floor_level: 0,
            level_override: None,
            aggressive: true,
            lifecycle: MonsterLifecycle::Ambient,
            last_attack_at: 0,
            last_move_at: 0,
            move_budget: 0.0,
        };
        let bytes = rmp_serde::to_vec(&monster).unwrap();
        let decoded: Monster = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(decoded.level_override, None);
        assert!(decoded.aggressive);
    }

    /// `PlayerId` must reach the client as a bare integer, not as a nested
    /// one-element array (what a newtype encodes to without
    /// `#[serde(transparent)]`) and not as a string. The client decodes this
    /// through wasm and reports neither mistake as an error — a wrong shape
    /// just makes players silently fail to match.
    #[test]
    fn player_id_is_a_bare_integer_on_the_wire() {
        let player = Player {
            id: 42.into(),
            name: "jake".into(),
            position: Position {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            rotation: 0.0,
            level: 1,
            health: 5,
            max_health: 5,
            class: CharacterClass::Knight,
            gender: Gender::default(),
            is_official_npc: false,
            torch_on: false,
            floor_level: 0,
            object_type: None,
            main_hand: None,
            back: None,
            object_id: None,
            last_combat_at: 0,
            client_kind: ClientKind::default(),
        };
        // rmp_serde writes the struct as a positional array, so `id` is the
        // first element — and 42 fits msgpack's single-byte positive fixint.
        let bytes = rmp_serde::to_vec(&player).unwrap();
        assert_eq!(bytes[1], 42, "id must encode as a bare msgpack integer");

        // Standalone too, so bare id fields are covered.
        let id_bytes = rmp_serde::to_vec(&PlayerId::from(7)).unwrap();
        assert_eq!(id_bytes, vec![7]);
        assert_eq!(rmp_serde::from_slice::<u64>(&id_bytes).unwrap(), 7);
    }

    #[test]
    fn player_roundtrips_with_none_object_type() {
        let player = Player {
            id: 1.into(),
            name: "jake".into(),
            position: Position {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            rotation: 0.0,
            level: 3,
            health: 17,
            max_health: 17,
            class: CharacterClass::Knight,
            gender: Gender::default(),
            is_official_npc: false,
            torch_on: true,
            floor_level: 0,
            object_type: None,
            main_hand: None,
            back: None,
            object_id: None,
            last_combat_at: 0,
            client_kind: ClientKind::default(),
        };
        let bytes = rmp_serde::to_vec(&player).unwrap();
        let decoded: Player = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(decoded.object_type, None);
        assert!(decoded.torch_on);
    }
}
