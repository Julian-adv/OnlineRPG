/// Calculate XP awarded for killing a monster.
/// Formula: 1 + level² + guard_bonus
/// guard_bonus: 2 XP per guard point above the baseline 10.
pub fn monster_xp(level: u8, guard: u8) -> u32 {
    let base = 1u32 + (level as u32) * (level as u32);
    let guard_bonus = u32::from(guard.saturating_sub(10)) * 2;
    base + guard_bonus
}

/// Party XP bonus per eligible member beyond the first. A full 5-member
/// party pools exactly double the solo award.
pub const PARTY_XP_BONUS_PER_EXTRA_PERCENT: u64 = 25;

/// XZ distance from the dying monster within which a living, same-floor
/// party member shares the kill XP. Sized to cover one hunting ground
/// (ambient spawns land within 60m of each member) without letting a
/// party leech across the map.
pub const PARTY_XP_SHARE_RADIUS: f32 = 150.0;

/// Equal per-member share of a monster's XP among `eligible_members` party
/// members. The killing blow gets no special cut — it only triggers the
/// split. The pool gains the party bonus, the split is floored, and each
/// member is guaranteed 1 XP (0 stays 0). Padding a party with idle members
/// cannot beat soloing, which holds only while the bonus stays at or below
/// 100% per extra member — see `party_share_never_beats_soloing`.
pub fn party_xp_share(base_xp: u32, eligible_members: u32) -> u32 {
    let n = eligible_members.max(1);
    if n == 1 {
        return base_xp;
    }
    let percent = 100 + PARTY_XP_BONUS_PER_EXTRA_PERCENT * u64::from(n - 1);
    let total = (u64::from(base_xp) * percent / 100) as u32;
    (total / n).max(1.min(base_xp))
}

/// Minimum cumulative XP required to reach the given level.
/// Level 1: 0, Level n (n>=2): 20 * 2^(n-2)
/// Saturates at u64::MAX for astronomically high levels (~62+).
pub fn xp_for_level(level: u32) -> u64 {
    if level <= 1 {
        return 0;
    }
    let shift = level - 2;
    if shift >= 64 {
        return u64::MAX;
    }
    20u64.saturating_mul(1u64 << shift)
}

