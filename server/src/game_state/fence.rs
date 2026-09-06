use super::{
    auth_db,
    inventory::{consume_one, serialize_inventory, stack_into_bag, BagInsert},
    GameState,
};
use crate::auth::{AuthError, AuthService};
use crate::types::{PlayerId, ServerMessage};
use onlinerpg_shared::fence::{self, Fence, FenceAxis, FenceEdge};
use onlinerpg_terrain::coords::{world_to_tile, wrap_tile_x};
use std::collections::{HashMap, HashSet};

fn sample_positions(edge: FenceEdge) -> [(f32, f32); 3] {
    [0.0, 0.5, 1.0].map(|t| {
        (
            onlinerpg_shared::wrap_world_x(
                edge.x as f32 + if edge.axis == FenceAxis::X { t } else { 0.0 },
            ),
            edge.z as f32 + if edge.axis == FenceAxis::Z { t } else { 0.0 },
        )
    })
}

#[derive(Default)]
pub(super) struct FenceIndex {
    buckets: HashMap<(i32, i32), HashMap<FenceEdge, Fence>>,
}

impl FenceIndex {
    fn bucket(edge: FenceEdge) -> (i32, i32) {
        (edge.x.div_euclid(32), edge.z.div_euclid(32))
    }

    fn get(&self, edge: &FenceEdge) -> Option<&Fence> {
        self.buckets.get(&Self::bucket(*edge))?.get(edge)
    }

    fn insert(&mut self, fence: Fence) {
        self.buckets
            .entry(Self::bucket(fence.edge))
            .or_default()
            .insert(fence.edge, fence);
    }

    fn remove(&mut self, edge: &FenceEdge) {
        let key = Self::bucket(*edge);
        if let Some(bucket) = self.buckets.get_mut(&key) {
            bucket.remove(edge);
            if bucket.is_empty() {
                self.buckets.remove(&key);
            }
        }
    }

    fn group(&self, edge: FenceEdge) -> Vec<Fence> {
        self.buckets
            .get(&Self::bucket(edge))
            .map(|bucket| bucket.values().cloned().collect())
            .unwrap_or_default()
    }

    pub(super) fn nearby(&self, position: &crate::types::Position) -> Vec<&Fence> {
        let radius = super::EVENT_DELIVERY_RADIUS;
        let mut nearby = Vec::new();
        let min_z = ((position.z - radius) / 32.0).floor() as i32;
        let max_z = ((position.z + radius) / 32.0).floor() as i32;
        for x in ((position.x - radius) / 32.0).floor() as i32
            ..=((position.x + radius) / 32.0).floor() as i32
        {
            let bx = (onlinerpg_shared::wrap_world_x((x * 32) as f32) as i32).div_euclid(32);
            for z in min_z..=max_z {
                if let Some(bucket) = self.buckets.get(&(bx, z)) {
                    nearby.extend(
                        bucket
                            .values()
                            .filter(|f| f.edge.center(f.y).dist_xz_sq(position) <= radius * radius),
                    );
                }
            }
        }
        nearby
    }
}

impl GameState {
    async fn fence_heights(&self, edge: FenceEdge) -> std::io::Result<[f32; 3]> {
        let mut heights = [0.0; 3];
        for (i, (x, z)) in sample_positions(edge).into_iter().enumerate() {
            heights[i] = self.height_sampler.sample_height(x, z).await?;
        }
        Ok(heights)
    }

    pub async fn load_fences(&self, auth: &AuthService) -> Result<(), AuthError> {
        let auth = auth.clone();
        let loaded = auth_db(move || auth.load_fences()).await?;
        let mut groups: HashMap<String, Vec<Fence>> = HashMap::new();
        let mut fences = self.fences.write().await;
        for record in loaded {
            if !record.edge.valid() {
                return Err(AuthError::Database("Invalid saved fence".to_string()));
            }
            let y = self
                .fence_heights(record.edge)
                .await
                .map_err(|e| AuthError::Database(format!("Failed to sample fence terrain: {e}")))?
                .into_iter()
                .fold(f32::INFINITY, f32::min);
            let fence = Fence {
                edge: record.edge,
                owner_id: record.owner_id,
                y,
            };
            groups
                .entry(fence.edge.cache_key())
                .or_default()
                .push(fence.clone());
            fences.insert(fence);
        }
        let mut cache = self.passability_write();
        for (key, group) in groups {
            fence::sync_passability(&mut cache, &key, &group);
        }
        Ok(())
    }

