use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;
use tracing::info;

/// One LLM NPC in the game-data registry (`data/npcs.json`). Every NPC has
/// a row; the trading fields are optional — per `doc/ECONOMY.md`, any NPC
/// *may* trade as a resident (economy phase 3): finite wallet refilled by a
/// salary, buys only its wishlist (at a premium), and sells from its real
/// inventory. Money pumps are blocked structurally: wishlist items are kept
/// (never resold), and only non-wishlist bag items are for sale.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct NpcDefinition {
    pub id: String,
    #[serde(rename = "npcName")]
    pub npc_name: String,
    /// Item def ids this NPC wants to buy from players. Empty = the NPC
    /// does not trade as a resident.
    #[serde(default, deserialize_with = "crate::semicolon_list::deserialize")]
    pub wishlist: Vec<String>,
    /// Percentage of base price paid for wishlist items. Above 100 by
    /// design: buying from a merchant and delivering here is intended
    /// content, capped by the NPC's wallet.
    #[serde(rename = "wishlistRatePercent", default)]
    pub wishlist_rate_percent: u32,
    /// Gold credited per game day (smallest unit) — the controlled faucet
    /// funding this NPC's wallet.
    #[serde(rename = "salaryPerDay", default)]
    pub salary_per_day: i64,
    /// Salary stops accumulating past this wallet balance.
    #[serde(rename = "walletCap", default)]
    pub wallet_cap: i64,
    /// Personal belongings, hidden from walk-in stock: only a player the
    /// NPC personally offered a deal on the item may buy it. Refilled into
    /// the NPC's bag on join.
    #[serde(default, deserialize_with = "crate::semicolon_list::deserialize")]
    pub keepsakes: Vec<String>,
}

impl NpcDefinition {
    /// Whether this NPC trades as a resident (has a wishlist or keepsakes).
    pub fn trades(&self) -> bool {
        !self.wishlist.is_empty() || !self.keepsakes.is_empty()
    }

    pub fn wants(&self, item_def_id: &str) -> bool {
        self.wishlist.iter().any(|id| id == item_def_id)
    }

    pub fn keeps(&self, item_def_id: &str) -> bool {
        self.keepsakes.iter().any(|id| id == item_def_id)
    }
}

/// NPC registry keyed by NPC name (NPCs are agent-controlled players, so
/// the stable identity is the character name).
pub struct NpcDefs {
    by_npc_name: HashMap<String, NpcDefinition>,
}

impl NpcDefs {
    fn load() -> Self {
        let data = include_str!("../../data/npcs.json");
        let by_id: HashMap<String, NpcDefinition> =
            serde_json::from_str(data).expect("Failed to parse npcs.json");

        for def in by_id.values() {
            // A name must resolve to exactly one trading model: an NPC is
            // either a merchant (catalog shop) or a resident trader.
            assert!(
                !def.trades()
                    || crate::merchant_defs::merchant_defs()
                        .get_by_npc_name(&def.npc_name)
                        .is_none(),
                "NPC {} is defined both as a merchant and a resident trader",
                def.npc_name
            );
            // Wishlist items are never resold; a keepsake is offer-only
            // sellable — the two sets must not overlap.
            assert!(
                !def.wishlist.iter().any(|id| def.keeps(id)),
                "NPC {} lists an item as both wishlist and keepsake",
                def.npc_name
            );
        }

        info!("Loaded {} NPC definition(s)", by_id.len());
        let by_npc_name = by_id
            .into_values()
            .map(|def| (def.npc_name.clone(), def))
            .collect();

        Self { by_npc_name }
    }

    /// The NPC's resident-trader definition, if it trades.
    pub fn get_trader_by_npc_name(&self, npc_name: &str) -> Option<&NpcDefinition> {
        self.by_npc_name.get(npc_name).filter(|def| def.trades())
    }
}

pub fn npc_defs() -> &'static NpcDefs {
    static DEFS: OnceLock<NpcDefs> = OnceLock::new();
    DEFS.get_or_init(NpcDefs::load)
}
