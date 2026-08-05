use std::collections::HashMap;

use crate::merchant_defs::{merchant_defs, MerchantDefinition};
use crate::npc_defs::{npc_defs, NpcDefinition};
use crate::types::{PlayerId, ServerMessage};
use onlinerpg_shared::messages::{BagLineItem, BuybackEntry, DealKind, StockEntry, TradeLineItem};
use tracing::info;

use super::combat::reachable_dist_sq;
use super::deals::{buy_price, deal_half_band_pct, resident_half_band_pct, sell_payout, DealEntry};
use super::inventory::{draw_from_bag, stack_into_bag, BagInsert};
use super::StoredBuyback;

/// Maximum distance between player and trader for any shop interaction.
const MAX_TRADE_DISTANCE: f32 = 6.0;

/// Most recent sold units kept per (player, merchant) for buyback; older
/// entries are dropped oldest-first.
const BUYBACK_CAP: usize = 10;

/// How long a sold unit stays repurchasable. Long enough to cover finding out
/// about a mis-sell later; a restart clears the map anyway, so this only binds
/// on a long uptime, where it is what keeps the map from growing forever.
const BUYBACK_TTL_MS: u64 = 24 * 60 * 60 * 1000;

/// How often expired entries are reclaimed. Reads filter expiry inline, so
/// this only bounds memory against a 24h TTL — hourly is plenty.
pub const BUYBACK_SWEEP_PERIOD: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// How many `tick_shop_holds` ticks a single open trade window holds an NPC in
/// place before its movement is released anyway. The tick runs on the server's
/// 8s loop, so 4 ticks ≈ 32s. Stops a player pinning an NPC indefinitely by
/// keeping the window open; if the NPC then wanders off the client closes the
/// window at trade range.
const SHOP_HOLD_TICKS: u8 = 4;

/// How an NPC trades (economy phases 1–3). Merchants sell a data-defined
/// catalog with unlimited stock and an effectively unlimited wallet;
/// residents (non-merchants) buy only their wishlist from a finite,
/// salary-funded wallet and sell from their real inventory.
#[derive(Clone)]
pub(crate) enum TraderDef {
    Merchant(MerchantDefinition),
    Resident(NpcDefinition),
}

impl TraderDef {
    pub(crate) fn npc_name(&self) -> &str {
        match self {
            TraderDef::Merchant(def) => &def.npc_name,
            TraderDef::Resident(def) => &def.npc_name,
        }
    }

    /// Haggle eligibility and pricing for one item: the payout rate and the
    /// CHA-derived band half-width, or why the offer is rejected. Keeps the
    /// trader-kind rules (catalog scope, wishlist disjointness, band width)
    /// in one place.
    pub(crate) fn haggle_params(
        &self,
        kind: DealKind,
        item_def_id: &str,
        cha: u8,
    ) -> Result<(u32, i32), &'static str> {
        match self {
            TraderDef::Merchant(m) => {
                if kind == DealKind::Buy && !m.sells(item_def_id) {
                    return Err("that item is not in your catalog");
                }
                Ok((m.sell_rate_percent, deal_half_band_pct(cha)))
            }
            TraderDef::Resident(r) => {
                // Keep the buy/sell item sets disjoint (see resident_stock).
                match kind {
                    DealKind::Sell if !r.wants(item_def_id) => {
                        Err("that item is not on your wishlist")
                    }
                    DealKind::Buy if r.wants(item_def_id) => {
                        Err("you do not resell your wishlist items")
                    }
                    _ => Ok((r.wishlist_rate_percent, resident_half_band_pct(cha))),
                }
            }
        }
    }
}

/// The still-valid entries of one buyback list. Expiry is filtered here on
/// every read rather than swept globally mid-trade (see `tick_buyback_expiry`).
fn live_entries(list: &[StoredBuyback], now_ms: u64) -> Vec<BuybackEntry> {
    list.iter()
        .filter(|stored| stored.is_live(now_ms))
        .map(|stored| stored.entry.clone())
        .collect()
}

/// Look up how an NPC (by character name) trades, if at all.
pub(crate) fn trader_def_by_name(npc_name: &str) -> Option<TraderDef> {
    if let Some(def) = merchant_defs().get_by_npc_name(npc_name) {
        return Some(TraderDef::Merchant(def.clone()));
    }
    npc_defs()
        .get_trader_by_npc_name(npc_name)
        .map(|def| TraderDef::Resident(def.clone()))
}

impl super::GameState {
    async fn send_trade_error(&self, player_id: &PlayerId, message: &str) {
        self.send_direct_message(
            player_id,
            ServerMessage::TradeError {
                message: message.to_string(),
            },
        )
        .await;
    }

    /// Abort a buy/sell whose single-use deal was already taken: put the
    /// deal back and (optionally) tell the player why. Callers drop any
    /// gold/inventory locks before awaiting this.
    async fn fail_trade(
        &self,
        player_id: &PlayerId,
        npc_name: &str,
        item_def_id: &str,
        kind: DealKind,
        deal: Option<super::deals::DealEntry>,
        message: Option<&'static str>,
    ) {
        self.restore_deal(player_id, npc_name, item_def_id, kind, deal)
            .await;
        if let Some(message) = message {
            self.send_trade_error(player_id, message).await;
        }
    }

    async fn send_gold_update(&self, player_id: &PlayerId) {
        let gold = self.get_player_gold(player_id).await;
        self.send_direct_message(player_id, ServerMessage::GoldUpdate { gold })
            .await;
    }

    /// Tell the trading NPC's LLM that a player completed a trade with it.
    /// `npc_gold` is passed in rather than read here: a batch sends one notice
    /// per unit, and the wallet is already final by then.
    async fn send_trade_notice(
        &self,
        npc_player_id: &PlayerId,
        player_name: String,
        item_def_id: &str,
        kind: DealKind,
        price: i64,
        npc_gold: i64,
    ) {
        self.send_direct_message(
            npc_player_id,
            ServerMessage::TradeNotice {
                player_name,
                item_def_id: item_def_id.to_string(),
                kind,
                price,
                npc_gold,
            },
        )
        .await;
    }

    /// Validate that `npc_player_id` is a trading NPC within range of the
    /// player. Returns the trader definition on success.
    async fn validate_trader(
        &self,
        player_id: &PlayerId,
        npc_player_id: &PlayerId,
    ) -> Result<TraderDef, &'static str> {
        let players = self.players.read().await;
        let player = players.get(player_id).ok_or("Player not found")?;
        let npc = players.get(npc_player_id).ok_or("Trader not found")?;

        if !npc.is_official_npc {
            return Err("That character is not a trader");
        }
        let def = trader_def_by_name(&npc.name).ok_or("This NPC does not trade")?;

        let dist_sq = reachable_dist_sq(
            player.position,
            player.floor_level,
            npc.position,
            npc.floor_level,
        )
        .ok_or("The trader is on another floor")?;
        if dist_sq > MAX_TRADE_DISTANCE * MAX_TRADE_DISTANCE {
            return Err("Too far away to trade");
        }

