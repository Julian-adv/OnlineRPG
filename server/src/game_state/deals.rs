//! LLM haggling (economy phase 2): NPC merchants propose price modifiers
//! via `ClientMessage::OfferDeal`; this module is the server-side
//! enforcement the design in `doc/ECONOMY.md` requires. The LLM only
//! *suggests* a modifier — the server clamps it to a CHA-derived price
//! band, charges it against daily budgets, applies a per-player cooldown,
//! and logs every decision (accepted or rejected) with the LLM's reason.

use std::collections::HashMap;

use onlinerpg_shared::messages::{ActiveDeal, DealKind};
use onlinerpg_shared::NPC_SIGHT_RADIUS;
use tracing::info;

use crate::types::{PlayerId, ServerMessage};

use super::combat::reachable_dist_sq;
use super::trading::trader_def_by_name;

/// Band half-width in percentage points at CHA 10.
pub(crate) const DEAL_BASE_HALF_BAND_PCT: i32 = 10;
/// Band half-width floor (very low CHA).
pub(crate) const DEAL_MIN_HALF_BAND_PCT: i32 = 5;
/// Band half-width ceiling (very high CHA). The money-pump invariant below
/// must hold at this value for every merchant's sell rate.
pub(crate) const DEAL_MAX_HALF_BAND_PCT: i32 = 25;
/// Resident (non-merchant) band limits: twice the merchant band, per
/// doc/ECONOMY.md ("가격 밴드: 넓음"). Residents are exempt from the
/// money-pump invariant — their buy/sell item sets are disjoint and the
/// finite wallet caps any loss.
pub(crate) const RESIDENT_MIN_HALF_BAND_PCT: i32 = 10;
pub(crate) const RESIDENT_MAX_HALF_BAND_PCT: i32 = 50;

/// How long a granted deal stays redeemable (real time).
const DEAL_TTL_MS: u64 = 5 * 60 * 1000;
/// Minimum real time between *accepted* offers per (merchant, player).
const DEAL_COOLDOWN_MS: u64 = 30 * 1000;
/// Total discount value a trading NPC (merchant or resident) may grant per
/// game day (smallest unit).
const MERCHANT_DAILY_DISCOUNT_BUDGET: i64 = 10_000;
/// Total discount value a player may receive per game day across merchants.
const PLAYER_DAILY_DISCOUNT_CAP: i64 = 4_000;

/// Money-pump invariant: even at maximum band width, the cheapest possible
/// buy must still cost more than the best possible sell pays out, so no
/// sequence of LLM decisions can make buy→sell profitable.
pub(crate) fn band_invariant_holds(sell_rate_percent: u32, index_min_percent: u32) -> bool {
    let min_buy_pct_of_base =
        i64::from(100 - DEAL_MAX_HALF_BAND_PCT) * i64::from(index_min_percent);
    let max_sell_pct_of_base =
        i64::from(sell_rate_percent) * i64::from(100 + DEAL_MAX_HALF_BAND_PCT);
    min_buy_pct_of_base > max_sell_pct_of_base
}

/// Half-width of the price band for a player with the given CHA. Higher
/// CHA lets the LLM swing prices further in either direction (NetHack's
/// charisma pricing, band edition).
pub(crate) fn deal_half_band_pct(cha: i32) -> i32 {
    (DEAL_BASE_HALF_BAND_PCT + 2 * (cha - 10)).clamp(DEAL_MIN_HALF_BAND_PCT, DEAL_MAX_HALF_BAND_PCT)
}

/// Resident band half-width: twice the merchant width before clamping, so
/// "정말 필요한 물건엔 프리미엄 허용" while CHA still matters.
pub(crate) fn resident_half_band_pct(cha: i32) -> i32 {
    (2 * (DEAL_BASE_HALF_BAND_PCT + 2 * (cha - 10)))
        .clamp(RESIDENT_MIN_HALF_BAND_PCT, RESIDENT_MAX_HALF_BAND_PCT)
}

