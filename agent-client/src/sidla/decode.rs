//! Downlink: validated packets to the action envelope the driver already
//! speaks.
//!
//! Emitting the existing `{thought, actions}` JSON rather than driver types
//! keeps the protocol layer behind the backend seam — nothing downstream of
//! `LlmBackend` needs to know SIDLA exists.
//!
//! Only C (Engage) and D (Mission) carry decisions. A and B are telemetry
//! echoed back by the agent; the one exception is the subject reporting its
//! own death, which is a fact the engine must act on.

use serde_json::{json, Value};

use super::encode::{EntityKind, Uplink};
use super::packet::{Act, EntityId, Header, Obj, Packet};
use super::schema::Violation;
use super::shuffle;

/// How far a flee action puts between the agent and what it is fleeing.
const FLEE_DISTANCE: f32 = 15.0;
/// Radius of the waypoint a patrol or search objective walks to.
const PATROL_RADIUS: f32 = 12.0;

/// Translate a validated frame. `variation` seeds the deterministic choice of
/// heading for objectives that name no target; the same value always yields
/// the same heading.
pub fn to_envelope(
    packets: &[Packet],
    uplink: &Uplink,
    variation: u64,
) -> Result<Value, Violation> {
    let mut actions: Vec<Value> = Vec::new();
    let mut trace: Vec<String> = Vec::new();

    for packet in packets {
        let action = match packet.h {
            Header::A => self_death(packet, uplink),
            Header::B => None,
            Header::C => engage(packet, uplink)?,
            Header::D => mission(packet, uplink, variation)?,
        };
        if let Some(action) = action {
            trace.push(summarise(packet));
            actions.push(action);
        }
    }

    if actions.is_empty() {
        return Err(Violation::NoCommand);
    }

    Ok(json!({
        "thought": format!("SIDLA {}", trace.join("; ")),
        "actions": actions,
    }))
}

/// The subject reporting `STA = Dead` is the one A packet that commands
/// something. Another entity's death is just an observation.
fn self_death(packet: &Packet, uplink: &Uplink) -> Option<Value> {
    let is_subject = packet.sub.as_ref() == Some(&uplink.subject);
    let dead = packet.sta == Some(super::packet::Sta::Dead);
    (is_subject && dead).then(|| json!({"type": "respawn"}))
}

fn engage(packet: &Packet, uplink: &Uplink) -> Result<Option<Value>, Violation> {
    let act = packet.act.ok_or(Violation::MissingRequired {
        header: Header::C,
        field: super::packet::Field::Act,
    })?;
    let tar = packet.tar.as_ref().ok_or(Violation::MissingRequired {
        header: Header::C,
        field: super::packet::Field::Tar,
    })?;

    match act {
        Act::None => Ok(Some(json!({"type": "wait"}))),
        Act::Talk => match &packet.msg {
            Some(msg) => Ok(Some(json!({"type": "say", "message": msg}))),
            None => Ok(Some(json!({"type": "move", "target": tar.to_string()}))),
        },
        Act::Attack => match kind_of(tar, uplink) {
            Some(EntityKind::Monster(id)) => Ok(Some(json!({"type": "attack", "monster_id": id}))),
            _ => Err(unknown_target(tar, "attack")),
        },
        Act::Gift => match kind_of(tar, uplink) {
            Some(EntityKind::Player(name)) => {
                Ok(Some(json!({"type": "open_trade", "player": name})))
            }
            _ => Err(unknown_target(tar, "gift")),
        },
        Act::Flee => Ok(Some(flee_from(tar, uplink))),
    }
}

fn mission(packet: &Packet, uplink: &Uplink, variation: u64) -> Result<Option<Value>, Violation> {
    let obj = packet.obj.ok_or(Violation::MissingRequired {
        header: Header::D,
        field: super::packet::Field::Obj,
    })?;

    let action = match obj {
        Obj::None | Obj::Defend | Obj::Ambush => json!({"type": "wait"}),
        Obj::Patrol | Obj::Search => waypoint(uplink, variation),
        Obj::Escort => {
            let tar = packet.tar.as_ref().ok_or(Violation::MissingRequired {
                header: Header::D,
                field: super::packet::Field::Tar,
            })?;
            json!({"type": "move", "target": tar.to_string()})
        }
        Obj::Raid | Obj::Charge | Obj::Exterminate => match nearest_hostile(packet, uplink) {
            Some(id) => json!({"type": "attack", "monster_id": id}),
            None => json!({"type": "wait"}),
        },
    };
    Ok(Some(action))
}

