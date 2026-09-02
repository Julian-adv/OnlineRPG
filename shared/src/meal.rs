//! Table meals: a dish an inn maid sets on the table in front of a seated
//! guest, eaten in place by clicking it (doc/HUNGER.md).

use serde::{Deserialize, Serialize};

/// A served dish standing on a table top. Wire type (positional array — never
/// reorder fields, append only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meal {
    pub id: u64,
    pub item_def_id: String,
    /// Placement id of the chair it was served to; eating requires sitting
    /// on exactly this chair.
    pub chair_object_id: u32,
    pub for_player: crate::entity::PlayerId,
    /// Table-top point in front of the chair.
    pub position: crate::world::Position,
    /// Yaw facing the guest, in radians.
    pub rotation: f32,
    pub floor_level: i8,
    /// Finished: the plate stays on the table until the maid clears it.
    pub eaten: bool,
}

/// Table-top height above a `table` placement's origin: the GLB's top
/// surface (0.761 measured). 0.80 floated, 0.75 sat sunk in.
pub const TABLE_SURFACE_Y: f32 = 0.761;

/// Plate radius plus a little margin, kept inside the table edge.
pub const MEAL_EDGE_INSET_M: f32 = 0.23;

/// A chair belongs to the nearest table within this range (centre to centre).
pub const CHAIR_TABLE_RADIUS_M: f32 = 2.0;

/// How close the maid must stand to the chair to serve or clear.
pub const MEAL_SERVICE_RADIUS_M: f32 = 4.0;

/// A chair holds one of each: the plate to the guest's left, the cup to
/// their right, so an order of both never overlaps. A dish fills the guest
/// whatever they had; a drink only adds its own nutrition and keeps its
/// model when finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MealSlot {
    Dish,
    Drink,
}

impl MealSlot {
    pub fn for_category(category: Option<&str>) -> MealSlot {
        if category == Some("drink") {
            MealSlot::Drink
        } else {
            MealSlot::Dish
        }
    }

    /// Sideways shift along the table edge, positive toward the guest's
    /// left. The cup goes further out than the plate so both fit.
    pub fn lateral_m(self) -> f32 {
        match self {
            MealSlot::Dish => 0.10,
            MealSlot::Drink => -0.25,
        }
    }
}

/// What a maid may put on a table: food or drink that has a world model.
/// Everything else is refused server-side.
pub fn is_servable(
    category: Option<&str>,
    world_model: Option<&str>,
    nutrition: Option<u32>,
) -> bool {
    matches!(category, Some("food" | "drink")) && world_model.is_some() && nutrition.is_some()
}
