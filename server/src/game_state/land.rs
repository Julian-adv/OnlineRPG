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
    pub async fn tick_land_taxes(&self, auth: &AuthService) {
        let month = self
            .current_game_day()
            .div_euclid(super::time::GAME_DAYS_PER_MONTH);
        let mut last = self.land_tax_last_month.lock().await;
        if *last == Some(month) {
            return;
        }
        let online = self
            .player_characters
            .read()
            .await
            .values()
            .map(|(id, _, _)| *id)
            .collect::<Vec<_>>();
        let auth = auth.clone();
        match auth_db(move || auth.collect_land_taxes(month, &online)).await {
            Ok(()) => *last = Some(month),
            Err(error) => tracing::warn!(%error, "Failed to collect land taxes"),
        }
    }

    pub async fn land_account_action(
        &self,
        player_id: &PlayerId,
        merchant_id: &PlayerId,
        transfer: Option<(i64, bool)>,
        auth: &AuthService,
    ) {
        let result = self
            .try_land_account_action(player_id, merchant_id, transfer, auth)
            .await;
        let (account, error) = match result {
            Ok(account) => (account, None),
            Err(reason) => (Default::default(), Some(reason.to_string())),
        };
        let now = self.current_total_game_seconds();
        let month_seconds = super::time::GAME_DAYS_PER_MONTH * super::time::GAME_SECONDS_PER_DAY;
        let next = (now.div_euclid(month_seconds) + 1) * month_seconds;
        self.send_direct_message(
            player_id,
            ServerMessage::LandAccountState {
                merchant_player_id: *merchant_id,
                treasury: account.treasury,
                plots: account.plots,
                monthly_tax: account.monthly_tax(),
                next_tax: if account.free_months > 0 {
                    0
                } else {
                    account.monthly_tax()
                },
                next_due: Self::total_game_seconds_to_datetime(next),
                due_in_seconds: ((next - now) as f64 / super::time::GAME_SECONDS_PER_REAL_SECOND)
                    .ceil() as u64,
                missed: account.missed,
                recovery_cost: account.recovery_cost(),
                free_months: account.free_months,
                error,
            },
        )
        .await;
    }

    async fn try_land_account_action(
        &self,
        player_id: &PlayerId,
        merchant_id: &PlayerId,
        transfer: Option<(i64, bool)>,
        auth: &AuthService,
    ) -> Result<crate::auth::LandAccount, &'static str> {
        if !self
            .validate_trader(player_id, merchant_id)
            .await?
            .is_land_registrar()
        {
            return Err("Visit the Land Registrar to manage your tax account.");
        }
        self.tick_land_taxes(auth).await;
        let auth = auth.clone();
        let Some((amount, deposit)) = transfer else {
            let character_id = self
                .player_characters
                .read()
                .await
                .get(player_id)
                .map(|(id, _, _)| *id)
                .ok_or("Character not found.")?;
            return auth_db(move || auth.land_account(character_id))
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "Failed to read land account");
                    "Tax account is temporarily unavailable."
                });
        };
        let _persistence = self.persistence_lock.lock().await;
        if self
            .reject_if_trading(player_id, "transfer tax funds")
            .await
        {
            return Err("Finish your player trade before transferring tax funds.");
        }
        let mut character = self
            .get_player_save_data(player_id)
            .await
            .ok_or("Character not found.")?;
        let mut gold = self.player_gold.write().await;
        let balance = gold.get_mut(player_id).ok_or("Gold balance not found.")?;
        character.gold = *balance;
        let inventories = self.inventories.read().await;
        let inventory = inventories.get(player_id).ok_or("Inventory not found.")?;
        let rows = serialize_inventory(inventory);
        let (updated, account) =
            auth_db(move || auth.transfer_land_gold(character, &rows, amount, deposit))
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "Failed to transfer land gold");
                    "Transfer could not be saved. Your gold was not moved."
                })??;
        *balance = updated;
        drop(inventories);
        drop(gold);
        self.mark_dirty(player_id).await;
        self.send_direct_message(player_id, ServerMessage::GoldUpdate { gold: updated })
            .await;
        Ok(account)
    }

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
        self.tick_land_taxes(auth).await;
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
        let month = self
            .current_game_day()
            .div_euclid(super::time::GAME_DAYS_PER_MONTH);
        let estate_id = auth_db(move || auth.claim_homestead(&character, plot, &rows, month))
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