        Ok(def)
    }

    /// `register` records this player as actively shopping with the NPC so it
    /// stays put (see `register_shop_open`). Real window opens pass `true`;
    /// the NPC-pushed `open_trade` passes `false` because the player only sees
    /// an offer toast and may never accept — freezing the NPC on an ignored
    /// offer would strand it.
    pub async fn open_shop(&self, player_id: &PlayerId, npc_player_id: &PlayerId, register: bool) {
        let def = match self.validate_trader(player_id, npc_player_id).await {
            Ok(def) => def,
            Err(reason) => return self.send_trade_error(player_id, reason).await,
        };
        if register {
            self.register_shop_open(npc_player_id, player_id).await;
        }
        let active_deals = self.active_deals_for(player_id, def.npc_name()).await;
        let state = match def {
            TraderDef::Merchant(def) => ServerMessage::ShopState {
                merchant_player_id: *npc_player_id,
                merchant_name: def.npc_name.clone(),
                catalog: def.catalog.clone(),
                sell_rate_percent: def.sell_rate_percent,
                active_deals,
                wishlist: Vec::new(),
                stock: Vec::new(),
                buyback: self.buyback_list(player_id, &def.npc_name).await,
            },
            TraderDef::Resident(def) => ServerMessage::ShopState {
                merchant_player_id: *npc_player_id,
                merchant_name: def.npc_name.clone(),
                catalog: Vec::new(),
                sell_rate_percent: def.wishlist_rate_percent,
                active_deals,
                wishlist: def.wishlist.clone(),
                stock: self.resident_stock(npc_player_id, &def).await,
                buyback: Vec::new(),
            },
        };
        self.send_direct_message(player_id, state).await;
        self.send_gold_update(player_id).await;
    }

    /// NPC-initiated trade (LLM `open_trade` action): push the NPC's own
    /// shop window onto a nearby player's client.
    pub async fn open_trade(&self, npc_player_id: &PlayerId, target_player_id: &PlayerId) {
        let valid = {
            let players = self.players.read().await;
            match (players.get(npc_player_id), players.get(target_player_id)) {
                (Some(npc), _) if !npc.is_official_npc => Err("only NPCs can push trade windows"),
                (Some(npc), _) if trader_def_by_name(&npc.name).is_none() => {
                    Err("you have nothing to trade with")
                }
                (Some(_), None) => Err("that player is not here"),
                (Some(_), Some(target)) if target.is_official_npc => {
                    Err("trade windows can only be opened for players")
                }
                (Some(npc), Some(target)) => {
                    match reachable_dist_sq(
                        target.position,
                        target.floor_level,
                        npc.position,
                        npc.floor_level,
                    ) {
                        None => Err("the player is on another floor"),
                        Some(d) if d > MAX_TRADE_DISTANCE * MAX_TRADE_DISTANCE => {
                            Err("the player is too far away to trade — ask them to come closer")
                        }
                        Some(_) => Ok(()),
                    }
                }
                (None, _) => return,
            }
        };
        if let Err(reason) = valid {
            return self.send_trade_error(npc_player_id, reason).await;
        }
        // open_shop re-validates with the roles in their normal order and
        // sends ShopState + GoldUpdate to the target player. Don't register
        // the NPC as busy yet: the player only sees an offer toast and the
        // real window (with its own OpenShop) registers it on accept.
        self.open_shop(target_player_id, npc_player_id, false).await;
    }

    /// Record that `player_id` opened `merchant_id`'s trade window. When this
    /// is the merchant's first active customer, tell its LLM to hold position
    /// (`TradeBusy { busy: true }`) so it doesn't wander off mid-trade.
    async fn register_shop_open(&self, merchant_id: &PlayerId, player_id: &PlayerId) {
        let became_busy = {
            let mut shops = self.open_shops.write().await;
            let customers = shops.entry(*merchant_id).or_default();
            let was_empty = customers.is_empty();
            // or_insert (not insert): re-opening the same window doesn't
            // refresh the hold, so it can't be gamed to last forever.
            customers.entry(*player_id).or_insert(SHOP_HOLD_TICKS);
            was_empty
        };
        if became_busy {
            self.send_direct_message(merchant_id, ServerMessage::TradeBusy { busy: true })
                .await;
        }
    }

    /// Player closed `merchant_id`'s trade window. When the merchant has no
    /// remaining customers, release the LLM movement hold.
    pub async fn close_shop(&self, player_id: &PlayerId, merchant_id: &PlayerId) {
        let became_free = {
            let mut shops = self.open_shops.write().await;
            let Some(customers) = shops.get_mut(merchant_id) else {
                return;
            };
            customers.remove(player_id);
            let empty = customers.is_empty();
            if empty {
                shops.remove(merchant_id);
            }
            empty
        };
        if became_free {
            self.send_direct_message(merchant_id, ServerMessage::TradeBusy { busy: false })
                .await;
        }
    }

    /// Drop a disconnecting player from all shop tracking, whether it was a
    /// customer (release any merchants it leaves empty) or a merchant itself
    /// (just forget its entry — it's gone).
    pub async fn clear_shops_for_player(&self, player_id: &PlayerId) {
        let freed_merchants = {
            let mut shops = self.open_shops.write().await;
            shops.remove(player_id);
            let mut freed = Vec::new();
            shops.retain(|merchant_id, customers| {
                if customers.remove(player_id).is_some() && customers.is_empty() {
                    freed.push(*merchant_id);
                    false
                } else {
                    true
                }
            });
            freed
        };
        self.send_direct_message_to_players(
            &freed_merchants,
            ServerMessage::TradeBusy { busy: false },
        )
        .await;
    }

    /// Count down every open trade window's hold by one tick. A window whose
    /// hold runs out is dropped (the player keeps it open client-side, but the
    /// NPC is free to move — if it wanders off, the client closes the window at
    /// trade range). Merchants left with no holds are released (`TradeBusy`).
    /// Runs on the server's 8s tick, so `SHOP_HOLD_TICKS` (4) ≈ 32s.
    pub async fn tick_shop_holds(&self) {
        let freed_merchants = {
            let mut shops = self.open_shops.write().await;
            let mut freed = Vec::new();
            shops.retain(|merchant_id, customers| {
                customers.retain(|_, ticks| {
                    *ticks = ticks.saturating_sub(1);
                    *ticks > 0
                });
                if customers.is_empty() {
                    freed.push(*merchant_id);
                    false
                } else {
                    true
                }
            });
            freed
        };
        self.send_direct_message_to_players(
            &freed_merchants,
            ServerMessage::TradeBusy { busy: false },
        )
        .await;
    }

    /// A resident's purchasable stock: priced bag items that are not on its
    /// wishlist. Wishlist purchases are kept (never resold) so the
    /// buy/sell item sets stay disjoint — no money pump is possible even
    /// though the wishlist rate exceeds the sale price.
    async fn resident_stock(
        &self,
        npc_player_id: &PlayerId,
        def: &NpcDefinition,
    ) -> Vec<StockEntry> {
        let inventories = self.inventories.read().await;
        let Some(inv) = inventories.get(npc_player_id) else {
            return Vec::new();
        };
        let mut stock: Vec<StockEntry> = Vec::new();
        for item in &inv.bag {
            if def.wants(&item.item_def_id) {
                continue;
            }
            if self
                .item_defs
                .get(&item.item_def_id)
                .and_then(|d| d.base_price)
                .is_none()
            {
                continue;
            }
            match stock
                .iter_mut()
                .find(|entry| entry.item_def_id == item.item_def_id)
            {
                Some(entry) => entry.quantity += item.quantity,
                None => stock.push(StockEntry {
                    item_def_id: item.item_def_id.clone(),
                    quantity: item.quantity,
                }),
            }
        }
        stock
    }

    /// Buy one unit of `item_def_id` from a trading NPC. Merchants create
    /// the item from its definition (unlimited stock); residents transfer
    /// a unit out of their real inventory and pocket the gold.
    pub async fn buy_item(
        &self,
        player_id: &PlayerId,
        npc_player_id: &PlayerId,
        item_def_id: &str,
    ) {
        let def = match self.validate_trader(player_id, npc_player_id).await {
            Ok(def) => def,
            Err(reason) => return self.send_trade_error(player_id, reason).await,
        };

        match &def {
            TraderDef::Merchant(m) => {
                if !m.sells(item_def_id) {
                    return self
                        .send_trade_error(player_id, "The merchant does not sell that item")
                        .await;
                }
            }
            TraderDef::Resident(r) => {
                if r.wants(item_def_id) {
                    // Wishlist purchases are kept; see `resident_stock`.
                    return self
                        .send_trade_error(player_id, "They won't part with that")
                        .await;
                }
            }
        }

        let Some(base_price) = self
            .item_defs
            .get(item_def_id)
            .and_then(|item| item.base_price)
        else {
            return self
                .send_trade_error(player_id, "That item has no price")
                .await;
        };

        let npc_name = def.npc_name().to_string();
        let is_resident = matches!(def, TraderDef::Resident(_));

        // Single-use haggled modifier; must be restored if the buy fails.
        let deal = self
            .take_deal(player_id, &npc_name, item_def_id, DealKind::Buy)
            .await;
        let price = buy_price(base_price, deal.as_ref().map_or(0, |d| d.modifier_pct));

        let item_weight = self.item_defs.weight(item_def_id);
        let stackable = self.item_defs.stackable(item_def_id);
        let max_weight = self.max_carry_weight(player_id).await;
        let instance_id = self.next_instance_id().await;

        // Gold and inventory mutate under both write locks so a concurrent
        // request cannot double-spend between the check and the deduction.
        let (snapshot, npc_snapshot) = {
            let mut gold_map = self.player_gold.write().await;
            let Some(gold) = gold_map.get(player_id).copied() else {
                drop(gold_map);
                return self
                    .fail_trade(player_id, &npc_name, item_def_id, DealKind::Buy, deal, None)
                    .await;
            };
            if gold < price {
                drop(gold_map);
                return self
                    .fail_trade(
                        player_id,
                        &npc_name,
                        item_def_id,
                        DealKind::Buy,
                        deal,
                        Some("Not enough gold"),
                    )
                    .await;
            }

            let mut inventories = self.inventories.write().await;
            if inventories.get(player_id).is_none() {
                drop(inventories);
                drop(gold_map);
                return self
                    .fail_trade(player_id, &npc_name, item_def_id, DealKind::Buy, deal, None)
                    .await;
            };
            if self.calc_total_weight(&inventories[player_id]) + item_weight > max_weight {
                drop(inventories);
                drop(gold_map);
                return self
                    .fail_trade(
                        player_id,
                        &npc_name,
                        item_def_id,
                        DealKind::Buy,
                        deal,
                        Some("Too heavy to carry"),
                    )
                    .await;
            }

            // Residents have finite stock: take the unit out of their bag.
            let (npc_snapshot, purchased_enchant) = if is_resident {
                let Some(npc_inv) = inventories.get_mut(npc_player_id) else {
                    drop(inventories);
                    drop(gold_map);
                    return self
                        .fail_trade(
                            player_id,
                            &npc_name,
                            item_def_id,
                            DealKind::Buy,
                            deal,
                            Some("They have nothing to sell"),
                        )
                        .await;
                };
                let Some(&(purchased_enchant, _)) =
                    draw_from_bag(&mut npc_inv.bag, item_def_id, 1).first()
                else {
                    drop(inventories);
                    drop(gold_map);
                    return self
                        .fail_trade(
                            player_id,
                            &npc_name,
                            item_def_id,
                            DealKind::Buy,
                            deal,
                            Some("They are out of that item"),
                        )
                        .await;
                };
                (Some(npc_inv.clone()), purchased_enchant)
            } else {
                (None, 0)
            };

            let inv = inventories.get_mut(player_id).expect("checked above");
            stack_into_bag(
                &mut inv.bag,
                BagInsert::one(stackable, item_def_id, purchased_enchant, instance_id),
            );
            let snapshot = inv.clone();

            *gold_map.get_mut(player_id).expect("checked above") -= price;
            if is_resident {
                *gold_map.entry(*npc_player_id).or_insert(0) += price;
            }
            (snapshot, npc_snapshot)
        };

        let player_name = self.player_name_of(player_id).await;
        if let Some(entry) = deal {
            info!(
                target: "deal",
                "deal redeemed: npc={npc_name} player={player_name} item={item_def_id} kind=Buy \
                 modifier={} base={base_price} paid={price}",
                entry.modifier_pct
            );
            self.send_deal_cleared(player_id, npc_player_id, item_def_id, DealKind::Buy)
                .await;
        }
        info!("{player_name} bought {item_def_id} from {npc_name} for {price}");
        self.mark_dirty(player_id).await;
        self.mark_inventory_dirty(player_id).await;
        self.send_direct_message(
            player_id,
            ServerMessage::InventoryUpdated {
                inventory: snapshot,
            },
        )
        .await;
        self.send_gold_update(player_id).await;

        if let Some(npc_snapshot) = npc_snapshot {
            self.mark_dirty(npc_player_id).await;
            self.mark_inventory_dirty(npc_player_id).await;
            self.send_direct_message(
                npc_player_id,
                ServerMessage::InventoryUpdated {
                    inventory: npc_snapshot,
                },
            )
            .await;
            self.send_gold_update(npc_player_id).await;
        }
        let npc_gold = self.get_player_gold(npc_player_id).await;
        self.send_trade_notice(
            npc_player_id,
            player_name,
            item_def_id,
            DealKind::Buy,
            price,
            npc_gold,
        )
        .await;
    }

    /// Buy multiple units, possibly of different items, in one all-or-nothing
    /// transaction: every line is validated (catalog/wishlist membership,
    /// resident stock, carry weight, player gold) before anything is
    /// mutated. Mirrors `buy_item`'s per-unit rules, generalized to N lines,
    /// and merges each purchase into an existing matching bag stack instead
    /// of always creating a new one-unit entry.
    pub async fn buy_items(
        &self,
        player_id: &PlayerId,
        npc_player_id: &PlayerId,
        items: Vec<TradeLineItem>,
    ) {
        if items.is_empty() {
            return;
        }
        let def = match self.validate_trader(player_id, npc_player_id).await {
            Ok(def) => def,
            Err(reason) => return self.send_trade_error(player_id, reason).await,
        };
        let npc_name = def.npc_name().to_string();
        let is_resident = matches!(def, TraderDef::Resident(_));

        struct Plan {
            item_def_id: String,
            qty: u32,
            base_price: i64,
            deal_taken: Option<DealEntry>,
            price: i64,
        }

        struct Purchase<'a> {
            item_def_id: &'a str,
            qty: u32,
            enchant: i32,
        }

        let mut plans: Vec<Plan> = Vec::with_capacity(items.len());
        for req in &items {
            if req.qty == 0 {
                continue;
            }
            match &def {
                TraderDef::Merchant(m) => {
                    if !m.sells(&req.item_def_id) {
                        return self
                            .send_trade_error(player_id, "The merchant does not sell that item")
                            .await;
                    }
                }
                TraderDef::Resident(r) => {
                    if r.wants(&req.item_def_id) {
                        return self
                            .send_trade_error(player_id, "They won't part with that")
                            .await;
                    }
                }
            }
            let Some(base_price) = self
                .item_defs
                .get(&req.item_def_id)
                .and_then(|item| item.base_price)
            else {
                return self
                    .send_trade_error(player_id, "That item has no price")
                    .await;
            };
            plans.push(Plan {
                item_def_id: req.item_def_id.clone(),
                qty: req.qty,
                base_price,
                deal_taken: None,
                price: 0,
            });
        }
        if plans.is_empty() {
            return;
        }

        // One id per unit, reserved before the locks rather than one counter
        // acquisition per unit under them. An aborted batch skips the range.
        let mut next_id = self
            .reserve_instance_ids(plans.iter().map(|p| p.qty as u64).sum())
            .await;
        let max_weight = self.max_carry_weight(player_id).await;

        let mut gold_map = self.player_gold.write().await;
        let mut inventories = self.inventories.write().await;

        if inventories.get(player_id).is_none() {
            drop(inventories);
            drop(gold_map);
            return self.send_trade_error(player_id, "Player not found").await;
        }

        if is_resident {
            let Some(npc_inv) = inventories.get(npc_player_id) else {
                drop(inventories);
                drop(gold_map);
                return self
                    .send_trade_error(player_id, "They have nothing to sell")
                    .await;
            };
            let mut requested: HashMap<&str, u32> = HashMap::new();
            for plan in &plans {
                *requested.entry(plan.item_def_id.as_str()).or_default() += plan.qty;
            }
            for (item_def_id, qty) in requested {
                let available: u32 = npc_inv
                    .bag
                    .iter()
                    .filter(|i| i.item_def_id == item_def_id)
                    .map(|i| i.quantity)
                    .sum();
                if available < qty {
                    drop(inventories);
                    drop(gold_map);
                    return self
                        .send_trade_error(player_id, "They are out of that item")
                        .await;
                }
            }
        }

        let added_weight: f32 = plans
            .iter()
            .map(|p| self.item_defs.weight(&p.item_def_id) * p.qty as f32)
            .sum();
        if self.calc_total_weight(&inventories[player_id]) + added_weight > max_weight {
            drop(inventories);
            drop(gold_map);
            return self.send_trade_error(player_id, "Too heavy to carry").await;
        }

        let mut total_price: i64 = 0;
        let taken = {
            let line_defs: Vec<&str> = plans.iter().map(|p| p.item_def_id.as_str()).collect();
            self.take_deals(player_id, &npc_name, DealKind::Buy, &line_defs)
                .await
        };
        for (plan, deal) in plans.iter_mut().zip(taken) {
            let deal_units: u32 = if deal.is_some() { 1 } else { 0 };
            let normal_units = plan.qty - deal_units;
            let price = buy_price(plan.base_price, deal.as_ref().map_or(0, |d| d.modifier_pct))
                * deal_units as i64
                + buy_price(plan.base_price, 0) * normal_units as i64;
            plan.deal_taken = deal;
            plan.price = price;
            total_price += price;
        }

        let gold = gold_map.get(player_id).copied().unwrap_or(0);
        if gold < total_price {
            let restore: Vec<(&str, DealEntry)> = plans
                .iter()
                .filter_map(|p| Some((p.item_def_id.as_str(), p.deal_taken.clone()?)))
                .collect();
            self.restore_deals(player_id, &npc_name, DealKind::Buy, &restore)
                .await;
            drop(inventories);
            drop(gold_map);
            return self.send_trade_error(player_id, "Not enough gold").await;
        }

        // Every line is now guaranteed to apply cleanly — mutate. Resident
        // stock draws fan a line out into one purchase per stock entry taken;
        // merchant catalogs are infinite, so a line maps straight through.
        let purchases: Vec<Purchase> = if is_resident {
            let npc_inv = inventories.get_mut(npc_player_id).expect("checked above");
            let mut purchases = Vec::new();
            for plan in &plans {
                for (enchant, qty) in draw_from_bag(&mut npc_inv.bag, &plan.item_def_id, plan.qty) {
                    purchases.push(Purchase {
                        item_def_id: &plan.item_def_id,
                        qty,
                        enchant,
                    });
                }
            }
            purchases
        } else {
            plans
                .iter()
                .map(|plan| Purchase {
                    item_def_id: &plan.item_def_id,
                    qty: plan.qty,
                    enchant: 0,
                })
                .collect()
        };

        {
            let inv = inventories.get_mut(player_id).expect("checked above");
            for purchase in &purchases {
                let stackable = self.item_defs.stackable(purchase.item_def_id);
                next_id += stack_into_bag(
                    &mut inv.bag,
                    BagInsert {
                        stackable,
                        item_def_id: purchase.item_def_id,
                        enchant: purchase.enchant,
                        first_instance_id: next_id,
                        quantity: purchase.qty,
                    },
                );
            }
        }

        let snapshot = inventories.get(player_id).expect("checked above").clone();
        let npc_snapshot = is_resident.then(|| {
            inventories
                .get(npc_player_id)
                .expect("checked above")
                .clone()
        });

        *gold_map.get_mut(player_id).expect("checked above") -= total_price;
        if is_resident {
            *gold_map.entry(*npc_player_id).or_insert(0) += total_price;
        }
        drop(inventories);
        drop(gold_map);

        let player_name = self.player_name_of(player_id).await;
        for plan in &plans {
            if let Some(entry) = &plan.deal_taken {
                info!(
                    target: "deal",
                    "deal redeemed: npc={npc_name} player={player_name} item={} kind=Buy \
                     modifier={} base={} paid={}",
                    plan.item_def_id, entry.modifier_pct, plan.base_price, plan.price
                );
                self.send_deal_cleared(player_id, npc_player_id, &plan.item_def_id, DealKind::Buy)
                    .await;
            }
            info!(
                "{player_name} bought {}x{} from {npc_name} for {}",
                plan.qty, plan.item_def_id, plan.price
            );
        }

        self.mark_dirty(player_id).await;
        self.mark_inventory_dirty(player_id).await;
        self.send_direct_message(
            player_id,
            ServerMessage::InventoryUpdated {
                inventory: snapshot,
            },
        )
        .await;
        self.send_gold_update(player_id).await;

        if let Some(npc_snapshot) = npc_snapshot {
            self.mark_dirty(npc_player_id).await;
            self.mark_inventory_dirty(npc_player_id).await;
            self.send_direct_message(
                npc_player_id,
                ServerMessage::InventoryUpdated {
                    inventory: npc_snapshot,
                },
            )
            .await;
            self.send_gold_update(npc_player_id).await;
        }
        let npc_gold = self.get_player_gold(npc_player_id).await;
        for plan in &plans {
            let normal_price = buy_price(plan.base_price, 0);
            for unit in 0..plan.qty {
                let unit_price = if unit == 0 {
                    plan.deal_taken.as_ref().map_or(normal_price, |deal| {
                        buy_price(plan.base_price, deal.modifier_pct)
                    })
                } else {
                    normal_price
                };
                self.send_trade_notice(
                    npc_player_id,
                    player_name.clone(),
                    &plan.item_def_id,
                    DealKind::Buy,
                    unit_price,
                    npc_gold,
                )
                .await;
            }
        }
    }

    /// Sell one unit of a bag item to a trading NPC. Merchants pay
    /// `base_price * sell_rate_percent / 100` and the item vanishes;
    /// residents only buy wishlist items, pay their premium rate out of a
    /// finite wallet, and keep the item in their real inventory.
    pub async fn sell_item(
        &self,
        player_id: &PlayerId,
        npc_player_id: &PlayerId,
        instance_id: u64,
    ) {
        let def = match self.validate_trader(player_id, npc_player_id).await {
            Ok(def) => def,
            Err(reason) => return self.send_trade_error(player_id, reason).await,
        };

        // Resolve the item def up front so any haggled sell bonus can be
        // looked up before taking the gold/inventory locks.
        let item_def_id = {
            let inventories = self.inventories.read().await;
            let Some(item) = inventories
                .get(player_id)
                .and_then(|inv| inv.bag.iter().find(|i| i.instance_id == instance_id))
            else {
                return self
                    .send_trade_error(player_id, "Item not found in bag")
                    .await;
            };
            item.item_def_id.clone()
        };
        let Some(base_price) = self
            .item_defs
            .get(&item_def_id)
            .and_then(|item| item.base_price)
        else {
            return self
                .send_trade_error(player_id, "They will not buy that")
                .await;
        };

        let rate = match &def {
            TraderDef::Merchant(m) => m.sell_rate_percent,
            TraderDef::Resident(r) => {
                if !r.wants(&item_def_id) {
                    return self
                        .send_trade_error(player_id, "They have no use for that")
                        .await;
                }
                r.wishlist_rate_percent
            }
        };
        let npc_name = def.npc_name().to_string();
        let is_resident = matches!(def, TraderDef::Resident(_));

        // Single-use haggled modifier; must be restored if the sell fails.
        let deal = self
            .take_deal(player_id, &npc_name, &item_def_id, DealKind::Sell)
            .await;
        let payout = sell_payout(
            base_price,
            rate,
            deal.as_ref().map_or(0, |d| d.modifier_pct),
        );

        let item_weight = self.item_defs.weight(&item_def_id);
        let npc_max_weight = self.max_carry_weight(npc_player_id).await;
        // Resident: the transferred unit's instance id. Merchant: the buyback
        // entry id, reused as the instance id on repurchase.
        let npc_instance_id = self.next_instance_id().await;

        let (snapshot, npc_snapshot, sold_enchant) = {
            let mut gold_map = self.player_gold.write().await;
            if !gold_map.contains_key(player_id) {
                drop(gold_map);
                return self
                    .fail_trade(
                        player_id,
                        &npc_name,
                        &item_def_id,
                        DealKind::Sell,
                        deal,
                        None,
                    )
                    .await;
            }
            // Residents pay out of a real wallet; merchants out of thin air.
            if is_resident {
                let npc_gold = gold_map.get(npc_player_id).copied().unwrap_or(0);
                if npc_gold < payout {
                    drop(gold_map);
                    return self
                        .fail_trade(
                            player_id,
                            &npc_name,
                            &item_def_id,
                            DealKind::Sell,
                            deal,
                            Some("They cannot afford that right now"),
                        )
                        .await;
                }
            }

            let mut inventories = self.inventories.write().await;
            let Some((idx, sold_enchant)) = inventories.get_mut(player_id).and_then(|inv| {
                inv.bag
                    .iter()
                    .position(|i| i.instance_id == instance_id)
                    .map(|idx| (idx, inv.bag[idx].enchant))
            }) else {
                drop(inventories);
                drop(gold_map);
                return self
                    .fail_trade(
                        player_id,
                        &npc_name,
                        &item_def_id,
                        DealKind::Sell,
                        deal,
                        Some("Item not found in bag"),
                    )
                    .await;
            };

            // The bought unit lands in the resident's real inventory, so it
            // is bound by their carry weight like any player.
            let npc_snapshot = if is_resident {
                let Some(npc_inv) = inventories.get_mut(npc_player_id) else {
                    drop(inventories);
                    drop(gold_map);
                    return self
                        .fail_trade(
                            player_id,
                            &npc_name,
                            &item_def_id,
                            DealKind::Sell,
                            deal,
                            None,
                        )
                        .await;
                };
                if self.calc_total_weight(npc_inv) + item_weight > npc_max_weight {
                    drop(inventories);
                    drop(gold_map);
                    return self
                        .fail_trade(
                            player_id,
                            &npc_name,
                            &item_def_id,
                            DealKind::Sell,
                            deal,
                            Some("They cannot carry any more"),
                        )
                        .await;
                }
                // Keep the sold unit's enchantment: a +3 sword stays +3 in
                // the resident's bag.
                let stackable = self.item_defs.stackable(&item_def_id);
                stack_into_bag(
                    &mut npc_inv.bag,
                    BagInsert::one(stackable, &item_def_id, sold_enchant, npc_instance_id),
                );
                Some(npc_inv.clone())
            } else {
                None
            };

            let inv = inventories.get_mut(player_id).expect("checked above");
            if inv.bag[idx].quantity > 1 {
                inv.bag[idx].quantity -= 1;
            } else {
                inv.bag.remove(idx);
            }
            let snapshot = inv.clone();

            *gold_map.get_mut(player_id).expect("checked above") += payout;
            if is_resident {
                *gold_map.get_mut(npc_player_id).expect("checked above") -= payout;
            }
            (snapshot, npc_snapshot, sold_enchant)
        };

        // The unit a merchant buys vanishes (no stock), so record it for
        // buyback at the exact payout — the only way to undo a mis-sell.
        if !is_resident {
            let buyback = self
                .record_buybacks(
                    player_id,
                    &npc_name,
                    vec![BuybackEntry {
                        entry_id: npc_instance_id,
                        item_def_id: item_def_id.clone(),
                        enchant: sold_enchant,
                        price: payout,
                    }],
                )
                .await;
            self.send_direct_message(
                player_id,
                ServerMessage::BuybackUpdated {
                    merchant_player_id: *npc_player_id,
                    buyback,
                },
            )
            .await;
        }

        let player_name = self.player_name_of(player_id).await;
        if let Some(entry) = deal {
            info!(
                target: "deal",
                "deal redeemed: npc={npc_name} player={player_name} item={item_def_id} kind=Sell \
                 modifier={} base={base_price} paid={payout}",
                entry.modifier_pct
            );
            self.send_deal_cleared(player_id, npc_player_id, &item_def_id, DealKind::Sell)
                .await;
        }
        info!("{player_name} sold {item_def_id} to {npc_name} for {payout}");
        self.mark_dirty(player_id).await;
        self.mark_inventory_dirty(player_id).await;
        self.send_direct_message(
            player_id,
            ServerMessage::InventoryUpdated {
                inventory: snapshot,
            },
        )
        .await;
        self.send_gold_update(player_id).await;

        if let Some(npc_snapshot) = npc_snapshot {
            self.mark_dirty(npc_player_id).await;
            self.mark_inventory_dirty(npc_player_id).await;
            self.send_direct_message(
                npc_player_id,
                ServerMessage::InventoryUpdated {
                    inventory: npc_snapshot,
                },
            )
            .await;
            self.send_gold_update(npc_player_id).await;
        }
        let npc_gold = self.get_player_gold(npc_player_id).await;
        self.send_trade_notice(
            npc_player_id,
            player_name,
            &item_def_id,
            DealKind::Sell,
            payout,
            npc_gold,
        )
        .await;
    }

    /// Sell multiple bag stacks in one all-or-nothing transaction: every line
    /// is validated (ownership, quantity, wishlist, resident carry weight,
    /// resident wallet) before anything is mutated, so a big cart either
    /// completes in full or leaves the player's bag/gold untouched. Mirrors
    /// `sell_item`'s per-unit rules, generalized to N stacks; deals are
    /// single-use per item def, so within one batch only the first line
    /// touching a given item def can redeem one.
    pub async fn sell_items(
        &self,
        player_id: &PlayerId,
        npc_player_id: &PlayerId,
        items: Vec<BagLineItem>,
    ) {
        if items.is_empty() {
            return;
        }
        let def = match self.validate_trader(player_id, npc_player_id).await {
            Ok(def) => def,
            Err(reason) => return self.send_trade_error(player_id, reason).await,
        };
        let npc_name = def.npc_name().to_string();
        let is_resident = matches!(def, TraderDef::Resident(_));
        let rate = match &def {
            TraderDef::Merchant(m) => m.sell_rate_percent,
            TraderDef::Resident(r) => r.wishlist_rate_percent,
        };

        struct Plan {
            instance_id: u64,
            qty: u32,
            item_def_id: String,
            enchant: i32,
            base_price: i64,
            deal_taken: Option<DealEntry>,
            payout: i64,
        }

        // Drop no-op lines before the locks: the plan loop below runs under
        // both write locks, so a client's padding must not reach it.
        let items: Vec<BagLineItem> = items.into_iter().filter(|i| i.qty > 0).collect();
        if items.is_empty() {
            return;
        }

        // One id per unit, reserved before the locks: a resident's received
        // units draw from it, a merchant's buyback entries do.
        let mut next_unit_id = self
            .reserve_instance_ids(items.iter().map(|i| i.qty as u64).sum())
            .await;
        // Only a resident carries what it buys.
        let npc_max_weight = if is_resident {
            Some(self.max_carry_weight(npc_player_id).await)
        } else {
            None
        };

        let mut gold_map = self.player_gold.write().await;
        let mut inventories = self.inventories.write().await;

        if !gold_map.contains_key(player_id) {
            drop(inventories);
            drop(gold_map);
            return self.send_trade_error(player_id, "Player not found").await;
        }

        let mut reserved: HashMap<u64, u32> = HashMap::new();
        let mut plans: Vec<Plan> = Vec::with_capacity(items.len());
        for req in &items {
            let Some(item) = inventories
                .get(player_id)
                .and_then(|inv| inv.bag.iter().find(|i| i.instance_id == req.instance_id))
            else {
                drop(inventories);
                drop(gold_map);
                return self
                    .send_trade_error(player_id, "Item not found in bag")
                    .await;
            };
            let already_reserved = *reserved.get(&req.instance_id).unwrap_or(&0);
            if already_reserved + req.qty > item.quantity {
                drop(inventories);
                drop(gold_map);
                return self
                    .send_trade_error(player_id, "Not enough of that item")
                    .await;
            }
            reserved.insert(req.instance_id, already_reserved + req.qty);

            let item_def_id = item.item_def_id.clone();
            let enchant = item.enchant;
            let Some(base_price) = self.item_defs.get(&item_def_id).and_then(|d| d.base_price)
            else {
                drop(inventories);
                drop(gold_map);
                return self
                    .send_trade_error(player_id, "They will not buy that")
                    .await;
            };
            if let TraderDef::Resident(r) = &def {
                if !r.wants(&item_def_id) {
                    drop(inventories);
                    drop(gold_map);
                    return self
                        .send_trade_error(player_id, "They have no use for that")
                        .await;
                }
            }
            plans.push(Plan {
                instance_id: req.instance_id,
                qty: req.qty,
                item_def_id,
                enchant,
                base_price,
                deal_taken: None,
                payout: 0,
            });
        }
        if let Some(npc_max_weight) = npc_max_weight {
            let Some(npc_inv) = inventories.get(npc_player_id) else {
                drop(inventories);
                drop(gold_map);
                return self.send_trade_error(player_id, "Trader not found").await;
            };
            let added_weight: f32 = plans
                .iter()
                .map(|p| self.item_defs.weight(&p.item_def_id) * p.qty as f32)
                .sum();
            if self.calc_total_weight(npc_inv) + added_weight > npc_max_weight {
                drop(inventories);
                drop(gold_map);
                return self
                    .send_trade_error(player_id, "They cannot carry any more")
                    .await;
            }
        }

        // Redeem any single-use deals now (each is consumed at most once
        // across the whole batch) and price every line; restore them all if
        // the resident wallet check below then fails.
        let mut total_payout: i64 = 0;
        let taken = {
            let line_defs: Vec<&str> = plans.iter().map(|p| p.item_def_id.as_str()).collect();
            self.take_deals(player_id, &npc_name, DealKind::Sell, &line_defs)
                .await
        };
        for (plan, deal) in plans.iter_mut().zip(taken) {
            let deal_units: u32 = if deal.is_some() { 1 } else { 0 };
            let normal_units = plan.qty - deal_units;
            let payout = sell_payout(
                plan.base_price,
                rate,
                deal.as_ref().map_or(0, |d| d.modifier_pct),
            ) * deal_units as i64
                + sell_payout(plan.base_price, rate, 0) * normal_units as i64;
            plan.deal_taken = deal;
            plan.payout = payout;
            total_payout += payout;
        }

        if is_resident {
            let npc_gold = gold_map.get(npc_player_id).copied().unwrap_or(0);
            if npc_gold < total_payout {
                let restore: Vec<(&str, DealEntry)> = plans
                    .iter()
                    .filter_map(|p| Some((p.item_def_id.as_str(), p.deal_taken.clone()?)))
                    .collect();
                self.restore_deals(player_id, &npc_name, DealKind::Sell, &restore)
                    .await;
                drop(inventories);
                drop(gold_map);
                return self
                    .send_trade_error(player_id, "They cannot afford that right now")
                    .await;
            }
        }

        // Every line is now guaranteed to apply cleanly — mutate.
        for plan in &plans {
            let inv = inventories.get_mut(player_id).expect("checked above");
            let idx = inv
                .bag
                .iter()
                .position(|i| i.instance_id == plan.instance_id)
                .expect("checked above");
            if inv.bag[idx].quantity > plan.qty {
                inv.bag[idx].quantity -= plan.qty;
            } else {
                inv.bag.remove(idx);
            }

            if is_resident {
                let stackable = self.item_defs.stackable(&plan.item_def_id);
                let npc_inv = inventories.get_mut(npc_player_id).expect("checked above");
                next_unit_id += stack_into_bag(
                    &mut npc_inv.bag,
                    BagInsert {
                        stackable,
                        item_def_id: &plan.item_def_id,
                        enchant: plan.enchant,
                        first_instance_id: next_unit_id,
                        quantity: plan.qty,
                    },
                );
            }
        }

        let snapshot = inventories.get(player_id).expect("checked above").clone();
        let npc_snapshot = is_resident.then(|| {
            inventories
                .get(npc_player_id)
                .expect("checked above")
                .clone()
        });

        *gold_map.get_mut(player_id).expect("checked above") += total_payout;
        if is_resident {
            *gold_map.get_mut(npc_player_id).expect("checked above") -= total_payout;
        }
        drop(inventories);
        drop(gold_map);

        // Merchants keep no stock, so every sold unit becomes its own buyback
        // entry at the normal per-unit price, recorded in one pass. The unit a
        // deal applied to is intentionally not buyback-able at its haggled
        // price; the normal rate is what repurchasing costs.
        let mut recorded = Vec::new();
        if !is_resident {
            for plan in &plans {
                let unit_price = sell_payout(plan.base_price, rate, 0);
                for _ in 0..plan.qty {
                    recorded.push(BuybackEntry {
                        entry_id: next_unit_id,
                        item_def_id: plan.item_def_id.clone(),
                        enchant: plan.enchant,
                        price: unit_price,
                    });
                    next_unit_id += 1;
                }
            }
        }
        if !recorded.is_empty() {
            let buyback = self.record_buybacks(player_id, &npc_name, recorded).await;
            self.send_direct_message(
                player_id,
                ServerMessage::BuybackUpdated {
                    merchant_player_id: *npc_player_id,
                    buyback,
                },
            )
            .await;
        }

        let player_name = self.player_name_of(player_id).await;
        for plan in &plans {
            if let Some(entry) = &plan.deal_taken {
                info!(
                    target: "deal",
                    "deal redeemed: npc={npc_name} player={player_name} item={} kind=Sell \
                     modifier={} base={} paid={}",
                    plan.item_def_id, entry.modifier_pct, plan.base_price, plan.payout
                );
                self.send_deal_cleared(player_id, npc_player_id, &plan.item_def_id, DealKind::Sell)
                    .await;
            }
            info!(
                "{player_name} sold {}x{} to {npc_name} for {}",
                plan.qty, plan.item_def_id, plan.payout
            );
        }

        self.mark_dirty(player_id).await;
        self.mark_inventory_dirty(player_id).await;
        self.send_direct_message(
            player_id,
            ServerMessage::InventoryUpdated {
                inventory: snapshot,
            },
        )
        .await;
        self.send_gold_update(player_id).await;

        if let Some(npc_snapshot) = npc_snapshot {
            self.mark_dirty(npc_player_id).await;
            self.mark_inventory_dirty(npc_player_id).await;
            self.send_direct_message(
                npc_player_id,
                ServerMessage::InventoryUpdated {
                    inventory: npc_snapshot,
                },
            )
            .await;
            self.send_gold_update(npc_player_id).await;
        }
        let npc_gold = self.get_player_gold(npc_player_id).await;
        for plan in &plans {
            let normal_payout = sell_payout(plan.base_price, rate, 0);
            for unit in 0..plan.qty {
                let unit_payout = if unit == 0 {
                    plan.deal_taken.as_ref().map_or(normal_payout, |deal| {
                        sell_payout(plan.base_price, rate, deal.modifier_pct)
                    })
                } else {
                    normal_payout
                };
                self.send_trade_notice(
                    npc_player_id,
                    player_name.clone(),
                    &plan.item_def_id,
                    DealKind::Sell,
                    unit_payout,
                    npc_gold,
                )
                .await;
            }
        }
    }

    /// The character behind a live player session, if any.
    pub(crate) async fn character_id_of(&self, player_id: &PlayerId) -> Option<i64> {
        let characters = self.player_characters.read().await;
        characters.get(player_id).map(|(char_id, _, _)| *char_id)
    }

    /// Reclaim expired entries globally — an offline character's pair is
    /// never reached by a read path, so nothing else would free it.
    pub async fn tick_buyback_expiry(&self) {
        let now_ms = Self::now_ms();
        let mut buybacks = self.buybacks.write().await;
        buybacks.retain(|_, list| {
            list.retain(|stored| stored.is_live(now_ms));
            !list.is_empty()
        });
    }

    async fn buyback_list(&self, player_id: &PlayerId, npc_name: &str) -> Vec<BuybackEntry> {
        let Some(char_id) = self.character_id_of(player_id).await else {
            return Vec::new();
        };
        let now_ms = Self::now_ms();
        let buybacks = self.buybacks.read().await;
        buybacks
            .get(&(char_id, npc_name.to_string()))
            .map(|list| live_entries(list, now_ms))
            .unwrap_or_default()
    }

    /// Append sold units to the character's buyback list with this merchant,
    /// dropping the oldest entries past `BUYBACK_CAP`. Returns the new list.
    /// One lock acquisition per sale, not per unit.
    async fn record_buybacks(
        &self,
        player_id: &PlayerId,
        npc_name: &str,
        entries: Vec<BuybackEntry>,
    ) -> Vec<BuybackEntry> {
        if entries.is_empty() {
            return Vec::new();
        }
        let Some(char_id) = self.character_id_of(player_id).await else {
            return Vec::new();
        };
        let now_ms = Self::now_ms();
        let mut buybacks = self.buybacks.write().await;
        let list = buybacks.entry((char_id, npc_name.to_string())).or_default();
        // Not the tick's job duplicated: expired entries left in place would
        // eat cap slots and evict live ones below.
        list.retain(|stored| stored.is_live(now_ms));
        list.extend(entries.into_iter().map(|entry| StoredBuyback {
            entry,
            expires_at_ms: now_ms + BUYBACK_TTL_MS,
        }));
        if list.len() > BUYBACK_CAP {
            let excess = list.len() - BUYBACK_CAP;
            list.drain(..excess);
        }
        live_entries(list, now_ms)
    }

    /// Repurchase a unit previously sold to a merchant, at the exact payout
    /// the player received — the round trip is gold-neutral, so no money
    /// pump is possible in either direction. Entries are scoped to the
    /// selling character, live in memory only, and expire after
    /// `BUYBACK_TTL_MS`.
    pub async fn buyback_item(
        &self,
        player_id: &PlayerId,
        npc_player_id: &PlayerId,
        entry_id: u64,
    ) {
        let def = match self.validate_trader(player_id, npc_player_id).await {
            Ok(def) => def,
            Err(reason) => return self.send_trade_error(player_id, reason).await,
        };
        // Residents keep bought units in their real inventory, already
        // repurchasable through `stock`.
        let TraderDef::Merchant(def) = def else {
            return self
                .send_trade_error(player_id, "They have nothing to buy back")
                .await;
        };
        let npc_name = def.npc_name.clone();
        let Some(char_id) = self.character_id_of(player_id).await else {
            return;
        };

        let now_ms = Self::now_ms();
        let entry = {
            let buybacks = self.buybacks.read().await;
            buybacks
                .get(&(char_id, npc_name.clone()))
                .and_then(|list| {
                    list.iter()
                        .find(|s| s.entry.entry_id == entry_id && s.is_live(now_ms))
                })
                .map(|stored| stored.entry.clone())
        };
        let Some(entry) = entry else {
            return self
                .send_trade_error(player_id, "That item is no longer available")
                .await;
        };

        let item_weight = self.item_defs.weight(&entry.item_def_id);
        let max_weight = self.max_carry_weight(player_id).await;

        let (snapshot, buyback) = {
            let mut gold_map = self.player_gold.write().await;
            let Some(gold) = gold_map.get(player_id).copied() else {
                drop(gold_map);
                return;
            };
            if gold < entry.price {
                drop(gold_map);
                return self.send_trade_error(player_id, "Not enough gold").await;
            }

            let mut inventories = self.inventories.write().await;
            if inventories.get(player_id).is_none() {
                drop(inventories);
                drop(gold_map);
                return;
            }
            if self.calc_total_weight(&inventories[player_id]) + item_weight > max_weight {
                drop(inventories);
                drop(gold_map);
                return self.send_trade_error(player_id, "Too heavy to carry").await;
            }

            // Consume the entry under the same critical section as the gold
            // deduction so a concurrent request cannot restore it twice.
            let mut buybacks = self.buybacks.write().await;
            let taken = buybacks
                .get_mut(&(char_id, npc_name.clone()))
                .and_then(|list| {
                    let idx = list
                        .iter()
                        .position(|s| s.entry.entry_id == entry_id && s.is_live(now_ms))?;
                    list.remove(idx);
                    Some(live_entries(list, now_ms))
                });
            let Some(buyback) = taken else {
                drop(buybacks);
                drop(inventories);
                drop(gold_map);
                return self
                    .send_trade_error(player_id, "That item is no longer available")
                    .await;
            };

            let inv = inventories.get_mut(player_id).expect("checked above");
            let stackable = self.item_defs.stackable(&entry.item_def_id);
            stack_into_bag(
                &mut inv.bag,
                BagInsert::one(stackable, &entry.item_def_id, entry.enchant, entry.entry_id),
            );
            let snapshot = inv.clone();
            *gold_map.get_mut(player_id).expect("checked above") -= entry.price;
            (snapshot, buyback)
        };

        let player_name = self.player_name_of(player_id).await;
        info!(
            "{player_name} bought back {} from {npc_name} for {}",
            entry.item_def_id, entry.price
        );
        self.mark_dirty(player_id).await;
        self.mark_inventory_dirty(player_id).await;
        self.send_direct_message(
            player_id,
            ServerMessage::InventoryUpdated {
                inventory: snapshot,
            },
        )
        .await;
        self.send_gold_update(player_id).await;
        self.send_direct_message(
            player_id,
            ServerMessage::BuybackUpdated {
                merchant_player_id: *npc_player_id,
                buyback,
            },
        )
        .await;
        let npc_gold = self.get_player_gold(npc_player_id).await;
        self.send_trade_notice(
            npc_player_id,
            player_name,
            &entry.item_def_id,
            DealKind::Buy,
            entry.price,
            npc_gold,
        )
        .await;
    }

    /// Repurchase multiple buyback entries in one all-or-nothing transaction:
    /// every entry must still be present and unexpired, and the combined
    /// price/weight must fit, before any of them are taken.
    pub async fn buyback_items(
        &self,
        player_id: &PlayerId,
        npc_player_id: &PlayerId,
        entry_ids: Vec<u64>,
    ) {
        if entry_ids.is_empty() {
            return;
        }
        let unique: std::collections::HashSet<u64> = entry_ids.iter().copied().collect();
        if unique.len() != entry_ids.len() {
            return self
                .send_trade_error(player_id, "Duplicate item in request")
                .await;
        }

        let def = match self.validate_trader(player_id, npc_player_id).await {
            Ok(def) => def,
            Err(reason) => return self.send_trade_error(player_id, reason).await,
        };
        let TraderDef::Merchant(def) = def else {
            return self
                .send_trade_error(player_id, "They have nothing to buy back")
                .await;
        };
        let npc_name = def.npc_name.clone();
        let Some(char_id) = self.character_id_of(player_id).await else {
            return;
        };

        let now_ms = Self::now_ms();

        let entries: Vec<BuybackEntry> = {
            let buybacks = self.buybacks.read().await;
            let Some(list) = buybacks.get(&(char_id, npc_name.clone())) else {
                return self
                    .send_trade_error(player_id, "That item is no longer available")
                    .await;
            };
            let mut resolved = Vec::with_capacity(entry_ids.len());
            for id in &entry_ids {
                let Some(stored) = list
                    .iter()
                    .find(|s| s.entry.entry_id == *id && s.is_live(now_ms))
                else {
                    return self
                        .send_trade_error(player_id, "That item is no longer available")
                        .await;
                };
                resolved.push(stored.entry.clone());
            }
            resolved
        };

        let total_price: i64 = entries.iter().map(|e| e.price).sum();
        let added_weight: f32 = entries
            .iter()
            .map(|e| self.item_defs.weight(&e.item_def_id))
            .sum();
        let max_weight = self.max_carry_weight(player_id).await;

        let (snapshot, buyback) = {
            let mut gold_map = self.player_gold.write().await;
            let Some(gold) = gold_map.get(player_id).copied() else {
                drop(gold_map);
                return;
            };
            if gold < total_price {
                drop(gold_map);
                return self.send_trade_error(player_id, "Not enough gold").await;
            }

            let mut inventories = self.inventories.write().await;
            if inventories.get(player_id).is_none() {
                drop(inventories);
                drop(gold_map);
                return;
            }
            if self.calc_total_weight(&inventories[player_id]) + added_weight > max_weight {
                drop(inventories);
                drop(gold_map);
                return self.send_trade_error(player_id, "Too heavy to carry").await;
            }

            // Re-validate every entry is still present before removing any of
            // them, so a concurrent repurchase racing this one can't leave the
            // batch half-applied.
            let mut buybacks = self.buybacks.write().await;
            let Some(list) = buybacks.get_mut(&(char_id, npc_name.clone())) else {
                drop(buybacks);
                drop(inventories);
                drop(gold_map);
                return self
                    .send_trade_error(player_id, "That item is no longer available")
                    .await;
            };
            let mut indices = Vec::with_capacity(entry_ids.len());
            for id in &entry_ids {
                let Some(idx) = list
                    .iter()
                    .position(|s| s.entry.entry_id == *id && s.is_live(now_ms))
                else {
                    drop(buybacks);
                    drop(inventories);
                    drop(gold_map);
                    return self
                        .send_trade_error(player_id, "That item is no longer available")
                        .await;
                };
                indices.push(idx);
            }
            indices.sort_unstable_by(|a, b| b.cmp(a));
            for idx in indices {
                list.remove(idx);
            }
            let buyback = live_entries(list, now_ms);

            let inv = inventories.get_mut(player_id).expect("checked above");
            for entry in &entries {
                let stackable = self.item_defs.stackable(&entry.item_def_id);
                stack_into_bag(
                    &mut inv.bag,
                    BagInsert::one(stackable, &entry.item_def_id, entry.enchant, entry.entry_id),
                );
            }
            let snapshot = inv.clone();
            *gold_map.get_mut(player_id).expect("checked above") -= total_price;
            (snapshot, buyback)
        };

        let player_name = self.player_name_of(player_id).await;
        for entry in &entries {
            info!(
                "{player_name} bought back {} from {npc_name} for {}",
                entry.item_def_id, entry.price
            );
        }
        self.mark_dirty(player_id).await;
        self.mark_inventory_dirty(player_id).await;
        self.send_direct_message(
            player_id,
            ServerMessage::InventoryUpdated {
                inventory: snapshot,
            },
        )
        .await;
        self.send_gold_update(player_id).await;
        self.send_direct_message(
            player_id,
            ServerMessage::BuybackUpdated {
                merchant_player_id: *npc_player_id,
                buyback,
            },
        )
        .await;
        let npc_gold = self.get_player_gold(npc_player_id).await;
        for entry in &entries {
            self.send_trade_notice(
                npc_player_id,
                player_name.clone(),
                &entry.item_def_id,
                DealKind::Buy,
                entry.price,
                npc_gold,
            )
            .await;
        }
    }
}
