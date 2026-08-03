//! Item definitions from `data/items.json`, mirroring the server's
//! `item_defs.rs`. Lets the agent work out what "use this item" means —
//! equip it, take it off, or drink it. Embedded at compile time like the
//! rest of the game data, so no runtime path to `data/` is needed.

use std::collections::HashMap;
use std::sync::OnceLock;

use onlinerpg_shared::inventory::{
    ArmorConstruction, EquipSlot, EquipmentKind, EquipmentLayer, GarmentForm, RepairFamily,
};
use onlinerpg_shared::skills::SkillId;
use onlinerpg_shared::{combat::construction_protection, PhysicalDamageType};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct ItemDef {
    pub name: String,
    #[serde(rename = "basePrice")]
    pub base_price: Option<i64>,
    #[serde(default)]
    #[allow(dead_code)]
    pub guard: Option<i32>,
    #[serde(rename = "equipSlot")]
    pub equip_slot: Option<EquipSlot>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(rename = "damageType", default)]
    #[allow(dead_code)]
    pub damage_type: Option<PhysicalDamageType>,
    #[serde(rename = "armorConstruction", default)]
    #[allow(dead_code)] // Retained for content parity and agent inspection tests.
    pub armor_construction: Option<ArmorConstruction>,
    #[serde(rename = "equipmentKind", default)]
    #[allow(dead_code)]
    pub equipment_kind: Option<EquipmentKind>,
    #[serde(rename = "equipmentLayer", default)]
    #[allow(dead_code)]
    pub equipment_layer: Option<EquipmentLayer>,
    #[serde(rename = "garmentForm", default)]
    #[allow(dead_code)]
    pub garment_form: Option<GarmentForm>,
    #[serde(rename = "weaponSkill", default)]
    pub weapon_skill: Option<SkillId>,
    #[serde(rename = "defenseSkill", default)]
    #[allow(dead_code)] // Retained for protocol/content parity and agent inspection tests.
    pub defense_skill: Option<SkillId>,
    #[serde(rename = "useSkill", default)]
    #[allow(dead_code)] // Retained for protocol/content parity and agent inspection tests.
    pub use_skill: Option<SkillId>,
    #[serde(rename = "maxDurability", default)]
    pub max_durability: Option<u32>,
    #[serde(rename = "repairFamily", default)]
    pub repair_family: Option<RepairFamily>,
    /// Usable straight from the bag — the items.csv flag, which the server
    /// validates against its `use_effect` dispatch at boot.
    #[serde(default)]
    pub consumable: bool,
}

