//! Player-to-player trading (doc/TRADE.md): a two-stage lock-then-confirm
//! window between two nearby players. Distinct from `trading.rs`, which is the
//! player↔NPC-merchant shop.

use super::combat::reachable_dist_sq;
use super::consent::{answer_consent, PendingConsent};
use super::inventory::{serialize_inventory, stack_into_bag, BagInsert};
use super::party::MAX_TARGET_NAME_CHARS;
use super::player::build_save_data;
use super::trading::MAX_TRADE_DISTANCE;
use crate::auth::{AuthService, TradeLedgerEntry};
use crate::types::{PlayerId, ServerMessage};
use onlinerpg_shared::inventory::PlayerInventory;
use onlinerpg_shared::messages::{
    PlayerTradeItem, PlayerTradeSide, PlayerTradeSlot, PlayerTradeState, PLAYER_TRADE_IDLE_TTL,
    PLAYER_TRADE_REQUEST_TTL,
};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{error, info};

/// Outstanding trade requests one player may have pending (spam brake), same
/// as parties and friends.
const PENDING_REQUEST_CAP: usize = 5;

/// Distinct slots one offer may name; the bag has no slot cap, so this bounds
/// the per-message work and the broadcast size instead.
const MAX_OFFER_SLOTS: usize = 64;

/// After a customer abandons a stall trade, that stall stays shut to them
/// this long (cancel/reopen spam brake).
const STALL_REOPEN_COOLDOWN: Duration = Duration::from_secs(10);

/// The item that places a tip hat. Offering it while the hat is out would
/// leave the standing hat keyed to its old owner.
const TIP_HAT_ITEM: &str = "tip_hat";

const TIMED_OUT: &str = "The trade timed out.";

pub(crate) struct TradeSide {
    player_id: PlayerId,
    pub(super) items: Vec<PlayerTradeItem>,
    copper: i64,
    locked: bool,
    confirmed: bool,
}

impl TradeSide {
    fn new(player_id: PlayerId) -> Self {
        Self {
            player_id,
            items: Vec::new(),
            copper: 0,
            locked: false,
            confirmed: false,
        }
    }

    /// Wire form with the name left for the caller, which fills it outside
    /// the trades lock.
    fn wire(&self) -> PlayerTradeSide {
        PlayerTradeSide {
            player_id: self.player_id,
            name: String::new(),
            items: self.items.clone(),
            copper: self.copper,
            locked: self.locked,
            confirmed: self.confirmed,
        }
    }
}

pub(crate) struct TradeSession {
    /// The initiator (the customer at a stall). Fixes which side is `a` in
    /// the ledger; it carries no privilege.
    a: TradeSide,
    b: TradeSide,
    pub(super) revision: u32,
    last_activity: Instant,
    /// Set when opened at a stall: range is measured to the table, not the
    /// owner.
    stall_id: Option<u64>,
}

impl TradeSession {
    fn sides_mut(&mut self, player_id: &PlayerId) -> Option<(&mut TradeSide, &mut TradeSide)> {
        if self.a.player_id == *player_id {
            Some((&mut self.a, &mut self.b))
        } else if self.b.player_id == *player_id {
            Some((&mut self.b, &mut self.a))
        } else {
            None
        }
    }

    pub(super) fn side(&self, player_id: &PlayerId) -> Option<&TradeSide> {
        [&self.a, &self.b]
            .into_iter()
            .find(|side| side.player_id == *player_id)
    }

    fn pair(&self) -> (PlayerId, PlayerId) {
        (self.a.player_id, self.b.player_id)
    }

    /// Any offer change invalidates both agreements and moves the revision, so
    /// a confirm already in flight arrives stale and is refused.
    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.a.confirmed = false;
        self.b.confirmed = false;
        self.last_activity = Instant::now();
    }
}

/// All trade state behind one lock. Ranked above `player_gold`/`inventories`
/// and never held while taking `players`.
#[derive(Default)]
pub(crate) struct PlayerTrades {
    next_id: u64,
    pub(super) sessions: HashMap<u64, TradeSession>,
    session_of: HashMap<PlayerId, u64>,
    /// (requester, target) → pending request.
    requests: HashMap<(PlayerId, PlayerId), PendingConsent>,
    /// (customer, owner) → when that stall opens for them again.
    stall_cooldowns: HashMap<(PlayerId, PlayerId), Instant>,
}

impl PlayerTrades {
    /// Drops idle sessions and expired requests; returns the pairs whose
    /// session ended so they can be told. Runs on the server tick only.
    fn sweep(&mut self) -> Vec<(PlayerId, PlayerId)> {
        let now = Instant::now();
        self.requests.retain(|_, request| request.expires_at > now);
        self.stall_cooldowns.retain(|_, until| *until > now);

        let expired: Vec<u64> = self
            .sessions
            .iter()
            .filter(|(_, session)| {
                now.duration_since(session.last_activity) > PLAYER_TRADE_IDLE_TTL
            })
            .map(|(id, _)| *id)
            .collect();
        expired
            .into_iter()
            .filter_map(|id| {
                let session = self.remove(id)?;
                self.cool_stall(&session);
                Some(session.pair())
            })
            .collect()
    }

    fn is_empty(&self) -> bool {
        self.sessions.is_empty() && self.requests.is_empty() && self.stall_cooldowns.is_empty()
    }

