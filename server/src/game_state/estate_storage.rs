use super::{
    auth_db,
    inventory::{serialize_inventory, stack_into_bag, BagInsert},
    GameState,
};
use crate::auth::{AuthError, AuthService, EstateDeposit};
use crate::types::{PlayerId, Position, ServerMessage};
use onlinerpg_shared::estate_storage::{
    estate_storage_def, is_estate_storage_item, EstateChest, INTERACTION_RANGE,
};
use onlinerpg_shared::furniture::{
    self, occupancy_fits_house_floor, point_in_house_floor, FurniturePlacement,
};
use onlinerpg_shared::inventory::GroundItem;
use onlinerpg_shared::messages::BagLineItem;
use std::collections::{HashMap, HashSet};

const MAX_TRANSFER_LINES: usize = 128;

#[derive(Default)]
pub(super) struct EstateChestIndex {
    by_id: HashMap<i64, EstateChest>,
    buckets: HashMap<(i32, i32), Vec<i64>>,
}

impl EstateChestIndex {
    fn bucket(position: &Position) -> (i32, i32) {
        (
            (onlinerpg_shared::wrap_world_x(position.x) / 32.0).floor() as i32,
            (position.z / 32.0).floor() as i32,
        )
    }

    fn insert(&mut self, chest: EstateChest) {
        if let Some(old) = self.by_id.insert(chest.id, chest.clone()) {
            self.remove_bucket(old.id, Self::bucket(&old.position));
        }
        self.buckets
            .entry(Self::bucket(&chest.position))
            .or_default()
            .push(chest.id);
    }

    fn remove_bucket(&mut self, id: i64, key: (i32, i32)) {
        if let Some(bucket) = self.buckets.get_mut(&key) {
            bucket.retain(|entry| *entry != id);
            if bucket.is_empty() {
                self.buckets.remove(&key);
            }
        }
    }

    fn remove(&mut self, id: i64) -> Option<EstateChest> {
        let chest = self.by_id.remove(&id)?;
        self.remove_bucket(id, Self::bucket(&chest.position));
        Some(chest)
    }

    fn get(&self, id: i64) -> Option<&EstateChest> {
        self.by_id.get(&id)
    }

    fn group(&self, key: (i32, i32)) -> Vec<EstateChest> {
        self.buckets
            .get(&key)
            .into_iter()
            .flatten()
            .filter_map(|id| self.by_id.get(id).cloned())
            .collect()
    }

    fn overlaps(&self, candidate: &FurniturePlacement) -> bool {
        self.by_id.values().any(|chest| {
            let existing = placement(chest);
            furniture::placements_overlap(candidate, &existing)
        })
    }

    pub(super) fn nearby(&self, position: &Position, floor_level: i8) -> Vec<&EstateChest> {
        let radius = super::EVENT_DELIVERY_RADIUS;
        self.by_id
            .values()
            .filter(|chest| {
                chest.floor_level == floor_level
                    && chest.position.dist_xz_sq(position) <= radius * radius
            })
            .collect()
    }
}

fn cache_key(key: (i32, i32)) -> String {
    format!("furniture:estate-storage:{},{}", key.0, key.1)
}

fn placement(chest: &EstateChest) -> FurniturePlacement {
    let definition = estate_storage_def(&chest.item_def_id)
        .expect("loaded estate chest definition was validated");
    FurniturePlacement {
        id: chest.id as u32,
        type_id: definition.model_id.clone(),
        x: chest.position.x,
        y: chest.position.y,
        z: chest.position.z,
        rotation_deg: chest.rotation_deg,
        floor_level: chest.floor_level as u8,
    }
}

impl GameState {
    fn sync_estate_chest_bucket(&self, key: (i32, i32), group: &[EstateChest]) {
        let placements: Vec<_> = group.iter().map(placement).collect();
        let mut passability = self.passability_write();
        let key = cache_key(key);
        match furniture::build_furniture_passability_for_placements(&placements) {
            Some(runtime) => {
                passability.insert(key, runtime);
            }
            None => {
                passability.remove(&key);
            }
        }
    }

