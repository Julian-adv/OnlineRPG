use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::Position;

/// Server-authored movement burden from equipped items. Bag contents still
/// count toward carry capacity but do not slow movement in this first slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EquipmentBurdenTier {
    Unburdened,
    Light,
    Medium,
    Heavy,
}

impl EquipmentBurdenTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unburdened => "unburdened",
            Self::Light => "light",
            Self::Medium => "medium",
            Self::Heavy => "heavy",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Unburdened => "Unburdened",
            Self::Light => "Light",
            Self::Medium => "Medium",
            Self::Heavy => "Heavy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EquipmentBurden {
    pub equipped_weight: f32,
    pub max_carry_weight: f32,
    pub tier: EquipmentBurdenTier,
    pub movement_speed: f32,
}

/// Resolve gradual movement bands from equipped load as a fraction of the
/// character's carry capacity. Invalid inputs are clamped so legacy data can
/// never produce a negative or non-finite movement speed.
pub fn resolve_equipment_burden(equipped_weight: f32, max_carry_weight: f32) -> EquipmentBurden {
    let equipped_weight = if equipped_weight.is_finite() {
        equipped_weight.max(0.0)
    } else {
        0.0
    };
    let max_carry_weight = if max_carry_weight.is_finite() && max_carry_weight > 0.0 {
        max_carry_weight
    } else {
        1.0
    };
    let ratio = equipped_weight / max_carry_weight;
    let (tier, speed_multiplier) = if ratio <= 0.20 {
        (EquipmentBurdenTier::Unburdened, 1.0)
    } else if ratio <= 0.35 {
        (EquipmentBurdenTier::Light, 0.9)
    } else if ratio <= 0.50 {
        (EquipmentBurdenTier::Medium, 0.8)
    } else {
        (EquipmentBurdenTier::Heavy, 0.7)
    };
    EquipmentBurden {
        equipped_weight,
        max_carry_weight,
        tier,
        movement_speed: crate::world::PLAYER_MOVE_SPEED * speed_multiplier,
    }
}

/// Physical construction of worn body armor. Absent on clothing, shields,
/// accessories, weapons, and other items. Add a variant only with real content
/// that consumes it; broader future taxonomy lives in doc/ARMOR_SYSTEM.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArmorConstruction {
    Padded,
    Leather,
    Mail,
    Plate,
    Hybrid,
}

impl ArmorConstruction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Padded => "padded",
            Self::Leather => "leather",
            Self::Mail => "mail",
            Self::Plate => "plate",
            Self::Hybrid => "hybrid",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Padded => "Padded",
            Self::Leather => "Leather",
            Self::Mail => "Mail",
            Self::Plate => "Plate",
            Self::Hybrid => "Hybrid",
        }
    }
}

impl std::str::FromStr for ArmorConstruction {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "padded" => Ok(Self::Padded),
            "leather" => Ok(Self::Leather),
            "mail" => Ok(Self::Mail),
            "plate" => Ok(Self::Plate),
            "hybrid" => Ok(Self::Hybrid),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairFamily {
    Cloth,
    Leather,
    Metal,
    Hybrid,
}

impl RepairFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cloth => "cloth",
            Self::Leather => "leather",
            Self::Metal => "metal",
            Self::Hybrid => "hybrid",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Cloth => "Cloth",
            Self::Leather => "Leather",
            Self::Metal => "Metal",
            Self::Hybrid => "Hybrid",
        }
    }

    pub fn for_construction(construction: ArmorConstruction) -> Self {
        match construction {
            ArmorConstruction::Padded => Self::Cloth,
            ArmorConstruction::Leather => Self::Leather,
            ArmorConstruction::Mail | ArmorConstruction::Plate => Self::Metal,
            ArmorConstruction::Hybrid => Self::Hybrid,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurabilityCondition {
    Pristine,
    Worn,
    Damaged,
    Critical,
    Broken,
}

impl DurabilityCondition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pristine => "pristine",
            Self::Worn => "worn",
            Self::Damaged => "damaged",
            Self::Critical => "critical",
            Self::Broken => "broken",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Pristine => "Pristine",
            Self::Worn => "Worn",
            Self::Damaged => "Damaged",
            Self::Critical => "Critical",
            Self::Broken => "Broken",
        }
    }
}