    fn remove(&mut self, session_id: u64) -> Option<TradeSession> {
        let session = self.sessions.remove(&session_id)?;
        self.session_of.remove(&session.a.player_id);
        self.session_of.remove(&session.b.player_id);
        Some(session)
    }

    /// Charge the customer's reopen cooldown for a stall session they walked
    /// away from.
    fn cool_stall(&mut self, session: &TradeSession) {
        if session.stall_id.is_some() {
            self.stall_cooldowns
                .insert(session.pair(), Instant::now() + STALL_REOPEN_COOLDOWN);
        }
    }

    /// `player_id` walks out of their session (cancel, disconnect, death). A
    /// customer leaving a stall is charged; the owner leaving is not.
    fn leave(&mut self, player_id: &PlayerId) -> Option<(PlayerId, PlayerId)> {
        let session = self
            .session_id_of(player_id)
            .and_then(|id| self.remove(id))?;
        if session.a.player_id == *player_id {
            self.cool_stall(&session);
        }
        Some(session.pair())
    }

    fn session_id_of(&self, player_id: &PlayerId) -> Option<u64> {
        self.session_of.get(player_id).copied()
    }

    pub(super) fn get(&self, player_id: &PlayerId) -> Option<&TradeSession> {
        self.sessions.get(&self.session_id_of(player_id)?)
    }

    fn open(&mut self, a: PlayerId, b: PlayerId, stall_id: Option<u64>) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.sessions.insert(
            id,
            TradeSession {
                a: TradeSide::new(a),
                b: TradeSide::new(b),
                revision: 1,
                last_activity: Instant::now(),
                stall_id,
            },
        );
        self.session_of.insert(a, id);
        self.session_of.insert(b, id);
        id
    }
}

/// The bag entries backing an offer, resolved against a live inventory. Slots
/// naming the same instance are merged, so the result holds each instance
/// once, in first-seen order. `Err` carries the player-facing reason the
/// offer is not valid.
fn resolve_offer(
    inv: &PlayerInventory,
    slots: &[PlayerTradeSlot],
    item_defs: &crate::item_defs::ItemDefs,
    tip_hat_out: bool,
) -> Result<Vec<PlayerTradeItem>, String> {
    if slots.len() > MAX_OFFER_SLOTS {
        return Err("That's too much for one table.".to_string());
    }
    let slots: Vec<&PlayerTradeSlot> = slots.iter().filter(|slot| slot.quantity > 0).collect();
    let totals =
        super::checked_batch_quantities(slots.iter().map(|slot| (slot.instance_id, slot.quantity)))
            .ok_or_else(|| "You don't have that many.".to_string())?;
    let mut resolved: Vec<PlayerTradeItem> = Vec::with_capacity(totals.len());
    for slot in slots {
        if resolved
            .iter()
            .any(|entry| entry.instance_id == slot.instance_id)
        {
            continue;
        }
        let item = inv
            .bag
            .iter()
            .find(|item| item.instance_id == slot.instance_id)
            .ok_or_else(|| "You no longer have that item.".to_string())?;
        if item_defs.untradeable(&item.item_def_id) {
            let name = item_defs
                .get(&item.item_def_id)
                .map(|def| def.name.as_str())
                .unwrap_or("That item");
            return Err(format!("{name} cannot be traded."));
        }
        if tip_hat_out && item.item_def_id == TIP_HAT_ITEM {
            return Err("Pick your tip hat back up first.".to_string());
        }
        let quantity = totals[&slot.instance_id];
        if quantity > item.quantity {
            return Err("You don't have that many.".to_string());
        }
        resolved.push(PlayerTradeItem {
            instance_id: slot.instance_id,
            item_def_id: item.item_def_id.clone(),
            quantity,
            enchant: item.enchant,
            cape_color: item.cape_color.clone(),
            cape_texture: item.cape_texture.clone(),
        });
    }
    Ok(resolved)
}

/// Total weight of an offer, for the receiving side's carry check.
fn offer_weight(items: &[PlayerTradeItem], item_defs: &crate::item_defs::ItemDefs) -> f32 {
    items
        .iter()
        .map(|item| item_defs.weight(&item.item_def_id) * item.quantity as f32)
        .sum()
}

/// 1g = 100s = 10,000c, matching the client's display split.
fn format_copper(copper: i64) -> String {
    let (gold, rest) = (copper / 10_000, copper % 10_000);
    let (silver, copper) = (rest / 100, rest % 100);
    let mut parts = Vec::new();
    if gold > 0 {
        parts.push(format!("{gold}g"));
    }
    if silver > 0 {
        parts.push(format!("{silver}s"));
    }
    if copper > 0 || parts.is_empty() {
        parts.push(format!("{copper}c"));
    }
    parts.join(" ")
}

/// Ledger form: `(def, qty, ench)` only. `instance_id` is minted per session
/// and would be meaningless in a stored record.
fn ledger_items(items: &[PlayerTradeItem]) -> String {
    let entries: Vec<serde_json::Value> = items
        .iter()
        .map(|item| {
            serde_json::json!({
                "def": item.item_def_id,
                "qty": item.quantity,
                "ench": item.enchant,
            })
        })
        .collect();
    serde_json::Value::Array(entries).to_string()
}

