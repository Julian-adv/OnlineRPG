use onlinerpg_shared::inventory::{
    ArmorConstruction, EquipSlot, EquipmentKind, EquipmentLayer, GarmentForm,
};
use onlinerpg_shared::skills::SkillId;
use onlinerpg_shared::PhysicalDamageType;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

/// Chest roll chance for items whose home tier is below the dungeon's
/// (doc/ITEM_TIERS.md "하위 티어 이월템").
const CHEST_CARRYOVER_CHANCE: f32 = 0.10;

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
    /// restorative categories → healing) plus broad classification (armor,
    /// accessory, currency).
    #[serde(default)]
    pub category: Option<String>,
    /// Dice notation (e.g. "1d8", "6d4") whose meaning depends on `category`.
    /// Read it through `damage_dice()` / `heal_dice()` rather than directly.
    #[serde(default)]
    pub dice: Option<String>,
    #[serde(rename = "damageType", default)]
    pub damage_type: Option<PhysicalDamageType>,
    #[serde(default)]
    pub material: Option<String>,
    /// Physical build of worn body armor. Absent for shields, clothing,
    /// accessories, and non-armor items.
    #[serde(rename = "armorConstruction", default)]
    pub armor_construction: Option<ArmorConstruction>,
    #[serde(rename = "equipmentKind", default)]
    pub equipment_kind: Option<EquipmentKind>,
    #[serde(rename = "equipmentLayer", default)]
    pub equipment_layer: Option<EquipmentLayer>,
    #[serde(rename = "garmentForm", default)]
    pub garment_form: Option<GarmentForm>,
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
    /// Fish only — the fishing level a catch is locked behind. Absent or 0
    /// means available from the first cast.
    #[serde(rename = "minFishingLevel", default)]
    pub min_fishing_level: Option<u32>,
    /// Fish only — dice notation for rolled length in centimeters.
    #[serde(rename = "sizeDice", default)]
    pub size_dice: Option<String>,
    /// Fish only — rolled length at or above this is a trophy catch.
    #[serde(rename = "trophyCm", default)]
    pub trophy_cm: Option<u32>,
    /// Equipment only — the dungeon `chestTier` (dungeons.csv) this drops at.
    /// Opt-in: absent means never in any chest pool (doc/ITEM_TIERS.md).
    #[serde(rename = "chestTier", default)]
    pub chest_tier: Option<u8>,
    /// Per-chest-open roll chance at the item's home tier. Absent = 0
    /// (signature drops are guaranteed via dungeons.csv `chestDrops` instead).
    #[serde(rename = "chestChance", default)]
    pub chest_chance: Option<f32>,
    /// Usable from the bag. The clients read this flag; `load()` fails the
    /// boot if it ever disagrees with the `use_effect` dispatch.
    #[serde(default)]
    pub consumable: bool,
    /// Skill trained when this weapon resolves an accepted attack.
    #[serde(rename = "weaponSkill", default)]
    pub weapon_skill: Option<SkillId>,
    /// Defensive skill trained while this item is equipped and a server-owned
    /// attack resolves against its wearer.
    #[serde(rename = "defenseSkill", default)]
    pub defense_skill: Option<SkillId>,
    /// Skill trained by a valid server-resolved use of this consumable. This
    /// identifies the performed action, not manufacture of a finished product:
    /// bandaging trains Healing; drinking a potion does not.
    #[serde(rename = "useSkill", default)]
    pub use_skill: Option<SkillId>,
}

/// The effect produced by consuming a usable item via `use_item`, decided by
/// the item's `category`. One place to extend when a new consumable lands.
pub enum UseEffect {
    /// Restore HP by rolling the given dice notation.
    Heal {
        dice: String,
        skill: Option<SkillId>,
    },
    /// Teleport the user back to the town spawn point.
    TeleportTown,
    /// Add +1 enchantment to the wielded weapon (NetHack style).
    EnchantWeapon,
    /// Ask every party member to teleport to the reader's side.
    SummonParty,
    /// Open a fished-up coin pouch: roll the given dice for its copper.
    OpenCoinPouch(String),
}

impl ItemDefinition {
    pub fn is_weapon(&self) -> bool {
        self.category.as_deref() == Some("weapon")
    }

    pub fn is_body_armor(&self) -> bool {
        self.category.as_deref() == Some("armor")
            && self.equip_slot.is_some()
            && self.equip_slot != Some(EquipSlot::OffHand)
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

    /// A catch that lands in the bag sealed and pays out coins when opened
    /// (`use_item`). Its `dice` column is the copper roll (the
    /// category-decides-meaning pattern: weapon → damage, fish/potion →
    /// heal, coin_catch → gold). Production code dispatches through
    /// `use_effect`; the tests keep this named predicate for the economy
    /// guardrail.
    #[cfg(test)]
    pub fn is_coin_catch(&self) -> bool {
        self.category.as_deref() == Some("coin_catch")
    }

    /// Whether a catch of this item at `size_cm` is a trophy. Trophies are
    /// a fish concept — a nat-20 Old Boot is still just a (very large) boot —
    /// and fire on the natural-20 quality roll or on meeting `trophyCm`.
    pub fn trophy_at(&self, size_cm: u16, nat_twenty: bool) -> bool {
        self.is_fish()
            && (nat_twenty
                || self
                    .trophy_cm
                    .is_some_and(|threshold| u32::from(size_cm) >= threshold))
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
            "healing_potion" => self
                .dice
                .clone()
                .map(|dice| UseEffect::Heal { dice, skill: None }),
            "bandage" => self.dice.clone().map(|dice| UseEffect::Heal {
                dice,
                skill: self.use_skill,
            }),
            // Eating a fish heals by its dice — same plumbing as potions.
            "fish" => self
                .dice
                .clone()
                .map(|dice| UseEffect::Heal { dice, skill: None }),
            "return_scroll" => Some(UseEffect::TeleportTown),
            "enchant_scroll" => Some(UseEffect::EnchantWeapon),
            "party_summon_scroll" => Some(UseEffect::SummonParty),
            "coin_catch" => self.dice.clone().map(UseEffect::OpenCoinPouch),
            _ => None,
        }
    }
}

