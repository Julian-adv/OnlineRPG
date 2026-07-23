//! Per-character trained skills (fishing, and future gathering professions).
//! Deliberately separate from `CharacterAttributes`: attributes are rolled
//! once at creation, skills grow through play. `Skills` is the wire/persist
//! payload; the XP curve helpers live here so server, client (via wasm) and
//! agent-client all read the same numbers.
//!
//! Skill levels start at 0 (a character map with no entry means "never
//! trained") and cap at `SKILL_LEVEL_CAP` — the d20 ceiling, matching the
//! attribute scale rather than character levels' open-ended curve.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const SKILL_LEVEL_CAP: u32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SkillId {
    #[serde(rename = "fishing")]
    Fishing,
}

impl SkillId {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkillId::Fishing => "fishing",
        }
    }

    /// Player-facing name, shared so every surface capitalizes it the same way.
    pub fn display_name(&self) -> &'static str {
        match self {
            SkillId::Fishing => "Fishing",
        }
    }
}

impl std::str::FromStr for SkillId {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "fishing" => Ok(SkillId::Fishing),
            _ => Err(()),
        }
    }
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
        // …and an empty map round-trips as an empty map, not a null.
        let json = serde_json::to_string(&skills).unwrap();
        assert_eq!(json, r#"{"map":{}}"#);
    }
}
