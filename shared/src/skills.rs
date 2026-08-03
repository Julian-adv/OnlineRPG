//! Per-character trained skills, separate from `CharacterAttributes`: rolled
//! once vs. grown through play. The XP curve lives here so server, client
//! (wasm) and agent-client share the exact numbers. Levels run 0 (no entry =
//! never trained) to `SKILL_LEVEL_CAP`, the knob unlock ladders stretch
//! against.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const SKILL_LEVEL_CAP: u32 = 30;
pub const DEFAULT_WEAPON_MELEE_RANGE_METERS: f32 = 2.0;
pub const DEFAULT_WEAPON_ATTACK_COOLDOWN_MS: u32 = 1_533;
pub const SPEAR_MELEE_RANGE_METERS: f32 = 3.0;
pub const SPEAR_ATTACK_COOLDOWN_MS: u32 = 2_467;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SkillId {
    #[serde(rename = "fishing")]
    Fishing,
    #[serde(rename = "one_handed_sword")]
    OneHandedSword,
    #[serde(rename = "dagger")]
    Dagger,
    #[serde(rename = "spear")]
    Spear,
    #[serde(rename = "shield")]
    Shield,
    #[serde(rename = "healing")]
    Healing,
    #[serde(rename = "leather_armor")]
    LeatherArmor,
}

impl SkillId {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkillId::Fishing => "fishing",
            SkillId::OneHandedSword => "one_handed_sword",
            SkillId::Dagger => "dagger",
            SkillId::Spear => "spear",
            SkillId::Shield => "shield",
            SkillId::Healing => "healing",
            SkillId::LeatherArmor => "leather_armor",
        }
    }

    /// Player-facing name, shared so every surface capitalizes it the same way.
    pub fn display_name(&self) -> &'static str {
        match self {
            SkillId::Fishing => "Fishing",
            SkillId::OneHandedSword => "One-Handed Sword",
            SkillId::Dagger => "Dagger",
            SkillId::Spear => "Spear",
            SkillId::Shield => "Shield",
            SkillId::Healing => "Healing",
            SkillId::LeatherArmor => "Leather Armor",
        }
    }
}

impl std::str::FromStr for SkillId {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "fishing" => Ok(SkillId::Fishing),
            "one_handed_sword" => Ok(SkillId::OneHandedSword),
            "dagger" => Ok(SkillId::Dagger),
            "spear" => Ok(SkillId::Spear),
            "shield" => Ok(SkillId::Shield),
            "healing" => Ok(SkillId::Healing),
            "leather_armor" => Ok(SkillId::LeatherArmor),
            _ => Err(()),
        }
    }
}

/// Accuracy bonus for a supported trained weapon skill. Non-weapon skills do
/// not contribute to attack rolls.
pub fn weapon_skill_attack_bonus(skill: SkillId, skill_level: u32) -> i32 {
    match skill {
        SkillId::OneHandedSword | SkillId::Dagger | SkillId::Spear => {
            ((skill_level.min(SKILL_LEVEL_CAP) + 5) / 10).min(3) as i32
        }
        SkillId::Fishing | SkillId::Shield | SkillId::Healing | SkillId::LeatherArmor => 0,
    }
}

pub fn weapon_skill_melee_range(skill: SkillId) -> f32 {
    match skill {
        SkillId::Spear => SPEAR_MELEE_RANGE_METERS,
        SkillId::OneHandedSword
        | SkillId::Dagger
        | SkillId::Fishing
        | SkillId::Shield
        | SkillId::Healing
        | SkillId::LeatherArmor => DEFAULT_WEAPON_MELEE_RANGE_METERS,
    }
}

pub fn weapon_skill_attack_cooldown_ms(skill: SkillId) -> u32 {
    match skill {
        SkillId::Spear => SPEAR_ATTACK_COOLDOWN_MS,
        SkillId::OneHandedSword
        | SkillId::Dagger
        | SkillId::Fishing
        | SkillId::Shield
        | SkillId::Healing
        | SkillId::LeatherArmor => DEFAULT_WEAPON_ATTACK_COOLDOWN_MS,
    }
}

pub fn one_handed_sword_attack_bonus(skill_level: u32) -> i32 {
    weapon_skill_attack_bonus(SkillId::OneHandedSword, skill_level)
}

