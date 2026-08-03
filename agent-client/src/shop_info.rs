//! Trade knowledge injected into trading NPC system prompts. The server is
//! the authority on prices and haggling limits (`server/src/game_state/`);
//! this is the same data given to the LLM purely for roleplay, per
//! `doc/ECONOMY.md`. Game data is embedded at compile time like the server
//! does, so the agent-client needs no runtime path to `data/`.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Deserialize)]
struct MerchantRow {
    #[serde(rename = "npcName")]
    npc_name: String,
    #[serde(rename = "sellRatePercent")]
    sell_rate_percent: u32,
    catalog: String,
}

/// One row of the NPC registry (`data/npcs.json`). The trading fields are
/// optional — an empty wishlist means the NPC does not trade as a resident.
#[derive(Deserialize)]
pub struct NpcRow {
    #[serde(rename = "npcName")]
    pub npc_name: String,
    /// Role class: picks the prompt template and the auto-created
    /// character's class (e.g. "guard", "merchant").
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    wishlist: String,
    #[serde(rename = "wishlistRatePercent", default)]
    wishlist_rate_percent: u32,
    #[serde(rename = "salaryPerDay", default)]
    salary_per_day: i64,
    #[serde(rename = "walletCap", default)]
    wallet_cap: i64,
}

/// Registry lookup by NPC id, for resolving `[[npcs]]` config entries that
/// reference the registry instead of spelling every field out.
pub fn npc_by_id(id: &str) -> Option<&'static NpcRow> {
    npcs().get(id)
}

// The embedded game data is immutable at runtime, and the resident section
// is rebuilt on every LLM turn — parse each file once.
fn merchants() -> &'static HashMap<String, MerchantRow> {
    static CACHE: OnceLock<HashMap<String, MerchantRow>> = OnceLock::new();
    CACHE.get_or_init(|| {
        serde_json::from_str(include_str!("../../data/merchants.json")).unwrap_or_default()
    })
}

fn npcs() -> &'static HashMap<String, NpcRow> {
    static CACHE: OnceLock<HashMap<String, NpcRow>> = OnceLock::new();
    CACHE.get_or_init(|| {
        serde_json::from_str(include_str!("../../data/npcs.json")).unwrap_or_default()
    })
}

/// Format an amount in the smallest unit as gold/silver/copper
/// (1g = 100s = 10,000c), matching the human client's display.
pub fn format_price(copper: i64) -> String {
    let g = copper / 10_000;
    let s = (copper % 10_000) / 100;
    let c = copper % 100;
    let mut parts = Vec::new();
    if g > 0 {
        parts.push(format!("{g}g"));
    }
    if s > 0 {
        parts.push(format!("{s}s"));
    }
    if c > 0 || parts.is_empty() {
        parts.push(format!("{c}c"));
    }
    parts.join(" ")
}

/// What a merchant stocks (catalog item ids) and the share of base price they
/// pay for what players sell them. `None` for anyone who is not a shopkeeper.
/// The customer side reads this too, so a `buy` action can be matched against
/// what is actually on the shelf instead of every item in the game.
pub fn merchant_shop(npc_name: &str) -> Option<(Vec<&'static str>, u32)> {
    let merchant = merchants()
        .values()
        .find(|m| m.npc_name.eq_ignore_ascii_case(npc_name))?;
    Some((
        id_list(&merchant.catalog).collect(),
        merchant.sell_rate_percent,
    ))
}

/// The `;`-separated item id lists the game data stores catalogs and
/// wishlists as.
fn id_list(field: &str) -> impl Iterator<Item = &str> {
    field.split(';').map(str::trim).filter(|id| !id.is_empty())
}

/// What a customer standing near a merchant sees: the shelf, with prices, and
/// the rate they buy at. Without it the LLM has to guess item ids for `buy`,
/// since a shop window is something only the web client opens.
pub fn shop_line_for(npc_name: &str) -> Option<String> {
    let (catalog, sell_rate_percent) = merchant_shop(npc_name)?;
    let stock: Vec<String> = catalog
        .iter()
        .filter_map(|id| {
            let price = crate::item_defs::get(id)?.base_price?;
            Some(format!("{id} {}", format_price(price)))
        })
        .collect();
    if stock.is_empty() {
        return None;
    }
    Some(format!(
        "{npc_name} sells: {} — and pays {sell_rate_percent}% of base for what you sell them.",
        stock.join(", ")
    ))
}

/// Build the static "Your Shop" system-prompt section for a merchant NPC,
/// or `None` if the character is not a merchant. Non-merchant traders get
/// a per-turn section instead (`resident_trade_prompt_for`) so it can
/// disappear once their needs are met.
pub fn merchant_prompt_for(npc_name: &str) -> Option<String> {
    let (catalog, sell_rate_percent) = merchant_shop(npc_name)?;

    let mut section = String::from("## Your Shop\nItems you sell, with base prices:\n");
    for item_id in catalog {
        let Some(item) = crate::item_defs::get(item_id) else {
            continue;
        };
        let price = item.base_price.map_or("?".to_string(), format_price);
        section.push_str(&format!("- {item_id} ({}): {price}\n", item.name));
    }
    section.push_str(&format!(
        "You also buy any item that has a base price from players, paying {sell_rate_percent}% \
         of its base price.\n\
         Use the \"open_trade\" action to put your shop window on a nearby player's screen \
         when the conversation turns to trading.\n"
    ));
    Some(section)
}