pub fn durability_condition(current: u32, max: u32) -> Option<DurabilityCondition> {
    if max == 0 {
        return None;
    }
    let current = u64::from(current.min(max));
    let max = u64::from(max);
    Some(if current == 0 {
        DurabilityCondition::Broken
    } else if current * 4 <= max {
        DurabilityCondition::Critical
    } else if current * 2 <= max {
        DurabilityCondition::Damaged
    } else if current * 4 <= max * 3 {
        DurabilityCondition::Worn
    } else {
        DurabilityCondition::Pristine
    })
}

/// Minimum share of normal NPC resale value retained by broken durable gear.
pub const DURABILITY_VALUE_FLOOR_PERCENT: u32 = 25;

/// Smooth NPC resale-value multiplier for a durable item. Full condition is
/// worth 100%; broken gear retains a 25% salvage floor. Values above the
/// definition maximum clamp to full value, while a zero maximum is invalid.
pub fn durability_value_percent(current: u32, max: u32) -> Option<u32> {
    if max == 0 {
        return None;
    }
    let current = u64::from(current.min(max));
    let max = u64::from(max);
    let variable_percent = 100 - DURABILITY_VALUE_FLOOR_PERCENT;
    Some(DURABILITY_VALUE_FLOOR_PERCENT + (u64::from(variable_percent) * current / max) as u32)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EquipmentKind {
    Weapon,
    Tool,
    Clothing,
    BodyArmor,
    Shield,
    Accessory,
}

impl EquipmentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Weapon => "weapon",
            Self::Tool => "tool",
            Self::Clothing => "clothing",
            Self::BodyArmor => "body_armor",
            Self::Shield => "shield",
            Self::Accessory => "accessory",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Weapon => "Weapon",
            Self::Tool => "Tool",
            Self::Clothing => "Clothing",
            Self::BodyArmor => "Body Armor",
            Self::Shield => "Shield",
            Self::Accessory => "Accessory",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EquipmentLayer {
    Held,
    Primary,
    Accessory,
}

impl EquipmentLayer {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Held => "held",
            Self::Primary => "primary",
            Self::Accessory => "accessory",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Held => "Held",
            Self::Primary => "Primary",
            Self::Accessory => "Accessory",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GarmentForm {
    Helmet,
    Cuirass,
    Leggings,
    Gloves,
    Boots,
    Hauberk,
    Robe,
    Coat,
}

impl GarmentForm {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Helmet => "helmet",
            Self::Cuirass => "cuirass",
            Self::Leggings => "leggings",
            Self::Gloves => "gloves",
            Self::Boots => "boots",
            Self::Hauberk => "hauberk",
            Self::Robe => "robe",
            Self::Coat => "coat",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Helmet => "Helmet",
            Self::Cuirass => "Cuirass",
            Self::Leggings => "Leggings",
            Self::Gloves => "Gloves",
            Self::Boots => "Boots",
            Self::Hauberk => "Hauberk",
            Self::Robe => "Robe",
            Self::Coat => "Coat",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyRegion {
    Head,
    Torso,
    Arms,
    Hands,
    Legs,
    Feet,
}

pub const BODY_REGIONS: [BodyRegion; 6] = [
    BodyRegion::Head,
    BodyRegion::Torso,
    BodyRegion::Arms,
    BodyRegion::Hands,
    BodyRegion::Legs,
    BodyRegion::Feet,
];
pub const BODY_COVERAGE_SCALE: u32 = 100;

impl BodyRegion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Torso => "torso",
            Self::Arms => "arms",
            Self::Hands => "hands",
            Self::Legs => "legs",
            Self::Feet => "feet",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Head => "Head",
            Self::Torso => "Torso",
            Self::Arms => "Arms",
            Self::Hands => "Hands",
            Self::Legs => "Legs",
            Self::Feet => "Feet",
        }
    }

    pub fn coverage_weight(self) -> u32 {
        match self {
            Self::Head => 10,
            Self::Torso => 40,
            Self::Arms => 15,
            Self::Hands => 5,
            Self::Legs => 20,
            Self::Feet => 10,
        }
    }

    fn order(self) -> u8 {
        match self {
            Self::Head => 0,
            Self::Torso => 1,
            Self::Arms => 2,
            Self::Hands => 3,
            Self::Legs => 4,
            Self::Feet => 5,
        }
    }
}