/// Unit price a player pays when buying at `modifier_pct`.
pub(crate) fn buy_price(base_price: i64, modifier_pct: i32) -> i64 {
    (base_price * (100 + i64::from(modifier_pct)) / 100).max(1)
}

/// Unit payout a player receives when selling at `modifier_pct`.
pub(crate) fn sell_payout(base_price: i64, sell_rate_percent: u32, modifier_pct: i32) -> i64 {
    (base_price * i64::from(sell_rate_percent) * (100 + i64::from(modifier_pct)) / 10_000).max(1)
}

/// A granted deal in words. Which sign favors the player flips with the
/// side, so the wording is derived here rather than from the raw percentage.
pub(crate) fn deal_notice(
    merchant_name: &str,
    item_name: &str,
    kind: DealKind,
    modifier_pct: i32,
    ttl_ms: u64,
) -> String {
    let pct = modifier_pct.abs();
    let mins = (ttl_ms / 60_000).max(1);
    let offer = match kind {
        DealKind::Buy if modifier_pct < 0 => format!("{pct}% off {item_name}"),
        DealKind::Buy => format!("{item_name} at {pct}% above the usual price"),
        DealKind::Sell if modifier_pct > 0 => {
            format!("{pct}% over the usual price for your {item_name}")
        }
        DealKind::Sell => format!("{pct}% under the usual price for your {item_name}"),
    };
    format!("{merchant_name} offers you {offer} ({mins} min).")
}

