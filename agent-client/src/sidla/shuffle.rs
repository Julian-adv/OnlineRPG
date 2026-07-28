//! Integer shuffling: the sanctioned source of behavioural variety.
//!
//! Greedy decoding makes the agent repeat itself given the same world. Rather
//! than reintroduce sampling inside inference — which would take determinism
//! back — variety is added here, after validation, by permuting an enum value
//! among others that are interchangeable for the situation.
//!
//! The mixer is SplitMix64: same seed, same turn, same result. A run is still
//! reproducible; it is only reproducible as a sequence rather than as a single
//! repeated decision.

use super::packet::{Obj, Packet};

/// One round of SplitMix64.
pub fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut x = z;
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// A value in `[0, 1)` derived from `seed`.
pub fn unit_fraction(seed: u64) -> f32 {
    (mix(seed) >> 40) as f32 / (1u64 << 24) as f32
}

fn pick<T: Copy>(options: &[T], seed: u64) -> T {
    options[(mix(seed) % options.len() as u64) as usize]
}

/// Objectives that read as the same intent — a patrolling guard may sweep or
/// search without contradicting its orders. Nothing that changes who gets hurt
/// is ever substituted.
const IDLE_OBJECTIVES: [Obj; 2] = [Obj::Patrol, Obj::Search];

/// Vary an already-valid packet. Only the idle objectives are permuted:
/// substituting an `ACT` would change what the agent does to whom, which is
/// the planner's decision and not this layer's to make.
pub fn vary(packet: &Packet, seed: u64) -> Packet {
    let mut out = packet.clone();
    if let Some(obj) = packet.obj {
        if IDLE_OBJECTIVES.contains(&obj) {
            out.obj = Some(pick(&IDLE_OBJECTIVES, seed));
        }
    }
    out
}

/// Seed for turn `turn` of a run started with `seed`.
pub fn turn_seed(seed: u64, turn: u64) -> u64 {
    mix(seed ^ turn.wrapping_mul(0x2545_F491_4F6C_DD1D))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidla::packet::{Act, EntityId, Iff};

    #[test]
    fn the_same_seed_always_mixes_to_the_same_value() {
        for seed in [0, 1, 42, u64::MAX] {
            assert_eq!(mix(seed), mix(seed));
        }
    }

    #[test]
    fn different_seeds_diverge() {
        assert_ne!(mix(1), mix(2));
        assert_ne!(turn_seed(7, 1), turn_seed(7, 2));
    }

    #[test]
    fn unit_fractions_stay_in_range_and_spread_out() {
        let mut low = 0;
        let mut high = 0;
        for turn in 0..512u64 {
            let f = unit_fraction(turn_seed(9, turn));
            assert!((0.0..1.0).contains(&f), "{f}");
            if f < 0.5 {
                low += 1;
            } else {
                high += 1;
            }
        }
        assert!(low > 150 && high > 150, "low {low}, high {high}");
    }

    #[test]
    fn an_idle_objective_may_be_substituted_for_its_equivalent() {
        let packet = Packet::mission(EntityId::name("Mika"), Obj::Patrol);
        let seen: std::collections::HashSet<Obj> = (0..64)
            .map(|turn| vary(&packet, turn_seed(3, turn)).obj.unwrap())
            .collect();
        assert_eq!(seen.len(), 2, "expected both idle objectives, got {seen:?}");
        assert!(seen.iter().all(|o| IDLE_OBJECTIVES.contains(o)));
    }

    #[test]
    fn variation_is_reproducible_for_a_given_turn() {
        let packet = Packet::mission(EntityId::name("Mika"), Obj::Patrol);
        for turn in 0..16u64 {
            let seed = turn_seed(11, turn);
            assert_eq!(vary(&packet, seed), vary(&packet, seed));
        }
    }

    #[test]
    fn a_committing_objective_is_never_substituted() {
        for obj in [Obj::Exterminate, Obj::Escort, Obj::Defend, Obj::None] {
            let packet = Packet::mission(EntityId::name("Mika"), obj);
            for turn in 0..32u64 {
                assert_eq!(vary(&packet, turn_seed(5, turn)).obj, Some(obj));
            }
        }
    }

    #[test]
    fn an_engagement_is_never_rewritten() {
        let packet = Packet::engage(
            EntityId::name("Mika"),
            EntityId::name("slime_1"),
            Act::Attack,
        );
        for turn in 0..32u64 {
            assert_eq!(vary(&packet, turn_seed(5, turn)), packet);
        }
    }

    #[test]
    fn an_observation_is_never_rewritten() {
        let packet = Packet::track(
            EntityId::name("Mika"),
            EntityId::name("slime_1"),
            Iff::Hostile,
        )
        .with_rel(-100);
        for turn in 0..32u64 {
            assert_eq!(vary(&packet, turn_seed(5, turn)), packet);
        }
    }
}
