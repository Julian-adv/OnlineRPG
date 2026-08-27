use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;
use tracing::info;

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct MerchantDefinition {
    pub id: String,
    #[serde(rename = "npcName")]
    pub npc_name: String,
    /// Percentage of base price the merchant pays when a player sells.
    #[serde(rename = "sellRatePercent")]
    pub sell_rate_percent: u32,
    /// Item def ids the merchant sells (unlimited stock).
    #[serde(deserialize_with = "crate::semicolon_list::deserialize")]
    pub catalog: Vec<String>,
}

impl MerchantDefinition {
    pub fn sells(&self, item_def_id: &str) -> bool {
        self.catalog.iter().any(|id| id == item_def_id)
    }
}

/// Merchant definitions keyed by NPC name. NPCs are agent-controlled players,
/// so the stable identity the server sees is the character name.
pub struct MerchantDefs {
    by_npc_name: HashMap<String, MerchantDefinition>,
}

impl MerchantDefs {
    fn load() -> Self {
        let data = include_str!("../../data/merchants.json");
        let by_id: HashMap<String, MerchantDefinition> =
            serde_json::from_str(data).expect("Failed to parse merchants.json");

        // Money-pump invariant: even with maximum haggling in both
        // directions and the price index at its floor, buying must always
        // cost more than selling pays.
        let index_min = crate::world_config::world_config()
            .pricing
            .index_min_percent();
        for def in by_id.values() {
            assert!(
                crate::game_state::band_invariant_holds(def.sell_rate_percent, index_min),
                "merchant {} sellRatePercent {} breaks the haggling band invariant",
                def.id,
                def.sell_rate_percent
            );
        }

        info!("Loaded {} merchant definition(s)", by_id.len());
        let by_npc_name = by_id
            .into_values()
            .map(|def| (def.npc_name.clone(), def))
            .collect();

        Self { by_npc_name }
    }

    pub fn get_by_npc_name(&self, npc_name: &str) -> Option<&MerchantDefinition> {
        self.by_npc_name.get(npc_name)
    }
}

pub fn merchant_defs() -> &'static MerchantDefs {
    static DEFS: OnceLock<MerchantDefs> = OnceLock::new();
    DEFS.get_or_init(MerchantDefs::load)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_merchant_sells_the_fishing_rod() {
        // Fishing is only reachable if a player can actually buy a rod — the
        // rod is not a starter item, and it is (correctly) excluded from
        // dungeon loot. Some merchant must stock it. See doc/FISHING.md.
        let defs = MerchantDefs::load();
        assert!(
            defs.by_npc_name.values().any(|m| m.sells("fishing_rod")),
            "no merchant sells fishing_rod — the rod would be unobtainable"
        );
    }

    /// A merchant lives in two CSVs — the shop in `merchants.csv`, the NPC
    /// identity in `npcs.csv` — and adding only one of them leaves a shop no
    /// agent can ever run.
    #[test]
    fn every_merchant_has_an_npc_registry_entry() {
        let registry: serde_json::Value =
            serde_json::from_str(include_str!("../../data/npcs.json")).unwrap();
        for name in MerchantDefs::load().by_npc_name.keys() {
            assert!(
                registry
                    .as_object()
                    .unwrap()
                    .values()
                    .any(|npc| npc["npcName"] == name.as_str()),
                "merchant {name} has no entry in npcs.csv"
            );
        }
    }

    /// Rica sleeps at night, so trade would stop with the town asleep unless
    /// someone else keeps a counter open (see doc/TODO.md).
    #[test]
    fn a_merchant_stocks_night_essentials() {
        let defs = MerchantDefs::load();
        let night = defs
            .by_npc_name
            .get("Wick")
            .expect("the night merchant is missing from merchants.csv");
        for essential in ["torch", "healing_potion", "bread"] {
            assert!(
                night.sells(essential),
                "the night merchant does not stock {essential}"
            );
        }
    }
}
