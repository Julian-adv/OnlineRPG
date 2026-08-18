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
    #[serde(default)]
    keepsakes: String,
}

impl NpcRow {
    /// Keepsakes this NPC could actually offer: the listed ids that carry a
    /// base price. `keepsake_section` never offers an unpriced one, so an
    /// unpriced entry (a worn instrument, say) is fair game to wear out.
    pub fn offerable_keepsake_ids(&self) -> impl Iterator<Item = &str> {
        id_list(&self.keepsakes)
            .filter(|id| crate::item_defs::get(id).is_some_and(|d| d.base_price.is_some()))
    }
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

/// Build the per-turn "## Personal Trading" section for a non-merchant
/// trader: what it wants to buy (its wishlist) and what it might part
/// with (its keepsakes), as one personal-business section for the same
/// trusted audience. Both halves are structural, not prompt steering:
/// wishlist items already in the bag drop out (and `include_wishlist`
/// carries the post-purchase satiation), keepsakes are offered only while
/// still in the bag, and an empty `audience` — the nearby regulars past
/// the trade favor threshold — omits the whole section. The server hides
/// keepsakes from walk-in stock and only sells to a player holding the
/// NPC's own `offer_deal`; unpriced ones (a bard's worn instrument) can
/// never sell and are left out.
pub fn resident_trade_prompt_for(
    npc_name: &str,
    bag: &[onlinerpg_shared::inventory::ItemInstance],
    include_wishlist: bool,
    audience: &[String],
) -> Option<String> {
    if audience.is_empty() {
        return None;
    }
    let trader = npcs()
        .values()
        .find(|t| t.npc_name.eq_ignore_ascii_case(npc_name))?;

    let wanted: Vec<&str> = if include_wishlist {
        id_list(&trader.wishlist)
            .filter(|id| !bag.iter().any(|item| item.item_def_id == *id))
            .collect()
    } else {
        Vec::new()
    };
    let held: Vec<(&str, &str, i64)> = trader
        .offerable_keepsake_ids()
        .filter(|id| bag.iter().any(|item| item.item_def_id == *id))
        .filter_map(|id| {
            let item = crate::item_defs::get(id)?;
            Some((id, item.name.as_str(), item.base_price?))
        })
        .collect();
    if wanted.is_empty() && held.is_empty() {
        return None;
    }

    let mut section = format!(
        "## Personal Trading\n\
         You are not a shopkeeper; this is personal business, and only \
         these travelers have earned your trust for it: {}. Never \
         advertise it, never trade personally with anyone not named here, \
         and never with another NPC. When talk turns that way, put your \
         trade window on their screen:\n\
         {{\"type\": \"open_trade\", \"player\": \"PlayerName\"}}\n",
        audience.join(", "),
    );

    if !wanted.is_empty() {
        section.push_str(&format!(
            "Things you want to buy — your wallet is your own money \
             (salary {} per game day, capped at {}); check \"Your gold\" \
             first and say so in character if you cannot afford it. You \
             keep what you buy, never reselling it:\n",
            format_price(trader.salary_per_day),
            format_price(trader.wallet_cap),
        ));
        for item_id in &wanted {
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
            "To pay a bonus for something you need right now:\n\
             {{\"type\": \"offer_deal\", \"player\": \"PlayerName\", \"item\": \
             \"{}\", \"kind\": \"sell\", \"modifier_pct\": 10, \"reason\": \
             \"I need it today\"}}\n",
            wanted[0],
        ));
    }

    if let Some((first, ..)) = held.first().copied() {
        section.push_str(
            "Things you might part with — personal belongings in your bag, \
             not stock; nobody can buy one unless you offer it first, and \
             offering is a choice, not a duty — only when the moment feels \
             right, each person asked at most once. Keep them in your bag, \
             not worn:\n",
        );
        for (item_id, name, price) in &held {
            section.push_str(&format!(
                "- {item_id} ({name}): base price {}\n",
                format_price(*price)
            ));
        }
        section.push_str(&format!(
            "To offer one, pair a \"say\" naming the thing and its price \
             with an unlock (modifier_pct 0 sells at base price, negative \
             is a friend's discount; it lasts a few minutes), then the \
             trade window:\n\
             {{\"type\": \"offer_deal\", \"player\": \"PlayerName\", \"item\": \
             \"{first}\", \"kind\": \"buy\", \"modifier_pct\": 0, \"reason\": \
             \"a regular who has earned it\"}}\n\
             {{\"type\": \"open_trade\", \"player\": \"PlayerName\"}}\n",
        ));
    }

