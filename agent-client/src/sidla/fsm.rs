//! The fallback the protocol falls back to.
//!
//! A discarded packet must not leave the agent doing nothing — that is how a
//! rejected turn becomes a frozen NPC. This module answers the same uplink
//! with a valid packet derived from the world alone, so refusing a bad reply
//! costs behaviour rather than removing it.
//!
//! The policy is a short ordered ladder with no randomness: the first rule
//! that matches decides, ties break on identifier, and the result is a packet
//! that passes `super::schema::validate` by construction.

use super::encode::{Track, Uplink};
use super::packet::{Act, Iff, Loc, Obj, Packet, Sta};

/// Below this fraction of health the agent disengages instead of trading hits.
pub const FLEE_HEALTH_PCT: u8 = 25;
/// A hostile nearer than this is worth engaging without being told to.
pub const ENGAGE_RADIUS: f32 = 20.0;

/// Decide what to do from the world alone.
///
/// Ladder, in order: report death, run from a hostile while badly hurt, engage
/// the nearest hostile in reach, then patrol.
pub fn decide(uplink: &Uplink) -> Packet {
    let subject = uplink.subject.clone();

    if uplink.subject_sta == Sta::Dead {
        let loc = uplink
            .subject_position
            .as_ref()
            .map(|p| Loc::Coord([p.x, p.y, p.z]))
            .unwrap_or_else(|| Loc::Zone("unknown".into()));
        return Packet::ppli(subject, Sta::Dead, loc).with_hp(0);
    }

    if let Some(threat) = nearest_hostile_in_reach(uplink) {
        if uplink.subject_hp_pct <= FLEE_HEALTH_PCT {
            return Packet::engage(subject, threat.id.clone(), Act::Flee);
        }
        return Packet::engage(subject, threat.id.clone(), Act::Attack);
    }

    Packet::mission(subject, Obj::Patrol)
}

/// The nearest hostile within `ENGAGE_RADIUS`. `Uplink::tracks` is already
/// sorted nearest-first with identifier tie-breaks, so the first match is the
/// deterministic choice.
fn nearest_hostile_in_reach(uplink: &Uplink) -> Option<&Track> {
    let from = uplink.subject_position.as_ref()?;
    uplink.tracks.iter().find(|t| {
        t.iff == Iff::Hostile
            && t.sta != Sta::Dead
            && t.position.dist_xz_sq(from) <= ENGAGE_RADIUS * ENGAGE_RADIUS
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidla::encode::EntityKind;
    use crate::sidla::packet::{EntityId, Header};
    use crate::sidla::schema;
    use onlinerpg_shared::Position;

    fn pos(x: f32, z: f32) -> Position {
        Position { x, y: 0.0, z }
    }

    fn track(id: &str, x: f32, z: f32, iff: Iff) -> Track {
        Track {
            id: EntityId::name(id),
            kind: EntityKind::Monster(id.to_string()),
            position: pos(x, z),
            sta: Sta::Idle,
            iff,
            rel: 0,
            hp_pct: 100,
        }
    }

    fn uplink(sta: Sta, hp_pct: u8, tracks: Vec<Track>) -> Uplink {
        Uplink {
            subject: EntityId::name("Mika"),
            subject_position: Some(pos(0.0, 0.0)),
            subject_sta: sta,
            subject_hp_pct: hp_pct,
            tracks,
            packets: Vec::new(),
        }
    }

    #[test]
    fn every_decision_is_a_schema_valid_packet() {
        let cases = [
            uplink(Sta::Dead, 0, vec![]),
            uplink(Sta::Idle, 100, vec![]),
            uplink(
                Sta::Idle,
                100,
                vec![track("slime_1", 0.0, 5.0, Iff::Hostile)],
            ),
            uplink(
                Sta::Panic,
                10,
                vec![track("slime_1", 0.0, 5.0, Iff::Hostile)],
            ),
            uplink(
                Sta::Idle,
                100,
                vec![track("townsfolk", 1.0, 0.0, Iff::Friend)],
            ),
        ];
        for case in &cases {
            let packet = decide(case);
            schema::validate(&packet)
                .unwrap_or_else(|e| panic!("fallback emitted an invalid packet: {e}"));
        }
    }

    #[test]
    fn death_is_reported_before_anything_else_is_considered() {
        let packet = decide(&uplink(
            Sta::Dead,
            0,
            vec![track("slime_1", 0.0, 1.0, Iff::Hostile)],
        ));
        assert_eq!(packet.h, Header::A);
        assert_eq!(packet.sta, Some(Sta::Dead));
    }

    /// The uplink hands over tracks already sorted nearest-first, so the
    /// ladder takes the first hostile it finds rather than re-measuring.
    #[test]
    fn a_healthy_agent_engages_the_first_hostile_in_the_frame() {
        let packet = decide(&uplink(
            Sta::Idle,
            100,
            vec![
                track("slime_near", 0.0, 3.0, Iff::Hostile),
                track("slime_far", 0.0, 15.0, Iff::Hostile),
            ],
        ));
        assert_eq!(packet.h, Header::C);
        assert_eq!(packet.act, Some(Act::Attack));
        assert_eq!(packet.tar, Some(EntityId::name("slime_near")));
    }

    #[test]
    fn a_badly_hurt_agent_flees_the_same_target_it_would_have_fought() {
        let tracks = vec![track("slime_1", 0.0, 3.0, Iff::Hostile)];
        let fought = decide(&uplink(Sta::Idle, 100, tracks.clone()));
        let fled = decide(&uplink(Sta::Panic, FLEE_HEALTH_PCT, tracks));
        assert_eq!(fought.act, Some(Act::Attack));
        assert_eq!(fled.act, Some(Act::Flee));
        assert_eq!(fought.tar, fled.tar);
    }

    #[test]
    fn a_hostile_out_of_reach_is_not_engaged() {
        let packet = decide(&uplink(
            Sta::Idle,
            100,
            vec![track("slime_1", 0.0, ENGAGE_RADIUS + 5.0, Iff::Hostile)],
        ));
        assert_eq!(packet.h, Header::D);
        assert_eq!(packet.obj, Some(Obj::Patrol));
    }

    #[test]
    fn a_dead_hostile_is_not_engaged() {
        let mut t = track("slime_1", 0.0, 3.0, Iff::Hostile);
        t.sta = Sta::Dead;
        let packet = decide(&uplink(Sta::Idle, 100, vec![t]));
        assert_eq!(packet.obj, Some(Obj::Patrol));
    }

    #[test]
    fn a_friend_is_never_a_target() {
        let packet = decide(&uplink(
            Sta::Idle,
            100,
            vec![
                track("guard", 1.0, 0.0, Iff::Friend),
                track("merchant", 2.0, 0.0, Iff::Neutral),
            ],
        ));
        assert_eq!(packet.h, Header::D);
    }

    #[test]
    fn an_empty_world_yields_a_patrol() {
        let packet = decide(&uplink(Sta::Idle, 100, vec![]));
        assert_eq!(packet.h, Header::D);
        assert_eq!(packet.obj, Some(Obj::Patrol));
    }

    #[test]
    fn the_same_world_always_yields_the_same_decision() {
        let case = uplink(
            Sta::Idle,
            60,
            vec![
                track("slime_2", 0.0, 4.0, Iff::Hostile),
                track("slime_1", 0.0, 4.0, Iff::Hostile),
            ],
        );
        let first = decide(&case);
        for _ in 0..64 {
            assert_eq!(decide(&case), first);
        }
    }
}