impl ItemDef {
    pub fn is_consumable(&self) -> bool {
        self.consumable
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

pub fn equipment_summary(item_def_id: &str) -> Option<String> {
    let def = get(item_def_id)?;
    let mut details = Vec::new();
    if let Some(guard) = def.guard.filter(|guard| *guard != 0) {
        details.push(format!("Guard {guard:+}"));
    }
    if let Some(construction) = def.armor_construction {
        details.push(format!("{} construction", construction.display_name()));
        if def.equip_slot == Some(EquipSlot::Chest) {
            let protection = [
                PhysicalDamageType::Slash,
                PhysicalDamageType::Pierce,
                PhysicalDamageType::Blunt,
            ]
            .into_iter()
            .filter_map(|damage_type| {
                let amount = construction_protection(Some(construction), damage_type);
                (amount > 0).then(|| format!("{} {amount}", damage_type.display_name()))
            })
            .collect::<Vec<_>>();
            if !protection.is_empty() {
                details.push(format!("Protection {}", protection.join(", ")));
            }
        }
    }
    if let Some(skill) = def.defense_skill {
        details.push(format!("Skill {}", skill.display_name()));
    }
    if let Some(family) = def.repair_family {
        if def.category.as_deref() == Some("armor_repair_kit") {
            details.push(format!("Repairs {} armor", family.display_name()));
        } else {
            details.push(format!("Repair family {}", family.display_name()));
        }
    }
    (!details.is_empty()).then(|| details.join("; "))
}

pub fn equipment_instance_summary(
    item: &onlinerpg_shared::inventory::ItemInstance,
) -> Option<String> {
    let mut details = equipment_summary(&item.item_def_id)
        .map(|summary| vec![summary])
        .unwrap_or_default();
    if let (Some(current), Some(max)) = (
        item.durability,
        get(&item.item_def_id).and_then(|def| def.max_durability),
    ) {
        if current == 0 {
            details.push(format!("BROKEN, condition 0/{max}"));
        } else {
            details.push(format!("Condition {current}/{max}"));
        }
    }
    (!details.is_empty()).then(|| details.join("; "))
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
    fn weapon_skills_follow_shared_item_metadata() {
        assert_eq!(
            get("iron_sword").and_then(|def| def.weapon_skill),
            Some(SkillId::OneHandedSword)
        );
        assert_eq!(
            get("dagger").and_then(|def| def.weapon_skill),
            Some(SkillId::Dagger)
        );
        assert_eq!(
            get("spear").and_then(|def| def.weapon_skill),
            Some(SkillId::Spear)
        );
        assert_eq!(get("torch").and_then(|def| def.weapon_skill), None);
        for id in ["iron_sword", "dagger", "goblin_sword", "small_sword"] {
            assert_eq!(
                get(id).and_then(|def| def.damage_type),
                Some(PhysicalDamageType::Slash),
                "{id} damage type"
            );
        }
        assert_eq!(
            get("spear").and_then(|def| def.damage_type),
            Some(PhysicalDamageType::Pierce)
        );
        assert_eq!(
            get("torch").and_then(|def| def.damage_type),
            Some(PhysicalDamageType::Blunt)
        );
        assert_eq!(get("fishing_rod").and_then(|def| def.damage_type), None);
    }

    #[test]
    fn shield_skill_follows_shared_item_metadata() {
        for id in ["wooden_shield", "raven_shield"] {
            assert_eq!(
                get(id).and_then(|def| def.defense_skill),
                Some(SkillId::Shield)
            );
        }
        assert_eq!(get("torch").and_then(|def| def.defense_skill), None);
    }

    #[test]
    fn armor_construction_and_skill_follow_shared_item_metadata() {
        assert_eq!(
            get("leather_armor").and_then(|def| def.armor_construction),
            Some(ArmorConstruction::Leather)
        );
        assert_eq!(
            get("leather_armor").and_then(|def| def.defense_skill),
            Some(SkillId::LeatherArmor)
        );
        assert_eq!(get("leather_armor").and_then(|def| def.guard), Some(2));
        assert_eq!(
            get("chain_mail").and_then(|def| def.armor_construction),
            Some(ArmorConstruction::Mail)
        );
        assert_eq!(get("chain_mail").and_then(|def| def.guard), Some(5));
        assert_eq!(get("chain_mail").and_then(|def| def.defense_skill), None);
        assert_eq!(
            get("breastplate").and_then(|def| def.armor_construction),
            Some(ArmorConstruction::Plate)
        );
        assert_eq!(get("breastplate").and_then(|def| def.guard), Some(7));
        assert_eq!(get("breastplate").and_then(|def| def.defense_skill), None);
        assert_eq!(
            get("wooden_shield").and_then(|def| def.armor_construction),
            None
        );
        assert_eq!(
            get("traveler_robe").and_then(|def| def.equipment_kind),
            Some(EquipmentKind::Clothing)
        );
        assert_eq!(
            get("traveler_robe").and_then(|def| def.garment_form),
            Some(GarmentForm::Robe)
        );
        assert_eq!(
            get("padded_battle_robe").and_then(|def| def.armor_construction),
            Some(ArmorConstruction::Padded)
        );
        assert_eq!(
            get("brigandine_coat").and_then(|def| def.armor_construction),
            Some(ArmorConstruction::Hybrid)
        );
        assert_eq!(get("brigandine_coat").and_then(|def| def.guard), Some(2));
        assert_eq!(
            get("padded_battle_robe").and_then(|def| def.max_durability),
            Some(40)
        );
        assert_eq!(
            get("leather_armor").and_then(|def| def.max_durability),
            Some(60)
        );
        assert_eq!(
            get("chain_mail").and_then(|def| def.max_durability),
            Some(90)
        );
        assert_eq!(
            get("breastplate").and_then(|def| def.max_durability),
            Some(120)
        );
        assert_eq!(
            get("brigandine_coat").and_then(|def| def.max_durability),
            Some(100)
        );
        for id in ["traveler_robe", "padded_battle_robe", "brigandine_coat"] {
            assert_eq!(
                get(id).and_then(|def| def.equipment_layer),
                Some(EquipmentLayer::Primary)
            );
            assert_eq!(get(id).and_then(|def| def.defense_skill), None);
        }
        assert_eq!(
            equipment_summary("leather_armor").as_deref(),
            Some(
                "Guard +2; Leather construction; Protection Slash 1, Pierce 1, Blunt 1; Skill Leather Armor; Repair family Leather"
            )
        );
        assert_eq!(
            equipment_summary("padded_battle_robe").as_deref(),
            Some("Padded construction; Protection Slash 1, Blunt 2; Repair family Cloth")
        );
        assert_eq!(
            equipment_summary("chain_mail").as_deref(),
            Some("Guard +5; Mail construction; Protection Slash 2, Pierce 1; Repair family Metal")
        );
        assert_eq!(
            equipment_summary("breastplate").as_deref(),
            Some("Guard +7; Plate construction; Protection Slash 3, Pierce 3, Blunt 1; Repair family Metal")
        );
        assert_eq!(
            equipment_summary("brigandine_coat").as_deref(),
            Some("Guard +2; Hybrid construction; Protection Slash 2, Pierce 2, Blunt 2; Repair family Hybrid")
        );
        assert_eq!(
            equipment_summary("leather_helmet").as_deref(),
            Some("Guard +1; Leather construction")
        );
        for (id, family) in [
            ("cloth_repair_kit", RepairFamily::Cloth),
            ("leather_repair_kit", RepairFamily::Leather),
            ("metal_repair_kit", RepairFamily::Metal),
            ("hybrid_repair_kit", RepairFamily::Hybrid),
        ] {
            assert_eq!(get(id).and_then(|def| def.repair_family), Some(family));
            assert_eq!(
                equipment_summary(id),
                Some(format!("Repairs {} armor", family.display_name()))
            );
        }
    }

    #[test]
    fn equipped_instance_summary_exposes_condition_and_breakage() {
        let mut armor = onlinerpg_shared::inventory::ItemInstance {
            instance_id: 1,
            item_def_id: "leather_armor".to_string(),
            quantity: 1,
            enchant: 0,
            durability: Some(17),
        };
        assert!(equipment_instance_summary(&armor)
            .unwrap()
            .contains("Condition 17/60"));
        armor.durability = Some(0);
        assert!(equipment_instance_summary(&armor)
            .unwrap()
            .contains("BROKEN, condition 0/60"));
    }

    #[test]
    fn healing_use_skill_follows_shared_item_metadata() {
        assert_eq!(
            get("bandage").and_then(|def| def.use_skill),
            Some(SkillId::Healing)
        );
        assert!(get("bandage").is_some_and(ItemDef::is_consumable));
        for id in [
            "healing_potion",
            "raw_minnow",
            "raw_trout",
            "scroll_of_return",
        ] {
            assert_eq!(get(id).and_then(|def| def.use_skill), None);
        }
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