/// Guard added while an item explicitly mapped to the Shield skill is
/// equipped. This is separate from (and added once after) the item's own
/// `guard` value.
pub fn shield_skill_guard_bonus(skill_level: u32) -> i32 {
    ((skill_level.min(SKILL_LEVEL_CAP) + 5) / 10).min(3) as i32
}

/// Guard added once while the explicitly mapped primary body armor is worn.
/// Construction-specific variants enter this match only with an implemented
/// skill vertical slice.
pub fn armor_skill_guard_bonus(skill: SkillId, skill_level: u32) -> i32 {
    match skill {
        SkillId::LeatherArmor => ((skill_level.min(SKILL_LEVEL_CAP) + 5) / 10).min(3) as i32,
        SkillId::Fishing
        | SkillId::OneHandedSword
        | SkillId::Dagger
        | SkillId::Spear
        | SkillId::Shield
        | SkillId::Healing => 0,
    }
}

/// Flat HP added when applying an explicitly mapped Healing treatment. The
/// Bandage's dice remain the primary effect; finished products such as Healing
/// Potions do not receive this bonus.
pub fn healing_skill_hp_bonus(skill_level: u32) -> u32 {
    ((skill_level.min(SKILL_LEVEL_CAP) + 5) / 10).min(3)
}

/// XP required to go from `level - 1` to `level` (level ≥ 1): `100 · level²`.
/// Early levels come fast (level 1 after 100 XP), the last few are a real
/// investment — same feel as the character curve without its doubling.
pub fn skill_xp_cost(level: u32) -> u64 {
    let l = u64::from(level);
    100 * l * l
}

/// Minimum cumulative XP required to hold the given level. Level 0: 0.
/// Closed form of `Σ 100·l²`: `100 · n(n+1)(2n+1)/6`.
pub fn skill_xp_for_level(level: u32) -> u64 {
    let n = u64::from(level.min(SKILL_LEVEL_CAP));
    100 * n * (n + 1) * (2 * n + 1) / 6
}

/// Current skill level from cumulative XP, capped at `SKILL_LEVEL_CAP`.
pub fn skill_level_from_xp(xp: u64) -> u32 {
    let mut level = 0;
    while level < SKILL_LEVEL_CAP && xp >= skill_xp_for_level(level + 1) {
        level += 1;
    }
    level
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillProgress {
    pub level: u32,
    pub xp: u64,
}

/// Outcome of one XP grant, shaped for the `SkillXpGained` message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillXpResult {
    pub xp_amount: u64,
    pub total_xp: u64,
    pub new_level: u32,
    pub leveled_up: bool,
}

/// Every skill a character has trained. Keys are absent until first trained,
/// so a fresh character serializes as an empty map and old save rows load
/// unchanged (`#[serde(default)]` at the embed sites).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Skills {
    pub map: HashMap<SkillId, SkillProgress>,
}

impl Skills {
    /// Progress in `skill`; level 0 / 0 XP when never trained.
    pub fn get(&self, skill: SkillId) -> SkillProgress {
        self.map.get(&skill).copied().unwrap_or_default()
    }

