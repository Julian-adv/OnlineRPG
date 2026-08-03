//! Shared types and protocol between the Rust server, agent-client, and
//! the WASM web client. Definitions are split by concern across small
//! sibling modules and re-exported here so external callers can keep
//! using flat `onlinerpg_shared::Position` paths regardless of where the
//! type now lives.

pub mod character;
pub mod combat;
pub mod dungeon;
pub mod entity;
pub mod fishing;
pub mod furniture;
pub mod housing;
pub mod inventory;
pub mod messages;
pub mod monster_ai;
pub mod pathfinding;
pub mod skills;
pub mod tree_format;
pub mod world;
pub mod worldgen;
pub mod xp;

/// Repo-root-relative path of the NPC auth token file: written by the server
/// on first run, read by agent-client (whose cwd is one level down).
pub const NPC_TOKEN_PATH_FROM_ROOT: &str = "data/npc_token";

/// Wire protocol version, sent in `ClientMessage::ClientInfo` and checked for
/// exact equality by the server. Bump it whenever a message shape or its
/// meaning changes: clients we cannot redeploy (agent-clients on other
/// machines) must be refused with an "update me" notice rather than left to
/// fail at a random later message. See `doc/REMOTE_AGENT_CLIENT.md`.
/// v6: attack-rejection ack.
/// v7: trained skills (SkillsUpdate/SkillXpGained) + fishing
///     (cast/bite/struggle messages) — see `doc/FISHING.md`.
/// v8: fishing struggle rounds replaced by continuous FishingFight beats.
/// v9: `Player.main_hand` + PlayerMainHandChanged equipment broadcast.
/// v10: party (PartyInvite/PartyRespond/PartyLeave, PartyState snapshots).
/// v11: party positions poll (RequestPartyPositions → PartyPositions) for
///      world-map member markers.
/// v12: equip slots hands/back/shirt (doc/ITEM_TIERS.md 선행 작업 #1).
/// v13: party summoning scroll (PartySummonReceived → PartySummonRespond).
/// v14: one-handed sword skill metadata/progression + player attack cooldown.
/// v15: dagger skill metadata/progression using the shared weapon-skill model.
/// v16: spear skill identity plus its authoritative range/cadence profile.
/// v17: Shield skill identity plus effective-Guard updates.
/// v18: Healing skill identity plus server-authoritative treatment training
/// and potency.
/// v19: Leather Armor skill identity carried by the generic skill messages.
/// v20: typed physical damage and authoritative mitigation breakdowns.
/// v21: authoritative equipped-load burden tier and movement speed.
pub const PROTOCOL_VERSION: u32 = 21;

/// WebSocket close code sent when the handshake is refused (wrong protocol
/// version, or traffic before `ClientInfo`). Lives outside the serialized
/// message shape on purpose: a client too old to understand a new message can
/// still read a close code, so this is the one signal that reaches a stale
/// build and tells it to stop reconnecting. Never renumber it.
pub const CLOSE_CODE_PROTOCOL_MISMATCH: u16 = 4001;

/// WebSocket close code sent when a source IP opens sessions faster than
/// `conn_limit` allows. Unlike a protocol refusal this is transient, so
/// clients should keep retrying on their normal backoff.
pub const CLOSE_CODE_RATE_LIMITED: u16 = 4002;

/// WebSocket close code sent when the server drops a connection for going
/// quiet — no login inside the unauth grace period, or no heartbeat in game.
/// Transient like a rate limit, so clients retry on their normal backoff; the
/// point is that they learn the server closed them and why, instead of reading
/// a bare TCP FIN and having to guess whether the path died.
pub const CLOSE_CODE_IDLE_TIMEOUT: u16 = 4003;

#[cfg(target_arch = "wasm32")]
mod wasm_api;