/// Build the per-turn "Your Personal Trading" section for a non-merchant
/// trader. Prompt steering alone cannot stop an LLM from chasing its
/// wishlist forever, so satiation is structural: items already in the
/// NPC's bag are omitted, and once every wish is satisfied the section —
/// the temptation itself — vanishes from the prompt entirely. The bag is
/// server-persisted, so the desire only returns if the item leaves it.
pub fn resident_trade_prompt_for(
    npc_name: &str,
    bag: &[onlinerpg_shared::inventory::ItemInstance],
) -> Option<String> {
    let trader = npcs()
        .values()
        .find(|t| t.npc_name.eq_ignore_ascii_case(npc_name))?;

    let wanted: Vec<&str> = id_list(&trader.wishlist)
        .filter(|id| !bag.iter().any(|item| item.item_def_id == *id))
        .collect();
    if wanted.is_empty() {
        return None;
    }

    let mut section = String::from(
        "## Your Personal Trading\n\
         You are not a shopkeeper, but you personally want to buy a few things \
         you need — from passing players only; other NPCs are not your suppliers:\n",
    );
    for item_id in wanted {
        let Some(item) = crate::item_defs::get(item_id) else {
            continue;
        };
        let base = item.base_price.unwrap_or(0);
        section.push_str(&format!(
            "- {item_id} ({}): you pay {} ({}% of the {} base price)\n",
            item.name,
            format_price(base * i64::from(trader.wishlist_rate_percent) / 100),
            trader.wishlist_rate_percent,
            format_price(base),
        ));
    }
    section.push_str(&format!(
        "Your wallet is your own money — salary {} per game day, and it stops \
         accumulating at {}. Check \"Your gold\" in the world state before promising \
         to buy anything; if you can't afford it, say so in character.\n\
         You keep what you buy (you need it; you never resell wishlist items), but \
         players may buy other items out of your bag at base price.\n\
         When a conversation turns to trading, use the \"open_trade\" action to put \
         your trade window on the player's screen. Use \"offer_deal\" with kind \
         \"sell\" and a positive modifier_pct to pay a bonus for something you \
         really need right now.\n",
        format_price(trader.salary_per_day),
        format_price(trader.wallet_cap),
    ));
    Some(section)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_mixed_denominations() {
        assert_eq!(format_price(0), "0c");
        assert_eq!(format_price(50), "50c");
        assert_eq!(format_price(2_500), "25s");
        assert_eq!(format_price(10_000), "1g");
        assert_eq!(format_price(12_345), "1g 23s 45c");
    }

    fn bag(ids: &[&str]) -> Vec<onlinerpg_shared::inventory::ItemInstance> {
        ids.iter()
            .enumerate()
            .map(|(i, id)| onlinerpg_shared::inventory::ItemInstance {
                instance_id: i as u64 + 1,
                item_def_id: (*id).to_string(),
                quantity: 1,
                enchant: 0,
                durability: None,
            })
            .collect()
    }

    #[test]
    fn registry_resolves_name_and_class_by_id() {
        let karl = npc_by_id("karl").expect("karl is in the registry");
        assert_eq!(karl.npc_name, "Karl");
        assert_eq!(karl.class, "guard");
        let rica = npc_by_id("rica").expect("rica is in the registry");
        assert_eq!(rica.npc_name, "Rica");
        assert_eq!(rica.class, "merchant");
        assert!(npc_by_id("nobody").is_none());
    }

    #[test]
    fn rica_has_a_shop_section() {
        let section = merchant_prompt_for("Rica").expect("Rica is a merchant");
        assert!(section.contains("dagger"));
        assert!(section.contains("25s"));
        assert!(section.contains("40%"));
        assert!(section.contains("open_trade"));
        assert!(merchant_prompt_for("Karl").is_none());
    }

    /// A customer sees the same shelf the shopkeeper does, with prices, so a
    /// `buy` action can name something real instead of a guessed item id.
    #[test]
    fn a_customer_sees_ricas_shelf_and_her_rate() {
        let line = shop_line_for("rica").expect("case does not matter");
        assert!(line.contains("dagger 25s"), "{line}");
        assert!(line.contains("healing_potion"), "{line}");
        assert!(line.contains("40%"), "{line}");
        assert!(shop_line_for("Karl").is_none(), "a resident keeps no shelf");

        let (catalog, rate) = merchant_shop("Rica").expect("Rica is a merchant");
        assert_eq!(rate, 40);
        assert!(catalog.contains(&"dagger"));
        assert!(
            !catalog.contains(&"iron_sword"),
            "tier-2+ weapons are dungeon loot, not shelf stock, so a buy cannot resolve to them"
        );
    }

    #[test]
    fn karl_wishlist_satiates_per_owned_item() {
        let section =
            resident_trade_prompt_for("Karl", &bag(&[])).expect("Karl wants torch and dagger");
        assert!(section.contains("torch"));
        assert!(section.contains("dagger"));
        assert!(section.contains("120%"));
        assert!(section.contains("50s"), "salary 5000 formats as 50s");
        assert!(section.contains("open_trade"));

        // Owning a torch removes it from the wants; owning both removes the
        // whole section — the temptation disappears from the prompt.
        let section =
            resident_trade_prompt_for("Karl", &bag(&["torch"])).expect("Karl still wants a dagger");
        assert!(!section.contains("torch ("));
        assert!(section.contains("dagger"));
        assert!(resident_trade_prompt_for("Karl", &bag(&["torch", "dagger"])).is_none());

        assert!(resident_trade_prompt_for("Nobody", &bag(&[])).is_none());
        assert!(resident_trade_prompt_for("Rica", &bag(&[])).is_none());
    }
}
