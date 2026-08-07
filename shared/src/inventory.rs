use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{PlayerId, Position};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EquipSlot {
    Head,
    MainHand,
    OffHand,
    Chest,
    Ear,
    Neck,
    Belt,
    Pants,
    Boots,
    Ring,
    RingLeft,
    Hands,
    Back,
    Shirt,
}

impl EquipSlot {
    pub fn as_str(&self) -> &'static str {
        match self {
            EquipSlot::Head => "head",
            EquipSlot::MainHand => "main_hand",
            EquipSlot::OffHand => "off_hand",
            EquipSlot::Chest => "chest",
            EquipSlot::Ear => "ear",
            EquipSlot::Neck => "neck",
            EquipSlot::Belt => "belt",
            EquipSlot::Pants => "pants",
            EquipSlot::Boots => "boots",
            EquipSlot::Ring => "ring",
            EquipSlot::RingLeft => "ring_left",
            EquipSlot::Hands => "hands",
            EquipSlot::Back => "back",
            EquipSlot::Shirt => "shirt",
        }
    }

    /// For slots that have an alternate (e.g. ring/ring_left),
    /// returns the alternate slot. Used when the primary is occupied.
    pub fn alternate(&self) -> Option<Self> {
        match self {
            EquipSlot::Ring => Some(EquipSlot::RingLeft),
            EquipSlot::RingLeft => Some(EquipSlot::Ring),
            _ => None,
        }
    }
}

impl std::str::FromStr for EquipSlot {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "head" => Ok(EquipSlot::Head),
            "main_hand" => Ok(EquipSlot::MainHand),
            "off_hand" => Ok(EquipSlot::OffHand),
            "chest" => Ok(EquipSlot::Chest),
            "ear" => Ok(EquipSlot::Ear),
            "neck" => Ok(EquipSlot::Neck),
            "belt" => Ok(EquipSlot::Belt),
            "pants" => Ok(EquipSlot::Pants),
            "boots" => Ok(EquipSlot::Boots),
            "ring" => Ok(EquipSlot::Ring),
            "ring_left" => Ok(EquipSlot::RingLeft),
            "hands" => Ok(EquipSlot::Hands),
            "back" => Ok(EquipSlot::Back),
            "shirt" => Ok(EquipSlot::Shirt),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemInstance {
    pub instance_id: u64,
    pub item_def_id: String,
    pub quantity: u32,
    /// Weapon enchantment level (+N to attack and damage rolls). Zero for
    /// everything but enchanted weapons; `default` keeps old payloads valid.
    #[serde(default)]
    pub enchant: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlayerInventory {
    pub bag: Vec<ItemInstance>,
    pub equipped: HashMap<EquipSlot, ItemInstance>,
}

impl PlayerInventory {
    pub fn has_equipped_item(&self, slot: EquipSlot, item_def_id: &str) -> bool {
        self.equipped
            .get(&slot)
            .is_some_and(|item| item.item_def_id == item_def_id)
    }

    /// Any torch variant in the off-hand lights the player.
    pub fn is_torch_lit(&self) -> bool {
        self.equipped
            .get(&EquipSlot::OffHand)
            .is_some_and(|item| TORCH_ITEM_IDS.contains(&item.item_def_id.as_str()))
    }

    /// Equipped main-hand item def id, as broadcast to nearby players.
    pub fn main_hand_def_id(&self) -> Option<String> {
        self.equipped
            .get(&EquipSlot::MainHand)
            .map(|item| item.item_def_id.clone())
    }

    /// Everything the player carries: bag and worn gear alike.
    pub fn items(&self) -> impl Iterator<Item = &ItemInstance> {
        self.bag.iter().chain(self.equipped.values())
    }

    /// Whether the player carries the item anywhere, bag or worn.
    pub fn has_item(&self, item_def_id: &str) -> bool {
        self.items().any(|item| item.item_def_id == item_def_id)
    }
}

/// Item defs that act as a carried light source.
pub const TORCH_ITEM_IDS: &[&str] = &["torch", "worn_torch"];

/// A ground item's default unit count, for payloads written before stacks
/// could land on the ground.
fn one_unit() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundItem {
    pub instance_id: u64,
    pub item_def_id: String,
    pub position: Position,
    pub floor_level: i8,
    /// Units in this pile. Only a stackable def ever exceeds 1: dropping a
    /// stack lands one pile of N rather than N scattered items.
    #[serde(default = "one_unit")]
    pub quantity: u32,
    /// Carries a dropped weapon's enchantment so picking it back up
    /// doesn't wipe it.
    #[serde(default)]
    pub enchant: i32,
    /// The player who put it there, if one did — loot and world drops carry
    /// `None`. On the item rather than the spawn message so attribution
    /// survives AOI churn and rejoins: a busker's uncollected tip is still
    /// its tip after a reconnect.
    #[serde(default)]
    pub dropped_by: Option<PlayerId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_SLOTS: &[EquipSlot] = &[
        EquipSlot::Head,
        EquipSlot::MainHand,
        EquipSlot::OffHand,
        EquipSlot::Chest,
        EquipSlot::Ear,
        EquipSlot::Neck,
        EquipSlot::Belt,
        EquipSlot::Pants,
        EquipSlot::Boots,
        EquipSlot::Ring,
        EquipSlot::RingLeft,
        EquipSlot::Hands,
        EquipSlot::Back,
        EquipSlot::Shirt,
    ];

    #[test]
    fn equip_slot_str_roundtrip() {
        for slot in ALL_SLOTS {
            let s = slot.as_str();
            let back: EquipSlot = s.parse().expect("parse should accept as_str output");
            assert_eq!(&back, slot, "roundtrip failed for {s}");
            let wire = serde_json::to_string(slot).unwrap();
            assert_eq!(
                wire,
                format!("\"{s}\""),
                "serde wire name must match as_str"
            );
        }
    }

    #[test]
    fn equip_slot_from_str_rejects_unknown() {
        assert!("".parse::<EquipSlot>().is_err());
        assert!("shoulder".parse::<EquipSlot>().is_err());
        assert!("Head".parse::<EquipSlot>().is_err());
    }

    #[test]
    fn equip_slot_alternate_is_symmetric_for_rings() {
        assert_eq!(EquipSlot::Ring.alternate(), Some(EquipSlot::RingLeft));
        assert_eq!(EquipSlot::RingLeft.alternate(), Some(EquipSlot::Ring));
    }

    #[test]
    fn equip_slot_alternate_none_for_unique_slots() {
        for slot in ALL_SLOTS {
            if matches!(slot, EquipSlot::Ring | EquipSlot::RingLeft) {
                continue;
            }
            assert_eq!(
                slot.alternate(),
                None,
                "slot {:?} should not have an alternate",
                slot
            );
        }
    }

    #[test]
    fn player_inventory_default_is_empty() {
        let inv = PlayerInventory::default();
        assert!(inv.bag.is_empty());
        assert!(inv.equipped.is_empty());
    }
}
