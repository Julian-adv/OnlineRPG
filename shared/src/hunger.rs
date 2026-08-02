//! Hunger (satiation) tuning constants and state bands. doc/HUNGER.md.
//!
//! Satiation is a per-character integer 0..=1000, server-authoritative.
//! Buff framing: Well-Fed grants bonuses, Hungry/Stuffed are the plain
//! baseline, only Weak (and food poisoning) penalize.

use serde::{Deserialize, Serialize};

pub const SATIATION_MAX: u32 = 1000;
/// New characters start comfortably fed.
pub const SATIATION_START: u32 = 700;
/// Respawn resets to the Well-Fed floor so death never compounds starvation.
pub const SATIATION_RESPAWN: u32 = 300;

/// One point drains per this many real seconds while online (540/game-day).
pub const DECAY_INTERVAL_SECS: u64 = 20;
/// Food poisoning drains satiation this many times faster.
pub const POISON_DRAIN_MULT: u32 = 4;

/// Eating below this leaves Well-Fed reachable but never overshoots into
/// Stuffed: the result clamps to `SOFT_CAP_TARGET`. Eating at or above it
/// applies the full nutrition (deliberate overeating).
pub const SOFT_CAP_THRESHOLD: u32 = 800;
pub const SOFT_CAP_TARGET: u32 = 850;

pub const WELL_FED_MIN: u32 = 300;
pub const WELL_FED_MAX: u32 = 850;
pub const HUNGRY_MIN: u32 = 100;

/// Raw fish nutrition, species-independent (cooking unlocks the real value).
pub const RAW_FISH_NUTRITION: u32 = 40;
/// Chance eating raw fish inflicts food poisoning, in percent.
pub const FOOD_POISONING_PCT: u32 = 70;
pub const FOOD_POISONING_MS: u64 = 5 * 60 * 1000;
/// Food poisoning multiplier on move speed, attack speed and carry weight.
pub const POISON_MULT: f32 = 0.6;

pub const WELL_FED_MOVE_MULT: f32 = 1.1;
pub const WELL_FED_ATTACK_MULT: f32 = 1.1;
pub const WELL_FED_CARRY_MULT: f32 = 1.15;
pub const WEAK_MOVE_MULT: f32 = 0.75;
pub const WEAK_ATTACK_MULT: f32 = 0.75;
pub const WEAK_CARRY_MULT: f32 = 0.6;

pub const CAMPFIRE_DURATION_MS: u64 = 10 * 60 * 1000;
/// Raw fish used within this range of a campfire grills instead of being eaten.
pub const CAMPFIRE_GRILL_RADIUS: f32 = 3.0;
pub const GRILL_CAST_MS: u64 = 3000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HungerState {
    Stuffed,
    WellFed,
    Hungry,
    Weak,
}

pub fn hunger_state(satiation: u32) -> HungerState {
    if satiation > WELL_FED_MAX {
        HungerState::Stuffed
    } else if satiation >= WELL_FED_MIN {
        HungerState::WellFed
    } else if satiation >= HUNGRY_MIN {
        HungerState::Hungry
    } else {
        HungerState::Weak
    }
}

/// (move, attack, carry) multipliers for a band, before food poisoning.
pub fn state_multipliers(state: HungerState) -> (f32, f32, f32) {
    match state {
        HungerState::WellFed => (
            WELL_FED_MOVE_MULT,
            WELL_FED_ATTACK_MULT,
            WELL_FED_CARRY_MULT,
        ),
        HungerState::Weak => (WEAK_MOVE_MULT, WEAK_ATTACK_MULT, WEAK_CARRY_MULT),
        HungerState::Stuffed | HungerState::Hungry => (1.0, 1.0, 1.0),
    }
}

/// Effective (move, attack, carry) multipliers, poison stacking multiplicatively.
pub fn effective_multipliers(satiation: u32, poisoned: bool) -> (f32, f32, f32) {
    let (m, a, c) = state_multipliers(hunger_state(satiation));
    if poisoned {
        (m * POISON_MULT, a * POISON_MULT, c * POISON_MULT)
    } else {
        (m, a, c)
    }
}

/// Satiation after eating `nutrition`, applying the anti-overshoot soft cap.
pub fn apply_nutrition(satiation: u32, nutrition: u32) -> u32 {
    let raised = (satiation + nutrition).min(SATIATION_MAX);
    if satiation < SOFT_CAP_THRESHOLD {
        raised.min(SOFT_CAP_TARGET)
    } else {
        raised
    }
}

/// A lit campfire visible to nearby players. Wire type (positional array —
/// never reorder fields, append only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Campfire {
    pub id: u64,
    pub position: crate::world::Position,
    pub floor_level: i8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_cover_the_whole_range() {
        assert_eq!(hunger_state(0), HungerState::Weak);
        assert_eq!(hunger_state(99), HungerState::Weak);
        assert_eq!(hunger_state(100), HungerState::Hungry);
        assert_eq!(hunger_state(299), HungerState::Hungry);
        assert_eq!(hunger_state(300), HungerState::WellFed);
        assert_eq!(hunger_state(850), HungerState::WellFed);
        assert_eq!(hunger_state(851), HungerState::Stuffed);
        assert_eq!(hunger_state(1000), HungerState::Stuffed);
    }

    #[test]
    fn soft_cap_blocks_accidental_overeating() {
        // Normal-range meals never overshoot into Stuffed.
        assert_eq!(apply_nutrition(500, 540), SOFT_CAP_TARGET);
        assert_eq!(apply_nutrition(799, 900), SOFT_CAP_TARGET);
        // Deliberate overeating at/above the threshold goes through.
        assert_eq!(apply_nutrition(800, 100), 900);
        assert_eq!(apply_nutrition(850, 60), 910);
        // Hard cap.
        assert_eq!(apply_nutrition(900, 500), SATIATION_MAX);
        // Small meals below the cap are untouched.
        assert_eq!(apply_nutrition(300, 60), 360);
    }

    #[test]
    fn multipliers_follow_the_buff_framing() {
        assert_eq!(effective_multipliers(500, false), (1.1, 1.1, 1.15));
        assert_eq!(effective_multipliers(200, false), (1.0, 1.0, 1.0));
        assert_eq!(effective_multipliers(900, false), (1.0, 1.0, 1.0));
        assert_eq!(effective_multipliers(50, false), (0.75, 0.75, 0.6));
        let (m, _, _) = effective_multipliers(50, true);
        assert!((m - 0.45).abs() < 1e-6);
    }
}