/// Determine current level from cumulative XP. No upper bound.
pub fn level_from_xp(xp: u64) -> u32 {
    let mut level = 1u32;
    while let Some(next) = level.checked_add(1) {
        let threshold = xp_for_level(next);
        if xp < threshold {
            break;
        }
        level = next;
        if threshold == u64::MAX {
            break;
        }
    }
    level
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeathPenaltyResult {
    pub old_level: u32,
    pub new_level: u32,
    pub old_xp: u64,
    pub new_xp: u64,
    pub xp_penalty: u64,
    pub leveled_down: bool,
}

/// Apply death penalty to cumulative XP.
/// - XP loss: 15% of current level band (minimum 1)
/// - If below current level start XP, lose exactly 1 level (unless level 1)
/// - On level down, guarantee at least 30% progress in lower level band
pub fn apply_death_penalty(old_xp: u64) -> DeathPenaltyResult {
    let old_level = level_from_xp(old_xp);
    let level_start_xp = xp_for_level(old_level);
    let next_level_xp = xp_for_level(old_level.saturating_add(1));
    let level_band = next_level_xp.saturating_sub(level_start_xp);
    let xp_penalty = (level_band.saturating_mul(15) / 100).max(1);

    let mut new_xp = old_xp.saturating_sub(xp_penalty);
    let mut new_level = old_level;
    let mut leveled_down = false;

    if old_level > 1 && new_xp < level_start_xp {
        leveled_down = true;
        new_level = old_level - 1;

        let lower_start_xp = xp_for_level(new_level);
        let lower_next_xp = xp_for_level(new_level.saturating_add(1));
        let lower_band = lower_next_xp.saturating_sub(lower_start_xp);
        let recovery_floor = lower_start_xp.saturating_add(lower_band.saturating_mul(30) / 100);
        new_xp = new_xp.max(recovery_floor);
    }

    DeathPenaltyResult {
        old_level,
        new_level,
        old_xp,
        new_xp,
        xp_penalty,
        leveled_down,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monster_xp_no_guard_bonus() {
        // level 3, guard 10: 1 + 9 + 0 = 10
        assert_eq!(monster_xp(3, 10), 10);
    }

    #[test]
    fn monster_xp_below_baseline_guard_has_no_penalty() {
        assert_eq!(monster_xp(3, 8), 10);
    }

    #[test]
    fn monster_xp_guard_above_baseline() {
        // level 5, guard 12: 1 + 25 + 4 = 30
        assert_eq!(monster_xp(5, 12), 30);
    }

    #[test]
    fn monster_xp_high_guard() {
        // level 8, guard 13: 1 + 64 + 6 = 71
        assert_eq!(monster_xp(8, 13), 71);
    }

    #[test]
    fn xp_for_level_thresholds() {
        assert_eq!(xp_for_level(1), 0);
        assert_eq!(xp_for_level(2), 20);
        assert_eq!(xp_for_level(3), 40);
        assert_eq!(xp_for_level(10), 5120);
        assert_eq!(xp_for_level(11), 10240);
    }

    #[test]
    fn level_from_xp_basic() {
        assert_eq!(level_from_xp(0), 1);
        assert_eq!(level_from_xp(19), 1);
        assert_eq!(level_from_xp(20), 2);
        assert_eq!(level_from_xp(39), 2);
        assert_eq!(level_from_xp(40), 3);
        assert_eq!(level_from_xp(5120), 10);
        assert_eq!(level_from_xp(10240), 11);
    }

    #[test]
    fn xp_for_level_no_overflow() {
        // level 61: last level where 20 * 2^(n-2) fits in u64
        assert!(xp_for_level(61) < u64::MAX);
        // level 62+: saturates at u64::MAX
        assert_eq!(xp_for_level(62), u64::MAX);
        assert_eq!(xp_for_level(100), u64::MAX);
        assert_eq!(xp_for_level(u32::MAX), u64::MAX);
    }

    #[test]
    fn level_from_xp_max_does_not_panic() {
        // Should terminate without panic at extreme values
        let _ = level_from_xp(u64::MAX);
        let _ = level_from_xp(u64::MAX - 1);
    }

    #[test]
    fn party_share_solo_is_unchanged() {
        assert_eq!(party_xp_share(17, 1), 17);
        assert_eq!(party_xp_share(17, 0), 17);
    }

    #[test]
    fn party_share_three_members_orc() {
        // 17 * 1.50 = 25 -> 8 each, remainder dropped.
        assert_eq!(party_xp_share(17, 3), 8);
    }

    #[test]
    fn party_share_full_party_boss() {
        // 107 * 2.00 = 214 -> 42 each.
        assert_eq!(party_xp_share(107, 5), 42);
    }

    #[test]
    fn party_share_tiny_monster_pays_everyone_one() {
        // kobold (2 XP): the floored split would be 0; everyone gets 1.
        assert_eq!(party_xp_share(2, 3), 1);
        assert_eq!(party_xp_share(2, 5), 1);
        assert_eq!(party_xp_share(1, 5), 1);
        assert_eq!(party_xp_share(0, 5), 0);
    }

    #[test]
    fn party_share_never_beats_soloing() {
        // Regression for the remainder exploit: with two or more members no
        // share — the killer's included — may exceed the solo award.
        for base in [1u32, 2, 5, 10, 17, 28, 107, 401] {
            for n in 2u32..=5 {
                assert!(
                    party_xp_share(base, n) <= base,
                    "share for base {base} n {n} exceeds solo"
                );
            }
        }
    }

    #[test]
    fn death_penalty_without_level_down() {
        // Level 2 band = 20, penalty = 3.
        let result = apply_death_penalty(30);
        assert_eq!(result.old_level, 2);
        assert_eq!(result.new_level, 2);
        assert_eq!(result.xp_penalty, 3);
        assert_eq!(result.new_xp, 27);
        assert!(!result.leveled_down);
    }

    #[test]
    fn death_penalty_with_single_level_down_and_recovery_floor() {
        // Old XP 20 (Lv2 start). Penalty 3 => 17, level-down to Lv1.
        // Lv1 band = 20, recovery floor = 6. max(17, 6) = 17.
        let result = apply_death_penalty(20);
        assert_eq!(result.old_level, 2);
        assert_eq!(result.new_level, 1);
        assert_eq!(result.new_xp, 17);
        assert!(result.leveled_down);
    }

    #[test]
    fn death_penalty_never_levels_down_from_level_one() {
        let result = apply_death_penalty(1);
        assert_eq!(result.old_level, 1);
        assert_eq!(result.new_level, 1);
        assert_eq!(result.new_xp, 0);
        assert!(!result.leveled_down);
    }
}
