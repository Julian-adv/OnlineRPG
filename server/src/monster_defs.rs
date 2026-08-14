use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

use crate::game::combat;

fn default_weapon_drop_chance() -> f32 {
    1.0
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
    #[serde(rename = "attackImpactDelay", default)]
    pub attack_impact_delay: u32,
    #[serde(rename = "attackDamageTextDelay", default)]
    pub attack_damage_text_delay: u32,
    #[serde(rename = "damageRoll")]
    #[serde(default)]
    pub damage_roll: Option<String>,
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
}

impl MonsterDefinition {
    pub fn max_health(&self) -> u32 {
        self.health
            .unwrap_or_else(|| combat::monster_max_health_for_level(self.level))
    }

    pub fn attack_bonus(&self) -> i32 {
        self.attack_bonus
            .unwrap_or_else(|| combat::level_attack_bonus(u32::from(self.level)))
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
        let defs: HashMap<String, MonsterDefinition> =
            serde_json::from_str(data).expect("Failed to parse monsters.json");

        info!("Loaded {} monster definitions", defs.len());
        for (id, def) in &defs {
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