    pub async fn save_terrain_heightmap(
        &self,
        tx: i32,
        tz: i32,
        data: &[u8],
    ) -> std::io::Result<()> {
        let tx = wrap_tile_x(tx);
        let _persistence = self.persistence_lock.lock().await;
        self.terrain_io.write_heightmap(tx, tz, data).await?;
        self.height_sampler.update_tile(tx, tz, data).await?;
        let owners: HashMap<_, _> = self
            .player_characters
            .read()
            .await
            .iter()
            .map(|(id, (character, _, _))| (*id, *character))
            .collect();
        let players = self.players.read().await;
        let mut fences = self.fences.write().await;
        let mut changed = Vec::new();
        for bx in (tx * 64 - 33).div_euclid(32)..=(tx * 64 + 32).div_euclid(32) {
            let bx = (onlinerpg_shared::wrap_world_x((bx * 32) as f32) as i32).div_euclid(32);
            for bz in
                (i64::from(tz) * 64 - 33).div_euclid(32)..=(i64::from(tz) * 64 + 32).div_euclid(32)
            {
                let Ok(bz) = i32::try_from(bz) else { continue };
                let Some(bucket) = fences.buckets.get(&(bx, bz)) else {
                    continue;
                };
                for fence in bucket.values() {
                    if !sample_positions(fence.edge)
                        .iter()
                        .any(|&(x, z)| world_to_tile(x) == tx && world_to_tile(z) == tz)
                    {
                        continue;
                    }
                    let y = self
                        .fence_heights(fence.edge)
                        .await?
                        .into_iter()
                        .fold(f32::INFINITY, f32::min);
                    if fence.y != y {
                        changed.push(Fence { y, ..fence.clone() });
                    }
                }
            }
        }
        let mut groups = HashSet::new();
        for fence in &changed {
            fences.insert(fence.clone());
            groups.insert(FenceIndex::bucket(fence.edge));
        }
        for key in groups {
            let group: Vec<_> = fences.buckets[&key].values().cloned().collect();
            fence::sync_passability(
                &mut self.passability_write(),
                &group[0].edge.cache_key(),
                &group,
            );
        }
        let messages: Vec<_> = players
            .values()
            .filter_map(|player| {
                let added: Vec<_> = changed
                    .iter()
                    .filter(|fence| {
                        player.floor_level == 0
                            && (owners.get(&player.id) == Some(&fence.owner_id)
                                || player.position.dist_xz_sq(&fence.edge.center(fence.y))
                                    <= super::EVENT_DELIVERY_RADIUS.powi(2))
                    })
                    .cloned()
                    .collect();
                (!added.is_empty()).then_some((player.id, added))
            })
            .collect();
        drop(players);
        for (id, added) in messages {
            self.send_direct_message(
                &id,
                ServerMessage::FenceVisibility {
                    added,
                    removed: vec![],
                },
            )
            .await;
        }
        Ok(())
    }

    pub async fn try_start_fence_mode(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        auth: &AuthService,
    ) -> bool {
        let is_fence = self
            .inventories
            .read()
            .await
            .get(player_id)
            .is_some_and(|inv| {
                inv.bag.iter().any(|item| {
                    item.instance_id == instance_id
                        && item.item_def_id == fence::ITEM_ID
                        && item.quantity > 0
                })
            });
        if !is_fence {
            return false;
        }
        self.start_fence_mode(player_id, auth).await;
        true
    }

    pub(super) async fn refresh_fence_owners(
        &self,
        player_id: &PlayerId,
        auth: &AuthService,
    ) -> Result<(), AuthError> {
        let _persistence = self.persistence_lock.lock().await;
        let auth = auth.clone();
        let records = auth_db(move || auth.load_fences()).await?;
        let players = self.players.read().await;
        let mut fences = self.fences.write().await;
        let mut changed = Vec::new();
        for record in records {
            if let Some(fence) = fences.get(&record.edge) {
                if fence.owner_id != record.owner_id {
                    let fence = Fence {
                        owner_id: record.owner_id,
                        ..fence.clone()
                    };
                    fences.insert(fence.clone());
                    changed.push(fence);
                }
            }
        }
        let messages: Vec<_> = players
            .values()
            .filter_map(|player| {
                let added: Vec<_> = changed
                    .iter()
                    .filter(|fence| {
                        player.id == *player_id
                            || (player.floor_level == 0
                                && player.position.dist_xz_sq(&fence.edge.center(fence.y))
                                    <= super::EVENT_DELIVERY_RADIUS.powi(2))
                    })
                    .cloned()
                    .collect();
                (!added.is_empty()).then_some((player.id, added))
            })
            .collect();
        drop(players);
        for (id, added) in messages {
            self.send_direct_message(
                &id,
                ServerMessage::FenceVisibility {
                    added,
                    removed: vec![],
                },
            )
            .await;
        }
        Ok(())
    }

    pub async fn start_fence_mode(&self, player_id: &PlayerId, auth: &AuthService) {
        self.start_landscaping_mode(
            player_id,
            auth,
            onlinerpg_shared::landscaping::LandscapingTool::Fence,
            false,
        )
        .await;
    }