/// Take `quantity` units of `instance_id` out of the bag.
fn take_from_bag(inv: &mut PlayerInventory, instance_id: u64, quantity: u32) -> bool {
    let Some(idx) = inv
        .bag
        .iter()
        .position(|item| item.instance_id == instance_id)
    else {
        return false;
    };
    if inv.bag[idx].quantity < quantity {
        return false;
    }
    if inv.bag[idx].quantity == quantity {
        inv.bag.remove(idx);
    } else {
        inv.bag[idx].quantity -= quantity;
    }
    true
}

fn within_trade_range(dist_sq: Option<f32>) -> bool {
    dist_sq.is_some_and(|dist_sq| dist_sq <= MAX_TRADE_DISTANCE * MAX_TRADE_DISTANCE)
}

impl super::GameState {
    /// Units of `instance_id` this player currently has on a trade table. The
    /// soft reservation: sell, drop, use and equip all subtract this from what
    /// they may touch. One read lock; players outside a trade are the common
    /// case here.
    pub(super) async fn trade_reserved_quantity(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
    ) -> u32 {
        let trades = self.player_trades.read().await;
        trades
            .get(player_id)
            .and_then(|session| session.side(player_id))
            .map(|side| {
                side.items
                    .iter()
                    .filter(|item| item.instance_id == instance_id)
                    .map(|item| item.quantity)
                    .sum()
            })
            .unwrap_or(0)
    }

    /// Whether any of `instance_id` is reserved, with the message to refuse
    /// with. Convenience for the sell/drop/use/equip guards.
    pub(super) async fn reject_if_trade_reserved(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        action: &str,
    ) -> bool {
        if self.trade_reserved_quantity(player_id, instance_id).await == 0 {
            return false;
        }
        self.send_system_message(
            player_id,
            &format!("You can't {action} something you've put on the trade table."),
        )
        .await;
        true
    }

    /// Refuse a def-keyed bulk action while a trade is open. Those paths draw
    /// by definition and lowest enchant first, so they cannot be reconciled
    /// against instance-level reservations.
    pub(super) async fn reject_if_trading(&self, player_id: &PlayerId, action: &str) -> bool {
        if self.live_session(player_id).await.is_none() {
            return false;
        }
        self.send_system_message(
            player_id,
            &format!("Finish your trade before you {action} in bulk."),
        )
        .await;
        true
    }

    async fn announce_ended(
        &self,
        ended: impl IntoIterator<Item = (PlayerId, PlayerId)>,
        completed: bool,
        message: &str,
    ) {
        for (a, b) in ended {
            for player_id in [a, b] {
                self.send_direct_message(
                    &player_id,
                    ServerMessage::PlayerTradeEnded {
                        completed,
                        message: message.to_string(),
                    },
                )
                .await;
            }
        }
    }

    async fn trade_error(&self, player_id: &PlayerId, message: &str) {
        self.send_direct_message(
            player_id,
            ServerMessage::PlayerTradeError {
                message: message.to_string(),
            },
        )
        .await;
    }