/// What a deal costs the merchant relative to undiscounted prices. Markups
/// (buy modifier > 0) and lowball sell offers cost nothing.
fn deal_cost(base_price: i64, sell_rate_percent: u32, kind: DealKind, modifier_pct: i32) -> i64 {
    match kind {
        DealKind::Buy => (base_price - buy_price(base_price, modifier_pct)).max(0),
        DealKind::Sell => (sell_payout(base_price, sell_rate_percent, modifier_pct)
            - sell_payout(base_price, sell_rate_percent, 0))
        .max(0),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct DealKey {
    pub player_id: PlayerId,
    pub merchant_name: String,
    pub item_def_id: String,
    pub kind: DealKind,
}

fn deal_key(
    player_id: &PlayerId,
    merchant_name: &str,
    item_def_id: &str,
    kind: DealKind,
) -> DealKey {
    DealKey {
        player_id: *player_id,
        merchant_name: merchant_name.to_string(),
        item_def_id: item_def_id.to_string(),
        kind,
    }
}

/// A granted, not-yet-redeemed deal. Single-use: redeeming one unit
/// consumes it (its cost was already charged to the budgets at grant).
#[derive(Debug, Clone)]
pub(crate) struct DealEntry {
    pub modifier_pct: i32,
    pub expires_at_ms: u64,
}

/// Daily haggling ledgers, reset when the game day rolls over. Keyed by
/// character name (stable across sessions), not session player id.
#[derive(Default)]
pub(crate) struct DealLedgers {
    game_day: i64,
    npc_granted: HashMap<String, i64>,
    player_received: HashMap<String, i64>,
    /// (merchant name, player name) → real-time ms of the last accepted offer.
    last_accepted_offer: HashMap<(String, String), u64>,
}

impl DealLedgers {
    fn roll_over_if_needed(&mut self, game_day: i64) {
        if self.game_day != game_day {
            *self = DealLedgers {
                game_day,
                ..DealLedgers::default()
            };
        }
    }
}

impl super::GameState {
    /// A player's effective CHA for haggling: base attribute plus equipped
    /// `cha+N` effect items, mirroring `effective_guard`.
    async fn effective_cha(&self, player_id: &PlayerId) -> i32 {
        let base = {
            let chars = self.player_characters.read().await;
            chars
                .get(player_id)
                .map(|(_, _, attrs)| i32::from(attrs.cha))
                .unwrap_or(10)
        };
        let bonus = {
            let inventories = self.inventories.read().await;
            inventories
                .get(player_id)
                .map(|inv| self.equipped_bonus(inv, |def| def.cha_bonus()))
                .unwrap_or(0)
        };
        base + bonus
    }

    /// Handle an NPC's `OfferDeal`: validate, clamp to the band, charge
    /// budgets, store the deal, and notify both sides. Every decision is
    /// logged under the `deal` target with the LLM's reason.
    pub async fn offer_deal(
        &self,
        npc_player_id: &PlayerId,
        target_player_id: &PlayerId,
        item_def_id: &str,
        kind: DealKind,
        modifier_pct: i32,
        reason: &str,
    ) {
        let (merchant_name, target_name) = {
            let players = self.players.read().await;
            let Some(npc) = players.get(npc_player_id) else {
                return;
            };
            if !npc.is_official_npc {
                info!(target: "deal", "deal rejected: non-NPC {} sent OfferDeal", npc.name);
                return;
            }
            let npc_name = npc.name.clone();

            let Some(target) = players.get(target_player_id) else {
                return self
                    .reject_deal(
                        npc_player_id,
                        &npc_name,
                        target_player_id,
                        "?",
                        item_def_id,
                        kind,
                        modifier_pct,
                        reason,
                        "target player not found",
                    )
                    .await;
            };
            let target_name = target.name.clone();
            if target.is_official_npc {
                return self
                    .reject_deal(
                        npc_player_id,
                        &npc_name,
                        target_player_id,
                        &target_name,
                        item_def_id,
                        kind,
                        modifier_pct,
                        reason,
                        "deals can only be offered to players",
                    )
                    .await;
            }

            let why = match reachable_dist_sq(
                target.position,
                target.floor_level,
                npc.position,
                npc.floor_level,
            ) {
                None => Some("player is on another floor"),
                Some(d) if d > NPC_SIGHT_RADIUS * NPC_SIGHT_RADIUS => {
                    Some("player is too far away")
                }
                Some(_) => None,
            };
            if let Some(why) = why {
                return self
                    .reject_deal(
                        npc_player_id,
                        &npc_name,
                        target_player_id,
                        &target_name,
                        item_def_id,
                        kind,
                        modifier_pct,
                        reason,
                        why,
                    )
                    .await;
            }
            (npc_name, target_name)
        };

        let reject = |why: &'static str| {
            self.reject_deal(
                npc_player_id,
                &merchant_name,
                target_player_id,
                &target_name,
                item_def_id,
                kind,
                modifier_pct,
                reason,
                why,
            )
        };

        if self.is_npc_asleep(&merchant_name) {
            return reject("you cannot haggle while asleep").await;
        }
        let Some(def) = trader_def_by_name(&merchant_name) else {
            return reject("you have nothing to trade with").await;
        };
        let Some(base_price) = self
            .item_defs
            .get(item_def_id)
            .and_then(|item| item.base_price)
        else {
            return reject("that item has no price").await;
        };

        // Clamp the requested modifier to the target's CHA-derived band.
        let cha = self.effective_cha(target_player_id).await;
        let (rate, half_band) = match def.haggle_params(kind, item_def_id, cha) {
            Ok(params) => params,
            Err(why) => return reject(why).await,
        };
        let applied = modifier_pct.clamp(-half_band, half_band);
        let cost = deal_cost(base_price, rate, kind, applied);

        let now_ms = Self::now_ms();
        let game_day = self.current_game_day();

        // Cooldown + daily budgets, all under the ledger lock.
        {
            let mut ledgers = self.deal_ledgers.write().await;
            ledgers.roll_over_if_needed(game_day);

            let cooldown_key = (merchant_name.clone(), target_name.clone());
            if let Some(last) = ledgers.last_accepted_offer.get(&cooldown_key) {
                if now_ms.saturating_sub(*last) < DEAL_COOLDOWN_MS {
                    drop(ledgers);
                    return reject("haggling cooldown is active for this player").await;
                }
            }
            let npc_granted = ledgers
                .npc_granted
                .entry(merchant_name.clone())
                .or_default();
            if *npc_granted + cost > MERCHANT_DAILY_DISCOUNT_BUDGET {
                drop(ledgers);
                return reject("your discount budget for today is exhausted").await;
            }
            *npc_granted += cost;
            let player_received = ledgers
                .player_received
                .entry(target_name.clone())
                .or_default();
            if *player_received + cost > PLAYER_DAILY_DISCOUNT_CAP {
                // Roll back the merchant ledger we just charged.
                *ledgers
                    .npc_granted
                    .entry(merchant_name.clone())
                    .or_default() -= cost;
                drop(ledgers);
                return reject("this player has reached today's discount limit").await;
            }
            *player_received += cost;
            ledgers.last_accepted_offer.insert(cooldown_key, now_ms);
        }

        let expires_at_ms = now_ms + DEAL_TTL_MS;
        {
            let mut deals = self.deals.write().await;
            deals.retain(|_, entry| entry.expires_at_ms > now_ms);
            deals.insert(
                deal_key(target_player_id, &merchant_name, item_def_id, kind),
                DealEntry {
                    modifier_pct: applied,
                    expires_at_ms,
                },
            );
        }

        info!(
            target: "deal",
            "deal accepted: npc={merchant_name} player={target_name} item={item_def_id} \
             kind={kind:?} requested={modifier_pct} applied={applied} cost={cost} reason={reason:?}"
        );

        self.send_direct_message(
            target_player_id,
            ServerMessage::DealUpdated {
                merchant_player_id: *npc_player_id,
                item_def_id: item_def_id.to_string(),
                kind,
                modifier_pct: applied,
                expires_in_secs: (DEAL_TTL_MS / 1000) as u32,
            },
        )
        .await;
        // The modifier is only drawn inside the trade window, which the player
        // may never open — say it in chat too. A 0% grant is a keepsake
        // unlock, not a price: it has nothing to announce.
        if applied != 0 {
            let item_name = self
                .item_defs
                .get(item_def_id)
                .map_or(item_def_id, |def| def.name.as_str());
            self.send_system_message(
                target_player_id,
                deal_notice(&merchant_name, item_name, kind, applied, DEAL_TTL_MS),
            )
            .await;
        }
        self.send_direct_message(
            npc_player_id,
            ServerMessage::DealResult {
                target_player_id: *target_player_id,
                target_player_name: target_name,
                item_def_id: item_def_id.to_string(),
                kind,
                accepted: true,
                applied_modifier_pct: applied,
                message: if applied == modifier_pct {
                    "deal granted".to_string()
                } else {
                    format!("deal granted, but clamped from {modifier_pct}% to {applied}%")
                },
            },
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn reject_deal(
        &self,
        npc_player_id: &PlayerId,
        merchant_name: &str,
        target_player_id: &PlayerId,
        target_name: &str,
        item_def_id: &str,
        kind: DealKind,
        modifier_pct: i32,
        reason: &str,
        why: &str,
    ) {
        info!(
            target: "deal",
            "deal rejected ({why}): npc={merchant_name} player={target_name} item={item_def_id} \
             kind={kind:?} requested={modifier_pct} reason={reason:?}"
        );
        self.send_direct_message(
            npc_player_id,
            ServerMessage::DealResult {
                target_player_id: *target_player_id,
                target_player_name: target_name.to_string(),
                item_def_id: item_def_id.to_string(),
                kind,
                accepted: false,
                applied_modifier_pct: 0,
                message: format!("offer rejected: {why}"),
            },
        )
        .await;
    }

    /// Remove and return the player's live deal for this item, if any.
    /// Callers must `restore_deal` it if the trade subsequently fails.
    pub(crate) async fn take_deal(
        &self,
        player_id: &PlayerId,
        merchant_name: &str,
        item_def_id: &str,
        kind: DealKind,
    ) -> Option<DealEntry> {
        self.take_deals(player_id, merchant_name, kind, &[item_def_id])
            .await
            .pop()
            .flatten()
    }

    /// Redeem the deals for a batch's lines, in order under one lock: a deal
    /// is single-use, so only the first line touching a def gets one. An
    /// expired entry is consumed rather than restored.
    pub(crate) async fn take_deals(
        &self,
        player_id: &PlayerId,
        merchant_name: &str,
        kind: DealKind,
        item_def_ids: &[&str],
    ) -> Vec<Option<DealEntry>> {
        let now_ms = Self::now_ms();
        let mut deals = self.deals.write().await;
        item_def_ids
            .iter()
            .map(|item_def_id| {
                deals
                    .remove(&deal_key(player_id, merchant_name, item_def_id, kind))
                    .filter(|entry| entry.expires_at_ms > now_ms)
            })
            .collect()
    }

    /// Put back every deal a failed batch took, in one lock acquisition.
    /// A batch that took none never touches the lock.
    pub(crate) async fn restore_deals(
        &self,
        player_id: &PlayerId,
        merchant_name: &str,
        kind: DealKind,
        taken: &[(&str, DealEntry)],
    ) {
        if taken.is_empty() {
            return;
        }
        let mut deals = self.deals.write().await;
        for (item_def_id, entry) in taken {
            deals.insert(
                deal_key(player_id, merchant_name, item_def_id, kind),
                entry.clone(),
            );
        }
    }

    /// Put back a deal taken by `take_deal` after a failed trade.
    /// No-op on `None` so bail-out paths can pass the taken deal directly.
    pub(crate) async fn restore_deal(
        &self,
        player_id: &PlayerId,
        merchant_name: &str,
        item_def_id: &str,
        kind: DealKind,
        entry: Option<DealEntry>,
    ) {
        let Some(entry) = entry else { return };
        self.restore_deals(player_id, merchant_name, kind, &[(item_def_id, entry)])
            .await
    }

    /// Notify a player that a deal was consumed (or cleared).
    pub(crate) async fn send_deal_cleared(
        &self,
        player_id: &PlayerId,
        merchant_player_id: &PlayerId,
        item_def_id: &str,
        kind: DealKind,
    ) {
        self.send_direct_message(
            player_id,
            ServerMessage::DealUpdated {
                merchant_player_id: *merchant_player_id,
                item_def_id: item_def_id.to_string(),
                kind,
                modifier_pct: 0,
                expires_in_secs: 0,
            },
        )
        .await;
    }

    /// Test-only: lift the per-player offer cooldown so budget limits can
    /// be exercised without waiting out real time.
    #[cfg(test)]
    pub(crate) async fn clear_deal_cooldowns_for_test(&self) {
        self.deal_ledgers.write().await.last_accepted_offer.clear();
    }

    #[cfg(test)]
    pub(crate) async fn deal_ledger_state_for_test(
        &self,
        merchant_name: &str,
        player_name: &str,
    ) -> (i64, i64, Option<u64>) {
        let ledgers = self.deal_ledgers.read().await;
        (
            ledgers.npc_granted.get(merchant_name).copied().unwrap_or(0),
            ledgers
                .player_received
                .get(player_name)
                .copied()
                .unwrap_or(0),
            ledgers
                .last_accepted_offer
                .get(&(merchant_name.to_string(), player_name.to_string()))
                .copied(),
        )
    }

    /// The player's live deals with one merchant, for `ShopState`.
    pub(crate) async fn active_deals_for(
        &self,
        player_id: &PlayerId,
        merchant_name: &str,
    ) -> Vec<ActiveDeal> {
        let now_ms = Self::now_ms();
        let deals = self.deals.read().await;
        deals
            .iter()
            .filter(|(key, entry)| {
                key.player_id == *player_id
                    && key.merchant_name == merchant_name
                    && entry.expires_at_ms > now_ms
            })
            .map(|(key, entry)| ActiveDeal {
                item_def_id: key.item_def_id.clone(),
                kind: key.kind,
                modifier_pct: entry.modifier_pct,
                expires_in_secs: ((entry.expires_at_ms - now_ms) / 1000) as u32,
            })
            .collect()
    }
}
