//! Item definitions from `data/items.json`, mirroring the server's
//! `item_defs.rs`. Lets the agent work out what "use this item" means —
//! equip it, take it off, or drink it. Embedded at compile time like the
//! rest of the game data, so no runtime path to `data/` is needed.

use std::collections::HashMap;
use std::sync::OnceLock;

use onlinerpg_shared::inventory::EquipSlot;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct ItemDef {
    pub name: String,
    #[serde(rename = "basePrice")]
    pub base_price: Option<i64>,
    #[serde(rename = "equipSlot")]
    pub equip_slot: Option<EquipSlot>,
    #[serde(default)]
    pub category: Option<String>,
    /// Usable straight from the bag — the items.csv flag, which the server
    /// validates against its `use_effect` dispatch at boot.
    #[serde(default)]
    pub consumable: bool,
}

impl ItemDef {
    pub fn is_consumable(&self) -> bool {
        self.consumable
    }

    /// What `/play_music` requires the performer to carry.
    pub fn is_instrument(&self) -> bool {
        self.category.as_deref() == Some("instrument")
    }

    /// Using it sets a tip hat down or picks it back up (server's toggle).
    pub fn is_tip_hat(&self) -> bool {
        self.category.as_deref() == Some("tip_hat")
    }

    /// A dungeon floor key (doc/DUNGEON_REWARD.md): worth grabbing on sight.
    pub fn is_dungeon_key(&self) -> bool {
        self.category.as_deref() == Some("dungeon_key")
    }
}

fn defs() -> &'static HashMap<String, ItemDef> {
    static CACHE: OnceLock<HashMap<String, ItemDef>> = OnceLock::new();
    CACHE.get_or_init(|| {
        serde_json::from_str(include_str!("../../data/items.json")).unwrap_or_default()
    })
}

pub fn all_ids() -> Vec<&'static str> {
    defs().keys().map(String::as_str).collect()
}

pub fn get(item_def_id: &str) -> Option<&'static ItemDef> {
    defs().get(item_def_id)
}

/// Pick the item the agent meant out of a candidate list — what it carries,
/// or what lies in front of it. An exact def id or display name wins; failing
/// that, the first candidate whose id or name contains the request, so an
/// agent that says "torch" while holding a worn_torch means that one. Never
/// names something outside the list.
pub fn resolve_named<'a>(candidates: &[&'a str], asked: &str) -> Option<&'a str> {
    let exact = |id: &str| {
        id.eq_ignore_ascii_case(asked)
            || get(id).is_some_and(|d| d.name.eq_ignore_ascii_case(asked))
    };
    if let Some(id) = candidates.iter().find(|id| exact(id)).copied() {
        return Some(id);
    }
    let asked = asked.to_lowercase();
    candidates
        .iter()
        .find(|id| {
            id.to_lowercase().contains(&asked)
                || get(id).is_some_and(|d| d.name.to_lowercase().contains(&asked))
        })
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torch_is_off_hand_gear_not_a_consumable() {
        let def = get("torch").expect("torch is defined");
        assert_eq!(def.equip_slot, Some(EquipSlot::OffHand));
        assert!(!def.is_consumable());
    }

    #[test]
    fn potions_are_consumable() {
        let def = get("healing_potion").expect("healing potion is defined");
        assert!(def.is_consumable());
        assert!(def.equip_slot.is_none());
    }

    /// The hand-kept category list this flag replaced had drifted — it
    /// missed coin_catch. The data flag covers it.
    #[test]
    fn coin_catches_are_consumable() {
        let def = get("sunken_coin_pouch").expect("coin pouch is defined");
        assert!(def.is_consumable());
    }

    #[test]
    fn carried_lookup_prefers_an_exact_match() {
        let bag = ["torch", "worn_torch"];
        assert_eq!(resolve_named(&bag, "torch"), Some("torch"));
        assert_eq!(resolve_named(&bag, "Torch"), Some("torch"));
        assert_eq!(resolve_named(&bag, "worn_torch"), Some("worn_torch"));
    }

    /// A starter character carries a worn_torch, not a torch — asking for
    /// "torch" must find the one it actually has.
    #[test]
    fn carried_lookup_falls_back_to_what_is_held() {
        let bag = ["worn_torch", "healing_potion"];
        assert_eq!(resolve_named(&bag, "torch"), Some("worn_torch"));
        assert_eq!(
            resolve_named(&bag, "Healing Potion"),
            Some("healing_potion")
        );
    }

    #[test]
    fn carried_lookup_never_invents_an_item() {
        assert!(resolve_named(&["worn_torch"], "iron_sword").is_none());
        assert!(resolve_named(&[], "torch").is_none());
    }
}