fn valid_dice_notation(notation: &str) -> bool {
    let Some((count, sides)) = notation.split_once('d') else {
        return false;
    };
    !count.is_empty()
        && !sides.is_empty()
        && count.parse::<u32>().is_ok_and(|value| value > 0)
        && sides.parse::<u32>().is_ok_and(|value| value > 0)
}

fn validate_weapon_skill(def: &ItemDefinition) -> Result<(), String> {
    if def.weapon_skill.is_none() {
        return Ok(());
    }
    if !matches!(
        def.weapon_skill,
        Some(SkillId::OneHandedSword | SkillId::Dagger | SkillId::Spear)
    ) {
        return Err("weaponSkill is not supported for weapon combat".to_string());
    }
    if !def.is_weapon() {
        return Err("weaponSkill requires category 'weapon'".to_string());
    }
    if def.equip_slot != Some(EquipSlot::MainHand) {
        return Err("weaponSkill requires equipSlot 'main_hand'".to_string());
    }
    if !def.dice.as_deref().is_some_and(valid_dice_notation) {
        return Err("weaponSkill requires valid positive NdM damage dice".to_string());
    }
    Ok(())
}

fn validate_damage_type(def: &ItemDefinition) -> Result<(), String> {
    match (def.is_weapon(), def.damage_type) {
        (true, None) => Err("weapon requires damageType".to_string()),
        (false, Some(_)) => Err("damageType is only valid on weapons".to_string()),
        (true, Some(_)) if !def.dice.as_deref().is_some_and(valid_dice_notation) => {
            Err("typed weapon requires valid positive NdM damage dice".to_string())
        }
        _ => Ok(()),
    }
}

fn validate_armor_construction(def: &ItemDefinition) -> Result<(), String> {
    match (def.is_body_armor(), def.armor_construction) {
        (true, None) => Err("worn body armor requires armorConstruction".to_string()),
        (false, Some(_)) => Err("armorConstruction is only valid on worn body armor".to_string()),
        _ => Ok(()),
    }
}

fn expected_equipment_kind(def: &ItemDefinition) -> Result<Option<EquipmentKind>, String> {
    let Some(slot) = def.equip_slot else {
        return Ok(None);
    };

    match def.category.as_deref() {
        Some("weapon") => Ok(Some(EquipmentKind::Weapon)),
        Some("fishing_rod") => Ok(Some(EquipmentKind::Tool)),
        Some("clothing") => Ok(Some(EquipmentKind::Clothing)),
        Some("armor") if slot == EquipSlot::OffHand => Ok(Some(EquipmentKind::Shield)),
        Some("armor") => Ok(Some(EquipmentKind::BodyArmor)),
        Some("accessory") => Ok(Some(EquipmentKind::Accessory)),
        _ => Err("equippable item has an unsupported category".to_string()),
    }
}

fn validate_equipment_taxonomy(def: &ItemDefinition) -> Result<(), String> {
    let expected_kind = expected_equipment_kind(def)?;
    if def.equipment_kind != expected_kind {
        return Err(match expected_kind {
            Some(kind) => format!("equippable item requires equipmentKind '{}'", kind.as_str()),
            None => "non-equippable item may not define equipmentKind".to_string(),
        });
    }

    let expected_layer = expected_kind.map(|kind| match kind {
        EquipmentKind::Weapon | EquipmentKind::Tool | EquipmentKind::Shield => EquipmentLayer::Held,
        EquipmentKind::Clothing | EquipmentKind::BodyArmor => EquipmentLayer::Primary,
        EquipmentKind::Accessory => EquipmentLayer::Accessory,
    });
    if def.equipment_layer != expected_layer {
        return Err(match expected_layer {
            Some(layer) => format!(
                "equipmentKind '{}' requires equipmentLayer '{}'",
                expected_kind.unwrap().as_str(),
                layer.as_str()
            ),
            None => "non-equippable item may not define equipmentLayer".to_string(),
        });
    }

    let garment_kind = matches!(
        expected_kind,
        Some(EquipmentKind::Clothing | EquipmentKind::BodyArmor)
    );
    if garment_kind != def.garment_form.is_some() {
        return Err(if garment_kind {
            "worn garment requires garmentForm".to_string()
        } else {
            "garmentForm is only valid on clothing or body armor".to_string()
        });
    }

    if let Some(form) = def.garment_form {
        let valid_slot = match form {
            GarmentForm::Helmet => def.equip_slot == Some(EquipSlot::Head),
            GarmentForm::Cuirass | GarmentForm::Hauberk | GarmentForm::Robe | GarmentForm::Coat => {
                def.equip_slot == Some(EquipSlot::Chest)
            }
            GarmentForm::Leggings => def.equip_slot == Some(EquipSlot::Pants),
            GarmentForm::Gloves => def.equip_slot == Some(EquipSlot::Hands),
            GarmentForm::Boots => def.equip_slot == Some(EquipSlot::Boots),
        };
        if !valid_slot {
            return Err(format!(
                "garmentForm '{}' is invalid for this equipSlot",
                form.as_str()
            ));
        }
    }

    if expected_kind == Some(EquipmentKind::Clothing) && def.guard.is_some_and(|guard| guard != 0) {
        return Err("ordinary clothing may not grant Guard".to_string());
    }

    Ok(())
}