    pub async fn edit_fence(
        &self,
        player_id: &PlayerId,
        edge: FenceEdge,
        place: bool,
        auth: &AuthService,
    ) {
        let error = self
            .try_edit_fence(player_id, edge, place, auth)
            .await
            .err()
            .map(str::to_string);
        self.send_direct_message(player_id, ServerMessage::FenceEditResult { error })
            .await;
    }

    async fn try_edit_fence(
        &self,
        player_id: &PlayerId,
        edge: FenceEdge,
        place: bool,
        auth: &AuthService,
    ) -> Result<(), &'static str> {
        if !edge.valid() {
            return Err("Invalid fence edge.");
        }
        self.tick_land_taxes(auth).await;
        let _persistence = self.persistence_lock.lock().await;
        if self.reject_if_trading(player_id, "edit fences").await {
            return Err("Finish your player trade first.");
        }
        let mut character = self
            .get_player_save_data(player_id)
            .await
            .ok_or("Character not found.")?;
        let max_weight = self.max_carry_weight(player_id).await;
        let armor_mult = self.armor_weight_mult(player_id).await;
        let next_id = self.next_instance_id().await;
        let players = self.players.read().await;
        let player = players.get(player_id).ok_or("Character not found.")?;
        if player.health == 0 || player.floor_level != 0 {
            return Err("Stand on outdoor ground while alive to edit fences.");
        }
        let mut fences = self.fences.write().await;
        let existing = fences.get(&edge).cloned();
        if place && existing.is_some() {
            return Err("A fence is already on that edge.");
        }
        if !place && existing.is_none() {
            return Err("That fence has already been removed.");
        }
        let y = if let Some(fence) = &existing {
            fence.y
        } else {
            let heights = self
                .fence_heights(edge)
                .await
                .map_err(|_| "Terrain is unavailable here.")?;
            for (x, z) in sample_positions(edge) {
                if self
                    .water_depth_at(x, z)
                    .await
                    .is_none_or(|depth| depth > 0.1)
                {
                    return Err("Fences need dry ground.");
                }
            }
            let min = heights.iter().copied().fold(f32::INFINITY, f32::min);
            let max = heights.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            if max - min > 0.5 {
                return Err("This edge is too steep for a fence.");
            }
            min
        };
        let fence = Fence {
            edge,
            y,
            owner_id: character.character_id,
        };
        let gold = self.player_gold.read().await;
        character.gold = gold
            .get(player_id)
            .copied()
            .ok_or("Gold balance not found.")?;
        let mut inventories = self.inventories.write().await;
        let inventory = inventories
            .get_mut(player_id)
            .ok_or("Inventory not found.")?;
        let mut updated = inventory.clone();
        if place {
            let instance_id = updated
                .bag
                .iter()
                .find(|i| i.item_def_id == fence::ITEM_ID && i.quantity > 0)
                .map(|i| i.instance_id)
                .ok_or("You have no wooden fences left. Recover one or leave placement mode.")?;
            consume_one(&mut updated, instance_id);
        } else {
            if self.calc_total_weight(&updated, armor_mult) + self.item_defs.weight(fence::ITEM_ID)
                > max_weight
            {
                return Err("Your bag is too heavy to recover this fence.");
            }
            if updated
                .bag
                .iter()
                .any(|i| i.item_def_id == fence::ITEM_ID && i.quantity == u32::MAX)
            {
                return Err("Your fence stack is full.");
            }
            stack_into_bag(
                &mut updated.bag,
                BagInsert::one(true, fence::ITEM_ID, 0, next_id),
            );
        }
        let rows = serialize_inventory(&updated);
        let auth = auth.clone();
        let saved = fence.clone();
        auth_db(move || auth.save_fence_edit(&character, &rows, &saved, place))
            .await
            .map_err(|error| {
                tracing::warn!(%error, "Failed to save fence edit");
                "The fence edit could not be saved. Your inventory was not changed."
            })??;
        *inventory = updated.clone();
        if place {
            fences.insert(fence.clone());
        } else {
            fences.remove(&edge);
        }
        let key = edge.cache_key();
        let group = fences.group(edge);
        fence::sync_passability(&mut self.passability_write(), &key, &group);
        let recipients: Vec<_> = players
            .values()
            .filter(|p| {
                p.id == *player_id
                    || (p.floor_level == 0
                        && p.position.dist_xz_sq(&edge.center(y))
                            <= super::EVENT_DELIVERY_RADIUS.powi(2))
            })
            .map(|p| p.id)
            .collect();
        drop(inventories);
        drop(gold);
        drop(players);
        self.send_direct_message_to_players(
            &recipients,
            ServerMessage::FenceVisibility {
                added: if place { vec![fence] } else { vec![] },
                removed: if place { vec![] } else { vec![edge] },
            },
        )
        .await;
        drop(fences);
        self.mark_inventory_dirty(player_id).await;
        self.send_inventory_snapshot(player_id, updated).await;
        Ok(())
    }
}