    pub async fn load_estate_chests(&self, auth: &AuthService) -> Result<(), AuthError> {
        let auth = auth.clone();
        let loaded = auth_db(move || auth.load_estate_chests()).await?;
        let mut index = self.estate_chests.write().await;
        for chest in loaded {
            if chest.floor_level < 0
                || !chest.position.x.is_finite()
                || !chest.position.z.is_finite()
                || estate_storage_def(&chest.item_def_id).is_none()
            {
                return Err(AuthError::Database(
                    "Invalid saved estate chest".to_string(),
                ));
            }
            index.insert(chest);
        }
        let groups: Vec<_> = index
            .buckets
            .keys()
            .copied()
            .map(|key| (key, index.group(key)))
            .collect();
        drop(index);
        for (key, group) in groups {
            self.sync_estate_chest_bucket(key, &group);
        }
        Ok(())
    }

    pub async fn try_start_estate_chest_mode(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        auth: &AuthService,
    ) -> bool {
        let item_def_id = self
            .inventories
            .read()
            .await
            .get(player_id)
            .and_then(|inventory| {
                inventory
                    .bag
                    .iter()
                    .find(|item| item.instance_id == instance_id)
            })
            .filter(|item| item.quantity > 0 && is_estate_storage_item(&item.item_def_id))
            .map(|item| item.item_def_id.clone());
        let Some(item_def_id) = item_def_id else {
            return false;
        };
        if self
            .reject_if_defeated(player_id, "You must be alive to place storage.")
            .await
            || self.reject_if_trading(player_id, "place storage").await
        {
            return true;
        }
        self.tick_land_taxes(auth).await;
        let Some(owner_id) = self
            .player_characters
            .read()
            .await
            .get(player_id)
            .map(|entry| entry.0)
        else {
            return true;
        };
        let auth = auth.clone();
        match auth_db(move || auth.estate_storage_plots(owner_id)).await {
            Ok(plots) if !plots.is_empty() => {
                self.send_direct_message(
                    player_id,
                    ServerMessage::EstateChestMode {
                        instance_id,
                        item_def_id,
                        owner_id,
                        plots,
                    },
                )
                .await;
            }
            Ok(_) => {
                self.send_system_message(player_id, "You need an active estate first.")
                    .await
            }
            Err(error) => {
                tracing::warn!(%error, "Failed to load estate storage permissions");
                self.send_system_message(
                    player_id,
                    "Storage placement is temporarily unavailable.",
                )
                .await;
            }
        }
        true
    }

    pub async fn place_estate_chest(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        requested: Position,
        rotation_deg: f32,
        floor_level: i8,
        auth: &AuthService,
    ) {
        let error = self
            .try_place_estate_chest(
                player_id,
                instance_id,
                requested,
                rotation_deg,
                floor_level,
                auth,
            )
            .await
            .err()
            .map(str::to_string);
        self.send_direct_message(player_id, ServerMessage::EstateChestEditResult { error })
            .await;
    }

