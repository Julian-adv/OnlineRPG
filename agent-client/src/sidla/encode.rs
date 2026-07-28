//! Uplink: `SharedState` to SIDLA packets, with no natural-language step.
//!
//! Every entity in sight contributes two packets — an A (PPLI) carrying where
//! it is and how it is doing, and a B (Track) carrying how we identify it.
//! Splitting them that way is what keeps each header's field set disjoint:
//! position belongs to the entity reporting it, identification belongs to the
//! observer.
//!
//! The frame is a pure function of the state snapshot. Entities are emitted in
//! a stable order (distance, then id) so the same world produces byte-identical
//! bytes, which is what makes a turn reproducible in the first place.

use onlinerpg_shared::{Monster, MonsterState, Player, Position};

use super::packet::{EntityId, Iff, Loc, Packet, Sta};
use crate::state::{SharedState, NPC_SIGHT_RADIUS};

/// Below this fraction of maximum health an entity reports `Panic` rather
/// than its activity, so the frame carries the fact that matters most.
pub const PANIC_HEALTH_FRACTION: f32 = 0.3;

/// What an entity is, so a decoded target can be turned back into the right
/// kind of game command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityKind {
    Monster(String),
    Player(String),
}

/// One entity as the uplink saw it. Retained beside the packets so the
/// downlink can resolve a target without a second pass over the world.
#[derive(Debug, Clone)]
pub struct Track {
    pub id: EntityId,
    pub kind: EntityKind,
    pub position: Position,
    pub sta: Sta,
    pub iff: Iff,
    pub rel: i32,
    pub hp_pct: u8,
}

/// A complete uplink: the packets to send, plus the index needed to read the
/// reply against the same snapshot.
#[derive(Debug, Clone)]
pub struct Uplink {
    pub subject: EntityId,
    pub subject_position: Option<Position>,
    pub subject_sta: Sta,
    pub subject_hp_pct: u8,
    pub tracks: Vec<Track>,
    pub packets: Vec<Packet>,
}

impl Uplink {
    pub fn find(&self, id: &EntityId) -> Option<&Track> {
        self.tracks.iter().find(|t| &t.id == id)
    }

    /// Tracks identified as hostile, nearest first. Ordering is inherited
    /// from `tracks`, which is already sorted deterministically.
    pub fn hostiles(&self) -> impl Iterator<Item = &Track> {
        self.tracks.iter().filter(|t| t.iff == Iff::Hostile)
    }
}

fn hp_pct(health: u32, max_health: u32) -> u8 {
    if max_health == 0 {
        return 0;
    }
    let pct = (health as f32 / max_health as f32 * 100.0).round();
    pct.clamp(0.0, 100.0) as u8
}

fn coord(p: &Position) -> Loc {
    Loc::Coord([
        (p.x * 10.0).round() / 10.0,
        (p.y * 10.0).round() / 10.0,
        (p.z * 10.0).round() / 10.0,
    ])
}

/// A monster's state maps onto the dictionary's activity codes; a badly hurt
/// one reports `Panic` regardless of what it is doing.
fn monster_sta(m: &Monster) -> Sta {
    if m.health == 0 || m.state == MonsterState::Dead {
        return Sta::Dead;
    }
    if (m.health as f32) < m.max_health as f32 * PANIC_HEALTH_FRACTION {
        return Sta::Panic;
    }
    match m.state {
        MonsterState::Idle => Sta::Idle,
        MonsterState::Walk | MonsterState::Run => Sta::Moving,
        MonsterState::Attack | MonsterState::Hit => Sta::Engaged,
        MonsterState::Dead => Sta::Dead,
    }
}

fn player_sta(p: &Player) -> Sta {
    if p.health == 0 {
        Sta::Dead
    } else if (p.health as f32) < p.max_health as f32 * PANIC_HEALTH_FRACTION {
        Sta::Panic
    } else {
        Sta::Idle
    }
}

/// A monster that attacks on sight is hostile; one that only retaliates is
/// neutral until provoked.
fn monster_iff(m: &Monster) -> Iff {
    if m.aggressive {
        Iff::Hostile
    } else {
        Iff::Neutral
    }
}

fn player_iff(p: &Player) -> Iff {
    if p.is_official_npc {
        Iff::Friend
    } else {
        Iff::Unknown
    }
}

/// Affinity derived from identification. A placeholder: the game has no
/// relationship store yet, so REL is a deterministic function of IFF rather
/// than a remembered score. Swap this for a lookup when one exists — nothing
/// else in the protocol needs to change.
fn rel_from_iff(iff: Iff) -> i32 {
    match iff {
        Iff::Friend => 50,
        Iff::Hostile => -100,
        Iff::Neutral | Iff::Unknown => 0,
    }
}

