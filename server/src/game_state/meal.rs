//! Table meals: an inn maid sets a dish on the table in front of a seated
//! guest, who eats it in place (doc/HUNGER.md). Keyed by id, never persisted.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use onlinerpg_shared::furniture::{solid_occupancy, FurniturePlacement};
use onlinerpg_shared::meal::{
    is_servable, Meal, MealSlot, CHAIR_TABLE_RADIUS_M, MEAL_EDGE_INSET_M, MEAL_SERVICE_RADIUS_M,
    TABLE_SURFACE_Y,
};
use onlinerpg_shared::{Player, PlayerId, ServerMessage};

use super::passability::regions_around;
use super::GameState;
use crate::types::Position;

/// How long a plate stays after its guest leaves the chair — the maid's walk
/// over to clear it, with margin.
const MEAL_LINGER: Duration = Duration::from_secs(90);
/// An untouched plate is cleared eventually even with the guest still seated.
const MEAL_MAX_AGE: Duration = Duration::from_secs(30 * 60);
/// The guest's broadcast position is where they stood when they took the
/// chair, so the chair placement must be about that close.
const CHAIR_NEAR_GUEST_M: f32 = 3.0;

const CHAIR: &str = "chair";
const TABLE: &str = "table";

pub(crate) struct MealEntry {
    pub meal: Meal,
    pub slot: MealSlot,
    pub expires_at: Instant,
}

/// Chairs and tables by region, retained so a served plate can be resolved
/// to a table top server-side.
pub(crate) type DiningIndex = HashMap<(i32, i32), Vec<FurniturePlacement>>;

fn seated_on(p: &Player, chair_object_id: u32, floor_level: i8) -> bool {
    p.object_type.as_deref() == Some(CHAIR)
        && p.object_id == Some(chair_object_id)
        && p.floor_level == floor_level
}

/// Table-top point in front of `chair` on `table` for `slot`, and the yaw
/// facing the chair. The chair, shifted sideways for the slot, is mapped
/// into the table's local frame, clamped inside the edge, and mapped back —
/// so two chairs on one table get two spots, and a plate and a cup for one
/// chair sit side by side.
pub(crate) fn plate_spot(
    chair: &FurniturePlacement,
    table: &FurniturePlacement,
    slot: MealSlot,
) -> (Position, f32) {
    let occ = solid_occupancy(TABLE).expect("table footprint");
    let (s, c) = table.rotation_deg.to_radians().sin_cos();
    // The guest faces the table; their left is the forward vector turned
    // a quarter turn (rotation 0 faces +Z, so facing +Z puts left at +X).
    let (fx, fz) = {
        let (dx, dz) = (table.x - chair.x, table.z - chair.z);
        let len = (dx * dx + dz * dz).sqrt().max(1e-3);
        (dx / len, dz / len)
    };
    let lateral = slot.lateral_m();
    let dx = chair.x + fz * lateral - table.x;
    let dz = chair.z - fx * lateral - table.z;
    let lx = (dx * c - dz * s).clamp(occ.min_x + MEAL_EDGE_INSET_M, occ.max_x - MEAL_EDGE_INSET_M);
    let lz = (dx * s + dz * c).clamp(occ.min_z + MEAL_EDGE_INSET_M, occ.max_z - MEAL_EDGE_INSET_M);
    let x = table.x + (lx * c + lz * s);
    let z = table.z + (-lx * s + lz * c);
    let rotation = onlinerpg_shared::world::bearing_xz(chair.x - x, chair.z - z).unwrap_or(0.0);
    (
        Position {
            x,
            y: table.y + TABLE_SURFACE_Y,
            z,
        },
        rotation,
    )
}

impl GameState {
    pub(super) fn sync_dining(&self, rx: i32, rz: i32, placements: &[FurniturePlacement]) {
        let dining: Vec<FurniturePlacement> = placements
            .iter()
            .filter(|p| p.type_id == CHAIR || p.type_id == TABLE)
            .cloned()
            .collect();
        let mut index = self.dining.write().unwrap_or_else(|e| e.into_inner());
        if dining.is_empty() {
            index.remove(&(rx, rz));
        } else {
            index.insert((rx, rz), dining);
        }
    }

