use super::{auth_db, inventory::serialize_inventory, GameState};
use crate::auth::AuthService;
use crate::types::{Player, PlayerId, ServerMessage};
use onlinerpg_terrain::land::{plot_addr, LandGrade, PlotAddr};

pub(super) fn plot_key(addr: PlotAddr) -> (i32, i32, u8) {
    let tile = (addr.index / 4) as i32;
    (
        addr.rx * 16 + tile % 16,
        addr.rz * 16 + tile / 16,
        (addr.index % 4) as u8,
    )
}

impl GameState {
    pub async fn try_preview_land_claim(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        auth: &AuthService,
    ) -> bool {
        let is_document = self
            .inventories
            .read()
            .await
            .get(player_id)
            .is_some_and(|inv| {
                inv.bag.iter().any(|item| {
                    item.instance_id == instance_id
                        && item.item_def_id == "land_deed"
                        && item.quantity == 1
                })
            });
        if !is_document {
            return false;
        }
        let players = self.players.read().await;
        let Some(player) = players.get(player_id).cloned() else {
            return true;
        };
        drop(players);
        let plot @ (tile_x, tile_z, quadrant) =
            plot_key(plot_addr(player.position.x, player.position.z));
        let reason = self
            .check_land_preview(player_id, instance_id, &player, plot, auth)
            .await
            .err()
            .map(str::to_string);
        self.send_direct_message(
            player_id,
            ServerMessage::LandClaimPrompt {
                instance_id,
                tile_x,
                tile_z,
                quadrant,
                reason,
            },
        )
        .await;
        true
    }

    async fn check_land_preview(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        player: &Player,
        plot: (i32, i32, u8),
        auth: &AuthService,
    ) -> Result<(), &'static str> {
        if self.trade_reserved_quantity(player_id, instance_id).await > 0 {
            return Err("This Land Deed is reserved for trade.");
        }
        let addr = claim_location(player, plot)?;
        self.check_land_grade(addr).await?;
        let character_id = self
            .player_characters
            .read()
            .await
            .get(player_id)
            .map(|(id, _, _)| *id)
            .ok_or("Character not found.")?;
        let auth = auth.clone();
        auth_db(move || auth.check_homestead_claim(character_id, plot))
            .await
            .map_err(|error| {
                tracing::warn!(%error, "Failed to check land claim");
                "Land registration is temporarily unavailable."
            })?
    }

    async fn check_land_grade(&self, addr: PlotAddr) -> Result<(), &'static str> {
        let grades = self
            .terrain_io
            .read_land_grades(addr.rx, addr.rz)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "Failed to read land grades");
                "Land registration is temporarily unavailable."
            })?;
        let grade = match grades {
            Some(grades) => LandGrade::try_from(grades[addr.index])
                .map_err(|_| "This plot has an invalid land grade.")?,
            None => crate::land_grades::default_grade(addr),
        };
        match grade {
            LandGrade::Homestead => Ok(()),
            LandGrade::Crown => Err("Crown land cannot be claimed with a Land Deed."),
            LandGrade::Reserved => Err("This plot is reserved and cannot be claimed."),
        }
    }

    pub async fn claim_land(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        plot: (i32, i32, u8),
        auth: &AuthService,
    ) {
        let result = self
            .try_claim_land(player_id, instance_id, plot, auth)
            .await;
        match result {
            Ok(estate_id) => {
                let (tile_x, tile_z, quadrant) = plot;
                self.send_direct_message(
                    player_id,
                    ServerMessage::LandClaimed {
                        estate_id,
                        tile_x,
                        tile_z,
                        quadrant,
                    },
                )
                .await;
            }
            Err(reason) => {
                self.send_direct_message(
                    player_id,
                    ServerMessage::LandRejected {
                        reason: reason.to_string(),
                    },
                )
                .await
            }
        }
    }

    async fn try_claim_land(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        plot: (i32, i32, u8),
        auth: &AuthService,
    ) -> Result<i64, &'static str> {
        if self.trade_reserved_quantity(player_id, instance_id).await > 0 {
            return Err("This Land Deed is reserved for trade.");
        }
        let persistence = self.persistence_lock.lock().await;
        let character = self
            .get_player_save_data(player_id)
            .await
            .ok_or("Character not found.")?;
        let players = self.players.read().await;
        let player = players.get(player_id).ok_or("Character not found.")?;
        let addr = claim_location(player, plot)?;
        self.check_land_grade(addr).await?;
        let mut inventories = self.inventories.write().await;
        let inventory = inventories
            .get_mut(player_id)
            .ok_or("Inventory not found.")?;
        let index = inventory
            .bag
            .iter()
            .position(|item| {
                item.instance_id == instance_id
                    && item.item_def_id == "land_deed"
                    && item.quantity == 1
            })
            .ok_or("Land Deed not found in your bag.")?;
        let mut updated = inventory.clone();
        updated.bag.remove(index);
        let rows = serialize_inventory(&updated);
        let auth = auth.clone();
        let estate_id = auth_db(move || auth.claim_homestead(&character, plot, &rows))
            .await
            .map_err(|error| {
                tracing::warn!(%error, "Failed to commit land claim");
                "Land registration could not be saved. Your Land Deed was not consumed."
            })??;
        *inventory = updated.clone();
        drop(inventories);
        drop(players);
        drop(persistence);
        self.mark_inventory_dirty(player_id).await;
        self.send_inventory_snapshot(player_id, updated).await;
        Ok(estate_id)
    }
}

fn claim_location(player: &Player, plot: (i32, i32, u8)) -> Result<PlotAddr, &'static str> {
    if player.health == 0 {
        return Err("You must be alive to claim land.");
    }
    if player.level < 10 {
        return Err("You must be level 10 or higher to use a Land Deed.");
    }
    if player.floor_level != 0 {
        return Err("Stand on outdoor ground to claim land.");
    }
    let addr = plot_addr(player.position.x, player.position.z);
    if !player.position.x.is_finite()
        || !player.position.z.is_finite()
        || !(-16..16).contains(&addr.rz)
    {
        return Err("This location is outside the claimable world.");
    }
    if plot_key(addr) != plot {
        return Err("You left the selected plot. Use the Land Deed again.");
    }
    Ok(addr)
}
