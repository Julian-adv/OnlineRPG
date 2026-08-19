//! Server view of the shared dungeon entrance registry
//! (`onlinerpg_shared::dungeon`, embedded from data-src/dungeons.csv). The
//! registry itself is shared because it is a contract: `entrance_at` is what
//! `validated_dungeon_floor` accepts a claimed dungeon floor by, so the agent
//! and the browser must answer it identically or the server resets their floor
//! mid-descent.
//!
//! What stays here is the one genuinely server-side part: validating
//! `chestDrops` against the item table.

use tracing::info;

pub use onlinerpg_shared::dungeon::DungeonEntranceDef;
use onlinerpg_shared::dungeon::{entrance, entrance_at, entrances};
use onlinerpg_shared::Position;

#[derive(Debug, Clone)]
pub struct DungeonDefs;

impl DungeonDefs {
    /// Validate against the item and monster tables: every `chestDrops` entry
    /// must name a real item and `boss` a real monster; a typo'd entry panics
    /// at startup rather than silently handing out a broken item or spawning
    /// nothing (mirrors the world-drop table).
    pub fn load(
        item_defs: &crate::item_defs::ItemDefs,
        monster_defs: &crate::monster_defs::MonsterDefs,
    ) -> Self {
        info!("Loaded {} dungeon entrances", entrances().len());
        for def in entrances() {
            info!(
                "  {} \"{}\" at ({:.1}, {:.1}, {:.1})",
                def.id, def.name, def.x, def.y, def.z
            );
            for chest_drop in &def.chest_drops {
                let item = item_defs.get(chest_drop).unwrap_or_else(|| {
                    panic!(
                        "dungeon '{}' chestDrops entry '{}' has no matching item definition",
                        def.id, chest_drop
                    )
                });
                assert_eq!(
                    item.chest_tier,
                    Some(def.chest_tier),
                    "dungeon '{}' signature '{}' must carry the dungeon's chestTier",
                    def.id,
                    chest_drop
                );
            }
            assert!(
                monster_defs.get(&def.boss).is_some(),
                "dungeon '{}' boss '{}' has no matching monster definition",
                def.id,
                def.boss
            );
        }

        // An opted-in item at or below the deepest built dungeon must drop
        // somewhere — as a signature or via a positive roll chance. Items
        // tiered above every built dungeon are legitimately parked for a
        // future dungeon (doc/ITEM_TIERS.md).
        let max_tier = entrances().iter().map(|d| d.chest_tier).max().unwrap_or(0);
        let signatures: std::collections::HashSet<&str> = entrances()
            .iter()
            .flat_map(|d| d.chest_drops.iter().map(String::as_str))
            .collect();
        for item in item_defs.all() {
            let Some(tier) = item.chest_tier else {
                continue;
            };
            assert!(
                tier > max_tier
                    || signatures.contains(item.id.as_str())
                    || item.chest_chance.is_some_and(|c| c > 0.0),
                "item '{}' (chestTier {tier}) is neither a signature drop nor has a chestChance — it can never drop",
                item.id
            );
        }
        Self
    }

    pub fn get(&self, id: &str) -> Option<&'static DungeonEntranceDef> {
        entrance(id)
    }

    pub fn all(&self) -> impl Iterator<Item = &'static DungeonEntranceDef> {
        entrances().iter()
    }

    /// Entrance whose grid footprint contains the given XZ position.
    pub fn entrance_at(&self, x: f32, z: f32) -> Option<&'static DungeonEntranceDef> {
        entrance_at(x, z)
    }
}

/// Where a position is, for log lines: the dungeon and depth underground, the
/// house floor above ground level, or the open surface.
pub fn place_label(position: &Position, floor_level: i8) -> String {
    if floor_level < 0 {
        return match entrance_at(position.x, position.z) {
            Some(entrance) => format!("dungeon '{}' depth {}", entrance.id, -floor_level),
            None => format!("dungeon floor {floor_level}"),
        };
    }
    if floor_level > 0 {
        return format!("floor {floor_level}");
    }
    "surface".to_string()
}