fn validate_defense_skill(def: &ItemDefinition) -> Result<(), String> {
    match def.defense_skill {
        None => return Ok(()),
        Some(SkillId::Shield) => {
            if def.category.as_deref() != Some("armor") {
                return Err("Shield defenseSkill requires category 'armor'".to_string());
            }
            if def.equip_slot != Some(EquipSlot::OffHand) {
                return Err("Shield defenseSkill requires equipSlot 'off_hand'".to_string());
            }
        }
        Some(SkillId::LeatherArmor) => {
            if !def.is_body_armor() {
                return Err("Leather Armor defenseSkill requires worn body armor".to_string());
            }
            if def.equip_slot != Some(EquipSlot::Chest) {
                return Err("Leather Armor defenseSkill requires equipSlot 'chest'".to_string());
            }
            if def.armor_construction != Some(ArmorConstruction::Leather) {
                return Err(
                    "Leather Armor defenseSkill requires armorConstruction 'leather'".to_string(),
                );
            }
            if def.equipment_layer != Some(EquipmentLayer::Primary) {
                return Err(
                    "Leather Armor defenseSkill requires equipmentLayer 'primary'".to_string(),
                );
            }
        }
        Some(_) => {
            return Err("defenseSkill is not supported for defense combat".to_string());
        }
    }
    if !def.guard.is_some_and(|guard| guard > 0) {
        return Err("defenseSkill requires a positive guard value".to_string());
    }
    Ok(())
}

