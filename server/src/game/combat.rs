use rand::Rng;

pub struct AttackResult {
    pub hit: bool,
    pub roll: u8,
    pub damage: u32,
}

/// Parse dice notation like "1d6", "2d8" into (count, sides)
fn parse_damage_roll(damage_roll: &str) -> (u32, u32) {
    let parts: Vec<&str> = damage_roll.split('d').collect();
    if parts.len() == 2 {
        let count = parts[0].parse().unwrap_or(1);
        let sides = parts[1].parse().unwrap_or(6);
        (count, sides)
    } else {
        (1, 6) // default 1d6
    }
}

/// Roll dice notation like "6d4" and return the summed total (minimum 1).
/// Used for consumable healing where there's no attack roll, just the dice.
pub fn roll_dice(notation: &str) -> u32 {
    let (count, sides) = parse_damage_roll(notation);
    let mut rng = rand::thread_rng();
    let mut total: u32 = 0;
    for _ in 0..count {
        total += rng.gen_range(1..=sides);
    }
    total.max(1)
}

pub fn ability_modifier(score: u8) -> i32 {
    (i32::from(score) - 10).div_euclid(2)
}

pub fn level_attack_bonus(level: u32) -> i32 {
    (level / 2) as i32
}

pub fn player_attack_bonus(
    player_level: u32,
    strength: u8,
    weapon_enchant: i32,
    weapon_skill_bonus: i32,
) -> i32 {
    level_attack_bonus(player_level)
        + ability_modifier(strength)
        + weapon_enchant
        + weapon_skill_bonus
}

pub fn player_damage_bonus(strength: u8, weapon_enchant: i32) -> i32 {
    ability_modifier(strength) + weapon_enchant
}

pub const WEAPON_SKILL_MISS_XP: u64 = 5;
pub const WEAPON_SKILL_HIT_XP: u64 = 10;
pub const WEAPON_SKILL_KILL_XP: u64 = 20;
pub const SHIELD_SKILL_HIT_XP: u64 = 5;
pub const SHIELD_SKILL_AVOID_XP: u64 = 10;
pub const ARMOR_SKILL_HIT_XP: u64 = 5;

pub fn weapon_skill_attack_xp(hit: bool, killing_blow: bool) -> u64 {
    if killing_blow {
        WEAPON_SKILL_KILL_XP
    } else if hit {
        WEAPON_SKILL_HIT_XP
    } else {
        WEAPON_SKILL_MISS_XP
    }
}

/// Shield practice comes from every accepted monster swing: turning the blow
/// aside is worth more, while absorbing a hit still teaches the defender.
pub fn shield_skill_defense_xp(monster_hit: bool) -> u64 {
    if monster_hit {
        SHIELD_SKILL_HIT_XP
    } else {
        SHIELD_SKILL_AVOID_XP
    }
}

/// Body-armor practice requires a landed, server-resolved blow. A miss may
/// train Shield's deflection, but it never reached the worn armor.
pub fn armor_skill_defense_xp(monster_hit: bool) -> u64 {
    if monster_hit {
        ARMOR_SKILL_HIT_XP
    } else {
        0
    }
}

pub fn monster_max_health_for_level(level: u8) -> u32 {
    // Average of level d8, rounded up: Lv3 -> 14, Lv4 -> 18.
    (u32::from(level).max(1) * 9).div_ceil(2)
}

pub fn monster_damage_roll_for_level(level: u8) -> &'static str {
    match level {
        0..=2 => "1d4",
        3..=4 => "1d6",
        5..=6 => "1d8",
        7..=8 => "2d6",
        9..=12 => "2d8",
        _ => "3d6",
    }
}

pub fn roll_attack(
    attack_bonus: i32,
    target_guard: i32,
    damage_roll: &str,
    damage_bonus: i32,
) -> AttackResult {
    roll_attack_with_extra_damage_roll(attack_bonus, target_guard, damage_roll, None, damage_bonus)
}

pub fn roll_attack_with_extra_damage_roll(
    attack_bonus: i32,
    target_guard: i32,
    damage_roll: &str,
    extra_damage_roll: Option<&str>,
    damage_bonus: i32,
) -> AttackResult {
    let mut rng = rand::thread_rng();

    let roll = rng.gen_range(1..=20);
    let hit = i32::from(roll) + attack_bonus > target_guard;
    let mut damage = 0;

    if hit {
        let mut total: i64 = i64::from(damage_bonus);
        for roll in std::iter::once(damage_roll).chain(extra_damage_roll) {
            let (count, sides) = parse_damage_roll(roll);
            for _ in 0..count {
                total += i64::from(rng.gen_range(1..=sides));
            }
        }
        // Hit always deals at least 1, even if bonus drives the roll non-positive.
        damage = total.max(1) as u32;
    }

    AttackResult { hit, roll, damage }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_damage_roll_is_added_on_hit() {
        let result = roll_attack_with_extra_damage_roll(20, 0, "1d1", Some("2d1"), 0);

        assert!(result.hit);
        assert_eq!(result.damage, 3);
    }

    #[test]
    fn extra_damage_roll_is_ignored_on_miss() {
        let result = roll_attack_with_extra_damage_roll(-20, 20, "1d1", Some("2d1"), 0);

        assert!(!result.hit);
        assert_eq!(result.damage, 0);
    }

    #[test]
    fn level_defaults_scale_monsters() {
        assert_eq!(level_attack_bonus(1), 0);
        assert_eq!(level_attack_bonus(4), 2);
        assert_eq!(monster_max_health_for_level(0), 5);
        assert_eq!(monster_max_health_for_level(3), 14);
        assert_eq!(monster_max_health_for_level(4), 18);
        assert_eq!(monster_damage_roll_for_level(3), "1d6");
        assert_eq!(monster_damage_roll_for_level(7), "2d6");
    }

    #[test]
    fn player_attack_components_stack_without_changing_damage() {
        assert_eq!(player_attack_bonus(10, 14, 3, 2), 12);
        assert_eq!(player_damage_bonus(14, 3), 5);
        assert_eq!(player_attack_bonus(10, 14, 3, 0), 10);
        assert_eq!(player_damage_bonus(14, 3), 5);
    }

    #[test]
    fn weapon_skill_xp_matches_accepted_attack_outcomes() {
        assert_eq!(weapon_skill_attack_xp(false, false), 5);
        assert_eq!(weapon_skill_attack_xp(true, false), 10);
        assert_eq!(weapon_skill_attack_xp(true, true), 20);
    }

    #[test]
    fn shield_skill_xp_rewards_avoids_but_still_trains_on_hits() {
        assert_eq!(shield_skill_defense_xp(false), 10);
        assert_eq!(shield_skill_defense_xp(true), 5);
    }
}