    async fn try_place_estate_chest(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        requested: Position,
        rotation_deg: f32,
        floor_level: i8,
        auth: &AuthService,
    ) -> Result<(), &'static str> {
        if !requested.x.is_finite() || !requested.z.is_finite() || !rotation_deg.is_finite() {
            return Err("Invalid chest placement.");
        }
        self.tick_land_taxes(auth).await;
        let _persistence = self.persistence_lock.lock().await;
        if self.reject_if_trading(player_id, "place storage").await {
            return Err("Finish your player trade first.");
        }
        let item_def_id = self
            .inventories
            .read()
            .await
            .get(player_id)
            .and_then(|inventory| {
                inventory
                    .bag
                    .iter()
                    .find(|item| item.instance_id == instance_id && item.quantity > 0)
            })
            .map(|item| item.item_def_id.clone())
            .ok_or("That storage chest is no longer in your bag.")?;
        let definition =
            estate_storage_def(&item_def_id).ok_or("That item is not an estate storage chest.")?;
        let mut character = self
            .get_player_save_data(player_id)
            .await
            .ok_or("Character not found.")?;
        let (player_health, player_floor, player_position) = {
            let players = self.players.read().await;
            let player = players.get(player_id).ok_or("Character not found.")?;
            (player.health, player.floor_level, player.position)
        };
        if player_health == 0
            || floor_level < definition.min_floor
            || floor_level > definition.max_floor
            || player_floor != floor_level
        {
            return Err("Stand alive on the floor where you want to place the chest.");
        }
        let snapped_rotation = ((rotation_deg / definition.rotation_step).round()
            * definition.rotation_step)
            .rem_euclid(360.0);
        let mut position = Position {
            x: onlinerpg_shared::wrap_world_x(
                (requested.x / definition.snap_step).round() * definition.snap_step,
            ),
            y: requested.y,
            z: (requested.z / definition.snap_step).round() * definition.snap_step,
        };
        let houses = self.housing_io.read_all_houses().await.map_err(|error| {
            tracing::warn!(%error, "Failed to read housing for furniture placement");
            "Housing is temporarily unavailable."
        })?;
        let floor = floor_level as u8;
        let player_house = houses
            .iter()
            .find(|house| point_in_house_floor(house, player_position.x, player_position.z, floor));
        let target_house = houses
            .iter()
            .find(|house| point_in_house_floor(house, position.x, position.z, floor));
        if let Some(house) = player_house {
            let occupancy = furniture::solid_occupancy(&definition.model_id)
                .ok_or("Chest placement data is unavailable.")?;
            if !occupancy_fits_house_floor(
                house,
                &occupancy,
                position.x,
                position.z,
                snapped_rotation,
                floor,
                definition.floor_edge_clearance,
            ) {
                return Err("Keep the whole chest inside the current building floor.");
            }
        } else if target_house.is_some() || floor_level > 0 {
            return Err("Enter the building floor before placing furniture there.");
        }
        position.y = if player_house.is_some() {
            onlinerpg_shared::pathfinding::get_floor_y_base(
                &self.passability_read(),
                position.x,
                position.z,
                floor,
            )
            .ok_or("Place the chest on the current building floor.")?
        } else {
            self.height_sampler
                .sample_height(position.x, position.z)
                .await
                .map_err(|_| "Terrain is unavailable here.")?
        };
        let candidate = FurniturePlacement {
            id: 0,
            type_id: definition.model_id.clone(),
            x: position.x,
            y: position.y,
            z: position.z,
            rotation_deg: snapped_rotation,
            floor_level: floor,
        };
        if self.estate_chests.read().await.overlaps(&candidate) {
            return Err("Another storage chest already occupies this space.");
        }
        let collision_radius = if player_house.is_some() {
            definition.indoor_collision_radius
        } else {
            definition.outdoor_collision_radius
        };
        if onlinerpg_shared::pathfinding::is_circle_blocked_on_floor(
            &self.passability_read(),
            position.x,
            position.z,
            collision_radius,
            floor_level as u8,
            Some(position.y),
        ) {
            return Err("Something blocks the chest here.");
        }
        let mut inventories = self.inventories.write().await;
        let inventory = inventories
            .get_mut(player_id)
            .ok_or("Inventory not found.")?;
        let mut updated = inventory.clone();
        let index = updated
            .bag
            .iter()
            .position(|item| item.instance_id == instance_id && item.item_def_id == item_def_id)
            .ok_or("That storage chest is no longer in your bag.")?;
        if updated.bag[index].quantity > 1 {
            updated.bag[index].quantity -= 1;
        } else {
            updated.bag.remove(index);
        }
        let gold = self.player_gold.read().await;
        character.gold = *gold.get(player_id).ok_or("Gold balance not found.")?;
        let rows = serialize_inventory(&updated);
        let auth = auth.clone();
        let placed_item_def_id = item_def_id.clone();
        let chest = auth_db(move || {
            auth.place_estate_chest(
                &character,
                &rows,
                placed_item_def_id,
                position,
                snapped_rotation,
                floor_level,
            )
        })
        .await
        .map_err(|error| {
            tracing::warn!(%error, "Failed to place estate chest");
            "The chest could not be saved. Your inventory was not changed."
        })??;
        *inventory = updated.clone();
        let key = EstateChestIndex::bucket(&chest.position);
        let mut chests = self.estate_chests.write().await;
        chests.insert(chest.clone());
        let group = chests.group(key);
        drop(chests);
        self.sync_estate_chest_bucket(key, &group);
        drop(gold);
        drop(inventories);
        let recipients: Vec<_> = self
            .players
            .read()
            .await
            .values()
            .filter(|candidate| {
                candidate.floor_level == chest.floor_level
                    && candidate.position.dist_xz_sq(&chest.position)
                        <= super::EVENT_DELIVERY_RADIUS.powi(2)
            })
            .map(|candidate| candidate.id)
            .collect();
        self.send_direct_message_to_players(
            &recipients,
            ServerMessage::EstateChestVisibility {
                added: vec![chest],
                removed: vec![],
            },
        )
        .await;
        self.mark_inventory_dirty(player_id).await;
        self.send_inventory_snapshot(player_id, updated).await;
        Ok(())
    }

    async fn accessible_chest(
        &self,
        player_id: &PlayerId,
        chest_id: i64,
    ) -> Result<EstateChest, &'static str> {
        let players = self.players.read().await;
        let player = players.get(player_id).ok_or("Character not found.")?;
        let chests = self.estate_chests.read().await;
        let chest = chests
            .get(chest_id)
            .cloned()
            .ok_or("That storage chest is not here.")?;
        if chest.floor_level != player.floor_level
            || chest.position.dist_xz_sq(&player.position) > INTERACTION_RANGE.powi(2)
        {
            return Err("Move closer to the storage chest.");
        }
        Ok(chest)
    }

    async fn send_estate_chest_state(
        &self,
        player_id: &PlayerId,
        chest_id: i64,
        auth: &AuthService,
    ) {
        let Some(character_id) = self
            .player_characters
            .read()
            .await
            .get(player_id)
            .map(|entry| entry.0)
        else {
            return;
        };
        let auth = auth.clone();
        let result = auth_db(move || auth.estate_chest_state(chest_id, character_id)).await;
        let (state, error) = match result {
            Ok(Ok(state)) => (Some(state), None),
            Ok(Err(error)) => (None, Some(error.to_string())),
            Err(error) => {
                tracing::warn!(%error, "Failed to load estate chest");
                (
                    None,
                    Some("Storage is temporarily unavailable.".to_string()),
                )
            }
        };
        self.send_direct_message(player_id, ServerMessage::EstateChestState { state, error })
            .await;
    }

    pub async fn open_estate_chest(&self, player_id: &PlayerId, chest_id: i64, auth: &AuthService) {
        if let Err(error) = self.accessible_chest(player_id, chest_id).await {
            self.send_direct_message(
                player_id,
                ServerMessage::EstateChestState {
                    state: None,
                    error: Some(error.to_string()),
                },
            )
            .await;
            return;
        }
        self.send_estate_chest_state(player_id, chest_id, auth)
            .await;
    }

    pub async fn transfer_estate_items(
        &self,
        player_id: &PlayerId,
        chest_id: i64,
        deposits: Vec<BagLineItem>,
        withdrawals: Vec<BagLineItem>,
        expected_revision: u64,
        auth: &AuthService,
    ) {
        let result = self
            .try_transfer_estate_items(
                player_id,
                chest_id,
                &deposits,
                &withdrawals,
                expected_revision,
                auth,
            )
            .await;
        if let Err(error) = result {
            self.send_direct_message(
                player_id,
                ServerMessage::EstateChestState {
                    state: None,
                    error: Some(error.to_string()),
                },
            )
            .await;
        }
        self.send_estate_chest_state(player_id, chest_id, auth)
            .await;
    }

    async fn try_transfer_estate_items(
        &self,
        player_id: &PlayerId,
        chest_id: i64,
        deposits: &[BagLineItem],
        withdrawals: &[BagLineItem],
        expected_revision: u64,
        auth: &AuthService,
    ) -> Result<(), &'static str> {
        self.accessible_chest(player_id, chest_id).await?;
        if deposits.is_empty() && withdrawals.is_empty() {
            return Err("Select at least one item.");
        }
        if deposits.len() + withdrawals.len() > MAX_TRANSFER_LINES {
            return Err("Too many storage selections.");
        }
        if self
            .reject_if_trading(player_id, "use estate storage")
            .await
        {
            return Err("Finish your player trade first.");
        }
        let incoming_ids = withdrawals
            .iter()
            .try_fold(0u64, |sum, line| sum.checked_add(u64::from(line.qty)))
            .ok_or("Invalid storage quantity.")?;
        let mut next_id = self.reserve_instance_ids(incoming_ids).await;
        let max_weight = self.max_carry_weight(player_id).await;
        let armor_mult = self.armor_weight_mult(player_id).await;
        let _persistence = self.persistence_lock.lock().await;
        let Some(character_id) = self
            .player_characters
            .read()
            .await
            .get(player_id)
            .map(|entry| entry.0)
        else {
            return Err("Character not found.");
        };
        let auth_read = auth.clone();
        let state = auth_db(move || auth_read.estate_chest_state(chest_id, character_id))
            .await
            .map_err(|_| "Storage is temporarily unavailable.")??;
        if state.revision != expected_revision {
            return Err("The chest changed. Its contents were refreshed.");
        }
        if !deposits.is_empty() && !state.can_deposit {
            return Err("Overdue estates can withdraw items but cannot store new ones.");
        }
        let mut character = self
            .get_player_save_data(player_id)
            .await
            .ok_or("Character not found.")?;
        let mut inventories = self.inventories.write().await;
        let inventory = inventories
            .get_mut(player_id)
            .ok_or("Inventory not found.")?;
        let mut seen_deposits = HashSet::new();
        let mut deposit_plans = Vec::with_capacity(deposits.len());
        for line in deposits {
            if line.qty == 0 || !seen_deposits.insert(line.instance_id) {
                return Err("Invalid storage selection.");
            }
            let item = inventory
                .bag
                .iter()
                .find(|item| item.instance_id == line.instance_id)
                .cloned()
                .ok_or("An item is no longer in your bag.")?;
            if line.qty > item.quantity {
                return Err("Invalid storage quantity.");
            }
            if is_estate_storage_item(&item.item_def_id)
                || self.item_defs.untradeable(&item.item_def_id)
            {
                return Err("Bound or untradeable items cannot be stored.");
            }
            let stackable = self
                .item_defs
                .get(&item.item_def_id)
                .is_some_and(|def| def.stackable);
            if !stackable && line.qty != 1 {
                return Err("Invalid storage quantity.");
            }
            deposit_plans.push(EstateDeposit {
                item,
                quantity: line.qty,
                stackable,
            });
        }

        let mut seen_withdrawals = HashSet::new();
        let mut withdrawal_plans = Vec::with_capacity(withdrawals.len());
        for line in withdrawals {
            if line.qty == 0 || !seen_withdrawals.insert(line.instance_id) {
                return Err("Invalid storage selection.");
            }
            let item = state
                .items
                .iter()
                .find(|item| item.instance_id == line.instance_id)
                .cloned()
                .ok_or("An item is no longer in the chest.")?;
            if line.qty > item.quantity {
                return Err("Invalid storage quantity.");
            }
            let stackable = self
                .item_defs
                .get(&item.item_def_id)
                .is_some_and(|def| def.stackable);
            if !stackable && line.qty != 1 {
                return Err("Invalid storage quantity.");
            }
            withdrawal_plans.push((item, line.qty, stackable));
        }

        let storage_weight: f32 = state
            .items
            .iter()
            .map(|stored| self.item_defs.weight(&stored.item_def_id) * stored.quantity as f32)
            .sum();
        let deposited_weight: f32 = deposit_plans
            .iter()
            .map(|plan| self.item_defs.weight(&plan.item.item_def_id) * plan.quantity as f32)
            .sum();
        let withdrawn_weight: f32 = withdrawal_plans
            .iter()
            .map(|(item, quantity, _)| self.item_defs.weight(&item.item_def_id) * *quantity as f32)
            .sum();
        let final_storage_weight = storage_weight - withdrawn_weight + deposited_weight;
        if !deposits.is_empty() && final_storage_weight > state.max_weight + f32::EPSILON {
            return Err("The storage chest cannot hold that much weight.");
        }

        let deposited_carry_weight: f32 = deposit_plans
            .iter()
            .map(|plan| {
                self.item_defs
                    .weight_with(&plan.item.item_def_id, armor_mult)
                    * plan.quantity as f32
            })
            .sum();
        let withdrawn_carry_weight: f32 = withdrawal_plans
            .iter()
            .map(|(item, quantity, _)| {
                self.item_defs.weight_with(&item.item_def_id, armor_mult) * *quantity as f32
            })
            .sum();
        let final_player_weight = self.calc_total_weight(inventory, armor_mult)
            - deposited_carry_weight
            + withdrawn_carry_weight;
        if !withdrawals.is_empty() && final_player_weight > max_weight + f32::EPSILON {
            return Err("Your bag cannot hold that much weight.");
        }

        let mut updated = inventory.clone();
        for plan in &deposit_plans {
            let index = updated
                .bag
                .iter()
                .position(|entry| entry.instance_id == plan.item.instance_id)
                .ok_or("An item is no longer in your bag.")?;
            if updated.bag[index].quantity == plan.quantity {
                updated.bag.remove(index);
            } else {
                updated.bag[index].quantity -= plan.quantity;
            }
        }
        for (item, quantity, stackable) in &withdrawal_plans {
            if *stackable
                && updated.bag.iter().any(|entry| {
                    entry.item_def_id == item.item_def_id
                        && entry.enchant == item.enchant
                        && entry.cape_color == item.cape_color
                        && entry.cape_texture == item.cape_texture
                        && entry.quantity > u32::MAX - *quantity
                })
            {
                return Err("A bag stack is full.");
            }
            let used = stack_into_bag(
                &mut updated.bag,
                BagInsert {
                    stackable: *stackable,
                    item_def_id: &item.item_def_id,
                    enchant: item.enchant,
                    cape_color: item.cape_color.clone(),
                    cape_texture: item.cape_texture.clone(),
                    first_instance_id: next_id,
                    quantity: *quantity,
                },
            );
            next_id += used;
        }
        let gold = self.player_gold.read().await;
        character.gold = *gold.get(player_id).ok_or("Gold balance not found.")?;
        let rows = serialize_inventory(&updated);
        let auth = auth.clone();
        let deposits = deposit_plans;
        let withdrawals = withdrawals.to_vec();
        auth_db(move || {
            auth.transfer_estate_items(
                &character,
                &rows,
                chest_id,
                &deposits,
                &withdrawals,
                expected_revision,
            )
        })
        .await
        .map_err(|error| {
            tracing::warn!(%error, "Failed to transfer estate items");
            "The items could not be transferred. Your inventory was not changed."
        })??;
        *inventory = updated.clone();
        drop(gold);
        drop(inventories);
        self.mark_inventory_dirty(player_id).await;
        self.send_inventory_snapshot(player_id, updated).await;
        Ok(())
    }

    pub async fn recover_estate_chest(
        &self,
        player_id: &PlayerId,
        chest_id: i64,
        auth: &AuthService,
    ) {
        let error = self
            .try_recover_estate_chest(player_id, chest_id, auth)
            .await
            .err()
            .map(str::to_string);
        self.send_direct_message(player_id, ServerMessage::EstateChestEditResult { error })
            .await;
    }

    async fn try_recover_estate_chest(
        &self,
        player_id: &PlayerId,
        chest_id: i64,
        auth: &AuthService,
    ) -> Result<(), &'static str> {
        let chest = self.accessible_chest(player_id, chest_id).await?;
        let definition = estate_storage_def(&chest.item_def_id)
            .ok_or("This storage chest has an unknown type.")?;
        let max_weight = self.max_carry_weight(player_id).await;
        let armor_mult = self.armor_weight_mult(player_id).await;
        let next_id = self.next_instance_id().await;
        let _persistence = self.persistence_lock.lock().await;
        let mut character = self
            .get_player_save_data(player_id)
            .await
            .ok_or("Character not found.")?;
        let players = self.players.read().await;
        let mut inventories = self.inventories.write().await;
        let inventory = inventories
            .get_mut(player_id)
            .ok_or("Inventory not found.")?;
        let drop_recovered_chest = self.calc_total_weight(inventory, armor_mult)
            + self.item_defs.weight(&chest.item_def_id)
            > max_weight;
        let mut updated = inventory.clone();
        if !drop_recovered_chest {
            stack_into_bag(
                &mut updated.bag,
                BagInsert::one(false, &chest.item_def_id, 0, next_id),
            );
        }
        let gold = self.player_gold.read().await;
        character.gold = *gold.get(player_id).ok_or("Gold balance not found.")?;
        let rows = serialize_inventory(&updated);
        let auth = auth.clone();
        auth_db(move || auth.recover_estate_chest(&character, &rows, chest_id))
            .await
            .map_err(|error| {
                tracing::warn!(%error, "Failed to recover estate chest");
                "The chest could not be recovered. Your inventory was not changed."
            })??;
        *inventory = updated.clone();
        let key = EstateChestIndex::bucket(&chest.position);
        let mut chests = self.estate_chests.write().await;
        chests.remove(chest_id);
        let group = chests.group(key);
        drop(chests);
        self.sync_estate_chest_bucket(key, &group);
        let recipients: Vec<_> = players
            .values()
            .filter(|candidate| {
                candidate.floor_level == chest.floor_level
                    && candidate.position.dist_xz_sq(&chest.position)
                        <= super::EVENT_DELIVERY_RADIUS.powi(2)
            })
            .map(|candidate| candidate.id)
            .collect();
        drop(gold);
        drop(inventories);
        drop(players);
        self.send_direct_message_to_players(
            &recipients,
            ServerMessage::EstateChestVisibility {
                added: vec![],
                removed: vec![chest_id],
            },
        )
        .await;
        if drop_recovered_chest {
            self.spawn_ground_item(GroundItem {
                instance_id: next_id,
                item_def_id: definition.id.clone(),
                position: chest.position,
                floor_level: chest.floor_level,
                quantity: 1,
                enchant: 0,
                cape_color: None,
                cape_texture: None,
                dropped_by: Some(*player_id),
            })
            .await;
        } else {
            self.mark_inventory_dirty(player_id).await;
            self.send_inventory_snapshot(player_id, updated).await;
        }
        Ok(())
    }
}
