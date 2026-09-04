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

/// Monsters scale on level, not the player's `level / 2`: guard climbs with
/// gear far faster than with levels (doc/COMBAT.md).
pub fn monster_attack_bonus(level: u8) -> i32 {
    i32::from(level)
}

/// Dungeon floors with no place bonus — the whole tier-1 dungeon, so a
/// beginner's first basement plays at the monsters' own numbers.
pub const DUNGEON_SAFE_DEPTH: i32 = 5;
/// To-hit per floor below the safe depth.
pub const DUNGEON_DEPTH_ATTACK_BONUS: i32 = 3;
/// Surface metres from town with no place bonus.
pub const SURFACE_SAFE_METERS: f32 = 300.0;
/// Metres beyond the safe ring per point of to-hit.
pub const SURFACE_METERS_PER_ATTACK_BONUS: f32 = 20.0;

/// The to-hit a *place* lends its monsters. Player guard climbs with gear and
/// armour enchants far past any monster level, so danger has to ride where you
/// hunt rather than what you meet: dungeon depth, or distance from town on the
/// surface (doc/COMBAT.md). Each has a free band — tier-1 floors, the town's
/// surroundings — so the bonus only ever erodes guard someone chose to test.
pub fn place_attack_bonus(floor_level: i8, town_distance: f32) -> i32 {
    if floor_level < 0 {
        return (i32::from(floor_level.unsigned_abs()) - DUNGEON_SAFE_DEPTH).max(0)
            * DUNGEON_DEPTH_ATTACK_BONUS;
    }
    ((town_distance - SURFACE_SAFE_METERS).max(0.0) / SURFACE_METERS_PER_ATTACK_BONUS) as i32
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

/// Only bounds the loop — five extra rolls already reach a total of 120.
const MAX_EXPLOSIONS: u32 = 5;

/// Roll the attack die: an exploding d20 — a natural 20 rolls again and adds,
/// repeating while 20s come up. Returns the first die (what the client shows)
/// and the total. Keeps every guard reachable without flooring each attack at
/// the flat 5% a nat-20 auto-hit would impose (doc/COMBAT.md).
fn roll_exploding_d20<R: Rng>(rng: &mut R) -> (u8, i32) {
    let first = rng.gen_range(1..=20);
    let mut total = i32::from(first);
    let mut last = first;
    for _ in 0..MAX_EXPLOSIONS {
        if last != 20 {
            break;
        }
        last = rng.gen_range(1..=20);
        total += i32::from(last);
    }
    (first, total)
}

/// Whether an attack total lands.
fn resolves_as_hit(roll_total: i32, attack_bonus: i32, target_guard: i32) -> bool {
    roll_total + attack_bonus > target_guard
}

pub fn roll_attack_with_extra_damage_roll(
    attack_bonus: i32,
    target_guard: i32,
    damage_roll: &str,
    extra_damage_roll: Option<&str>,
    damage_bonus: i32,
) -> AttackResult {
    let mut rng = rand::thread_rng();

    let (roll, roll_total) = roll_exploding_d20(&mut rng);
    let hit = resolves_as_hit(roll_total, attack_bonus, target_guard);
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
        // Explosions can still land, so scan for a miss.
        let miss = (0..200)
            .map(|_| roll_attack_with_extra_damage_roll(-20, 20, "1d1", Some("2d1"), 0))
            .find(|result| !result.hit)
            .expect("a miss within 200 rolls");

        assert_eq!(miss.damage, 0);
    }

    /// Exact hit probability under the exploding die, by enumeration.
    fn hit_probability(attack_bonus: i32, target_guard: i32) -> f64 {
        // P(total ≥ t) for an exploding d20, walking the 20-point bands.
        fn at_least(t: i32) -> f64 {
            if t <= 1 {
                return 1.0;
            }
            if t <= 20 {
                return f64::from(21 - t) / 20.0;
            }
            at_least(t - 20) / 20.0
        }
        at_least(target_guard - attack_bonus + 1)
    }

    #[test]
    fn a_short_attacker_keeps_a_chance_that_shrinks_with_the_gap() {
        // Below a natural 20 the die never explodes, so the plain comparison
        // still decides.
        assert!(resolves_as_hit(19, 2, 20));
        assert!(!resolves_as_hit(18, 2, 20));

        // The design anchors (doc/COMBAT.md): a kobold (+1) lands on guard 40
        // once in 400 swings, and no gap ever closes the door entirely.
        assert!((hit_probability(1, 40) - 0.0025).abs() < 1e-9);
        assert!(hit_probability(1, 50) < 0.0015);
        assert!(hit_probability(0, 200) > 0.0);

        // The implementation matches those odds. Guard 24 vs +2 is 4.5%: 20
        // falls two short, so the explosion has to roll a 3 or better.
        let mut rng = rand::thread_rng();
        let mut hits = 0;
        const TRIALS: u32 = 200_000;
        for _ in 0..TRIALS {
            let (first, total) = roll_exploding_d20(&mut rng);
            assert!((1..=20).contains(&first), "the reported die stays a d20");
            assert!(total >= i32::from(first));
            if resolves_as_hit(total, 2, 24) {
                hits += 1;
            }
        }
        // A 1%-wide window around 4.5% is ~20 sigma at this sample size.
        let rate = f64::from(hits) / f64::from(TRIALS);
        assert!(
            (rate - hit_probability(2, 24)).abs() < 0.01,
            "hit rate {rate} off the exact odds"
        );
    }

    #[test]
    fn monsters_attack_a_level_ahead_of_players() {
        // Players stay on level/2; monsters track the level itself so gear
        // growth doesn't outrun them.
        assert_eq!(level_attack_bonus(10), 5);
        assert_eq!(monster_attack_bonus(10), 10);
        // The Warrens boss vs a leather-set graduate (guard 21): 45%.
        let landed = (1..=20)
            .filter(|roll| resolves_as_hit(*roll, monster_attack_bonus(10), 21))
            .count();
        assert_eq!(landed, 9);
    }

    /// Where you hunt is the difficulty dial: free in the tier-1 basement and
    /// around town, climbing past any guard on the deepest floors.
    #[test]
    fn place_lends_monsters_their_to_hit() {
        // The whole tier-1 dungeon and the town's surroundings lend nothing.
        assert_eq!(place_attack_bonus(-5, 0.0), 0);
        assert_eq!(place_attack_bonus(0, 300.0), 0);
        // Depth counts floors, not metres — the surface distance under a
        // dungeon never reaches its monsters.
        assert_eq!(place_attack_bonus(-6, 9_999.0), 3);
        assert_eq!(place_attack_bonus(-10, 0.0), 15);
        assert_eq!(place_attack_bonus(-15, 0.0), 30);
        assert_eq!(place_attack_bonus(0, 560.0), 13);
        let landed = |level, bonus, guard| {
            (1..=20)
                .filter(|roll| resolves_as_hit(*roll, monster_attack_bonus(level) + bonus, guard))
                .count()
        };
        // A fully enchanted guard-61 player and a depth-scaled level-14
        // troll: untouchable in town, hit 15% of the time fifteen floors
        // down. Nothing about the monster changed — only where it stands.
        assert_eq!(landed(14, place_attack_bonus(0, 0.0), 61), 0);
        assert_eq!(landed(14, place_attack_bonus(-15, 0.0), 61), 3);
        // The design anchor: a shop-geared guard-15 beginner on the tier-1
        // bottom floor takes an orc's swing at coin-flip odds.
        assert_eq!(landed(4, place_attack_bonus(-5, 0.0), 15), 9);
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
}