fn validate_use_skill(def: &ItemDefinition) -> Result<(), String> {
    if def.use_skill.is_none() {
        return Ok(());
    }
    if def.use_skill != Some(SkillId::Healing) {
        return Err("useSkill is not supported for consumable use".to_string());
    }
    if def.category.as_deref() != Some("bandage") {
        return Err("Healing useSkill requires category 'bandage'".to_string());
    }
    if !def.consumable {
        return Err("Healing useSkill requires consumable true".to_string());
    }
    if !def.dice.as_deref().is_some_and(valid_dice_notation) {
        return Err("Healing useSkill requires valid positive NdM dice".to_string());
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct ItemDefs {
    defs: Arc<HashMap<String, ItemDefinition>>,
    /// Precomputed at load — defs are immutable, and bites roll every ~5-12 s
    /// per angler.
    catch_table: Arc<Vec<crate::game_state::fishing::CatchCandidate>>,
}

impl ItemDefs {
    pub fn load() -> Self {
        let data = include_str!("../../data/items.json");
        let defs: HashMap<String, ItemDefinition> =
            serde_json::from_str(data).expect("Failed to parse items.json");

        // A chestTier opt-in on a non-equippable or a fishing rod (bought
        // tool, doc/FISHING.md) is a data error — fail the boot, don't
        // silently filter it out of every chest.
        for def in defs.values() {
            if let Err(reason) = validate_equipment_taxonomy(def) {
                panic!("item '{}': {reason}", def.id);
            }
            if let Err(reason) = validate_armor_construction(def) {
                panic!("item '{}': {reason}", def.id);
            }
            if let Err(reason) = validate_weapon_skill(def) {
                panic!("item '{}': {reason}", def.id);
            }
            if let Err(reason) = validate_damage_type(def) {
                panic!("item '{}': {reason}", def.id);
            }
            if let Err(reason) = validate_defense_skill(def) {
                panic!("item '{}': {reason}", def.id);
            }
            if let Err(reason) = validate_use_skill(def) {
                panic!("item '{}': {reason}", def.id);
            }
            assert!(
                def.chest_tier.is_none() || (def.equip_slot.is_some() && !def.is_fishing_rod()),
                "item '{}' has a chestTier but is not chest-eligible equipment",
                def.id
            );
            // The clients' bag-use UX keys off the CSV flag; the server acts
            // on `use_effect`. Fail the boot the moment they disagree.
            assert!(
                def.consumable == def.use_effect().is_some(),
                "item '{}': consumable flag out of step with its use_effect",
                def.id
            );
        }

        info!("Loaded {} item definitions", defs.len());
        for (id, def) in &defs {
            info!(
                "  {} - weight:{} equipSlot:{:?} stackable:{}",
                id, def.weight, def.equip_slot, def.stackable
            );
        }

        let mut catch_table: Vec<_> = defs
            .values()
            .filter_map(|def| {
                Some(crate::game_state::fishing::CatchCandidate {
                    item_def_id: def.id.clone(),
                    rarity: def.rarity_tier.unwrap_or(1),
                    catch_weight: def.catch_weight?,
                    min_fishing_level: def.min_fishing_level.unwrap_or(0),
                })
            })
            .collect();
        catch_table.sort_by(|a, b| a.item_def_id.cmp(&b.item_def_id));

        Self {
            defs: Arc::new(defs),
            catch_table: Arc::new(catch_table),
        }
    }

    pub fn get(&self, item_def_id: &str) -> Option<&ItemDefinition> {
        self.defs.get(item_def_id)
    }

    pub fn all(&self) -> impl Iterator<Item = &ItemDefinition> {
        self.defs.values()
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

    pub fn damage_type_for_weapon_ref(&self, weapon_ref: &str) -> Option<PhysicalDamageType> {
        self.item_def_id_for_weapon_ref(weapon_ref)
            .and_then(|item_id| self.defs.get(&item_id))
            .and_then(|def| def.damage_type)
    }

    /// The chest roll table for a dungeon tier: every opted-in item
    /// (`chestTier` set) at or below the tier, paired with its per-open roll
    /// chance — its own `chestChance` at its home tier, a flat carryover
    /// chance below it so missed pieces can still be filled in upstairs.
    /// Independent per-item rolls keep set-completion odds stable as the
    /// pool grows (doc/ITEM_TIERS.md). Sorted for determinism. `chestTier`
    /// is the sole membership predicate — `load` rejects opt-ins that are
    /// not chest-eligible equipment.
    pub fn chest_roll_table(&self, dungeon_tier: u8) -> Vec<(String, f32)> {
        let mut rows: Vec<(String, f32)> = self
            .defs
            .values()
            .filter_map(|def| {
                let tier = def.chest_tier?;
                let chance = if tier == dungeon_tier {
                    def.chest_chance.unwrap_or(0.0)
                } else if tier < dungeon_tier {
                    CHEST_CARRYOVER_CHANCE
                } else {
                    return None;
                };
                Some((def.id.clone(), chance))
            })
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }

    pub fn weight(&self, item_def_id: &str) -> f32 {
        self.defs.get(item_def_id).map(|d| d.weight).unwrap_or(1.0)
    }

    /// The fishing catch table: every item def with a `catchWeight` — fish,
    /// junk flotsam (rarityTier 0 → no skill XP), and coin catches alike.
    /// Sorted by id for a deterministic cumulative walk.
    pub fn catch_table(&self) -> &[crate::game_state::fishing::CatchCandidate] {
        &self.catch_table
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onlinerpg_shared::skills::SKILL_LEVEL_CAP;
    use serde_json::json;

    fn table_ids(defs: &ItemDefs, tier: u8) -> Vec<String> {
        defs.chest_roll_table(tier)
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    fn weapon_skill_def(
        category: &str,
        equip_slot: &str,
        dice: &str,
        weapon_skill: &str,
    ) -> Result<ItemDefinition, serde_json::Error> {
        serde_json::from_value(json!({
            "id": "test_weapon",
            "name": "Test Weapon",
            "description": "Test definition",
            "weight": 1,
            "equipSlot": equip_slot,
            "stackable": false,
            "category": category,
            "dice": dice,
            "weaponSkill": weapon_skill
        }))
    }

    #[test]
    fn swords_dagger_and_spear_are_the_only_mapped_weapons() {
        let defs = ItemDefs::load();
        for id in [
            "iron_sword",
            "worn_iron_sword",
            "goblin_sword",
            "small_sword",
        ] {
            assert_eq!(
                defs.get(id).unwrap().weapon_skill,
                Some(SkillId::OneHandedSword),
                "{id} weapon skill"
            );
        }
        assert_eq!(
            defs.get("dagger").unwrap().weapon_skill,
            Some(SkillId::Dagger)
        );
        assert_eq!(
            defs.get("spear").unwrap().weapon_skill,
            Some(SkillId::Spear)
        );
        for id in ["torch", "worn_torch", "fishing_rod"] {
            assert_eq!(
                defs.get(id).unwrap().weapon_skill,
                None,
                "{id} stays unmapped"
            );
        }
    }

    #[test]
    fn weapon_skill_assignments_require_main_hand_weapons_with_valid_dice() {
        let valid = weapon_skill_def("weapon", "main_hand", "1d8", "one_handed_sword").unwrap();
        assert_eq!(validate_weapon_skill(&valid), Ok(()));
        let dagger = weapon_skill_def("weapon", "main_hand", "1d4", "dagger").unwrap();
        assert_eq!(validate_weapon_skill(&dagger), Ok(()));
        let spear = weapon_skill_def("weapon", "main_hand", "1d6", "spear").unwrap();
        assert_eq!(validate_weapon_skill(&spear), Ok(()));

        for (category, slot, dice, expected) in [
            ("armor", "main_hand", "1d8", "category 'weapon'"),
            ("weapon", "off_hand", "1d8", "equipSlot 'main_hand'"),
            ("weapon", "main_hand", "1d0", "positive NdM"),
            ("weapon", "main_hand", "bad", "positive NdM"),
        ] {
            let def = weapon_skill_def(category, slot, dice, "one_handed_sword").unwrap();
            let error = validate_weapon_skill(&def).unwrap_err();
            assert!(error.contains(expected), "unexpected error: {error}");
        }

        let fishing = weapon_skill_def("weapon", "main_hand", "1d8", "fishing").unwrap();
        assert!(validate_weapon_skill(&fishing)
            .unwrap_err()
            .contains("not supported"));
    }

    #[test]
    fn unknown_weapon_skill_ids_fail_deserialization() {
        let error = weapon_skill_def("weapon", "main_hand", "1d8", "long_blade").unwrap_err();
        assert!(error.to_string().contains("unknown variant"));
    }

    #[test]
    fn weapon_damage_types_are_explicit_and_validated() {
        let defs = ItemDefs::load();
        for id in [
            "iron_sword",
            "worn_iron_sword",
            "dagger",
            "goblin_sword",
            "small_sword",
        ] {
            assert_eq!(
                defs.get(id).unwrap().damage_type,
                Some(PhysicalDamageType::Slash),
                "{id} damage type"
            );
        }
        assert_eq!(
            defs.get("spear").unwrap().damage_type,
            Some(PhysicalDamageType::Pierce)
        );
        for id in ["torch", "worn_torch"] {
            assert_eq!(
                defs.get(id).unwrap().damage_type,
                Some(PhysicalDamageType::Blunt),
                "{id} damage type"
            );
        }
        assert_eq!(
            defs.damage_type_for_weapon_ref("goblin_sword"),
            Some(PhysicalDamageType::Slash)
        );
        assert_eq!(defs.get("fishing_rod").unwrap().damage_type, None);

        let make = |category: &str, dice: Option<&str>, damage_type: Option<&str>| {
            serde_json::from_value::<ItemDefinition>(json!({
                "id": "typed_item",
                "name": "Typed Item",
                "description": "Test definition",
                "weight": 1,
                "stackable": false,
                "category": category,
                "dice": dice,
                "damageType": damage_type
            }))
            .unwrap()
        };
        assert_eq!(
            validate_damage_type(&make("weapon", Some("1d4"), Some("slash"))),
            Ok(())
        );
        assert!(validate_damage_type(&make("weapon", Some("1d4"), None))
            .unwrap_err()
            .contains("requires damageType"));
        assert!(validate_damage_type(&make("armor", None, Some("blunt")))
            .unwrap_err()
            .contains("only valid on weapons"));
        assert!(
            validate_damage_type(&make("weapon", Some("1d0"), Some("pierce")))
                .unwrap_err()
                .contains("positive NdM")
        );
    }

    #[test]
    fn shields_and_leather_chest_are_the_only_mapped_defensive_items() {
        let defs = ItemDefs::load();
        for id in ["wooden_shield", "raven_shield"] {
            assert_eq!(
                defs.get(id).unwrap().defense_skill,
                Some(SkillId::Shield),
                "{id} defense skill"
            );
        }
        assert_eq!(
            defs.get("leather_armor").unwrap().defense_skill,
            Some(SkillId::LeatherArmor)
        );
        assert_eq!(
            defs.get("leather_armor").unwrap().guard,
            Some(1),
            "the second mitigation slice migrates one former Guard point"
        );
        for id in [
            "torch",
            "leather_helmet",
            "chain_mail",
            "breastplate",
            "ring_of_protection",
            "traveler_robe",
            "padded_battle_robe",
            "brigandine_coat",
        ] {
            assert_eq!(defs.get(id).unwrap().defense_skill, None, "{id} unmapped");
        }
    }

    #[test]
    fn shield_skill_assignments_require_guarded_off_hand_armor() {
        let make = |category: &str, slot: &str, guard: i32, skill: &str| {
            serde_json::from_value::<ItemDefinition>(json!({
                "id": "test_shield",
                "name": "Test Shield",
                "description": "Test definition",
                "weight": 1,
                "equipSlot": slot,
                "stackable": false,
                "category": category,
                "guard": guard,
                "defenseSkill": skill
            }))
        };
        let valid = make("armor", "off_hand", 1, "shield").unwrap();
        assert_eq!(validate_defense_skill(&valid), Ok(()));

        for (category, slot, guard, expected) in [
            ("accessory", "off_hand", 1, "category 'armor'"),
            ("armor", "head", 1, "equipSlot 'off_hand'"),
            ("armor", "off_hand", 0, "positive guard"),
        ] {
            let def = make(category, slot, guard, "shield").unwrap();
            assert!(validate_defense_skill(&def).unwrap_err().contains(expected));
        }
        let unsupported = make("armor", "off_hand", 1, "fishing").unwrap();
        assert!(validate_defense_skill(&unsupported)
            .unwrap_err()
            .contains("not supported"));
    }

    #[test]
    fn body_armor_construction_and_leather_skill_are_explicit() {
        let defs = ItemDefs::load();
        for id in [
            "leather_helmet",
            "leather_armor",
            "leather_gloves",
            "leather_pants",
            "leather_boots",
        ] {
            assert_eq!(
                defs.get(id).unwrap().armor_construction,
                Some(ArmorConstruction::Leather),
                "{id} construction"
            );
        }
        assert_eq!(
            defs.get("chain_mail").unwrap().armor_construction,
            Some(ArmorConstruction::Mail)
        );
        assert_eq!(
            defs.get("chain_mail").unwrap().guard,
            Some(3),
            "the Mail mitigation slice migrates two former Guard points"
        );
        assert_eq!(
            defs.get("padded_battle_robe").unwrap().armor_construction,
            Some(ArmorConstruction::Padded)
        );
        assert_eq!(
            defs.get("brigandine_coat").unwrap().armor_construction,
            Some(ArmorConstruction::Hybrid)
        );
        assert_eq!(
            defs.get("brigandine_coat").unwrap().guard,
            Some(2),
            "the Hybrid mitigation slice migrates two former Guard points"
        );
        for id in [
            "iron_helmet",
            "iron_gauntlets",
            "iron_boots",
            "breastplate",
            "plate_helmet",
            "plate_gauntlets",
            "plate_greaves",
            "plate_boots",
        ] {
            assert_eq!(
                defs.get(id).unwrap().armor_construction,
                Some(ArmorConstruction::Plate),
                "{id} construction"
            );
        }
        for id in ["wooden_shield", "raven_shield", "leather_belt"] {
            assert_eq!(defs.get(id).unwrap().armor_construction, None, "{id}");
        }
        assert_eq!(
            defs.get("breastplate").unwrap().guard,
            Some(4),
            "the Plate mitigation slice migrates three former Guard points"
        );

        let leather = serde_json::from_value::<ItemDefinition>(json!({
            "id": "test_leather",
            "name": "Test Leather",
            "description": "Test definition",
            "weight": 1,
            "equipSlot": "chest",
            "stackable": false,
            "category": "armor",
            "armorConstruction": "leather",
            "equipmentLayer": "primary",
            "guard": 1,
            "defenseSkill": "leather_armor"
        }))
        .unwrap();
        assert_eq!(validate_armor_construction(&leather), Ok(()));
        assert_eq!(validate_defense_skill(&leather), Ok(()));

        for (slot, construction, expected) in [
            ("head", "leather", "equipSlot 'chest'"),
            ("chest", "plate", "armorConstruction 'leather'"),
        ] {
            let def = serde_json::from_value::<ItemDefinition>(json!({
                "id": "bad_leather",
                "name": "Bad Leather",
                "description": "Test definition",
                "weight": 1,
                "equipSlot": slot,
                "stackable": false,
                "category": "armor",
                "armorConstruction": construction,
                "equipmentLayer": "primary",
                "guard": 1,
                "defenseSkill": "leather_armor"
            }))
            .unwrap();
            assert!(validate_defense_skill(&def).unwrap_err().contains(expected));
        }
    }

    #[test]
    fn armor_loadouts_land_in_explicit_strength_ten_burden_bands() {
        use onlinerpg_shared::inventory::{resolve_equipment_burden, EquipmentBurdenTier};

        let defs = ItemDefs::load();
        let burden = |ids: &[&str]| {
            let equipped_weight = ids.iter().map(|id| defs.get(id).unwrap().weight).sum();
            resolve_equipment_burden(equipped_weight, 150.0)
        };

        let padded = burden(&["padded_battle_robe"]);
        assert_eq!(padded.equipped_weight, 6.0);
        assert_eq!(padded.tier, EquipmentBurdenTier::Unburdened);

        let leather = burden(&[
            "leather_helmet",
            "leather_armor",
            "leather_gloves",
            "leather_pants",
            "leather_boots",
        ]);
        assert_eq!(leather.equipped_weight, 16.5);
        assert_eq!(leather.tier, EquipmentBurdenTier::Unburdened);

        let mail = burden(&["chain_mail", "iron_helmet", "iron_gauntlets", "iron_boots"]);
        assert_eq!(mail.equipped_weight, 67.0);
        assert_eq!(mail.tier, EquipmentBurdenTier::Medium);

        let plate = burden(&[
            "breastplate",
            "plate_helmet",
            "plate_gauntlets",
            "plate_greaves",
            "plate_boots",
        ]);
        assert_eq!(plate.equipped_weight, 43.0);
        assert_eq!(plate.tier, EquipmentBurdenTier::Light);

        let hybrid = burden(&["brigandine_coat"]);
        assert_eq!(hybrid.equipped_weight, 14.0);
        assert_eq!(hybrid.tier, EquipmentBurdenTier::Unburdened);
    }

    #[test]
    fn equipment_kind_layer_and_form_are_explicit_and_consistent() {
        let defs = ItemDefs::load();
        for (id, kind, layer, form) in [
            (
                "traveler_robe",
                EquipmentKind::Clothing,
                EquipmentLayer::Primary,
                GarmentForm::Robe,
            ),
            (
                "padded_battle_robe",
                EquipmentKind::BodyArmor,
                EquipmentLayer::Primary,
                GarmentForm::Robe,
            ),
            (
                "brigandine_coat",
                EquipmentKind::BodyArmor,
                EquipmentLayer::Primary,
                GarmentForm::Coat,
            ),
            (
                "chain_mail",
                EquipmentKind::BodyArmor,
                EquipmentLayer::Primary,
                GarmentForm::Hauberk,
            ),
        ] {
            let def = defs.get(id).unwrap();
            assert_eq!(def.equipment_kind, Some(kind), "{id} kind");
            assert_eq!(def.equipment_layer, Some(layer), "{id} layer");
            assert_eq!(def.garment_form, Some(form), "{id} form");
        }

        for (id, kind, layer) in [
            ("iron_sword", EquipmentKind::Weapon, EquipmentLayer::Held),
            ("fishing_rod", EquipmentKind::Tool, EquipmentLayer::Held),
            ("wooden_shield", EquipmentKind::Shield, EquipmentLayer::Held),
            (
                "ring_of_protection",
                EquipmentKind::Accessory,
                EquipmentLayer::Accessory,
            ),
        ] {
            let def = defs.get(id).unwrap();
            assert_eq!(def.equipment_kind, Some(kind), "{id} kind");
            assert_eq!(def.equipment_layer, Some(layer), "{id} layer");
            assert_eq!(def.garment_form, None, "{id} form");
        }
    }

    #[test]
    fn equipment_taxonomy_rejects_missing_or_contradictory_metadata() {
        let make = |extra: serde_json::Value| {
            let mut value = json!({
                "id": "test_robe",
                "name": "Test Robe",
                "description": "Test definition",
                "weight": 1,
                "equipSlot": "chest",
                "stackable": false,
                "category": "clothing"
            });
            value
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            serde_json::from_value::<ItemDefinition>(value).unwrap()
        };

        assert!(validate_equipment_taxonomy(&make(json!({})))
            .unwrap_err()
            .contains("equipmentKind 'clothing'"));
        assert!(validate_equipment_taxonomy(&make(json!({
            "equipmentKind": "clothing",
            "equipmentLayer": "held",
            "garmentForm": "robe"
        })))
        .unwrap_err()
        .contains("equipmentLayer 'primary'"));
        assert!(validate_equipment_taxonomy(&make(json!({
            "equipmentKind": "clothing",
            "equipmentLayer": "primary",
            "garmentForm": "helmet"
        })))
        .unwrap_err()
        .contains("invalid for this equipSlot"));
        assert!(validate_equipment_taxonomy(&make(json!({
            "equipmentKind": "clothing",
            "equipmentLayer": "primary",
            "garmentForm": "robe",
            "guard": 1
        })))
        .unwrap_err()
        .contains("may not grant Guard"));
        assert_eq!(
            validate_equipment_taxonomy(&make(json!({
                "equipmentKind": "clothing",
                "equipmentLayer": "primary",
                "garmentForm": "robe"
            }))),
            Ok(())
        );
    }

    #[test]
    fn bandage_is_the_only_mapped_healing_item() {
        let defs = ItemDefs::load();
        assert_eq!(
            defs.get("bandage").unwrap().use_skill,
            Some(SkillId::Healing)
        );
        for id in [
            "healing_potion",
            "raw_minnow",
            "raw_trout",
            "scroll_of_return",
        ] {
            assert_eq!(defs.get(id).unwrap().use_skill, None, "{id} unmapped");
        }
    }

    #[test]
    fn healing_use_skill_requires_a_real_bandage() {
        let make = |category: &str, dice: &str, consumable: bool, skill: &str| {
            serde_json::from_value::<ItemDefinition>(json!({
                "id": "test_healing_item",
                "name": "Test Healing Item",
                "description": "Test definition",
                "weight": 1,
                "stackable": true,
                "category": category,
                "dice": dice,
                "consumable": consumable,
                "useSkill": skill
            }))
        };
        let valid = make("bandage", "1d6", true, "healing").unwrap();
        assert_eq!(validate_use_skill(&valid), Ok(()));

        for (category, dice, consumable, expected) in [
            ("fish", "1d6", true, "category 'bandage'"),
            ("bandage", "1d6", false, "consumable true"),
            ("bandage", "bad", true, "positive NdM"),
        ] {
            let def = make(category, dice, consumable, "healing").unwrap();
            assert!(validate_use_skill(&def).unwrap_err().contains(expected));
        }
        let unsupported = make("bandage", "1d6", true, "fishing").unwrap();
        assert!(validate_use_skill(&unsupported)
            .unwrap_err()
            .contains("not supported"));
    }

    #[test]
    fn fishing_rod_is_not_dungeon_chest_treasure() {
        // Rods are bought, not looted from bosses: none carries a chestTier,
        // and load() fails the boot if one is ever given it.
        let defs = ItemDefs::load();
        let pool = table_ids(&defs, u8::MAX);
        assert!(
            !pool.contains(&"fishing_rod".to_string()),
            "fishing rod must not be in the dungeon chest loot pool"
        );
        // Sanity: opted-in combat gear still is.
        assert!(
            pool.contains(&"iron_boots".to_string()),
            "expected iron_boots in the chest pool"
        );
    }

    /// Pool membership and chances are laws derived from the defs, so adding
    /// an item can't stale this test. The doc/ITEM_TIERS.md placement is
    /// pinned only by the debut anchors, which move when the design does.
    #[test]
    fn chest_tiers_gate_endgame_loot_by_dungeon() {
        let defs = ItemDefs::load();
        let max_tier = defs.all().filter_map(|d| d.chest_tier).max().unwrap();

        for tier in 1..=max_tier {
            let mut expected: Vec<String> = defs
                .all()
                .filter(|def| def.chest_tier.is_some_and(|home| home <= tier))
                .map(|def| def.id.clone())
                .collect();
            expected.sort();
            assert_eq!(table_ids(&defs, tier), expected, "tier {tier} pool");

            for (id, chance) in defs.chest_roll_table(tier) {
                let def = defs.get(&id).unwrap();
                let want = if def.chest_tier == Some(tier) {
                    def.chest_chance.unwrap_or(0.0)
                } else {
                    CHEST_CARRYOVER_CHANCE
                };
                assert_eq!(chance, want, "{id} chance at tier {tier}");
            }
        }

        // Each set's core debuts one dungeon above its opener.
        let debut = |id: &str| defs.get(id).unwrap().chest_tier;
        assert_eq!(debut("leather_helmet"), Some(1));
        assert_eq!(debut("leather_armor"), Some(2));
        assert_eq!(debut("chain_mail"), Some(3));
        assert_eq!(debut("breastplate"), Some(4));
        assert_eq!(debut("ring_of_protection"), Some(5));
        // Weapons and cash valuables stay out of chests entirely.
        for id in ["iron_sword", "gold_ring", "healing_potion"] {
            assert_eq!(debut(id), None, "{id} must stay out of chest pools");
        }
    }

    /// The doc's farming target: completing a tier's new set pieces takes
    /// ~5 chest opens on average. Closed form for independent per-open
    /// rolls — E[all K collected] by inclusion–exclusion over geometrics.
    #[test]
    fn chest_chances_hit_five_run_completion() {
        let defs = ItemDefs::load();

        fn expected_opens_to_collect(chances: &[f32]) -> f64 {
            let k = chances.len() as u32;
            let mut expectation = 0.0;
            for mask in 1..(1u32 << k) {
                let mut miss_all = 1.0;
                for (i, &p) in chances.iter().enumerate() {
                    if mask & (1 << i) != 0 {
                        miss_all *= 1.0 - f64::from(p);
                    }
                }
                let sign = if mask.count_ones() % 2 == 1 {
                    1.0
                } else {
                    -1.0
                };
                expectation += sign / (1.0 - miss_all);
            }
            expectation
        }

        // Old Crypt (tier 1): signature leather_helmet is guaranteed; the
        // remaining set pieces must land in ~5 runs.
        let t1: Vec<f32> = defs
            .chest_roll_table(1)
            .into_iter()
            .filter(|(id, _)| id != "leather_helmet")
            .map(|(_, chance)| chance)
            .collect();
        let opens = expected_opens_to_collect(&t1);
        assert!(
            (4.0..=6.0).contains(&opens),
            "old_crypt set completion expects ~5 opens, got {opens:.2}"
        );

        // Tier-2 home pieces roll at the doc's K=4 constant even while the
        // set is still missing assets; carryovers roll at the flat 10%.
        let t2: std::collections::HashMap<String, f32> =
            defs.chest_roll_table(2).into_iter().collect();
        assert_eq!(t2["iron_boots"], 0.37);
        assert_eq!(t2["iron_helmet"], 0.37);
        assert_eq!(t2["leather_gloves"], 0.37);
        assert_eq!(t2["leather_boots"], 0.37);
        assert_eq!(t2["raven_shield"], 0.2);
        assert_eq!(t2["leather_armor"], 0.0, "signature rolls only as itself");
        for id in ["leather_helmet", "leather_pants", "leather_belt"] {
            assert_eq!(t2[id], CHEST_CARRYOVER_CHANCE, "{id} carries over at 10%");
        }
    }

    #[test]
    fn catch_table_spans_fish_junk_and_coins() {
        let defs = ItemDefs::load();
        let table = defs.catch_table();
        let ids: Vec<&str> = table.iter().map(|c| c.item_def_id.as_str()).collect();
        for expected in [
            "raw_minnow",
            "golden_sturgeon",
            "old_boot",
            "message_in_a_bottle",
            "sunken_coin_pouch",
        ] {
            assert!(
                ids.contains(&expected),
                "{expected} missing from catch table"
            );
        }
        // Junk and coin catches are rarity 0: the XP formula (10·rarity²)
        // grants nothing for them, and only fish carry tiers ≥ 1.
        for c in table {
            let def = defs.get(&c.item_def_id).unwrap();
            if def.is_fish() {
                assert!(c.rarity >= 1, "{} fish tier", c.item_def_id);
            } else {
                assert_eq!(c.rarity, 0, "{} must be tier 0 (no XP)", c.item_def_id);
            }
        }
    }

    /// The economy guardrail as a contract test: the expected *sell* value of
    /// one catch must stay at coin-pile magnitude (the game's repeatable gold
    /// faucet is 1–10c piles; a catch should be worth a couple of piles, not
    /// a wage) — and it must hold at every fishing level, not just at level 0.
    /// Averaging over raw `catchWeight` would only ever measure a beginner.
    #[test]
    fn expected_catch_value_stays_in_the_coin_pile_economy_at_every_level() {
        fn dice_avg(notation: &str) -> f64 {
            let (n, m) = notation.split_once('d').expect("NdM");
            let n: f64 = n.parse().unwrap();
            let m: f64 = m.parse().unwrap();
            n * (m + 1.0) / 2.0
        }
        let defs = ItemDefs::load();
        let table = defs.catch_table();
        let ev_at = |level: u32| -> f64 {
            let weights = crate::game_state::fishing::effective_weights(table, level);
            let total: f64 = weights.iter().map(|w| *w as f64).sum();
            weights
                .iter()
                .zip(table)
                .map(|(weight, c)| {
                    let def = defs.get(&c.item_def_id).unwrap();
                    let value = if def.is_coin_catch() {
                        // Coins arrive at face value.
                        def.dice.as_deref().map_or(0.0, dice_avg)
                    } else {
                        // Items sell at the merchant rate (Rica: 40%).
                        def.base_price.unwrap_or(0) as f64 * 0.4
                    };
                    *weight as f64 * value
                })
                .sum::<f64>()
                / total
        };

        let evs: Vec<f64> = (0..=SKILL_LEVEL_CAP).map(ev_at).collect();
        for (level, ev) in evs.iter().enumerate() {
            assert!(
                (5.0..=25.0).contains(ev),
                "expected sell value per catch at level {level} is {ev:.1}c — outside \
                 the 5–25c coin-pile band"
            );
        }
        assert!(
            evs.windows(2).all(|w| w[1] >= w[0]),
            "skill should never make an angler poorer"
        );
        // Mastery pays a better wage, not a different economy. Without this
        // the old additive weighting reached 10x and no test noticed.
        assert!(
            evs[evs.len() - 1] <= 4.0 * evs[0],
            "level {SKILL_LEVEL_CAP} earns {:.1}c vs {:.1}c at level 0 — that is a \
             different economy, not a better wage",
            evs[evs.len() - 1],
            evs[0]
        );
    }

    /// The flotsam price sheet: gag junk is worthless by design, the bottle
    /// pays a token, and the pouch pays through its dice — not a resale price.
    #[test]
    fn junk_pricing_matches_the_gag() {
        let defs = ItemDefs::load();
        assert!(
            defs.get("old_boot").unwrap().base_price.is_none(),
            "a boot is worthless by design"
        );
        assert!(defs.get("clump_of_kelp").unwrap().base_price.is_none());
        assert_eq!(
            defs.get("message_in_a_bottle").unwrap().base_price,
            Some(15)
        );
        let pouch = defs.get("sunken_coin_pouch").unwrap();
        assert!(pouch.is_coin_catch());
        assert_eq!(pouch.dice.as_deref(), Some("3d8"));
        assert!(
            pouch.base_price.is_none(),
            "the pouch pays via its dice, not a merchant sale"
        );
    }

    /// Trophies are gated to fish: junk never celebrates, a natural 20 always
    /// does on a fish, and the size threshold is an exact boundary.
    #[test]
    fn trophies_are_a_fish_concept() {
        let defs = ItemDefs::load();
        let boot = defs.get("old_boot").unwrap();
        assert!(
            !boot.trophy_at(200, true),
            "a nat-20 boot is still just a boot"
        );
        let minnow = defs.get("raw_minnow").unwrap();
        assert!(
            minnow.trophy_at(1, true),
            "a natural 20 is always a trophy on a fish"
        );
        let trout = defs.get("raw_trout").unwrap();
        let threshold = trout.trophy_cm.unwrap() as u16;
        assert!(trout.trophy_at(threshold, false));
        assert!(!trout.trophy_at(threshold - 1, false));
    }

    #[test]
    fn fishing_rod_is_a_rod_not_a_weapon() {
        let defs = ItemDefs::load();
        let rod = defs.get("fishing_rod").expect("fishing_rod def");
        assert!(rod.is_fishing_rod());
        assert!(!rod.is_weapon(), "the rod must not deal weapon damage");
    }
}
