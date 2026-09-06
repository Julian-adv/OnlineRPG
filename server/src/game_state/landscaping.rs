use super::{
    auth_db,
    inventory::{consume_one, serialize_inventory},
    GameState,
};
use crate::{
    auth::AuthService,
    types::{PlayerId, ServerMessage},
};
use onlinerpg_shared::landscaping::{
    self, LandscapingStroke, LandscapingTile, LandscapingTool, CLEARED_BYTES, TOOLBOX_ITEM,
};
use onlinerpg_terrain::coords::world_to_tile;
use std::collections::BTreeMap;

impl GameState {
    pub async fn save_terrain_splatmap(
        &self,
        tx: i32,
        tz: i32,
        data: &[u8],
    ) -> std::io::Result<()> {
        let _persistence = self.persistence_lock.lock().await;
        self.terrain_io.write_splatmap(tx, tz, data).await?;
        self.splat_sampler.update_tile(tx, tz, data).await;
        Ok(())
    }

    pub async fn try_use_landscaping_item(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        auth: &AuthService,
        is_admin: bool,
    ) -> bool {
        let item_id = self
            .inventories
            .read()
            .await
            .get(player_id)
            .and_then(|inv| {
                inv.bag
                    .iter()
                    .find(|item| item.instance_id == instance_id && item.quantity > 0)
            })
            .map(|item| item.item_def_id.clone());
        match item_id.as_deref() {
            Some(TOOLBOX_ITEM) => {
                self.start_landscaping_mode(player_id, auth, LandscapingTool::Ground, is_admin)
                    .await
            }
            Some(id) if landscaping::palette_for_item(id).is_some() => {
                if let Err(error) = self
                    .learn_landscaping_palette(player_id, instance_id, auth)
                    .await
                {
                    self.send_system_message(player_id, error).await;
                }
            }
            _ => return false,
        }
        true
    }

    pub async fn start_landscaping_mode(
        &self,
        player_id: &PlayerId,
        auth: &AuthService,
        tool: LandscapingTool,
        is_admin: bool,
    ) {
        if let Err(error) = self
            .try_start_landscaping_mode(player_id, auth, tool, is_admin)
            .await
        {
            self.send_system_message(player_id, error).await;
        }
    }

