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
            info!("  {} - chance:{}", entry.id, entry.chance);
        }

        Self {
            entries: Arc::new(entries),
        }
    }

    /// Roll every entry independently and return the item ids that dropped.
    pub fn roll<R: Rng>(&self, rng: &mut R) -> Vec<String> {
        roll_independent(self.entries.iter().map(|e| (e.id.as_str(), e.chance)), rng)
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