    /// The chair placement `chair_object_id` near the guest, and the table
    /// it belongs to. Ids are unique per region only, so both are found by
    /// proximity to a real position.
    fn resolve_table(
        &self,
        chair_object_id: u32,
        near: &Position,
        floor_level: i8,
    ) -> Option<(FurniturePlacement, FurniturePlacement)> {
        let floor = u8::try_from(floor_level).ok()?;
        let index = self.dining.read().unwrap_or_else(|e| e.into_inner());
        let nearby: Vec<&FurniturePlacement> = regions_around(near.x, near.z)
            .filter_map(|key| index.get(&key))
            .flatten()
            .filter(|p| p.floor_level == floor)
            .collect();
        let d2 = |p: &FurniturePlacement, x: f32, z: f32| (p.x - x).powi(2) + (p.z - z).powi(2);
        let chair = nearby
            .iter()
            .filter(|p| p.type_id == CHAIR && p.id == chair_object_id)
            .map(|p| (p, d2(p, near.x, near.z)))
            .filter(|(_, d)| *d <= CHAIR_NEAR_GUEST_M.powi(2))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(p, _)| p)?;
        let table = nearby
            .iter()
            .filter(|p| p.type_id == TABLE)
            .map(|p| (p, d2(p, chair.x, chair.z)))
            .filter(|(_, d)| *d <= CHAIR_TABLE_RADIUS_M.powi(2))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(p, _)| p)?;
        Some(((*chair).clone(), (*table).clone()))
    }

    /// An official NPC sets `item_def_id` on the table in front of the guest
    /// sitting on `chair_object_id`.
    pub async fn serve_meal(&self, player_id: &PlayerId, chair_object_id: u32, item_def_id: &str) {
        let Some(def) = self
            .item_defs
            .get(item_def_id)
            .filter(|d| is_servable(d.category.as_deref(), d.world_model.as_deref(), d.nutrition))
        else {
            self.send_system_message(
                player_id,
                format!("{item_def_id} is not a dish you can serve."),
            )
            .await;
            return;
        };
        let slot = MealSlot::for_category(def.category.as_deref());
        let Some((maid_name, maid_position, floor_level)) = ({
            let players = self.players.read().await;
            players
                .get(player_id)
                .filter(|p| p.is_official_npc)
                .map(|p| (p.name.clone(), p.position, p.floor_level))
        }) else {
            self.send_system_message(player_id, "Only the inn staff serve tables.")
                .await;
            return;
        };
        let near = self
            .players_within_position(&maid_position, floor_level, MEAL_SERVICE_RADIUS_M, None)
            .await;
        let guest = {
            let players = self.players.read().await;
            near.iter()
                .filter_map(|(id, _)| players.get(id))
                .find(|p| seated_on(p, chair_object_id, floor_level))
                .map(|p| (p.id, p.name.clone(), p.position))
        };
        let Some((guest_id, guest_name, guest_position)) = guest else {
            self.send_system_message(
                player_id,
                "Nobody is sitting on that chair within reach — stand beside the seat first.",
            )
            .await;
            return;
        };
        let Some((chair, table)) =
            self.resolve_table(chair_object_id, &guest_position, floor_level)
        else {
            self.send_system_message(player_id, "That seat has no table to set a plate on.")
                .await;
            return;
        };
        let (position, rotation) = plate_spot(&chair, &table, slot);
        let meal = Meal {
            id: self.next_instance_id().await,
            item_def_id: item_def_id.to_string(),
            chair_object_id,
            for_player: guest_id,
            position,
            rotation,
            floor_level,
            eaten: false,
        };

        // A second plate (or cup) for the same chair replaces the first.
        let replaced: Vec<Meal> = {
            let mut meals = self.meals.write().await;
            let replaced = meals
                .extract_if(|_, e| {
                    e.slot == slot
                        && e.meal.chair_object_id == chair_object_id
                        && e.meal.floor_level == floor_level
                        && e.meal.position.dist_xz_sq(&position) <= CHAIR_TABLE_RADIUS_M.powi(2)
                })
                .map(|(_, e)| e.meal)
                .collect();
            meals.insert(
                meal.id,
                MealEntry {
                    meal: meal.clone(),
                    slot,
                    expires_at: Instant::now() + MEAL_MAX_AGE,
                },
            );
            replaced
        };
        for old in replaced {
            self.announce_meal_removed(&old).await;
        }
        self.send_direct_message_to_players_within_position(
            &meal.position,
            meal.floor_level,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::MealPlaced { meal: meal.clone() },
            None,
        )
        .await;
        let name = self.item_name(item_def_id);
        self.send_system_message(
            player_id,
            format!("You set the {name} down in front of {guest_name}."),
        )
        .await;
        self.send_system_message(
            &guest_id,
            format!("{maid_name} sets a {name} down in front of you."),
        )
        .await;
    }

    /// Eat the plate served to the chair the player is sitting on. Fills
    /// satiation to the cap whatever it was; the emptied plate stays on the
    /// table for the maid to clear.
    pub async fn eat_meal(&self, player_id: &PlayerId, meal_id: u64) {
        if self
            .reject_if_defeated(player_id, "You can't eat while defeated")
            .await
        {
            return;
        }
        let Some((object_type, object_id, floor_level)) = ({
            let players = self.players.read().await;
            players
                .get(player_id)
                .map(|p| (p.object_type.clone(), p.object_id, p.floor_level))
        }) else {
            return;
        };
        let meal = {
            let mut meals = self.meals.write().await;
            let Some(e) = meals.get_mut(&meal_id) else {
                self.send_system_message(player_id, "That plate is gone.")
                    .await;
                return;
            };
            let seated_here = object_type.as_deref() == Some(CHAIR)
                && object_id == Some(e.meal.chair_object_id)
                && floor_level == e.meal.floor_level;
            if !seated_here {
                self.send_system_message(player_id, "Take the seat in front of the plate to eat.")
                    .await;
                return;
            }
            if e.meal.eaten {
                self.send_system_message(player_id, "You've already cleaned that plate.")
                    .await;
                return;
            }
            e.meal.eaten = true;
            e.meal.clone()
        };
        self.send_direct_message_to_players_within_position(
            &meal.position,
            meal.floor_level,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::MealEaten { meal_id },
            None,
        )
        .await;
        let name = self.item_name(&meal.item_def_id);
        let (amount, said, alcohol) = match self.item_defs.get(&meal.item_def_id) {
            Some(d) if MealSlot::for_category(d.category.as_deref()) == MealSlot::Drink => (
                d.nutrition.unwrap_or(0),
                format!("You drink the {name}."),
                d.alcohol,
            ),
            _ => (
                onlinerpg_shared::hunger::SATIATION_MAX,
                format!("You eat the {name}. You couldn't eat another bite."),
                None,
            ),
        };
        let (outcome, gained) = self.apply_eat(player_id, amount).await;
        self.send_system_message(player_id, said).await;
        self.settle_meal(player_id, outcome, gained).await;
        if let Some(units) = alcohol {
            self.apply_alcohol(player_id, units).await;
        }
    }

    /// An official NPC takes a plate away.
    pub async fn clear_meal(&self, player_id: &PlayerId, meal_id: u64) {
        let Some((position, floor_level)) = ({
            let players = self.players.read().await;
            players
                .get(player_id)
                .filter(|p| p.is_official_npc)
                .map(|p| (p.position, p.floor_level))
        }) else {
            return;
        };
        let removed = {
            let mut meals = self.meals.write().await;
            let reachable = meals.get(&meal_id).is_some_and(|e| {
                e.meal.floor_level == floor_level
                    && e.meal.position.dist_xz_sq(&position)
                        <= MEAL_SERVICE_RADIUS_M * MEAL_SERVICE_RADIUS_M
            });
            reachable
                .then(|| meals.remove(&meal_id).map(|e| e.meal))
                .flatten()
        };
        match removed {
            Some(meal) => {
                self.announce_meal_removed(&meal).await;
                let name = self.item_name(&meal.item_def_id);
                self.send_system_message(player_id, format!("You clear the {name} away."))
                    .await;
            }
            None => {
                self.send_system_message(player_id, "No plate within reach to clear.")
                    .await;
            }
        }
    }

    /// A plate whose guest left the chair lingers for the maid, then goes;
    /// an untouched one goes on its own eventually.
    pub async fn tick_meals(&self) {
        let guests: Vec<(u64, PlayerId, u32, i8)> = {
            let meals = self.meals.read().await;
            if meals.is_empty() {
                return;
            }
            meals
                .values()
                .map(|e| {
                    (
                        e.meal.id,
                        e.meal.for_player,
                        e.meal.chair_object_id,
                        e.meal.floor_level,
                    )
                })
                .collect()
        };
        let left: Vec<u64> = {
            let players = self.players.read().await;
            guests
                .into_iter()
                .filter(|(_, pid, chair, floor)| {
                    !players
                        .get(pid)
                        .is_some_and(|p| seated_on(p, *chair, *floor))
                })
                .map(|(id, ..)| id)
                .collect()
        };
        let now = Instant::now();
        let expired: Vec<Meal> = {
            let mut meals = self.meals.write().await;
            // `min` is idempotent, so re-stamping on every absent tick is fine.
            for id in &left {
                if let Some(e) = meals.get_mut(id) {
                    e.expires_at = e.expires_at.min(now + MEAL_LINGER);
                }
            }
            meals
                .extract_if(|_, e| e.expires_at <= now)
                .map(|(_, e)| e.meal)
                .collect()
        };
        for meal in expired {
            self.announce_meal_removed(&meal).await;
        }
    }

    async fn announce_meal_removed(&self, meal: &Meal) {
        self.send_direct_message_to_players_within_position(
            &meal.position,
            meal.floor_level,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::MealRemoved { meal_id: meal.id },
            None,
        )
        .await;
    }
}