    async fn try_start_landscaping_mode(
        &self,
        player_id: &PlayerId,
        auth: &AuthService,
        tool: LandscapingTool,
        is_admin: bool,
    ) -> Result<(), &'static str> {
        if self
            .reject_if_trading(player_id, "decorate your estate")
            .await
        {
            return Err("Finish your player trade first.");
        }
        self.tick_land_taxes(auth).await;
        self.refresh_fence_owners(player_id, auth)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "Failed to refresh fence ownership");
                "Estate editing is temporarily unavailable."
            })?;
        let _persistence = self.persistence_lock.lock().await;
        let owner_id = self
            .player_characters
            .read()
            .await
            .get(player_id)
            .map(|(id, _, _)| *id)
            .ok_or("Character not found.")?;
        let has_toolbox = self
            .inventories
            .read()
            .await
            .get(player_id)
            .is_some_and(|inv| {
                inv.bag
                    .iter()
                    .any(|item| item.item_def_id == TOOLBOX_ITEM && item.quantity > 0)
            });
        let auth = auth.clone();
        let (plots, palette) = auth_db(move || {
            Ok((
                auth.fence_plots(owner_id)?,
                auth.landscaping_palette(owner_id)?,
            ))
        })
        .await
        .map_err(|error| {
            tracing::warn!(%error, "Failed to load landscaping permissions");
            "Estate editing is temporarily unavailable."
        })?;
        let players = self.players.read().await;
        let player = players.get(player_id).ok_or("Character not found.")?;
        if player.health == 0 || player.floor_level != 0 {
            return Err("Stand on outdoor ground while alive to decorate your estate.");
        }
        if tool != LandscapingTool::Fence {
            if !has_toolbox {
                return Err("Carry a Landscaper's Toolbox to paint your estate.");
            }
            if !is_admin
                && !landscaping::owns_position(&plots, player.position.x, player.position.z)
            {
                return Err("Use the toolbox inside your own estate with no overdue taxes.");
            }
        }
        drop(players);
        self.send_direct_message(
            player_id,
            ServerMessage::LandscapingMode {
                owner_id,
                plots,
                palette,
                has_toolbox,
                tool,
            },
        )
        .await;
        Ok(())
    }

    async fn learn_landscaping_palette(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        auth: &AuthService,
    ) -> Result<(), &'static str> {
        let _persistence = self.persistence_lock.lock().await;
        if self
            .reject_if_trading(player_id, "learn a landscaping palette")
            .await
        {
            return Err("Finish your player trade first.");
        }
        let mut character = self
            .get_player_save_data(player_id)
            .await
            .ok_or("Character not found.")?;
        let players = self.players.read().await;
        let player = players.get(player_id).ok_or("Character not found.")?;
        if player.health == 0 {
            return Err("You must be alive to learn a landscaping palette.");
        }
        let gold = self.player_gold.read().await;
        character.gold = gold
            .get(player_id)
            .copied()
            .ok_or("Gold balance not found.")?;
        let mut inventories = self.inventories.write().await;
        let inventory = inventories
            .get_mut(player_id)
            .ok_or("Inventory not found.")?;
        let item = inventory
            .bag
            .iter()
            .find(|item| item.instance_id == instance_id && item.quantity > 0)
            .ok_or("That palette is no longer in your bag.")?;
        let slot = landscaping::palette_for_item(&item.item_def_id)
            .ok_or("That item is not a landscaping palette.")?;
        let name = self.item_name(&item.item_def_id);
        let mut updated = inventory.clone();
        consume_one(&mut updated, instance_id);
        let rows = serialize_inventory(&updated);
        let auth_copy = auth.clone();
        let learned =
            auth_db(move || auth_copy.unlock_landscaping_palette(&character, &rows, slot))
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "Failed to save landscaping palette");
                    "The palette could not be learned. Your inventory was not changed."
                })?;
        if !learned {
            return Err("You already know this palette. The sample book was not consumed.");
        }
        *inventory = updated.clone();
        drop(inventories);
        drop(gold);
        drop(players);
        self.mark_inventory_dirty(player_id).await;
        self.send_inventory_snapshot(player_id, updated).await;
        let owner = self
            .player_characters
            .read()
            .await
            .get(player_id)
            .map(|(id, _, _)| *id)
            .ok_or("Character not found.")?;
        let auth = auth.clone();
        if let Ok(palette) = auth_db(move || auth.landscaping_palette(owner)).await {
            self.send_direct_message(
                player_id,
                ServerMessage::LandscapingPaletteUnlocked { palette },
            )
            .await;
        }
        self.send_system_message(
            player_id,
            format!("Learned {name}. This ground material is permanently unlocked."),
        )
        .await;
        Ok(())
    }

    pub async fn edit_landscape(
        &self,
        player_id: &PlayerId,
        stroke: LandscapingStroke,
        auth: &AuthService,
        is_admin: bool,
    ) {
        let error = self
            .try_edit_landscape(player_id, stroke, auth, is_admin)
            .await
            .err()
            .map(str::to_string);
        self.send_direct_message(player_id, ServerMessage::LandscapeEditResult { error })
            .await;
    }

    async fn try_edit_landscape(
        &self,
        player_id: &PlayerId,
        stroke: LandscapingStroke,
        auth: &AuthService,
        is_admin: bool,
    ) -> Result<(), &'static str> {
        if !stroke.valid() {
            return Err("Invalid brush settings or road length.");
        }
        self.tick_land_taxes(auth).await;
        let _persistence = self.persistence_lock.lock().await;
        if self.reject_if_trading(player_id, "paint your estate").await {
            return Err("Finish your player trade first.");
        }
        let owner = self
            .player_characters
            .read()
            .await
            .get(player_id)
            .map(|(id, _, _)| *id)
            .ok_or("Character not found.")?;
        let inventories = self.inventories.read().await;
        if !inventories.get(player_id).is_some_and(|inv| {
            inv.bag
                .iter()
                .any(|item| item.item_def_id == TOOLBOX_ITEM && item.quantity > 0)
        }) {
            return Err("Carry a Landscaper's Toolbox to paint your estate.");
        }
        let auth = auth.clone();
        let (plots, palette) =
            auth_db(move || Ok((auth.fence_plots(owner)?, auth.landscaping_palette(owner)?)))
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "Failed to load landscaping permissions");
                    "Estate editing is temporarily unavailable."
                })?;
        if !palette.contains(&stroke.palette) {
            return Err("Learn this palette before using it.");
        }
        let players = self.players.read().await;
        let player = players.get(player_id).ok_or("Character not found.")?;
        if player.health == 0 || player.floor_level != 0 {
            return Err("Stand on outdoor ground while alive to decorate your estate.");
        }
        if !is_admin && !landscaping::owns_position(&plots, player.position.x, player.position.z) {
            return Err("Stand inside your own estate with no overdue taxes to paint.");
        }
        let samples = stroke.samples(if is_admin { None } else { Some(&plots) });
        if samples.is_empty() {
            return Err("This brush does not cover editable ground on your estate.");
        }
        let mut grouped = BTreeMap::new();
        for sample in samples {
            grouped
                .entry((
                    world_to_tile(sample.x as f32),
                    world_to_tile(sample.z as f32),
                ))
                .or_insert_with(Vec::new)
                .push(sample);
        }
        let mut updates = Vec::new();
        for ((tx, tz), samples) in grouped {
            let mut tile = match self
                .terrain_io
                .read_landscaping_tile(tx, tz)
                .await
                .map_err(terrain_error)?
            {
                Some(tile) => tile,
                None => LandscapingTile {
                    tile_x: tx,
                    tile_z: tz,
                    splat: self
                        .terrain_io
                        .read_splatmap(tx, tz)
                        .await
                        .map_err(terrain_error)?,
                    cleared: vec![0; CLEARED_BYTES],
                },
            };
            let mut changed = false;
            for sample in samples {
                let index = ((sample.z - (tz * 64 - 32)) * 64 + sample.x - (tx * 64 - 32)) as usize;
                if landscaping::paint_cell(
                    &mut tile.splat[index * 4..index * 4 + 4],
                    stroke.palette,
                    sample,
                ) {
                    changed = true;
                }
                let cell = &tile.splat[index * 4..index * 4 + 4];
                let dominant = if cell[2] >= 128 {
                    cell[0] & 15
                } else {
                    cell[0] >> 4
                };
                if !sample.fringe && dominant != 0 && !landscaping::is_cleared(&tile.cleared, index)
                {
                    landscaping::clear_cell(&mut tile.cleared, index);
                    changed = true;
                }
            }
            if changed {
                updates.push(tile);
            }
        }
        if updates.is_empty() {
            return Ok(());
        }
        let mut saved = Vec::new();
        let mut error = None;
        for tile in updates {
            if let Err(failure) = self.terrain_io.write_landscaping_tile(&tile).await {
                error = Some(terrain_error(failure));
                break;
            }
            self.splat_sampler
                .update_tile(tile.tile_x, tile.tile_z, &tile.splat)
                .await;
            saved.push(tile);
        }
        let messages: Vec<_> = players
            .values()
            .map(|recipient| {
                let mut nearby = Vec::new();
                let mut distant = Vec::new();
                for tile in &saved {
                    if recipient.id == *player_id
                        || recipient.position.dist_xz_sq(&crate::types::Position {
                            x: (tile.tile_x * 64) as f32,
                            y: 0.0,
                            z: (tile.tile_z * 64) as f32,
                        }) <= (super::EVENT_DELIVERY_RADIUS + 64.0).powi(2)
                    {
                        nearby.push(tile.clone());
                    } else {
                        distant.push((tile.tile_x, tile.tile_z));
                    }
                }
                (recipient.id, nearby, distant)
            })
            .collect();
        drop(players);
        drop(inventories);
        for (id, tiles, distant) in messages {
            if !tiles.is_empty() {
                self.send_direct_message(&id, ServerMessage::LandscapeChanged { tiles })
                    .await;
            }
            if !distant.is_empty() {
                self.send_direct_message(
                    &id,
                    ServerMessage::LandscapeInvalidated { tiles: distant },
                )
                .await;
            }
        }
        match error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

fn terrain_error(error: std::io::Error) -> &'static str {
    tracing::warn!(%error, "Landscape terrain operation failed");
    "Terrain is temporarily unavailable. Saved portions remain visible; try the brush again."
}