fn subject_sta(state: &SharedState, hostile_adjacent: bool) -> Sta {
    let Some(p) = state.self_player.as_ref() else {
        return Sta::Idle;
    };
    if p.health == 0 {
        return Sta::Dead;
    }
    if (p.health as f32) < p.max_health as f32 * PANIC_HEALTH_FRACTION {
        return Sta::Panic;
    }
    if state.trade_busy || state.self_fishing || hostile_adjacent {
        return Sta::Engaged;
    }
    Sta::Idle
}

/// Distance at which a hostile counts as adjacent for the subject's own
/// state report.
const ENGAGED_RADIUS: f32 = 5.0;

/// Encode the current world into an uplink frame.
pub fn encode(state: &SharedState) -> Uplink {
    let subject = state
        .self_player
        .as_ref()
        .map(|p| EntityId::name(&p.name))
        .unwrap_or_else(|| EntityId::name("self"));
    let subject_position = state.self_player.as_ref().map(|p| p.position);
    let sight_sq = NPC_SIGHT_RADIUS * NPC_SIGHT_RADIUS;

    let mut scored: Vec<(f32, Track)> = Vec::new();

    for p in state.nearby_players.values() {
        if state.self_player_id.as_ref() == Some(&p.id) {
            continue;
        }
        let d_sq = match subject_position.as_ref() {
            Some(sp) => {
                let d = p.position.dist_xz_sq(sp);
                if d > sight_sq {
                    continue;
                }
                d
            }
            None => 0.0,
        };
        let iff = player_iff(p);
        scored.push((
            d_sq,
            Track {
                id: EntityId::name(&p.name),
                kind: EntityKind::Player(p.name.clone()),
                position: p.position,
                sta: player_sta(p),
                iff,
                rel: rel_from_iff(iff),
                hp_pct: hp_pct(p.health, p.max_health),
            },
        ));
    }

    for m in state.nearby_monsters.values() {
        let d_sq = match subject_position.as_ref() {
            Some(sp) => {
                let d = m.position.dist_xz_sq(sp);
                if d > sight_sq {
                    continue;
                }
                d
            }
            None => 0.0,
        };
        if m.state == MonsterState::Dead {
            continue;
        }
        let iff = monster_iff(m);
        scored.push((
            d_sq,
            Track {
                id: EntityId::name(&m.id),
                kind: EntityKind::Monster(m.id.clone()),
                position: m.position,
                sta: monster_sta(m),
                iff,
                rel: rel_from_iff(iff),
                hp_pct: hp_pct(m.health, m.max_health),
            },
        ));
    }

    // Nearest first, ties broken by identifier: a HashMap iteration order must
    // not be able to reorder the frame.
    scored.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
    let tracks: Vec<Track> = scored.into_iter().map(|(_, t)| t).collect();

    let hostile_adjacent = tracks.iter().any(|t| {
        t.iff == Iff::Hostile && within(subject_position.as_ref(), &t.position, ENGAGED_RADIUS)
    });
    let subject_sta = subject_sta(state, hostile_adjacent);
    let subject_hp_pct = state
        .self_player
        .as_ref()
        .map(|p| hp_pct(p.health, p.max_health))
        .unwrap_or(0);

    let mut packets = Vec::with_capacity(tracks.len() * 2 + 1);
    if let Some(pos) = subject_position.as_ref() {
        packets
            .push(Packet::ppli(subject.clone(), subject_sta, coord(pos)).with_hp(subject_hp_pct));
    }
    for t in &tracks {
        packets.push(Packet::ppli(t.id.clone(), t.sta, coord(&t.position)).with_hp(t.hp_pct));
    }
    for t in &tracks {
        packets.push(Packet::track(subject.clone(), t.id.clone(), t.iff).with_rel(t.rel));
    }

    Uplink {
        subject,
        subject_position,
        subject_sta,
        subject_hp_pct,
        tracks,
        packets,
    }
}