    /// Grant XP, clamping cumulative XP to the cap's threshold so a maxed
    /// skill stops accumulating. Returns `None` when nothing changed
    /// (already at cap), so callers can skip the persist + message.
    pub fn add_xp(&mut self, skill: SkillId, amount: u64) -> Option<SkillXpResult> {
        let entry = self.map.entry(skill).or_default();
        let old_xp = entry.xp;
        let old_level = entry.level;
        let new_xp = old_xp
            .saturating_add(amount)
            .min(skill_xp_for_level(SKILL_LEVEL_CAP));
        if new_xp == old_xp {
            return None;
        }
        entry.xp = new_xp;
        entry.level = skill_level_from_xp(new_xp);
        Some(SkillXpResult {
            xp_amount: new_xp - old_xp,
            total_xp: new_xp,
            new_level: entry.level,
            leveled_up: entry.level > old_level,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_ids_round_trip_and_reject_unknown_values() {
        for (id, wire, display) in [
            (SkillId::Fishing, "fishing", "Fishing"),
            (
                SkillId::OneHandedSword,
                "one_handed_sword",
                "One-Handed Sword",
            ),
            (SkillId::Dagger, "dagger", "Dagger"),
            (SkillId::Spear, "spear", "Spear"),
            (SkillId::Shield, "shield", "Shield"),
            (SkillId::Healing, "healing", "Healing"),
            (SkillId::LeatherArmor, "leather_armor", "Leather Armor"),
        ] {
            assert_eq!(id.as_str(), wire);
            assert_eq!(id.display_name(), display);
            assert_eq!(wire.parse::<SkillId>(), Ok(id));
            assert_eq!(serde_json::to_string(&id).unwrap(), format!("\"{wire}\""));
            assert_eq!(
                serde_json::from_str::<SkillId>(&format!("\"{wire}\"")).unwrap(),
                id
            );
        }
        assert!("sword".parse::<SkillId>().is_err());
        assert!(serde_json::from_str::<SkillId>("\"sword\"").is_err());
    }

    #[test]
    fn one_handed_sword_bonus_uses_phase_one_boundaries() {
        for (level, expected) in [
            (0, 0),
            (4, 0),
            (5, 1),
            (14, 1),
            (15, 2),
            (24, 2),
            (25, 3),
            (30, 3),
            (u32::MAX, 3),
        ] {
            assert_eq!(
                one_handed_sword_attack_bonus(level),
                expected,
                "level {level}"
            );
        }
    }

    #[test]
    fn dagger_uses_the_shared_weapon_accuracy_ladder() {
        for (level, expected) in [(0, 0), (5, 1), (15, 2), (25, 3), (u32::MAX, 3)] {
            assert_eq!(weapon_skill_attack_bonus(SkillId::Dagger, level), expected);
        }
        assert_eq!(weapon_skill_attack_bonus(SkillId::Fishing, 30), 0);
        assert_eq!(weapon_skill_attack_bonus(SkillId::Shield, 30), 0);
        assert_eq!(weapon_skill_attack_bonus(SkillId::Healing, 30), 0);
        assert_eq!(weapon_skill_attack_bonus(SkillId::LeatherArmor, 30), 0);
    }

    #[test]
    fn shield_uses_the_defensive_guard_ladder() {
        for (level, expected) in [
            (0, 0),
            (4, 0),
            (5, 1),
            (14, 1),
            (15, 2),
            (24, 2),
            (25, 3),
            (30, 3),
            (u32::MAX, 3),
        ] {
            assert_eq!(shield_skill_guard_bonus(level), expected, "level {level}");
        }
    }

    #[test]
    fn leather_armor_uses_one_construction_guard_ladder() {
        for (level, expected) in [
            (0, 0),
            (4, 0),
            (5, 1),
            (14, 1),
            (15, 2),
            (24, 2),
            (25, 3),
            (30, 3),
            (u32::MAX, 3),
        ] {
            assert_eq!(
                armor_skill_guard_bonus(SkillId::LeatherArmor, level),
                expected,
                "level {level}"
            );
        }
        assert_eq!(armor_skill_guard_bonus(SkillId::Shield, 30), 0);
    }

    #[test]
    fn healing_uses_the_noncombat_hp_ladder() {
        for (level, expected) in [
            (0, 0),
            (4, 0),
            (5, 1),
            (14, 1),
            (15, 2),
            (24, 2),
            (25, 3),
            (30, 3),
            (u32::MAX, 3),
        ] {
            assert_eq!(healing_skill_hp_bonus(level), expected, "level {level}");
        }
    }

    #[test]
    fn spear_uses_its_content_backed_range_and_cadence() {
        assert_eq!(weapon_skill_attack_bonus(SkillId::Spear, 15), 2);
        assert_eq!(
            weapon_skill_melee_range(SkillId::Spear),
            SPEAR_MELEE_RANGE_METERS
        );
        assert_eq!(
            weapon_skill_attack_cooldown_ms(SkillId::Spear),
            SPEAR_ATTACK_COOLDOWN_MS
        );
        assert_eq!(
            weapon_skill_melee_range(SkillId::OneHandedSword),
            DEFAULT_WEAPON_MELEE_RANGE_METERS
        );
        assert_eq!(
            weapon_skill_attack_cooldown_ms(SkillId::Dagger),
            DEFAULT_WEAPON_ATTACK_COOLDOWN_MS
        );
    }

    #[test]
    fn xp_thresholds_match_per_level_costs() {
        assert_eq!(skill_xp_for_level(0), 0);
        assert_eq!(skill_xp_for_level(1), 100);
        assert_eq!(skill_xp_for_level(2), 500);
        assert_eq!(skill_xp_for_level(3), 1400);
        let mut sum = 0;
        for level in 1..=SKILL_LEVEL_CAP {
            sum += skill_xp_cost(level);
            assert_eq!(skill_xp_for_level(level), sum);
        }
    }

    #[test]
    fn level_from_xp_boundaries() {
        assert_eq!(skill_level_from_xp(0), 0);
        assert_eq!(skill_level_from_xp(99), 0);
        assert_eq!(skill_level_from_xp(100), 1);
        assert_eq!(skill_level_from_xp(499), 1);
        assert_eq!(skill_level_from_xp(500), 2);
        assert_eq!(skill_level_from_xp(u64::MAX), SKILL_LEVEL_CAP);
    }

    #[test]
    fn add_xp_levels_up_and_reports() {
        let mut skills = Skills::default();
        let r = skills.add_xp(SkillId::Fishing, 40).unwrap();
        assert_eq!(r.new_level, 0);
        assert!(!r.leveled_up);
        let r = skills.add_xp(SkillId::Fishing, 60).unwrap();
        assert_eq!(r.new_level, 1);
        assert!(r.leveled_up);
        assert_eq!(r.total_xp, 100);
        assert_eq!(skills.get(SkillId::Fishing).level, 1);
    }

    #[test]
    fn add_xp_clamps_at_cap_and_goes_quiet() {
        let mut skills = Skills::default();
        let cap_xp = skill_xp_for_level(SKILL_LEVEL_CAP);
        let r = skills.add_xp(SkillId::Fishing, u64::MAX).unwrap();
        assert_eq!(r.total_xp, cap_xp);
        assert_eq!(r.new_level, SKILL_LEVEL_CAP);
        assert!(r.leveled_up);
        // A maxed skill reports nothing — no dirty flag, no message.
        assert!(skills.add_xp(SkillId::Fishing, 1).is_none());
    }

    #[test]
    fn untrained_skill_reads_as_level_zero() {
        let skills = Skills::default();
        assert_eq!(skills.get(SkillId::Fishing), SkillProgress::default());
        assert_eq!(
            skills.get(SkillId::OneHandedSword),
            SkillProgress::default()
        );
        assert_eq!(skills.get(SkillId::Dagger), SkillProgress::default());
        assert_eq!(skills.get(SkillId::Spear), SkillProgress::default());
        assert_eq!(skills.get(SkillId::Shield), SkillProgress::default());
        assert_eq!(skills.get(SkillId::Healing), SkillProgress::default());
        assert_eq!(skills.get(SkillId::LeatherArmor), SkillProgress::default());
        // …and an empty map round-trips as an empty map, not a null.
        let json = serde_json::to_string(&skills).unwrap();
        assert_eq!(json, r#"{"map":{}}"#);
    }

    #[test]
    fn all_current_skill_progress_coexists() {
        let mut skills = Skills::default();
        skills.add_xp(SkillId::Fishing, 100).unwrap();
        skills.add_xp(SkillId::OneHandedSword, 500).unwrap();
        skills.add_xp(SkillId::Dagger, 10).unwrap();
        skills.add_xp(SkillId::Spear, 20).unwrap();
        skills.add_xp(SkillId::Shield, 30).unwrap();
        skills.add_xp(SkillId::Healing, 40).unwrap();
        skills.add_xp(SkillId::LeatherArmor, 50).unwrap();

        assert_eq!(skills.get(SkillId::Fishing).level, 1);
        assert_eq!(skills.get(SkillId::OneHandedSword).level, 2);
        assert_eq!(skills.get(SkillId::Dagger).xp, 10);
        assert_eq!(skills.get(SkillId::Spear).xp, 20);
        assert_eq!(skills.get(SkillId::Shield).xp, 30);
        assert_eq!(skills.get(SkillId::Healing).xp, 40);
        assert_eq!(skills.get(SkillId::LeatherArmor).xp, 50);
        let decoded: Skills =
            serde_json::from_str(&serde_json::to_string(&skills).unwrap()).unwrap();
        assert_eq!(decoded, skills);
    }
}
