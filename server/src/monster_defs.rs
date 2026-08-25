use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

use crate::game::combat;

fn default_weapon_drop_chance() -> f32 {
    1.0
}

/// Parse a `drops` column ("id:chance|id:chance"); a malformed entry fails
/// the boot here rather than a kill later.
fn parse_drops(id: &str, drops: Option<&str>) -> Vec<(String, f32)> {
    drops
        .unwrap_or("")
        .split('|')
        .filter(|s| !s.is_empty())
        .map(|entry| {
            let (item, chance) = entry.split_once(':').unwrap_or_else(|| {
                panic!("monster '{id}': drops entry '{entry}' must be id:chance")
            });
            let chance: f32 = chance.parse().unwrap_or_else(|_| {
                panic!("monster '{id}': drops entry '{entry}' has a bad chance")
            });
            assert!(
                (0.0..=1.0).contains(&chance),
                "monster '{id}': drops entry '{entry}' chance out of [0,1]"
            );
            (item.to_string(), chance)
        })
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct MonsterDefinition {
    pub id: String,
    pub name: String,
    pub model: String,
    #[serde(default)]
    pub health: Option<u32>,
    pub level: u8,
    pub guard: u8,
    #[serde(rename = "attackBonus", default)]
    pub attack_bonus: Option<i32>,
    #[serde(rename = "walkSpeed")]
    pub walk_speed: f32,
    #[serde(rename = "runSpeed")]
    pub run_speed: f32,
    #[serde(rename = "attackRange")]
    pub attack_range: f32,
    #[serde(rename = "chaseRange")]
    pub chase_range: f32,
    #[serde(rename = "attackCooldown")]
    pub attack_cooldown: u32,
    /// Behavior tree name (data-src/behavior_trees.json).
    #[serde(default)]
    pub behavior: Option<String>,
    #[serde(rename = "attackImpactDelay", default)]
    pub attack_impact_delay: u32,
    #[serde(rename = "attackDamageTextDelay", default)]
    pub attack_damage_text_delay: u32,
    #[serde(rename = "damageRoll")]
    #[serde(default)]
    pub damage_roll: Option<String>,
    /// Debuff id rolled on every hit (doc/DEBUFF.md).
    #[serde(rename = "hitDebuff", default)]
    pub hit_debuff: Option<String>,
    #[serde(default)]
    pub weapon: Option<String>,
    #[serde(rename = "weaponDropChance", default = "default_weapon_drop_chance")]
    pub weapon_drop_chance: f32,
    #[serde(rename = "weaponBone", default)]
    pub weapon_bone: Option<String>,
    #[serde(rename = "animIdle")]
    pub anim_idle: String,
    #[serde(rename = "animWalk")]
    pub anim_walk: String,
    #[serde(rename = "animRun")]
    pub anim_run: String,
    #[serde(rename = "animAttack")]
    pub anim_attack: String,
    #[serde(rename = "animAttackIdle", default)]
    pub anim_attack_idle: Option<String>,
    // Empty for monsters on the shared character packs — those have no hit clip.
    #[serde(rename = "animHit", default)]
    pub anim_hit: String,
    #[serde(rename = "animDie")]
    pub anim_die: String,
    #[serde(rename = "animDead")]
    pub anim_dead: String,
    #[serde(default)]
    pub material: Option<String>,
    /// Bonus drops rolled per kill, as "item_id:chance|item_id:chance".
    #[serde(default)]
    pub drops: Option<String>,
    /// `drops` parsed once at load; handlers never re-split the string.
    #[serde(skip)]
    drop_entries: Vec<(String, f32)>,
}

impl MonsterDefinition {
    pub fn max_health(&self) -> u32 {
        self.health
            .unwrap_or_else(|| combat::monster_max_health_for_level(self.level))
    }

    pub fn attack_bonus(&self) -> i32 {
        self.attack_bonus
            .unwrap_or_else(|| combat::monster_attack_bonus(self.level))
    }

    /// Depth-scaled copy: carries any csv override up by the levels depth
    /// added, instead of discarding it.
    pub fn attack_bonus_at(&self, level: u8) -> i32 {
        self.attack_bonus() + combat::monster_attack_bonus(level)
            - combat::monster_attack_bonus(self.level)
    }

    /// Parsed `drops` column: (item_def_id, chance) pairs.
    pub fn drop_entries(&self) -> &[(String, f32)] {
        &self.drop_entries
    }

    pub fn damage_roll(&self) -> String {
        self.damage_roll
            .clone()
            .unwrap_or_else(|| combat::monster_damage_roll_for_level(self.level).to_string())
    }
}

#[derive(Debug, Clone)]
pub struct MonsterDefs {
    defs: Arc<HashMap<String, MonsterDefinition>>,
}

impl MonsterDefs {
    pub fn load() -> Self {
        let data = include_str!("../../data/monsters.json");
        let mut defs: HashMap<String, MonsterDefinition> =
            serde_json::from_str(data).expect("Failed to parse monsters.json");
        for (id, def) in defs.iter_mut() {
            def.drop_entries = parse_drops(id, def.drops.as_deref());
        }

        info!("Loaded {} monster definitions", defs.len());
        for (id, def) in &defs {
            if let Some(debuff) = &def.hit_debuff {
                crate::debuff_defs::assert_debuff_exists(debuff, &format!("monster '{id}'"));
            }
            for (item_id, _) in def.drop_entries() {
                assert!(
                    crate::item_defs::item_defs().get(item_id).is_some(),
                    "monster '{id}' drops unknown item '{item_id}'"
                );
            }
            info!(
                "  {} - level:{} HP:{} guard:{} attackBonus:{} walkSpeed:{} runSpeed:{} attackRange:{} chaseRange:{} cooldown:{}ms damage:{}",
                id, def.level, def.max_health(), def.guard, def.attack_bonus(), def.walk_speed, def.run_speed,
                def.attack_range, def.chase_range, def.attack_cooldown,
                def.damage_roll()
            );
        }

        Self {
            defs: Arc::new(defs),
        }
    }

    pub fn get(&self, monster_type: &str) -> Option<&MonsterDefinition> {
        self.defs.get(monster_type)
    }

    pub fn ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.defs.keys().map(String::as_str).collect();
        ids.sort_unstable();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kobolds_drop_apples_rarely() {
        let defs = MonsterDefs::load();
        let kobold = defs.get("kobold").expect("kobold def");
        assert_eq!(kobold.drop_entries(), [("apple".to_string(), 0.01)]);
    }

    #[test]
    fn the_gnoll_claws_inflict_bleeding() {
        let defs = MonsterDefs::load();
        let gnoll = defs.get("gnoll").expect("gnoll def");
        assert_eq!(gnoll.hit_debuff.as_deref(), Some("bleed"));
        assert_eq!(gnoll.damage_roll(), "2d6");
    }

    #[test]
    fn depth_scaling_carries_a_hand_tuned_attack_bonus() {
        let defs = MonsterDefs::load();
        let hobgoblin = defs.get("hobgoblin").expect("hobgoblin def");
        let depth_gain = i32::from(8 - hobgoblin.level);
        assert_eq!(
            hobgoblin.attack_bonus_at(8) - hobgoblin.attack_bonus(),
            depth_gain,
            "depth adds its levels on top of whatever the row says"
        );

        let mut tuned = hobgoblin.clone();
        tuned.attack_bonus = Some(hobgoblin.attack_bonus() + 2);
        assert_eq!(
            tuned.attack_bonus_at(8),
            hobgoblin.attack_bonus_at(8) + 2,
            "the override survives the scaling"
        );
    }
}
