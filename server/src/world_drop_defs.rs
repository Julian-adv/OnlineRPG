use crate::item_defs::ItemDefs;
use rand::Rng;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

/// One entry in the global world-drop table: a rare bonus item that any loot
/// source (monster kill, chest, broken prop) can spill in addition to its
/// normal loot. The CSV row key doubles as the item definition id to spawn.
#[derive(Debug, Clone, Deserialize)]
pub struct WorldDropEntry {
    /// Item definition id to spawn (also the CSV row key).
    pub id: String,
    /// Independent per-loot-source probability in [0, 1] that this entry drops.
    pub chance: f32,
    /// Reduced chance used when the loot source's effective level is at or
    /// below `low_level_max_level`. Set on both together or on neither.
    #[serde(rename = "lowLevelChance", default)]
    pub low_level_chance: Option<f32>,
    /// Highest effective level that still counts as low level for this entry.
    #[serde(rename = "lowLevelMaxLevel", default)]
    pub low_level_max_level: Option<u8>,
}

impl WorldDropEntry {
    /// This entry's chance for a loot source of `source_level`. A source with
    /// no level (chest, prop) always rolls the full chance.
    fn chance_for(&self, source_level: Option<u8>) -> f32 {
        match (
            self.low_level_chance,
            self.low_level_max_level,
            source_level,
        ) {
            (Some(low), Some(max), Some(level)) if level <= max => low,
            _ => self.chance,
        }
    }
}

/// The world-drop table loaded from `data/world_drop.json`. Each entry is
/// rolled independently on every loot event, so a single kill can yield zero,
/// one, or (rarely) several bonus drops.
#[derive(Debug, Clone)]
pub struct WorldDropDefs {
    /// Sorted by id so rolls are deterministic given the same RNG sequence.
    entries: Arc<Vec<WorldDropEntry>>,
}

impl WorldDropDefs {
    /// Load and validate the table against `item_defs`. Every entry id must
    /// name a real item; a typo'd or stale entry panics at startup rather than
    /// silently failing to drop on every loot event.
    pub fn load(item_defs: &ItemDefs) -> Self {
        let data = include_str!("../../data/world_drop.json");
        let map: HashMap<String, WorldDropEntry> =
            serde_json::from_str(data).expect("Failed to parse world_drop.json");

        let mut entries: Vec<WorldDropEntry> = map.into_values().collect();
        entries.sort_by(|a, b| a.id.cmp(&b.id));

        info!("Loaded {} world drop entries", entries.len());
        for entry in &entries {
            assert!(
                item_defs.get(&entry.id).is_some(),
                "world_drop entry '{}' has no matching item definition",
                entry.id
            );
            assert_eq!(
                entry.low_level_chance.is_some(),
                entry.low_level_max_level.is_some(),
                "world_drop entry '{}' needs both lowLevelChance and lowLevelMaxLevel, or neither",
                entry.id
            );
            let low_rule = if let (Some(low), Some(max)) =
                (entry.low_level_chance, entry.low_level_max_level)
            {
                assert!(
                    low <= entry.chance,
                    "world_drop entry '{}' has a low-level chance above its full chance",
                    entry.id
                );
                format!(" (lvl<={max}: {low})")
            } else {
                String::new()
            };
            info!("  {} - chance:{}{low_rule}", entry.id, entry.chance);
        }

        Self {
            entries: Arc::new(entries),
        }
    }

    /// Roll every entry independently and return the item ids that dropped.
    /// `source_level` is the effective level of what yielded the loot — a
    /// killed monster's depth-scaled level — and lets an entry pay out less
    /// on weak prey. `None` (chest, prop) rolls every entry at full chance.
    pub fn roll<R: Rng>(&self, rng: &mut R, source_level: Option<u8>) -> Vec<String> {
        roll_independent(
            self.entries
                .iter()
                .map(|e| (e.id.as_str(), e.chance_for(source_level))),
            rng,
        )
    }
}

/// Roll each (id, chance) entry independently and return the ids that
/// dropped — the one mechanic behind world drops and chest loot, so their
/// semantics cannot diverge. Entry order must already be deterministic.
pub fn roll_independent<'a, R: Rng>(
    entries: impl IntoIterator<Item = (&'a str, f32)>,
    rng: &mut R,
) -> Vec<String> {
    entries
        .into_iter()
        .filter(|&(_, chance)| rng.gen::<f32>() < chance)
        .map(|(id, _)| id.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(low: Option<(f32, u8)>) -> WorldDropEntry {
        WorldDropEntry {
            id: "scroll_of_enchant_weapon".to_string(),
            chance: 0.01,
            low_level_chance: low.map(|(c, _)| c),
            low_level_max_level: low.map(|(_, l)| l),
        }
    }

    #[test]
    fn low_level_sources_roll_the_reduced_chance() {
        let scaled = entry(Some((0.005, 8)));
        assert_eq!(scaled.chance_for(Some(8)), 0.005);
        assert_eq!(scaled.chance_for(Some(1)), 0.005);
        assert_eq!(scaled.chance_for(Some(9)), 0.01);
        // Chests and props carry no level: always the full chance.
        assert_eq!(scaled.chance_for(None), 0.01);
        // An entry without the rule ignores the level entirely.
        assert_eq!(entry(None).chance_for(Some(1)), 0.01);
    }
}