/// A named target takes precedence; without one, the nearest hostile monster
/// in the uplink is the objective's subject.
fn nearest_hostile(packet: &Packet, uplink: &Uplink) -> Option<String> {
    if let Some(tar) = packet.tar.as_ref() {
        if let Some(EntityKind::Monster(id)) = kind_of(tar, uplink) {
            return Some(id);
        }
    }
    uplink.hostiles().find_map(|t| match &t.kind {
        EntityKind::Monster(id) => Some(id.clone()),
        _ => None,
    })
}

fn kind_of(id: &EntityId, uplink: &Uplink) -> Option<EntityKind> {
    uplink.find(id).map(|t| t.kind.clone())
}

fn unknown_target(id: &EntityId, verb: &str) -> Violation {
    Violation::Malformed(format!(
        "cannot {verb} `{id}`: not a tracked target of that kind"
    ))
}

/// Walk directly away from the target. Without positions for both there is
/// nothing to compute a heading from, so the agent holds instead of guessing.
fn flee_from(tar: &EntityId, uplink: &Uplink) -> Value {
    let (Some(from), Some(track)) = (uplink.subject_position.as_ref(), uplink.find(tar)) else {
        return json!({"type": "wait"});
    };
    let (dx, dz) = (from.x - track.position.x, from.z - track.position.z);
    let len = (dx * dx + dz * dz).sqrt();
    if len < f32::EPSILON {
        return json!({"type": "wait"});
    }
    json!({
        "type": "move",
        "x": round1(from.x + dx / len * FLEE_DISTANCE),
        "z": round1(from.z + dz / len * FLEE_DISTANCE),
    })
}

/// A patrol waypoint on a circle around the current position. The heading
/// comes from `variation`, so a fixed seed patrols the same spot every turn
/// and a varying one sweeps the area.
fn waypoint(uplink: &Uplink, variation: u64) -> Value {
    let Some(from) = uplink.subject_position.as_ref() else {
        return json!({"type": "wait"});
    };
    let angle = shuffle::unit_fraction(variation) * std::f32::consts::TAU;
    json!({
        "type": "move",
        "x": round1(from.x + angle.cos() * PATROL_RADIUS),
        "z": round1(from.z + angle.sin() * PATROL_RADIUS),
    })
}

fn round1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