fn within(from: Option<&Position>, to: &Position, radius: f32) -> bool {
    match from {
        Some(f) => to.dist_xz_sq(f) <= radius * radius,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidla::schema;
    use crate::sidla::wire;
    use crate::state::tests::{p, test_state};
    use onlinerpg_shared::{CharacterClass, PlayerId};

    fn player(id: u64, name: &str, pos: Position, official: bool) -> Player {
        Player {
            id: PlayerId::from(id),
            name: name.to_string(),
            position: pos,
            rotation: 0.0,
            level: 1,
            health: 100,
            max_health: 100,
            class: CharacterClass::Knight,
            gender: Default::default(),
            is_official_npc: official,
            torch_on: false,
            floor_level: 0,
            object_type: None,
            main_hand: None,
            object_id: None,
            last_combat_at: 0,
            client_kind: Default::default(),
        }
    }

    fn monster(id: &str, pos: Position, aggressive: bool) -> Monster {
        Monster {
            id: id.to_string(),
            monster_type: "slime".to_string(),
            position: pos,
            rotation: 0.0,
            state: MonsterState::Idle,
            owner_id: None,
            health: 10,
            max_health: 10,
            floor_level: 0,
            level_override: None,
            aggressive,
            last_attack_at: 0,
            last_move_at: 0,
            move_budget: 0.0,
        }
    }

    fn populated() -> SharedState {
        let (mut state, _rx) = test_state();
        let me = player(1, "Mika", p(0.0, 0.0, 0.0), false);
        state.self_player_id = Some(me.id);
        state.self_player = Some(me);

        let saori = player(2, "Saori", p(3.0, 0.0, 0.0), true);
        state.nearby_players.insert(saori.id, saori);
        state
            .nearby_monsters
            .insert("slime_2".into(), monster("slime_2", p(0.0, 0.0, 8.0), true));
        state.nearby_monsters.insert(
            "slime_1".into(),
            monster("slime_1", p(0.0, 0.0, 4.0), false),
        );
        state
    }

    #[test]
    fn every_emitted_packet_satisfies_the_schema() {
        let uplink = encode(&populated());
        for packet in &uplink.packets {
            schema::validate(packet)
                .unwrap_or_else(|e| panic!("uplink emitted an invalid packet: {e}\n{packet:?}"));
        }
    }

    #[test]
    fn each_entity_gets_one_ppli_and_one_track() {
        let uplink = encode(&populated());
        assert_eq!(uplink.tracks.len(), 3);
        let ppli = uplink
            .packets
            .iter()
            .filter(|p| p.h == crate::sidla::packet::Header::A)
            .count();
        let track = uplink
            .packets
            .iter()
            .filter(|p| p.h == crate::sidla::packet::Header::B)
            .count();
        assert_eq!(ppli, 4, "three entities plus the subject");
        assert_eq!(track, 3);
    }

    #[test]
    fn the_frame_is_identical_across_repeated_encodings() {
        let state = populated();
        let first = wire::render_json(&encode(&state).packets);
        for _ in 0..32 {
            assert_eq!(wire::render_json(&encode(&state).packets), first);
        }
    }

    #[test]
    fn tracks_are_ordered_by_distance() {
        let uplink = encode(&populated());
        let names: Vec<String> = uplink.tracks.iter().map(|t| t.id.to_string()).collect();
        assert_eq!(names, ["Saori", "slime_1", "slime_2"]);
    }

    #[test]
    fn identification_follows_aggression_and_officialdom() {
        let uplink = encode(&populated());
        let iff = |name: &str| uplink.find(&EntityId::name(name)).map(|t| t.iff);
        assert_eq!(iff("Saori"), Some(Iff::Friend));
        assert_eq!(iff("slime_1"), Some(Iff::Neutral));
        assert_eq!(iff("slime_2"), Some(Iff::Hostile));
    }

    #[test]
    fn affinity_stays_inside_the_dictionary_range() {
        let uplink = encode(&populated());
        for t in &uplink.tracks {
            assert!((-100..=100).contains(&t.rel), "{t:?}");
        }
    }

    #[test]
    fn a_dead_monster_is_left_out_of_the_frame() {
        let mut state = populated();
        state.nearby_monsters.get_mut("slime_1").unwrap().state = MonsterState::Dead;
        let uplink = encode(&state);
        assert!(uplink.find(&EntityId::name("slime_1")).is_none());
    }

    #[test]
    fn an_entity_beyond_sight_is_left_out_of_the_frame() {
        let mut state = populated();
        let far = NPC_SIGHT_RADIUS * 2.0;
        state.nearby_monsters.insert(
            "slime_far".into(),
            monster("slime_far", p(0.0, 0.0, far), true),
        );
        let uplink = encode(&state);
        assert!(uplink.find(&EntityId::name("slime_far")).is_none());
    }

    #[test]
    fn a_wounded_entity_reports_panic() {
        let mut state = populated();
        let m = state.nearby_monsters.get_mut("slime_1").unwrap();
        m.health = 1;
        let uplink = encode(&state);
        assert_eq!(
            uplink.find(&EntityId::name("slime_1")).map(|t| t.sta),
            Some(Sta::Panic)
        );
    }

    #[test]
    fn the_subject_reports_engaged_with_a_hostile_at_its_side() {
        let mut state = populated();
        state.nearby_monsters.get_mut("slime_2").unwrap().position = p(1.0, 0.0, 1.0);
        assert_eq!(encode(&state).subject_sta, Sta::Engaged);
    }

    #[test]
    fn the_subject_reports_dead_at_zero_health() {
        let mut state = populated();
        state.self_player.as_mut().unwrap().health = 0;
        assert_eq!(encode(&state).subject_sta, Sta::Dead);
    }

    #[test]
    fn a_state_frame_costs_fewer_tokens_than_the_prose_it_replaces() {
        let state = populated();
        let prose = wire::estimate_tokens(&state.format_world_state());
        let compact = wire::estimate_tokens(&wire::render_compact(&encode(&state).packets));
        assert!(
            compact < prose,
            "compact {compact} tokens vs prose {prose} tokens"
        );
    }
}