pub use character::{Character, CharacterAttributes, CharacterClass, Gender};
pub use combat::{PhysicalDamageResult, PhysicalDamageType};
pub use entity::{Monster, MonsterState, Player, PlayerId};
pub use inventory::{EquipmentBurden, EquipmentBurdenTier};
pub use messages::{
    deserialize_client_msg, deserialize_server_msg, serialize_client_msg, serialize_server_msg,
    ActiveDeal, AttackRejectReason, ClientMessage, DealKind, ServerMessage,
};
pub use world::{
    shortest_world_delta_x, wrap_world_x, GameDateTime, NoSpawnZone, Position,
    EVENT_DELIVERY_RADIUS, NPC_SIGHT_RADIUS, PLAYER_MOVE_SPEED, WORLD_MAX_X, WORLD_MIN_X,
    WORLD_WIDTH_X,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn roundtrip_client_message() {
        let msg = ClientMessage::PlayerMove {
            position: Position {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            rotation: 1.5,
            floor_level: 1,
            append: false,
        };
        let bytes = serialize_client_msg(&msg).unwrap();
        let decoded = deserialize_client_msg(&bytes).unwrap();
        match decoded {
            ClientMessage::PlayerMove {
                position,
                rotation,
                floor_level,
                ..
            } => {
                assert_eq!(position.x, 1.0);
                assert_eq!(position.y, 2.0);
                assert_eq!(position.z, 3.0);
                assert_eq!(rotation, 1.5);
                assert_eq!(floor_level, 1);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn roundtrip_unit_variant() {
        let msg = ClientMessage::RequestRespawn;
        let bytes = serialize_client_msg(&msg).unwrap();
        let decoded = deserialize_client_msg(&bytes).unwrap();
        assert!(matches!(decoded, ClientMessage::RequestRespawn));
    }

    #[test]
    fn roundtrip_server_message_with_hashmap() {
        let players = vec![Player {
            id: 1.into(),
            name: "Test".to_string(),
            position: Position {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            rotation: 0.0,
            level: 1,
            health: 10,
            max_health: 10,
            class: CharacterClass::Knight,
            gender: Gender::default(),
            is_official_npc: false,
            torch_on: false,
            floor_level: 0,
            object_type: None,
            main_hand: None,
            object_id: None,
            last_combat_at: 0,
            client_kind: Default::default(),
        }];
        // A monster with every Option None guards the wire format itself:
        // rmp_serde encodes structs as positional arrays, so any field that
        // fails to serialize (e.g. a future `skip_serializing_if`) shifts
        // the later fields and breaks this real serialize/deserialize path.
        let mut monsters = HashMap::new();
        monsters.insert(
            "m1".to_string(),
            Monster {
                id: "m1".to_string(),
                monster_type: "slime".to_string(),
                position: Position {
                    x: 1.0,
                    y: 0.0,
                    z: 1.0,
                },
                rotation: 0.0,
                state: MonsterState::Idle,
                owner_id: None,
                health: 8,
                max_health: 8,
                floor_level: 0,
                level_override: None,
                aggressive: true,
                last_attack_at: 0,
                last_move_at: 0,
                move_budget: 0.0,
            },
        );
        let msg = ServerMessage::GameState {
            players,
            monsters,
            ground_items: Vec::new(),
        };
        let bytes = serialize_server_msg(&msg).unwrap();
        let decoded = deserialize_server_msg(&bytes).unwrap();
        match decoded {
            ServerMessage::GameState {
                players, monsters, ..
            } => {
                assert_eq!(players.len(), 1);
                assert_eq!(players[0].id, PlayerId::from(1));
                let m = &monsters["m1"];
                assert_eq!(m.level_override, None);
                assert!(m.aggressive);
            }
            _ => panic!("Wrong variant"),
        }
    }

    const ALL_CLASSES: &[CharacterClass] = &[
        CharacterClass::Knight,
        CharacterClass::Barbarian,
        CharacterClass::Caveman,
        CharacterClass::Valkyrie,
        CharacterClass::Ranger,
        CharacterClass::Samurai,
        CharacterClass::Monk,
        CharacterClass::Priest,
        CharacterClass::Archaeologist,
        CharacterClass::Healer,
        CharacterClass::Rogue,
        CharacterClass::Wizard,
        CharacterClass::Tourist,
        CharacterClass::Merchant,
        CharacterClass::Guard,
    ];

    #[test]
    fn character_class_str_roundtrip() {
        for class in ALL_CLASSES {
            let s = class.as_str();
            let back: CharacterClass = s.parse().expect("parse should accept as_str output");
            assert_eq!(&back, class, "roundtrip failed for {s}");
        }
    }

    #[test]
    fn character_class_from_str_rejects_unknown() {
        assert!("".parse::<CharacterClass>().is_err());
        assert!("nonexistent".parse::<CharacterClass>().is_err());
        assert!("Knight".parse::<CharacterClass>().is_err());
    }

    #[test]
    fn only_operator_classes_are_unselectable() {
        for class in ALL_CLASSES {
            let expected = !matches!(class, CharacterClass::Merchant | CharacterClass::Guard);
            assert_eq!(
                class.is_player_selectable(),
                expected,
                "{} selectability",
                class.as_str()
            );
        }
    }

    #[test]
    fn character_class_hit_die_is_valid_polyhedron() {
        for class in ALL_CLASSES {
            let d = class.hit_die();
            assert!(
                matches!(d, 4 | 6 | 8 | 10 | 12 | 20),
                "class {:?} has non-standard hit die d{}",
                class,
                d
            );
        }
    }

    #[test]
    fn stat_adjustments_sum_to_zero() {
        // Balanced classes: the six adjustments must net to zero so no class
        // gains or loses total stat points before the 72-rebalance step.
        for class in ALL_CLASSES {
            for gender in [Gender::Male, Gender::Female] {
                let adj = class.stat_adjustments(gender);
                let sum: i32 = adj.iter().map(|v| *v as i32).sum();
                assert_eq!(
                    sum, 0,
                    "class {:?} gender {:?} adjustments sum to {} (expected 0): {:?}",
                    class, gender, sum, adj
                );
            }
        }
    }

    #[test]
    fn stat_adjustments_gender_variants_differ_only_where_expected() {
        // Knight, Barbarian, and Caveman have gendered splits; all others ignore gender.
        let gendered = [
            CharacterClass::Knight,
            CharacterClass::Barbarian,
            CharacterClass::Caveman,
        ];
        for class in ALL_CLASSES {
            let male = class.stat_adjustments(Gender::Male);
            let female = class.stat_adjustments(Gender::Female);
            if gendered.contains(class) {
                assert_ne!(
                    male, female,
                    "class {:?} expected gendered adjustments",
                    class
                );
            } else {
                assert_eq!(
                    male, female,
                    "class {:?} should have identical adjustments for both genders",
                    class
                );
            }
        }
    }

    #[test]
    fn no_spawn_zone_contains_inside_and_boundary() {
        let zone = NoSpawnZone {
            min_x: -10.0,
            min_z: -5.0,
            max_x: 10.0,
            max_z: 5.0,
        };
        assert!(zone.contains(0.0, 0.0));
        assert!(zone.contains(-10.0, -5.0));
        assert!(zone.contains(10.0, 5.0));
        assert!(zone.contains(-10.0, 5.0));
        assert!(zone.contains(10.0, -5.0));
        assert!(!zone.contains(-10.1, 0.0));
        assert!(!zone.contains(10.1, 0.0));
        assert!(!zone.contains(0.0, -5.1));
        assert!(!zone.contains(0.0, 5.1));
    }

    #[test]
    fn no_spawn_zone_degenerate_point_zone() {
        // A zero-area zone still matches its single point.
        let zone = NoSpawnZone {
            min_x: 3.0,
            min_z: 7.0,
            max_x: 3.0,
            max_z: 7.0,
        };
        assert!(zone.contains(3.0, 7.0));
        assert!(!zone.contains(3.0001, 7.0));
    }

    #[test]
    fn roundtrip_all_server_messages() {
        let mut skills = skills::Skills::default();
        skills.add_xp(skills::SkillId::Fishing, 100).unwrap();
        skills.add_xp(skills::SkillId::OneHandedSword, 500).unwrap();
        skills.add_xp(skills::SkillId::Dagger, 10).unwrap();
        skills.add_xp(skills::SkillId::Spear, 20).unwrap();
        skills.add_xp(skills::SkillId::Shield, 30).unwrap();
        skills.add_xp(skills::SkillId::Healing, 40).unwrap();
        skills.add_xp(skills::SkillId::LeatherArmor, 50).unwrap();
        let messages = vec![
            ServerMessage::AuthError {
                message: "bad".to_string(),
            },
            ServerMessage::PlayerLeft {
                player_id: 1.into(),
            },
            ServerMessage::PlayerDisappeared {
                player_id: 1.into(),
            },
            ServerMessage::MonsterDead {
                monster_id: "m1".to_string(),
                dropped_weapon_item_def_id: Some("goblin_sword".to_string()),
            },
            ServerMessage::PlayerAttacked {
                player_id: 1.into(),
                monster_id: "m1".to_string(),
                hit: true,
                roll: 18,
                damage_type: PhysicalDamageType::Slash,
                damage: 5,
            },
            ServerMessage::MonsterAttackedPlayer {
                monster_id: "m1".to_string(),
                player_id: 1.into(),
                hit: true,
                roll: 17,
                damage_type: PhysicalDamageType::Blunt,
                raw_damage: 6,
                mitigated_damage: 2,
                damage: 4,
                current_health: 6,
            },
            ServerMessage::MonsterProvoked {
                player_id: 1.into(),
                monster_id: "m1".to_string(),
            },
            ServerMessage::PlayerAttackRejected {
                monster_id: "m1".to_string(),
                reason: AttackRejectReason::Cooldown,
            },
            ServerMessage::SkillsUpdate { skills },
            ServerMessage::SkillXpGained {
                skill: skills::SkillId::Spear,
                xp_amount: 10,
                total_xp: 20,
                new_level: 0,
                leveled_up: false,
            },
            ServerMessage::SkillXpGained {
                skill: skills::SkillId::Shield,
                xp_amount: 10,
                total_xp: 40,
                new_level: 0,
                leveled_up: false,
            },
            ServerMessage::SkillXpGained {
                skill: skills::SkillId::Healing,
                xp_amount: 6,
                total_xp: 40,
                new_level: 0,
                leveled_up: false,
            },
            ServerMessage::SkillXpGained {
                skill: skills::SkillId::LeatherArmor,
                xp_amount: 5,
                total_xp: 50,
                new_level: 0,
                leveled_up: false,
            },
            ServerMessage::EquipmentBurdenUpdated {
                burden: inventory::resolve_equipment_burden(43.0, 150.0),
            },
            ServerMessage::Kicked {
                player_id: 1.into(),
                reason: "test".to_string(),
            },
            ServerMessage::ServerNotice {
                message: Some("restart soon".to_string()),
            },
            ServerMessage::ServerNotice { message: None },
        ];
        for msg in messages {
            let bytes = serialize_server_msg(&msg).unwrap();
            let decoded = deserialize_server_msg(&bytes).unwrap();
            // Just verify it roundtrips without error
            assert!(!format!("{:?}", decoded).is_empty());
        }
    }
}