fn summarise(packet: &Packet) -> String {
    let mut parts = vec![packet.h.as_str().to_string()];
    if let Some(act) = packet.act {
        parts.push(format!("ACT={}", act.code()));
    }
    if let Some(obj) = packet.obj {
        parts.push(format!("OBJ={}", obj.code()));
    }
    if let Some(sta) = packet.sta {
        parts.push(format!("STA={}", sta.code()));
    }
    if let Some(tar) = packet.tar.as_ref() {
        parts.push(format!("TAR={tar}"));
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidla::encode::Track;
    use crate::sidla::packet::{Iff, Loc, Sta};
    use onlinerpg_shared::Position;

    fn pos(x: f32, z: f32) -> Position {
        Position { x, y: 0.0, z }
    }

    fn track(id: &str, kind: EntityKind, x: f32, z: f32, iff: Iff) -> Track {
        Track {
            id: EntityId::name(id),
            kind,
            position: pos(x, z),
            sta: Sta::Idle,
            iff,
            rel: 0,
            hp_pct: 100,
        }
    }

    fn uplink() -> Uplink {
        Uplink {
            subject: EntityId::name("Mika"),
            subject_position: Some(pos(0.0, 0.0)),
            subject_sta: Sta::Idle,
            subject_hp_pct: 100,
            tracks: vec![
                track(
                    "Saori",
                    EntityKind::Player("Saori".into()),
                    3.0,
                    0.0,
                    Iff::Neutral,
                ),
                track(
                    "slime_1",
                    EntityKind::Monster("slime_1".into()),
                    0.0,
                    4.0,
                    Iff::Hostile,
                ),
            ],
            packets: Vec::new(),
        }
    }

    fn first_action(packets: &[Packet]) -> Value {
        let envelope = to_envelope(packets, &uplink(), 0).unwrap();
        envelope["actions"][0].clone()
    }

    #[test]
    fn talk_with_a_line_becomes_speech() {
        let packet = Packet::engage(EntityId::name("Mika"), EntityId::name("Saori"), Act::Talk)
            .with_msg("Long time, Saori.");
        assert_eq!(
            first_action(&[packet]),
            json!({"type": "say", "message": "Long time, Saori."})
        );
    }

    #[test]
    fn talk_without_a_line_approaches_the_target() {
        let packet = Packet::engage(EntityId::name("Mika"), EntityId::name("Saori"), Act::Talk);
        assert_eq!(
            first_action(&[packet]),
            json!({"type": "move", "target": "Saori"})
        );
    }

    #[test]
    fn attack_resolves_the_target_to_a_monster_id() {
        let packet = Packet::engage(
            EntityId::name("Mika"),
            EntityId::name("slime_1"),
            Act::Attack,
        );
        assert_eq!(
            first_action(&[packet]),
            json!({"type": "attack", "monster_id": "slime_1"})
        );
    }

    #[test]
    fn attacking_something_that_is_not_a_monster_is_refused() {
        for target in ["Saori", "a_monster_that_does_not_exist"] {
            let packet =
                Packet::engage(EntityId::name("Mika"), EntityId::name(target), Act::Attack);
            assert!(to_envelope(&[packet], &uplink(), 0).is_err(), "{target}");
        }
    }

    #[test]
    fn a_gift_opens_a_trade_with_a_player() {
        let packet = Packet::engage(EntityId::name("Mika"), EntityId::name("Saori"), Act::Gift);
        assert_eq!(
            first_action(&[packet]),
            json!({"type": "open_trade", "player": "Saori"})
        );
    }

    #[test]
    fn fleeing_moves_directly_away_from_the_target() {
        let packet = Packet::engage(EntityId::name("Mika"), EntityId::name("slime_1"), Act::Flee);
        let action = first_action(&[packet]);
        assert_eq!(action["type"], "move");
        assert_eq!(action["x"].as_f64().unwrap(), 0.0);
        assert_eq!(action["z"].as_f64().unwrap(), -(FLEE_DISTANCE as f64));
    }

    #[test]
    fn an_aggressive_objective_picks_the_nearest_hostile() {
        let packet = Packet::mission(EntityId::name("Mika"), Obj::Exterminate);
        assert_eq!(
            first_action(&[packet]),
            json!({"type": "attack", "monster_id": "slime_1"})
        );
    }

    #[test]
    fn a_holding_objective_waits() {
        for obj in [Obj::None, Obj::Defend, Obj::Ambush] {
            let packet = Packet::mission(EntityId::name("Mika"), obj);
            assert_eq!(first_action(&[packet]), json!({"type": "wait"}), "{obj:?}");
        }
    }

    #[test]
    fn a_patrol_waypoint_is_a_fixed_distance_out_and_repeatable() {
        let packet = Packet::mission(EntityId::name("Mika"), Obj::Patrol);
        let first = first_action(std::slice::from_ref(&packet));
        assert_eq!(first_action(&[packet]), first);
        let (x, z) = (
            first["x"].as_f64().unwrap() as f32,
            first["z"].as_f64().unwrap() as f32,
        );
        let d = (x * x + z * z).sqrt();
        assert!(
            (d - PATROL_RADIUS).abs() < 0.2,
            "waypoint {x},{z} is {d} out"
        );
    }

    #[test]
    fn an_escort_objective_without_a_target_is_refused() {
        let packet = Packet::mission(EntityId::name("Mika"), Obj::Escort);
        assert!(to_envelope(&[packet], &uplink(), 0).is_err());
    }

    #[test]
    fn the_subjects_own_death_report_triggers_a_respawn() {
        let packet = Packet::ppli(
            EntityId::name("Mika"),
            Sta::Dead,
            Loc::Coord([0.0, 0.0, 0.0]),
        );
        assert_eq!(first_action(&[packet]), json!({"type": "respawn"}));
    }

    #[test]
    fn another_entitys_death_report_commands_nothing() {
        let packets = [Packet::ppli(
            EntityId::name("slime_1"),
            Sta::Dead,
            Loc::Coord([0.0, 0.0, 4.0]),
        )];
        assert_eq!(
            to_envelope(&packets, &uplink(), 0),
            Err(Violation::NoCommand)
        );
    }

    #[test]
    fn a_frame_of_pure_observation_commands_nothing() {
        let packets = [Packet::track(
            EntityId::name("Mika"),
            EntityId::name("slime_1"),
            Iff::Hostile,
        )];
        assert_eq!(
            to_envelope(&packets, &uplink(), 0),
            Err(Violation::NoCommand)
        );
    }

    #[test]
    fn packets_decode_in_order_into_one_envelope() {
        let packets = [
            Packet::track(
                EntityId::name("Mika"),
                EntityId::name("slime_1"),
                Iff::Hostile,
            ),
            Packet::engage(EntityId::name("Mika"), EntityId::name("Saori"), Act::Talk)
                .with_msg("Behind you."),
            Packet::engage(
                EntityId::name("Mika"),
                EntityId::name("slime_1"),
                Act::Attack,
            ),
        ];
        let envelope = to_envelope(&packets, &uplink(), 0).unwrap();
        let actions = envelope["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0]["type"], "say");
        assert_eq!(actions[1]["type"], "attack");
        assert!(envelope["thought"].as_str().unwrap().contains("ACT=2"));
    }
}
