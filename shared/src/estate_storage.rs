use crate::{inventory::ItemInstance, Position};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

pub const INTERACTION_RANGE: f32 = 3.0;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EstateStorageDefinition {
    pub id: String,
    pub model_id: String,
    pub capacity_kg: f32,
    pub snap_step: f32,
    pub rotation_step: f32,
    pub footprint_width: f32,
    pub footprint_depth: f32,
    pub min_floor: i8,
    pub max_floor: i8,
    pub floor_edge_clearance: f32,
    pub indoor_collision_radius: f32,
    pub outdoor_collision_radius: f32,
}

impl EstateStorageDefinition {
    pub fn max_weight(&self) -> f32 {
        self.capacity_kg * 10.0
    }
}

pub fn estate_storage_defs() -> &'static HashMap<String, EstateStorageDefinition> {
    static DEFINITIONS: OnceLock<HashMap<String, EstateStorageDefinition>> = OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        let definitions: HashMap<String, EstateStorageDefinition> =
            serde_json::from_str(include_str!("../../data/estate_storage.json"))
                .expect("estate_storage.json is malformed");
        for (id, definition) in &definitions {
            assert_eq!(id, &definition.id, "estate storage key and id differ");
            assert!(
                definition.capacity_kg > 0.0,
                "{id}: capacity must be positive"
            );
            assert!(
                definition.snap_step > 0.0,
                "{id}: snap step must be positive"
            );
            assert!(
                definition.rotation_step > 0.0,
                "{id}: rotation step must be positive"
            );
            assert!(
                definition.footprint_width > 0.0 && definition.footprint_depth > 0.0,
                "{id}: footprint must be positive"
            );
            assert!(
                definition.min_floor <= definition.max_floor,
                "{id}: invalid floor range"
            );
            assert!(
                definition.floor_edge_clearance >= 0.0
                    && definition.indoor_collision_radius >= 0.0
                    && definition.outdoor_collision_radius >= 0.0,
                "{id}: placement margins must not be negative"
            );
            let occupancy = crate::furniture::solid_occupancy(&definition.model_id)
                .unwrap_or_else(|| panic!("{id}: model has no solid furniture footprint"));
            let measured_width = occupancy.max_x - occupancy.min_x;
            let measured_depth = occupancy.max_z - occupancy.min_z;
            assert!(
                (measured_width - definition.footprint_width).abs() < 0.001
                    && (measured_depth - definition.footprint_depth).abs() < 0.001,
                "{id}: configured footprint differs from the model footprint"
            );
        }
        definitions
    })
}

pub fn estate_storage_def(item_def_id: &str) -> Option<&'static EstateStorageDefinition> {
    estate_storage_defs().get(item_def_id)
}

pub fn is_estate_storage_item(item_def_id: &str) -> bool {
    estate_storage_def(item_def_id).is_some()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstateChest {
    pub id: i64,
    pub estate_id: i64,
    pub owner_id: i64,
    pub item_def_id: String,
    pub position: Position,
    pub rotation_deg: f32,
    pub floor_level: i8,
    pub overdue: bool,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstateChestState {
    pub chest_id: i64,
    pub item_def_id: String,
    pub revision: u64,
    pub max_weight: f32,
    pub can_deposit: bool,
    pub items: Vec<ItemInstance>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wooden_storage_definition_is_data_driven() {
        let definition = estate_storage_def("storage_chest").unwrap();
        assert_eq!(definition.model_id, "chest_animated");
        assert_eq!(definition.max_weight(), 500.0);
        assert_eq!(definition.rotation_step, 90.0);
    }
}
