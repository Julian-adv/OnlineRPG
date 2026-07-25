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

#[derive(Debug, Clone)]
pub struct DungeonDefs;

impl DungeonDefs {
    /// Validate against `item_defs`: every `chestDrops` entry must name a real
    /// item; a typo'd entry panics at startup rather than silently handing out
    /// a broken item (mirrors the world-drop table).
    pub fn load(item_defs: &crate::item_defs::ItemDefs) -> Self {
        info!("Loaded {} dungeon entrances", entrances().len());
        for def in entrances() {
            info!(
                "  {} \"{}\" at ({:.1}, {:.1}, {:.1})",
                def.id, def.name, def.x, def.y, def.z
            );
            for chest_drop in &def.chest_drops {
                assert!(
                    item_defs.get(chest_drop).is_some(),
                    "dungeon '{}' chestDrops entry '{}' has no matching item definition",
                    def.id,
                    chest_drop
                );
            }
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