pub fn body_coverage_percent(regions: impl IntoIterator<Item = BodyRegion>) -> u32 {
    let mut seen = [false; 6];
    regions
        .into_iter()
        .filter(|region| {
            let index = region.order() as usize;
            let is_new = !seen[index];
            seen[index] = true;
            is_new
        })
        .map(BodyRegion::coverage_weight)
        .sum::<u32>()
        .min(BODY_COVERAGE_SCALE)
}

impl std::str::FromStr for BodyRegion {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "head" => Ok(Self::Head),
            "torso" => Ok(Self::Torso),
            "arms" => Ok(Self::Arms),
            "hands" => Ok(Self::Hands),
            "legs" => Ok(Self::Legs),
            "feet" => Ok(Self::Feet),
            _ => Err(()),
        }
    }
}

pub fn parse_body_coverage(value: &str) -> Result<Vec<BodyRegion>, String> {
    if value.is_empty() {
        return Err("bodyCoverage must not be empty".to_string());
    }

    let mut regions = Vec::new();
    for token in value.split(';') {
        let region = token
            .parse::<BodyRegion>()
            .map_err(|()| format!("bodyCoverage has unknown region '{token}'"))?;
        if regions.contains(&region) {
            return Err(format!("bodyCoverage repeats region '{token}'"));
        }
        if regions
            .last()
            .is_some_and(|previous: &BodyRegion| previous.order() >= region.order())
        {
            return Err(
                "bodyCoverage must follow head;torso;arms;hands;legs;feet order".to_string(),
            );
        }
        regions.push(region);
    }
    Ok(regions)
}

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
    /// Remaining condition for durable item definitions. `None` means the
    /// definition is not durable; legacy durable rows are hydrated by the
    /// server from their definition before entering live inventory state.
    #[serde(default)]
    pub durability: Option<u32>,
}

impl ItemInstance {
    pub fn is_broken(&self) -> bool {
        self.durability == Some(0)
    }
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
}

