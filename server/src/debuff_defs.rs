//! Debuff definitions from data-src/debuffs.csv (doc/DEBUFF.md).

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

fn one() -> f32 {
    1.0
}

#[derive(Debug, Clone, Deserialize)]
pub struct DebuffDef {
    pub id: String,
    /// Percent chance to apply when the trigger lands.
    pub chance: u32,
    #[serde(rename = "durationSecs")]
    pub duration_secs: u64,
    /// Damage dealt every second while active.
    #[serde(default)]
    pub dps: u32,
    #[serde(rename = "moveMult", default = "one")]
    pub move_mult: f32,
    #[serde(rename = "attackMult", default = "one")]
    pub attack_mult: f32,
    #[serde(rename = "carryMult", default = "one")]
    pub carry_mult: f32,
    /// Satiation activity drain multiplier.
    #[serde(rename = "drainMult", default = "one")]
    pub drain_mult: f32,
    #[serde(rename = "blocksRegen", default)]
    pub blocks_regen: bool,
}

static DEFS: LazyLock<HashMap<String, DebuffDef>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../data/debuffs.json"))
        .expect("Failed to parse debuffs.json")
});

pub fn debuff_def(id: &str) -> Option<&'static DebuffDef> {
    DEFS.get(id)
}

/// Boot-time check for CSV columns that reference a debuff id.
pub fn assert_debuff_exists(id: &str, referrer: &str) {
    assert!(
        DEFS.contains_key(id),
        "{referrer}: unknown debuff '{id}' (see data-src/debuffs.csv)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_debuffs_parse_with_defaults() {
        let poison = debuff_def("food_poisoning").unwrap();
        assert_eq!((poison.chance, poison.dps), (70, 0));
        assert_eq!(poison.drain_mult, 4.0);
        assert!(poison.blocks_regen && poison.move_mult < 1.0);
        let bleed = debuff_def("bleed").unwrap();
        assert_eq!((bleed.dps, bleed.duration_secs), (1, 8));
        assert_eq!(
            (
                bleed.move_mult,
                bleed.attack_mult,
                bleed.carry_mult,
                bleed.drain_mult
            ),
            (1.0, 1.0, 1.0, 1.0)
        );
    }
}