    /// Push the whole session to both clients, each seeing itself as `you`.
    async fn broadcast_trade(&self, session_id: u64) {
        let snapshot = {
            let trades = self.player_trades.read().await;
            trades
                .sessions
                .get(&session_id)
                .map(|session| (session.revision, session.a.wire(), session.b.wire()))
        };
        let Some((revision, mut a, mut b)) = snapshot else {
            return;
        };
        {
            let players = self.players.read().await;
            let name_of = |id: &PlayerId| {
                players
                    .get(id)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "?".to_string())
            };
            a.name = name_of(&a.player_id);
            b.name = name_of(&b.player_id);
        }
        let (a_id, b_id) = (a.player_id, b.player_id);
        self.send_direct_message(
            &a_id,
            ServerMessage::PlayerTradeUpdate {
                state: PlayerTradeState {
                    revision,
                    you: a.clone(),
                    them: b.clone(),
                },
            },
        )
        .await;
        self.send_direct_message(
            &b_id,
            ServerMessage::PlayerTradeUpdate {
                state: PlayerTradeState {
                    revision,
                    you: b,
                    them: a,
                },
            },
        )
        .await;
    }

    /// Both players online, on speaking terms with the floor plan, and close
    /// enough. Checked when a request is sent, when it is accepted, and again
    /// at confirm.
    async fn trade_partners_in_range(&self, a: &PlayerId, b: &PlayerId) -> bool {
        let players = self.players.read().await;
        let (Some(pa), Some(pb)) = (players.get(a), players.get(b)) else {
            return false;
        };
        within_trade_range(reachable_dist_sq(
            pa.position,
            pa.floor_level,
            pb.position,
            pb.floor_level,
        ))
    }

    /// The stall still stands, its owner is online, and the customer is at
    /// the table. The owner may move about behind it.
    async fn customer_at_stall(
        &self,
        customer: &PlayerId,
        owner: &PlayerId,
        stall_id: u64,
    ) -> bool {
        let stall = self
            .stalls
            .read()
            .await
            .get(&stall_id)
            .filter(|stall| stall.owner == *owner)
            .map(|stall| (stall.position, stall.floor_level));
        let Some((position, floor_level)) = stall else {
            return false;
        };
        let players = self.players.read().await;
        if !players.contains_key(owner) {
            return false;
        }
        players.get(customer).is_some_and(|p| {
            within_trade_range(reachable_dist_sq(
                p.position,
                p.floor_level,
                position,
                floor_level,
            ))
        })
    }

    async fn session_in_range(&self, a: &PlayerId, b: &PlayerId, stall_id: Option<u64>) -> bool {
        match stall_id {
            Some(stall_id) => self.customer_at_stall(a, b, stall_id).await,
            None => self.trade_partners_in_range(a, b).await,
        }
    }

    /// Ask a named player to trade.
    pub async fn request_player_trade(&self, requester_id: &PlayerId, target_name: &str) {
        if target_name.chars().count() > MAX_TARGET_NAME_CHARS {
            self.trade_request_failed(requester_id, "", "that name is too long.")
                .await;
            return;
        }
        let target_id = self.player_id_by_name(target_name).await;
        let (requester, target) = {
            let players = self.players.read().await;
            let requester = players
                .get(requester_id)
                .map(|p| (p.name.clone(), p.is_official_npc));
            let target = target_id
                .and_then(|id| players.get(&id))
                .map(|p| (p.id, p.name.clone(), p.is_official_npc));
            (requester, target)
        };
        let Some((requester_name, requester_is_npc)) = requester else {
            return;
        };
        if requester_is_npc {
            self.trade_request_failed(
                requester_id,
                target_name,
                "trading is for player travelers.",
            )
            .await;
            return;
        }
        let Some((target_id, target_name, target_is_npc)) = target else {
            self.trade_request_failed(
                requester_id,
                target_name,
                &format!("no one called {target_name} is online."),
            )
            .await;
            return;
        };
        if target_id == *requester_id {
            self.trade_request_failed(requester_id, &target_name, "that's you.")
                .await;
            return;
        }
        // NPCs trade through the shop window, which has price bands, budgets
        // and cooldowns. A free-form item swap would route around all of it.
        if target_is_npc {
            self.trade_request_failed(
                requester_id,
                &target_name,
                &format!("{target_name} trades from their shop, not across a table."),
            )
            .await;
            return;
        }
        if !self.trade_partners_in_range(requester_id, &target_id).await {
            self.trade_request_failed(
                requester_id,
                &target_name,
                &format!("{target_name} is too far away to trade."),
            )
            .await;
            return;
        }

        // Read, not act on: a block may change only the final delivery, never
        // the outcome, or its presence becomes detectable.
        let suppressed = self.has_blocked(&target_id, &requester_name).await;

        enum Outcome {
            Deliver,
            AckOnly,
            Fail(String),
        }
        let outcome = {
            let mut trades = self.player_trades.write().await;
            if trades.session_id_of(requester_id).is_some() {
                Outcome::Fail("you're already trading.".to_string())
            } else if trades.session_id_of(&target_id).is_some() {
                Outcome::Fail(format!("{target_name} is already trading."))
            } else {
                let key = (*requester_id, target_id);
                if trades
                    .requests
                    .get(&key)
                    .is_some_and(PendingConsent::awaiting)
                {
                    Outcome::AckOnly
                } else {
                    let pending = trades
                        .requests
                        .keys()
                        .filter(|(from, _)| from == requester_id)
                        .count();
                    if pending >= PENDING_REQUEST_CAP {
                        Outcome::Fail("you have too many trade requests out.".to_string())
                    } else {
                        trades
                            .requests
                            .insert(key, PendingConsent::new(PLAYER_TRADE_REQUEST_TTL));
                        Outcome::Deliver
                    }
                }
            }
        };

        match outcome {
            Outcome::Fail(reason) => {
                self.trade_request_failed(requester_id, &target_name, &reason)
                    .await;
            }
            Outcome::AckOnly => {
                self.send_system_message(
                    requester_id,
                    &format!("You already asked {target_name} to trade."),
                )
                .await;
            }
            Outcome::Deliver => {
                if !suppressed {
                    self.send_direct_message(
                        &target_id,
                        ServerMessage::PlayerTradeRequested {
                            requester_id: *requester_id,
                            requester_name: requester_name.clone(),
                        },
                    )
                    .await;
                }
                self.send_system_message(requester_id, &format!("You ask {target_name} to trade."))
                    .await;
            }
        }
    }

    /// Open a trade against a laid-out stall. The stall standing there is its
    /// owner's consent, so there is no request step. Refusals go to chat: the
    /// trade window is not open yet to show them in.
    pub async fn request_player_trade_at_stall(&self, player_id: &PlayerId, stall_id: u64) {
        let Some(owner) = self
            .stalls
            .read()
            .await
            .get(&stall_id)
            .map(|stall| stall.owner)
        else {
            self.send_system_message(player_id, "That stall is gone.")
                .await;
            return;
        };
        if owner == *player_id {
            self.send_system_message(player_id, "That's your own stall.")
                .await;
            return;
        }
        let (customer, owner_is_npc) = {
            let players = self.players.read().await;
            (
                players
                    .get(player_id)
                    .map(|p| (p.name.clone(), p.is_official_npc)),
                players.get(&owner).map(|p| p.is_official_npc),
            )
        };
        let Some((customer_name, customer_is_npc)) = customer else {
            return;
        };
        if customer_is_npc {
            self.send_system_message(player_id, "Trading is for player travelers.")
                .await;
            return;
        }
        match owner_is_npc {
            None => {
                self.send_system_message(player_id, "That stall is gone.")
                    .await;
                return;
            }
            // An NPC's stall is their shop front: same intent, but it belongs
            // in the priced shop flow, not the free-form table.
            Some(true) => {
                self.open_shop(player_id, &owner, true).await;
                return;
            }
            Some(false) => {}
        }
        if !self.customer_at_stall(player_id, &owner, stall_id).await {
            self.send_system_message(player_id, "Step up to the stall first.")
                .await;
            return;
        }
        // Indistinguishable from a busy stall, so the block stays invisible.
        let busy = "The stallholder is busy with someone else.";
        if self.has_blocked(&owner, &customer_name).await {
            self.send_system_message(player_id, busy).await;
            return;
        }

        let opened = {
            let mut trades = self.player_trades.write().await;
            let key = (*player_id, owner);
            if trades.session_id_of(player_id).is_some() {
                Err("You're already trading.")
            } else if trades.session_id_of(&owner).is_some() {
                Err(busy)
            } else if trades
                .stall_cooldowns
                .get(&key)
                .is_some_and(|until| *until > Instant::now())
            {
                Err("Give the stallholder a moment.")
            } else {
                Ok(trades.open(*player_id, owner, Some(stall_id)))
            }
        };
        match opened {
            Err(reason) => self.send_system_message(player_id, reason).await,
            Ok(id) => self.broadcast_trade(id).await,
        }
    }

    async fn trade_request_failed(&self, requester_id: &PlayerId, target_name: &str, reason: &str) {
        self.send_direct_message(
            requester_id,
            ServerMessage::PlayerTradeRequestResult {
                target_name: target_name.to_string(),
                accepted: false,
                message: reason.to_string(),
            },
        )
        .await;
    }

    /// Accept or decline a pending trade request.
    pub async fn respond_player_trade(
        &self,
        target_id: &PlayerId,
        requester_id: &PlayerId,
        accept: bool,
    ) {
        let (target_name, requester_name) = {
            let players = self.players.read().await;
            (
                players.get(target_id).map(|p| p.name.clone()),
                players.get(requester_id).map(|p| p.name.clone()),
            )
        };
        let (Some(target_name), Some(requester_name)) = (target_name, requester_name) else {
            return;
        };

        let in_range = self.trade_partners_in_range(requester_id, target_id).await;
        let result = {
            let mut trades = self.player_trades.write().await;
            let key = (*requester_id, *target_id);
            if !answer_consent(&mut trades.requests, key, accept) {
                None
            } else if !accept {
                Some(Err("declined.".to_string()))
            } else if trades.session_id_of(target_id).is_some()
                || trades.session_id_of(requester_id).is_some()
            {
                Some(Err("one of you is already trading.".to_string()))
            } else if !in_range {
                Some(Err("you drifted too far apart.".to_string()))
            } else {
                Some(Ok(trades.open(*requester_id, *target_id, None)))
            }
        };

        match result {
            None => {}
            Some(Err(reason)) => {
                self.send_direct_message(
                    requester_id,
                    ServerMessage::PlayerTradeRequestResult {
                        target_name: target_name.clone(),
                        accepted: false,
                        message: reason,
                    },
                )
                .await;
            }
            Some(Ok(session_id)) => {
                self.send_direct_message(
                    requester_id,
                    ServerMessage::PlayerTradeRequestResult {
                        target_name: target_name.clone(),
                        accepted: true,
                        message: format!("{target_name} agrees to trade."),
                    },
                )
                .await;
                info!("{requester_name} and {target_name} opened a trade");
                self.broadcast_trade(session_id).await;
            }
        }
    }

    /// Replace the sender's whole side of the table.
    pub async fn set_player_trade_offer(
        &self,
        player_id: &PlayerId,
        slots: Vec<PlayerTradeSlot>,
        copper: i64,
    ) {
        if copper < 0 {
            self.trade_error(player_id, "That isn't an amount.").await;
            return;
        }
        let Some(session_id) = self.live_session(player_id).await else {
            return;
        };
        let gold = self
            .player_gold
            .read()
            .await
            .get(player_id)
            .copied()
            .unwrap_or(0);
        if copper > gold {
            self.trade_error(player_id, "You don't have that much.")
                .await;
            return;
        }
        let tip_hat_out = self.tip_hats.read().await.contains_key(player_id);
        let resolved = {
            let inventories = self.inventories.read().await;
            let Some(inv) = inventories.get(player_id) else {
                return;
            };
            resolve_offer(inv, &slots, &self.item_defs, tip_hat_out)
        };
        let resolved = match resolved {
            Ok(items) => items,
            Err(reason) => {
                self.trade_error(player_id, &reason).await;
                return;
            }
        };

        {
            let mut trades = self.player_trades.write().await;
            let Some(session) = trades.sessions.get_mut(&session_id) else {
                return;
            };
            let Some((mine, _)) = session.sides_mut(player_id) else {
                return;
            };
            if mine.locked {
                drop(trades);
                self.trade_error(player_id, "Unlock your offer to change it.")
                    .await;
                return;
            }
            mine.items = resolved;
            mine.copper = copper;
            session.touch();
        }
        self.broadcast_trade(session_id).await;
    }

    /// Freeze the sender's side at `revision`.
    pub async fn lock_player_trade(&self, player_id: &PlayerId, revision: u32) {
        let Some(session_id) = self.live_session(player_id).await else {
            return;
        };
        let stale = {
            let mut trades = self.player_trades.write().await;
            let Some(session) = trades.sessions.get_mut(&session_id) else {
                return;
            };
            if session.revision != revision {
                true
            } else {
                if let Some((mine, _)) = session.sides_mut(player_id) {
                    mine.locked = true;
                }
                session.last_activity = Instant::now();
                false
            }
        };
        if stale {
            self.trade_error(player_id, "The offer changed — look again.")
                .await;
        }
        self.broadcast_trade(session_id).await;
    }

    /// Reopen the sender's side, clearing both confirmations.
    pub async fn unlock_player_trade(&self, player_id: &PlayerId) {
        let Some(session_id) = self.live_session(player_id).await else {
            return;
        };
        {
            let mut trades = self.player_trades.write().await;
            let Some(session) = trades.sessions.get_mut(&session_id) else {
                return;
            };
            if let Some((mine, _)) = session.sides_mut(player_id) {
                mine.locked = false;
            }
            session.touch();
        }
        self.broadcast_trade(session_id).await;
    }

    /// Commit the sender's side; the second confirmation executes the swap.
    pub async fn confirm_player_trade(
        &self,
        player_id: &PlayerId,
        revision: u32,
        auth: &AuthService,
    ) {
        let scope = {
            let trades = self.player_trades.read().await;
            trades.session_id_of(player_id).and_then(|id| {
                let session = trades.sessions.get(&id)?;
                Some((
                    id,
                    session.a.player_id,
                    session.b.player_id,
                    session.stall_id,
                ))
            })
        };
        let Some((session_id, a_id, b_id, stall_id)) = scope else {
            return;
        };
        if !self.session_in_range(&a_id, &b_id, stall_id).await {
            self.end_trade(session_id, false, "You drifted too far apart.")
                .await;
            return;
        }

        enum Step {
            Stale,
            NotLocked,
            Waiting,
            /// Taken off the table here, so a second confirm arriving while
            /// the swap runs finds nothing to run it again on.
            Execute(TradeSession),
        }
        let step = {
            let mut trades = self.player_trades.write().await;
            let Some(session) = trades.sessions.get_mut(&session_id) else {
                return;
            };
            if session.revision != revision {
                Step::Stale
            } else {
                let Some((mine, theirs)) = session.sides_mut(player_id) else {
                    return;
                };
                if !mine.locked {
                    Step::NotLocked
                } else {
                    mine.confirmed = true;
                    let both = theirs.confirmed && theirs.locked;
                    session.last_activity = Instant::now();
                    if !both {
                        Step::Waiting
                    } else {
                        match trades.remove(session_id) {
                            Some(session) => Step::Execute(session),
                            None => return,
                        }
                    }
                }
            }
        };

        match step {
            Step::Stale => {
                self.trade_error(player_id, "The offer changed — look again.")
                    .await;
                self.broadcast_trade(session_id).await;
            }
            Step::NotLocked => {
                self.trade_error(player_id, "Lock your offer first.").await;
            }
            Step::Waiting => self.broadcast_trade(session_id).await,
            Step::Execute(session) => self.execute_trade(session, auth).await,
        }
    }

    async fn live_session(&self, player_id: &PlayerId) -> Option<u64> {
        self.player_trades.read().await.session_id_of(player_id)
    }

    /// Reaper on the server tick: idle sessions, stale requests and spent
    /// cooldowns. Nothing to do is one read lock.
    pub async fn sweep_player_trades(&self) {
        if self.player_trades.read().await.is_empty() {
            return;
        }
        let ended = self.player_trades.write().await.sweep();
        self.announce_ended(ended, false, TIMED_OUT).await;
    }

    /// End a session without swapping anything. An unfinished stall trade
    /// (the customer drifted off the table) charges the reopen cooldown.
    async fn end_trade(&self, session_id: u64, completed: bool, message: &str) {
        let ended = {
            let mut trades = self.player_trades.write().await;
            trades.remove(session_id).map(|session| {
                if !completed {
                    trades.cool_stall(&session);
                }
                session.pair()
            })
        };
        self.announce_ended(ended, completed, message).await;
    }

    /// Player-initiated cancel.
    pub async fn cancel_player_trade(&self, player_id: &PlayerId) {
        let ended = self.player_trades.write().await.leave(player_id);
        self.announce_ended(ended, false, "The trade was called off.")
            .await;
    }

    /// Disconnect and death drop any open session. Silent for the leaver;
    /// their partner is told.
    pub(super) async fn drop_player_trade(&self, player_id: &PlayerId, reason: &str) {
        let ended = {
            let mut trades = self.player_trades.write().await;
            trades
                .requests
                .retain(|(from, to), _| from != player_id && to != player_id);
            trades.leave(player_id)
        };
        self.announce_ended(ended, false, reason).await;
    }

    /// A packed-up stall takes its open trade with it. The owner's doing, so
    /// the customer is not charged.
    pub(super) async fn drop_stall_trade(&self, owner: &PlayerId, stall_id: u64) {
        let ended = {
            let mut trades = self.player_trades.write().await;
            trades
                .session_id_of(owner)
                .filter(|id| trades.sessions[id].stall_id == Some(stall_id))
                .and_then(|id| trades.remove(id))
                .map(|session| session.pair())
        };
        self.announce_ended(ended, false, "The stall was packed up.")
            .await;
    }

    /// The swap itself: validated and mutated under one pair of locks, then
    /// made durable before either side hears that it worked.
    async fn execute_trade(&self, session: TradeSession, auth: &AuthService) {
        let TradeSession { a, b, .. } = session;
        let (a_id, b_id) = (a.player_id, b.player_id);
        let (a_items, b_items, a_copper, b_copper) = (a.items, b.items, a.copper, b.copper);
        let ended = [(a_id, b_id)];

        // Instance ids come from their own lock; reserve before taking gold
        // and inventories so the ranks stay in order.
        let incoming_units = |items: &[PlayerTradeItem]| -> u64 {
            items
                .iter()
                .map(|item| {
                    if self.item_defs.stackable(&item.item_def_id) {
                        1
                    } else {
                        item.quantity as u64
                    }
                })
                .sum()
        };
        let reserved_ids = self
            .reserve_instance_ids(incoming_units(&a_items) + incoming_units(&b_items))
            .await;

        // Reads `player_characters`/`hunger`, which rank below the locks taken
        // next: they have to happen first.
        let a_capacity = self.max_carry_weight(&a_id).await;
        let b_capacity = self.max_carry_weight(&b_id).await;
        let characters = {
            let chars = self.player_characters.read().await;
            match (chars.get(&a_id), chars.get(&b_id)) {
                (Some((a_char, a_xp, _)), Some((b_char, b_xp, _))) => {
                    Some((*a_char, *a_xp, *b_char, *b_xp))
                }
                _ => None,
            }
        };
        let Some((a_character_id, a_xp, b_character_id, b_xp)) = characters else {
            self.announce_ended(ended, false, "The trade could not be completed.")
                .await;
            return;
        };

        // Held from the swap through its commit, so neither the periodic
        // flush nor a logout save can interleave a stale write.
        let persistence = self.persistence_lock.lock().await;
        let outcome = 'swap: {
            let mut gold = self.player_gold.write().await;
            let mut inventories = self.inventories.write().await;

            let a_gold_before = gold.get(&a_id).copied().unwrap_or(0);
            let b_gold_before = gold.get(&b_id).copied().unwrap_or(0);
            if a_gold_before < a_copper || b_gold_before < b_copper {
                break 'swap Err("Someone no longer has the coin they offered.");
            }
            if !inventories.contains_key(&a_id) || !inventories.contains_key(&b_id) {
                break 'swap Err("The trade could not be completed.");
            }
            let tip_hats = self.tip_hats.read().await;
            let revalidate = |id: &PlayerId, items: &[PlayerTradeItem]| -> bool {
                let Some(inv) = inventories.get(id) else {
                    return false;
                };
                let slots: Vec<PlayerTradeSlot> = items
                    .iter()
                    .map(|item| PlayerTradeSlot {
                        instance_id: item.instance_id,
                        quantity: item.quantity,
                    })
                    .collect();
                resolve_offer(inv, &slots, &self.item_defs, tip_hats.contains_key(id)).is_ok_and(
                    |resolved| {
                        resolved.len() == items.len()
                            && resolved.iter().zip(items).all(|(now, then)| {
                                now.item_def_id == then.item_def_id
                                    && now.enchant == then.enchant
                                    && now.cape_color == then.cape_color
                                    && now.cape_texture == then.cape_texture
                                    && now.quantity == then.quantity
                            })
                    },
                )
            };
            if !revalidate(&a_id, &a_items) || !revalidate(&b_id, &b_items) {
                break 'swap Err("Someone no longer has what they offered.");
            }
            drop(tip_hats);
            let a_out = offer_weight(&a_items, &self.item_defs);
            let b_out = offer_weight(&b_items, &self.item_defs);
            let a_after = self.calc_total_weight(&inventories[&a_id]) - a_out + b_out;
            let b_after = self.calc_total_weight(&inventories[&b_id]) - b_out + a_out;
            if a_after > a_capacity || b_after > b_capacity {
                break 'swap Err("Someone can't carry that much.");
            }

            let mut next_id = reserved_ids;
            let mut move_items = |from: &PlayerId, to: &PlayerId, items: &[PlayerTradeItem]| {
                for item in items {
                    let taken = inventories
                        .get_mut(from)
                        .is_some_and(|inv| take_from_bag(inv, item.instance_id, item.quantity));
                    // Cannot happen after revalidation; crediting anyway would mint.
                    if !taken {
                        error!(
                            "trade: {from} lost {} x{} between revalidation and take",
                            item.item_def_id, item.quantity
                        );
                        continue;
                    }
                    if let Some(inv) = inventories.get_mut(to) {
                        next_id += stack_into_bag(
                            &mut inv.bag,
                            BagInsert {
                                stackable: self.item_defs.stackable(&item.item_def_id),
                                item_def_id: &item.item_def_id,
                                enchant: item.enchant,
                                cape_color: item.cape_color.clone(),
                                cape_texture: item.cape_texture.clone(),
                                first_instance_id: next_id,
                                quantity: item.quantity,
                            },
                        );
                    }
                }
            };
            move_items(&a_id, &b_id, &a_items);
            move_items(&b_id, &a_id, &b_items);

            let a_gold_after = a_gold_before - a_copper + b_copper;
            let b_gold_after = b_gold_before - b_copper + a_copper;
            gold.insert(a_id, a_gold_after);
            gold.insert(b_id, b_gold_after);
            Ok((
                a_gold_before,
                a_gold_after,
                b_gold_before,
                b_gold_after,
                serialize_inventory(&inventories[&a_id]),
                serialize_inventory(&inventories[&b_id]),
            ))
        };

        let (a_gold_before, a_gold_after, b_gold_before, b_gold_after, a_rows, b_rows) =
            match outcome {
                Ok(values) => values,
                Err(reason) => {
                    drop(persistence);
                    self.announce_ended(ended, false, reason).await;
                    return;
                }
            };

        let save_data = {
            let players = self.players.read().await;
            let hunger = self.hunger.read().await;
            match (players.get(&a_id), players.get(&b_id)) {
                (Some(a), Some(b)) => Some(vec![
                    build_save_data(
                        a,
                        a_character_id,
                        a_xp,
                        a_gold_after,
                        super::hunger::satiation_for_save(&hunger, &a_id),
                    ),
                    build_save_data(
                        b,
                        b_character_id,
                        b_xp,
                        b_gold_after,
                        super::hunger::satiation_for_save(&hunger, &b_id),
                    ),
                ]),
                _ => None,
            }
        };

        let committed = match save_data {
            Some(characters) => {
                let ledger = TradeLedgerEntry {
                    a_character_id,
                    b_character_id,
                    a_gold_before,
                    a_gold_after,
                    b_gold_before,
                    b_gold_after,
                    a_items: ledger_items(&a_items),
                    b_items: ledger_items(&b_items),
                };
                let inventories = vec![(a_character_id, a_rows), (b_character_id, b_rows)];
                let auth = auth.clone();
                match tokio::task::spawn_blocking(move || {
                    auth.commit_trade(&characters, &inventories, &ledger)
                })
                .await
                {
                    Ok(Ok(())) => true,
                    Ok(Err(err)) => {
                        error!("trade commit failed, left to the periodic flush: {err}");
                        false
                    }
                    Err(err) => {
                        error!("trade commit task failed: {err}");
                        false
                    }
                }
            }
            None => {
                error!("trade between {a_id} and {b_id} swapped but a side is gone; left to the periodic flush");
                false
            }
        };
        // The swap already happened in memory; a missed commit is retried by
        // the periodic flush. Loud, because the ledger row is what went missing.
        if !committed {
            self.mark_dirty(&a_id).await;
            self.mark_dirty(&b_id).await;
            self.mark_inventory_dirty(&a_id).await;
            self.mark_inventory_dirty(&b_id).await;
        }
        drop(persistence);

        self.announce_ended(ended, true, "The trade is done.").await;
        self.report_trade(&a_id, &b_id, &a_items, a_copper, &b_items, b_copper)
            .await;
        self.send_gold_update(&a_id).await;
        self.send_gold_update(&b_id).await;
        self.push_inventory_update(&a_id).await;
        self.push_inventory_update(&b_id).await;
    }

    /// The private chat record each side keeps: their own evidence, and what a
    /// support report gets matched against the ledger with.
    async fn report_trade(
        &self,
        a_id: &PlayerId,
        b_id: &PlayerId,
        a_items: &[PlayerTradeItem],
        a_copper: i64,
        b_items: &[PlayerTradeItem],
        b_copper: i64,
    ) {
        let names = {
            let players = self.players.read().await;
            (
                players.get(a_id).map(|p| p.name.clone()),
                players.get(b_id).map(|p| p.name.clone()),
            )
        };
        let (Some(a_name), Some(b_name)) = names else {
            return;
        };
        let describe = |items: &[PlayerTradeItem], copper: i64| -> String {
            let mut parts: Vec<String> = items
                .iter()
                .map(|item| {
                    let name = self
                        .item_defs
                        .get(&item.item_def_id)
                        .map(|def| def.name.clone())
                        .unwrap_or_else(|| item.item_def_id.clone());
                    let name = if item.enchant != 0 {
                        format!("+{} {name}", item.enchant)
                    } else {
                        name
                    };
                    if item.quantity > 1 {
                        format!("{name} x{}", item.quantity)
                    } else {
                        name
                    }
                })
                .collect();
            if copper > 0 {
                parts.push(format_copper(copper));
            }
            if parts.is_empty() {
                "nothing".to_string()
            } else {
                parts.join(", ")
            }
        };
        let a_gave = describe(a_items, a_copper);
        let b_gave = describe(b_items, b_copper);
        self.send_system_message(
            a_id,
            &format!("Trade with {b_name}: you gave {a_gave}, you received {b_gave}."),
        )
        .await;
        self.send_system_message(
            b_id,
            &format!("Trade with {a_name}: you gave {b_gave}, you received {a_gave}."),
        )
        .await;
    }
}