/// Item defs that act as a carried light source.
pub const TORCH_ITEM_IDS: &[&str] = &["torch", "worn_torch"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundItem {
    pub instance_id: u64,
    pub item_def_id: String,
    pub position: Position,
    pub floor_level: i8,
    /// Carries a dropped weapon's enchantment so picking it back up
    /// doesn't wipe it.
    #[serde(default)]
    pub enchant: i32,
    /// Per-instance condition preserved while an item is on the ground.
    #[serde(default)]
    pub durability: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn armor_constructions_have_stable_data_names() {
        for (construction, wire, display) in [
            (ArmorConstruction::Padded, "padded", "Padded"),
            (ArmorConstruction::Leather, "leather", "Leather"),
            (ArmorConstruction::Mail, "mail", "Mail"),
            (ArmorConstruction::Plate, "plate", "Plate"),
            (ArmorConstruction::Hybrid, "hybrid", "Hybrid"),
        ] {
            assert_eq!(construction.as_str(), wire);
            assert_eq!(construction.display_name(), display);
            assert_eq!(
                serde_json::to_string(&construction).unwrap(),
                format!("\"{wire}\"")
            );
            assert_eq!(
                serde_json::from_str::<ArmorConstruction>(&format!("\"{wire}\"")).unwrap(),
                construction
            );
            assert_eq!(wire.parse::<ArmorConstruction>(), Ok(construction));
        }
        assert!(serde_json::from_str::<ArmorConstruction>("\"chain\"").is_err());
        assert!("chain".parse::<ArmorConstruction>().is_err());
    }

    #[test]
    fn repair_families_have_stable_names_and_construction_mapping() {
        for (family, wire, display) in [
            (RepairFamily::Cloth, "cloth", "Cloth"),
            (RepairFamily::Leather, "leather", "Leather"),
            (RepairFamily::Metal, "metal", "Metal"),
            (RepairFamily::Hybrid, "hybrid", "Hybrid"),
        ] {
            assert_eq!(family.as_str(), wire);
            assert_eq!(family.display_name(), display);
            assert_eq!(
                serde_json::to_string(&family).unwrap(),
                format!("\"{wire}\"")
            );
            assert_eq!(
                serde_json::from_str::<RepairFamily>(&format!("\"{wire}\"")).unwrap(),
                family
            );
        }

        for (construction, family) in [
            (ArmorConstruction::Padded, RepairFamily::Cloth),
            (ArmorConstruction::Leather, RepairFamily::Leather),
            (ArmorConstruction::Mail, RepairFamily::Metal),
            (ArmorConstruction::Plate, RepairFamily::Metal),
            (ArmorConstruction::Hybrid, RepairFamily::Hybrid),
        ] {
            assert_eq!(RepairFamily::for_construction(construction), family);
        }
        assert!(serde_json::from_str::<RepairFamily>("\"wood\"").is_err());
    }

    #[test]
    fn durability_condition_bands_have_stable_boundaries() {
        for (condition, wire, display) in [
            (DurabilityCondition::Pristine, "pristine", "Pristine"),
            (DurabilityCondition::Worn, "worn", "Worn"),
            (DurabilityCondition::Damaged, "damaged", "Damaged"),
            (DurabilityCondition::Critical, "critical", "Critical"),
            (DurabilityCondition::Broken, "broken", "Broken"),
        ] {
            assert_eq!(condition.as_str(), wire);
            assert_eq!(condition.display_name(), display);
            assert_eq!(
                serde_json::to_string(&condition).unwrap(),
                format!("\"{wire}\"")
            );
        }

        for (current, expected) in [
            (61, DurabilityCondition::Pristine),
            (60, DurabilityCondition::Pristine),
            (46, DurabilityCondition::Pristine),
            (45, DurabilityCondition::Worn),
            (31, DurabilityCondition::Worn),
            (30, DurabilityCondition::Damaged),
            (16, DurabilityCondition::Damaged),
            (15, DurabilityCondition::Critical),
            (1, DurabilityCondition::Critical),
            (0, DurabilityCondition::Broken),
        ] {
            assert_eq!(durability_condition(current, 60), Some(expected));
        }
        assert_eq!(durability_condition(1, 0), None);
    }

    #[test]
    fn durability_value_is_smooth_bounded_and_keeps_a_salvage_floor() {
        for (current, expected) in [
            (61, 100),
            (60, 100),
            (45, 81),
            (30, 62),
            (15, 43),
            (1, 26),
            (0, DURABILITY_VALUE_FLOOR_PERCENT),
        ] {
            assert_eq!(durability_value_percent(current, 60), Some(expected));
        }
        assert_eq!(durability_value_percent(1, 0), None);

        let values: Vec<u32> = (0..=60)
            .map(|current| durability_value_percent(current, 60).unwrap())
            .collect();
        assert!(values.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(values
            .iter()
            .all(|value| (DURABILITY_VALUE_FLOOR_PERCENT..=100).contains(value)));
    }

    #[test]
    fn equipment_burden_bands_are_gradual_stable_and_bounded() {
        for (weight, expected_tier, expected_speed) in [
            (0.0, EquipmentBurdenTier::Unburdened, 3.0),
            (30.0, EquipmentBurdenTier::Unburdened, 3.0),
            (30.1, EquipmentBurdenTier::Light, 2.7),
            (52.5, EquipmentBurdenTier::Light, 2.7),
            (52.6, EquipmentBurdenTier::Medium, 2.4),
            (75.0, EquipmentBurdenTier::Medium, 2.4),
            (75.1, EquipmentBurdenTier::Heavy, 2.1),
            (150.0, EquipmentBurdenTier::Heavy, 2.1),
        ] {
            let burden = resolve_equipment_burden(weight, 150.0);
            assert_eq!(burden.tier, expected_tier, "weight {weight}");
            assert!((burden.movement_speed - expected_speed).abs() < 0.000_001);
            assert!(burden.movement_speed > 0.0);
            assert!(burden.movement_speed <= crate::world::PLAYER_MOVE_SPEED);
        }
    }

    #[test]
    fn equipment_burden_sanitizes_invalid_inputs() {
        let burden = resolve_equipment_burden(f32::NAN, f32::INFINITY);
        assert_eq!(burden.equipped_weight, 0.0);
        assert_eq!(burden.max_carry_weight, 1.0);
        assert_eq!(burden.tier, EquipmentBurdenTier::Unburdened);
        assert_eq!(burden.movement_speed, crate::world::PLAYER_MOVE_SPEED);
    }

    #[test]
    fn equipment_taxonomy_has_stable_data_names() {
        for (kind, wire, display) in [
            (EquipmentKind::Weapon, "weapon", "Weapon"),
            (EquipmentKind::Tool, "tool", "Tool"),
            (EquipmentKind::Clothing, "clothing", "Clothing"),
            (EquipmentKind::BodyArmor, "body_armor", "Body Armor"),
            (EquipmentKind::Shield, "shield", "Shield"),
            (EquipmentKind::Accessory, "accessory", "Accessory"),
        ] {
            assert_eq!(kind.as_str(), wire);
            assert_eq!(kind.display_name(), display);
            assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{wire}\""));
        }

        for (layer, wire, display) in [
            (EquipmentLayer::Held, "held", "Held"),
            (EquipmentLayer::Primary, "primary", "Primary"),
            (EquipmentLayer::Accessory, "accessory", "Accessory"),
        ] {
            assert_eq!(layer.as_str(), wire);
            assert_eq!(layer.display_name(), display);
            assert_eq!(
                serde_json::to_string(&layer).unwrap(),
                format!("\"{wire}\"")
            );
        }

        for (form, wire, display) in [
            (GarmentForm::Helmet, "helmet", "Helmet"),
            (GarmentForm::Cuirass, "cuirass", "Cuirass"),
            (GarmentForm::Leggings, "leggings", "Leggings"),
            (GarmentForm::Gloves, "gloves", "Gloves"),
            (GarmentForm::Boots, "boots", "Boots"),
            (GarmentForm::Hauberk, "hauberk", "Hauberk"),
            (GarmentForm::Robe, "robe", "Robe"),
            (GarmentForm::Coat, "coat", "Coat"),
        ] {
            assert_eq!(form.as_str(), wire);
            assert_eq!(form.display_name(), display);
            assert_eq!(serde_json::to_string(&form).unwrap(), format!("\"{wire}\""));
        }
    }

    #[test]
    fn body_regions_and_authored_coverage_have_stable_data_names() {
        for (region, wire, display) in [
            (BodyRegion::Head, "head", "Head"),
            (BodyRegion::Torso, "torso", "Torso"),
            (BodyRegion::Arms, "arms", "Arms"),
            (BodyRegion::Hands, "hands", "Hands"),
            (BodyRegion::Legs, "legs", "Legs"),
            (BodyRegion::Feet, "feet", "Feet"),
        ] {
            assert_eq!(region.as_str(), wire);
            assert_eq!(region.display_name(), display);
            assert_eq!(wire.parse::<BodyRegion>(), Ok(region));
            assert_eq!(
                serde_json::to_string(&region).unwrap(),
                format!("\"{wire}\"")
            );
        }

        assert_eq!(
            parse_body_coverage("torso;arms;legs"),
            Ok(vec![BodyRegion::Torso, BodyRegion::Arms, BodyRegion::Legs])
        );
        for invalid in [
            "",
            "torso;wings",
            "torso;arms;arms",
            "legs;torso",
            "torso;;legs",
        ] {
            assert!(parse_body_coverage(invalid).is_err(), "{invalid}");
        }

        assert_eq!(body_coverage_percent(BODY_REGIONS), BODY_COVERAGE_SCALE);
        assert_eq!(
            body_coverage_percent([
                BodyRegion::Torso,
                BodyRegion::Arms,
                BodyRegion::Legs,
                BodyRegion::Legs,
            ]),
            75
        );
    }

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
