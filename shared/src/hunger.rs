//! Hunger (satiation) tuning constants and state bands. doc/HUNGER.md.

use serde::{Deserialize, Serialize};

pub const SATIATION_MAX: u32 = 1000;
/// New characters start comfortably fed.
pub const SATIATION_START: u32 = 700;
/// Respawn resets to the normal floor so death never compounds starvation.
pub const SATIATION_RESPAWN: u32 = 300;

pub const MOVEMENT_DRAIN_INTERVAL_SECS: f32 = 30.0;
pub const SPRINT_DRAIN_INTERVAL_SECS: f32 = 1.0;
pub const SPRINT_MOVE_MULT: f32 = 1.5;
pub const FOOD_REGEN_DURATION_SECS: u8 = 10;
/// Food poisoning drains satiation this many times faster.
pub const POISON_DRAIN_MULT: u32 = 4;

pub const NORMAL_MIN: u32 = 300;
pub const HUNGRY_MIN: u32 = 100;

/// Raw fish nutrition, species-independent (cooking unlocks the real value).
pub const RAW_FISH_NUTRITION: u32 = 40;
/// Chance eating raw fish inflicts food poisoning, in percent.
pub const FOOD_POISONING_PCT: u32 = 70;
pub const FOOD_POISONING_MS: u64 = 5 * 60 * 1000;
/// Food poisoning multiplier on move speed, attack speed and carry weight.
pub const POISON_MULT: f32 = 0.6;

pub const WEAK_MOVE_MULT: f32 = 0.75;
pub const WEAK_ATTACK_MULT: f32 = 0.75;
pub const WEAK_CARRY_MULT: f32 = 0.6;

pub const CAMPFIRE_DURATION_MS: u64 = 10 * 60 * 1000;
/// Raw fish used within this range of a campfire grills instead of being eaten.
pub const CAMPFIRE_GRILL_RADIUS: f32 = 3.0;
pub const GRILL_CAST_MS: u64 = 3000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HungerState {
    Normal,
    Hungry,
    Weak,
}

pub fn hunger_state(satiation: u32) -> HungerState {
    if satiation >= NORMAL_MIN {
        HungerState::Normal
    } else if satiation >= HUNGRY_MIN {
        HungerState::Hungry
    } else {
        HungerState::Weak
    }
}

/// (move, attack, carry) multipliers for a band, before food poisoning.
pub fn state_multipliers(state: HungerState) -> (f32, f32, f32) {
    match state {
        HungerState::Weak => (WEAK_MOVE_MULT, WEAK_ATTACK_MULT, WEAK_CARRY_MULT),
        HungerState::Normal | HungerState::Hungry => (1.0, 1.0, 1.0),
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

pub fn apply_nutrition(satiation: u32, nutrition: u32) -> u32 {
    satiation.saturating_add(nutrition).min(SATIATION_MAX)
}

pub fn food_healing(nutrition: u32) -> u32 {
    nutrition / 20
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
        assert_eq!(hunger_state(300), HungerState::Normal);
        assert_eq!(hunger_state(1000), HungerState::Normal);
    }

    #[test]
    fn nutrition_only_has_a_hard_cap() {
        assert_eq!(apply_nutrition(500, 540), SATIATION_MAX);
        assert_eq!(apply_nutrition(800, 100), 900);
        assert_eq!(apply_nutrition(900, 500), SATIATION_MAX);
        assert_eq!(apply_nutrition(300, 60), 360);
    }

    #[test]
    fn multipliers_only_penalize_weakness_and_poisoning() {
        assert_eq!(effective_multipliers(500, false), (1.0, 1.0, 1.0));
        assert_eq!(effective_multipliers(200, false), (1.0, 1.0, 1.0));
        assert_eq!(effective_multipliers(900, false), (1.0, 1.0, 1.0));
        assert_eq!(effective_multipliers(50, false), (0.75, 0.75, 0.6));
        let (m, _, _) = effective_multipliers(50, true);
        assert!((m - 0.45).abs() < 1e-6);
    }

    #[test]
    fn food_healing_uses_the_nutrition_ratio() {
        assert_eq!(food_healing(60), 3);
        assert_eq!(food_healing(540), 27);
    }
}
