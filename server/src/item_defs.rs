use onlinerpg_shared::inventory::EquipSlot;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct ItemDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub weight: f32,
    #[serde(rename = "equipSlot")]
    pub equip_slot: Option<EquipSlot>,
    #[serde(default)]
    pub stackable: bool,
    #[serde(rename = "worldModel")]
    pub world_model: Option<String>,
    /// Item kind that decides how `dice` is interpreted ("weapon" → damage,
    /// "consumable" → healing) plus broad classification (armor, accessory,
    /// currency).
    #[serde(default)]
    pub category: Option<String>,
    /// Dice notation (e.g. "1d8", "6d4") whose meaning depends on `category`.
    /// Read it through `damage_dice()` / `heal_dice()` rather than directly.
    #[serde(default)]
    pub dice: Option<String>,
    #[serde(default)]
    pub material: Option<String>,
    /// Base price in the smallest currency unit. Items without a price
    /// cannot be bought or sold.
    #[serde(rename = "basePrice")]
    pub base_price: Option<i64>,
    /// Guard (AC) bonus granted while this item is equipped. Summed across all
    /// equipped items and added to the wearer's base guard when attacked.
    #[serde(default)]
    pub guard: Option<i32>,
    /// Fish only — rarity tier 1 (common) … 5 (legendary). Drives catch
    /// weighting and skill XP (doc/FISHING.md).
    #[serde(rename = "rarityTier", default)]
    pub rarity_tier: Option<u32>,
    /// Fish only — relative weight in the catch table at fishing level 0.
    #[serde(rename = "catchWeight", default)]
    pub catch_weight: Option<u32>,
    /// Fish only — dice notation for rolled length in centimeters.
    #[serde(rename = "sizeDice", default)]
    pub size_dice: Option<String>,
    /// Fish only — rolled length at or above this is a trophy catch.
    #[serde(rename = "trophyCm", default)]
    pub trophy_cm: Option<u32>,
}

/// The effect produced by consuming a usable item via `use_item`, decided by
/// the item's `category`. One place to extend when a new consumable lands.
pub enum UseEffect {
    /// Restore HP by rolling the given dice notation.
    Heal(String),
    /// Teleport the user back to the town spawn point.
    TeleportTown,
    /// Add +1 enchantment to the wielded weapon (NetHack style).
    EnchantWeapon,
}

impl ItemDefinition {
    pub fn is_weapon(&self) -> bool {
        self.category.as_deref() == Some("weapon")
    }

    /// Main-hand tool that enables casting (`ClientMessage::FishingCast`).
    /// Not a weapon: no damage dice, so attacking with it rod-in-hand uses
    /// the bare-handed path.
    pub fn is_fishing_rod(&self) -> bool {
        self.category.as_deref() == Some("fishing_rod")
    }

    pub fn is_fish(&self) -> bool {
        self.category.as_deref() == Some("fish")
    }

    /// Damage dice if this item is a weapon, else `None`.
    pub fn damage_dice(&self) -> Option<&str> {
        if self.is_weapon() {
            self.dice.as_deref()
        } else {
            None
        }
    }

    /// The effect of using this item from the bag, or `None` if it isn't a
    /// consumable.
    pub fn use_effect(&self) -> Option<UseEffect> {
        match self.category.as_deref()? {
            "healing_potion" => self.dice.clone().map(UseEffect::Heal),
            // Eating a fish heals by its dice — same plumbing as potions.
            "fish" => self.dice.clone().map(UseEffect::Heal),
            "return_scroll" => Some(UseEffect::TeleportTown),
            "enchant_scroll" => Some(UseEffect::EnchantWeapon),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ItemDefs {
    defs: Arc<HashMap<String, ItemDefinition>>,
}

impl ItemDefs {
    pub fn load() -> Self {
        let data = include_str!("../../data/items.json");
        let defs: HashMap<String, ItemDefinition> =
            serde_json::from_str(data).expect("Failed to parse items.json");

        info!("Loaded {} item definitions", defs.len());
        for (id, def) in &defs {
            info!(
                "  {} - weight:{} equipSlot:{:?} stackable:{}",
                id, def.weight, def.equip_slot, def.stackable
            );
        }

        Self {
            defs: Arc::new(defs),
        }
    }

    pub fn get(&self, item_def_id: &str) -> Option<&ItemDefinition> {
        self.defs.get(item_def_id)
    }

    pub fn item_def_id_for_weapon_ref(&self, weapon_ref: &str) -> Option<String> {
        if self.defs.contains_key(weapon_ref) {
            return Some(weapon_ref.to_string());
        }

        if let Some(item_id) = weapon_ref
            .strip_suffix(".glb")
            .filter(|item_id| self.defs.contains_key(*item_id))
        {
            return Some(item_id.to_string());
        }

        self.defs
            .values()
            .find(|def| def.world_model.as_deref() == Some(weapon_ref))
            .map(|def| def.id.clone())
    }

    pub fn damage_dice_for_weapon_model(&self, weapon_model: &str) -> Option<String> {
        self.item_def_id_for_weapon_ref(weapon_model)
            .and_then(|item_id| self.defs.get(&item_id))
            .and_then(|def| def.damage_dice().map(str::to_string))
    }

    /// Equippable items at or above a price floor — the dungeon treasure
    /// chest loot pool. Sorted for determinism before the caller shuffles.
    pub fn equipment_ids_with_min_price(&self, min_price: i64) -> Vec<String> {
        let mut ids: Vec<String> = self
            .defs
            .values()
            .filter(|def| def.equip_slot.is_some())
            .filter(|def| def.base_price.is_some_and(|p| p >= min_price))
            .map(|def| def.id.clone())
            .collect();
        ids.sort();
        ids
    }

    pub fn weight(&self, item_def_id: &str) -> f32 {
        self.defs.get(item_def_id).map(|d| d.weight).unwrap_or(1.0)
    }

    /// The fishing catch table: every fish def with a catch weight, sorted
    /// by id for a deterministic cumulative walk.
    pub fn fish_catch_table(&self) -> Vec<crate::game_state::fishing::CatchCandidate> {
        let mut table: Vec<_> = self
            .defs
            .values()
            .filter(|def| def.is_fish())
            .filter_map(|def| {
                Some(crate::game_state::fishing::CatchCandidate {
                    item_def_id: def.id.clone(),
                    rarity: def.rarity_tier.unwrap_or(1),
                    catch_weight: def.catch_weight?,
                })
            })
            .collect();
        table.sort_by(|a, b| a.item_def_id.cmp(&b.item_def_id));
        table
    }
}