    section.push_str("Players may also buy other items out of your bag at base price.\n");
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
                cape_color: None,
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

    fn regulars(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn karl_wishlist_satiates_per_owned_item() {
        let section = resident_trade_prompt_for("Karl", &bag(&[]), true, &regulars(&["jake1"]))
            .expect("Karl wants torch and dagger");
        assert!(section.contains("Personal Trading"));
        assert!(section.contains("jake1"), "the section names its audience");
        assert!(section.contains("torch"));
        assert!(section.contains("dagger"));
        assert!(section.contains("120%"));
        assert!(section.contains("50s"), "salary 5000 formats as 50s");
        assert!(section.contains("open_trade"));

        // Owning a torch removes it from the wants; owning both removes the
        // whole section — the temptation disappears from the prompt.
        let section =
            resident_trade_prompt_for("Karl", &bag(&["torch"]), true, &regulars(&["jake1"]))
                .expect("Karl still wants a dagger");
        assert!(!section.contains("torch ("));
        assert!(section.contains("dagger"));
        assert!(resident_trade_prompt_for(
            "Karl",
            &bag(&["torch", "dagger"]),
            true,
            &regulars(&["jake1"])
        )
        .is_none());

        // Post-purchase satiation empties Karl's buy half, and he has no
        // keepsakes — nothing is emitted. Nor without a regular in sight.
        assert!(
            resident_trade_prompt_for("Karl", &bag(&[]), false, &regulars(&["jake1"])).is_none()
        );
        assert!(resident_trade_prompt_for("Karl", &bag(&[]), true, &[]).is_none());

        assert!(
            resident_trade_prompt_for("Nobody", &bag(&[]), true, &regulars(&["jake1"])).is_none()
        );
        assert!(
            resident_trade_prompt_for("Rica", &bag(&[]), true, &regulars(&["jake1"])).is_none()
        );
    }

    /// Signe's keepsake mandolin: the offer exists only while it is in her
    /// bag — the inverse of wishlist satiation — and only while a player
    /// who has earned it stands nearby. Her own worn mandolin has no
    /// price, so it never shows up as something to offer.
    #[test]
    fn signe_offers_her_spare_mandolin_only_while_she_still_has_it() {
        let section = resident_trade_prompt_for(
            "Signe",
            &bag(&["mandolin", "worn_mandolin"]),
            true,
            &regulars(&["jake1"]),
        )
        .expect("mandolin in bag and a regular nearby");
        assert!(section.contains("Personal Trading"));
        assert!(section.contains("- mandolin"));
        assert!(section.contains("40s"), "base 4000 formats as 40s");
        assert!(section.contains("jake1"), "the offer names who earned it");
        assert!(section.contains("offer_deal"));
        assert!(section.contains("open_trade"));
        assert!(
            !section.contains("worn_mandolin"),
            "her own instrument is not for sale"
        );
        assert!(
            !section.contains("want to buy"),
            "no wishlist, no buying urge"
        );

        assert!(
            resident_trade_prompt_for(
                "Signe",
                &bag(&["worn_mandolin", "clump_of_kelp"]),
                true,
                &regulars(&["jake1"])
            )
            .is_none(),
            "spare sold: only the unsellable worn instrument is left"
        );
    }

    /// The whole keepsake temptation stays out of the prompt until someone
    /// nearby has actually earned it — favor is the gate, not LLM judgement.
    #[test]
    fn keepsakes_stay_hidden_from_strangers() {
        assert!(
            resident_trade_prompt_for("Signe", &bag(&["mandolin", "worn_mandolin"]), true, &[])
                .is_none(),
            "no favored player nearby: no keepsake section at all"
        );
    }
}
