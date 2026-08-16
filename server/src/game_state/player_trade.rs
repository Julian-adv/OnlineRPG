//! Player-to-player trading (doc/TRADE.md): a two-stage lock-then-confirm
//! window between two nearby players. Distinct from `trading.rs`, which is the
//! player↔NPC-merchant shop.

use super::combat::reachable_dist_sq;
use super::consent::{answer_consent, PendingConsent};
use super::inventory::{serialize_inventory, stack_into_bag, BagInsert};
use super::player::build_save_data;
use crate::auth::{AuthService, TradeLedgerEntry};
use crate::types::{PlayerId, ServerMessage};
use onlinerpg_shared::inventory::PlayerInventory;
use onlinerpg_shared::messages::{
    PlayerTradeItem, PlayerTradeSide, PlayerTradeSlot, PlayerTradeState, PLAYER_TRADE_IDLE_TTL,
    PLAYER_TRADE_REQUEST_TTL,
};
use std::collections::HashMap;
use std::time::Instant;
use tracing::{error, info};

/// Matches the NPC shop's reach so both kinds of trade feel the same.
const MAX_TRADE_DISTANCE: f32 = 6.0;

/// Outstanding trade requests one player may have pending (spam brake), same
/// as parties and friends.
const PENDING_REQUEST_CAP: usize = 5;

/// Mirrors auth's character-name cap, like `party::MAX_TARGET_NAME_CHARS`.
const MAX_TARGET_NAME_CHARS: usize = 32;

/// The item that places a tip hat. Offering it while the hat is out would
/// leave the standing hat keyed to its old owner.
const TIP_HAT_ITEM: &str = "tip_hat";

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

    fn wire(&self, name: String) -> PlayerTradeSide {
        PlayerTradeSide {
            player_id: self.player_id,
            name,
            items: self.items.clone(),
            copper: self.copper,
            locked: self.locked,
            confirmed: self.confirmed,
        }
    }
}

pub(crate) struct TradeSession {
    /// The initiator. Fixes which side is `a` in the ledger; it carries no
    /// privilege.
    a: TradeSide,
    b: TradeSide,
    pub(super) revision: u32,
    last_activity: Instant,
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

    fn other(&self, player_id: &PlayerId) -> PlayerId {
        if self.a.player_id == *player_id {
            self.b.player_id
        } else {
            self.a.player_id
        }
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

/// All trade state behind one lock. Ranked above `player_gold`/`inventories`:
/// a commit takes this first and never the other way round.
#[derive(Default)]
pub(crate) struct PlayerTrades {
    next_id: u64,
    pub(super) sessions: HashMap<u64, TradeSession>,
    session_of: HashMap<PlayerId, u64>,
    /// (requester, target) → pending request, swept lazily like party invites.
    requests: HashMap<(PlayerId, PlayerId), PendingConsent>,
}

impl PlayerTrades {
    /// Drops idle sessions and expired requests. Called at the head of every
    /// trade action and from the reservation lookup, so an abandoned session
    /// cannot hold its owner's items hostage indefinitely.
    fn sweep(&mut self) -> Vec<(PlayerId, PlayerId)> {
        let now = Instant::now();
        self.requests.retain(|_, request| request.expires_at > now);

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
            .filter_map(|id| self.remove(id))
            .collect()
    }

    fn remove(&mut self, session_id: u64) -> Option<(PlayerId, PlayerId)> {
        let session = self.sessions.remove(&session_id)?;
        self.session_of.remove(&session.a.player_id);
        self.session_of.remove(&session.b.player_id);
        Some((session.a.player_id, session.b.player_id))
    }

    fn session_id_of(&self, player_id: &PlayerId) -> Option<u64> {
        self.session_of.get(player_id).copied()
    }

    pub(super) fn get(&self, player_id: &PlayerId) -> Option<&TradeSession> {
        self.sessions.get(&self.session_id_of(player_id)?)
    }

    fn open(&mut self, a: PlayerId, b: PlayerId) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.sessions.insert(
            id,
            TradeSession {
                a: TradeSide::new(a),
                b: TradeSide::new(b),
                revision: 1,
                last_activity: Instant::now(),
            },
        );
        self.session_of.insert(a, id);
        self.session_of.insert(b, id);
        id
    }
}

/// The bag entries backing an offer, resolved against a live inventory.
/// `Err` carries the player-facing reason the offer is not valid.
fn resolve_offer(
    inv: &PlayerInventory,
    slots: &[PlayerTradeSlot],
    item_defs: &crate::item_defs::ItemDefs,
    tip_hat_out: bool,
) -> Result<Vec<PlayerTradeItem>, String> {
    let mut resolved: Vec<PlayerTradeItem> = Vec::with_capacity(slots.len());
    for slot in slots {
        if slot.quantity == 0 {
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
        let already: u32 = resolved
            .iter()
            .filter(|entry| entry.instance_id == slot.instance_id)
            .map(|entry| entry.quantity)
            .sum();
        if already + slot.quantity > item.quantity {
            return Err("You don't have that many.".to_string());
        }
        resolved.push(PlayerTradeItem {
            instance_id: slot.instance_id,
            item_def_id: item.item_def_id.clone(),
            quantity: slot.quantity,
            enchant: item.enchant,
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

impl super::GameState {
    /// Units of `instance_id` this player currently has on a trade table. The
    /// soft reservation: sell, drop, use and equip all subtract this from what
    /// they may touch.
    pub(super) async fn trade_reserved_quantity(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
    ) -> u32 {
        let ended = {
            let mut trades = self.player_trades.write().await;
            let ended = trades.sweep();
            if ended.is_empty() {
                return trades
                    .get(player_id)
                    .and_then(|session| session.side(player_id))
                    .map(|side| {
                        side.items
                            .iter()
                            .filter(|item| item.instance_id == instance_id)
                            .map(|item| item.quantity)
                            .sum()
                    })
                    .unwrap_or(0);
            }
            ended
        };
        self.announce_ended(&ended, false, "The trade timed out.")
            .await;
        Box::pin(self.trade_reserved_quantity(player_id, instance_id)).await
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
    /// against instance-level reservations — and "finish your trade first" is
    /// a reason a player can act on.
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

    async fn announce_ended(&self, ended: &[(PlayerId, PlayerId)], completed: bool, message: &str) {
        for (a, b) in ended {
            for player_id in [a, b] {
                self.send_direct_message(
                    player_id,
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
        let payload = {
            let trades = self.player_trades.read().await;
            let Some(session) = trades.sessions.get(&session_id) else {
                return;
            };
            let names = self.players.read().await;
            let name_of = |id: &PlayerId| {
                names
                    .get(id)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "?".to_string())
            };
            let a = session.a.wire(name_of(&session.a.player_id));
            let b = session.b.wire(name_of(&session.b.player_id));
            (session.revision, a, b)
        };
        let (revision, a, b) = payload;
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
    /// at every step of the session.
    async fn trade_partners_in_range(&self, a: &PlayerId, b: &PlayerId) -> bool {
        let players = self.players.read().await;
        let (Some(pa), Some(pb)) = (players.get(a), players.get(b)) else {
            return false;
        };
        reachable_dist_sq(pa.position, pa.floor_level, pb.position, pb.floor_level)
            .is_some_and(|dist_sq| dist_sq <= MAX_TRADE_DISTANCE * MAX_TRADE_DISTANCE)
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
        let suppressed = {
            let blocked = self.blocked_names.read().await;
            blocked
                .get(&target_id)
                .is_some_and(|names| names.contains(&requester_name))
        };

        enum Outcome {
            Deliver,
            AckOnly,
            Fail(String),
        }
        let outcome = {
            let players = self.players.read().await;
            let mut trades = self.player_trades.write().await;
            trades.sweep();
            if trades.session_id_of(requester_id).is_some() {
                Outcome::Fail("you're already trading.".to_string())
            } else if trades.session_id_of(&target_id).is_some() {
                Outcome::Fail(format!("{target_name} is already trading."))
            } else if !players.contains_key(&target_id) {
                Outcome::Fail(format!("no one called {target_name} is online."))
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
    /// owner's consent, so there is no request step.
    pub async fn request_player_trade_at_stall(&self, player_id: &PlayerId, stall_id: u64) {
        let Some(owner) = self
            .stalls
            .read()
            .await
            .get(&stall_id)
            .map(|stall| stall.owner)
        else {
            self.trade_error(player_id, "That stall is gone.").await;
            return;
        };
        if owner == *player_id {
            self.trade_error(player_id, "That's your own stall.").await;
            return;
        }
        let owner_is_npc = {
            let players = self.players.read().await;
            players.get(&owner).map(|p| p.is_official_npc)
        };
        match owner_is_npc {
            None => {
                self.trade_error(player_id, "That stall is gone.").await;
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
        if !self.trade_partners_in_range(player_id, &owner).await {
            self.trade_error(player_id, "Step up to the stall first.")
                .await;
            return;
        }

        let session_id = {
            let mut trades = self.player_trades.write().await;
            trades.sweep();
            if trades.session_id_of(player_id).is_some() {
                None
            } else if trades.session_id_of(&owner).is_some() {
                Some(Err("The stallholder is busy with someone else."))
            } else {
                Some(Ok(trades.open(*player_id, owner)))
            }
        };
        match session_id {
            None => {
                self.trade_error(player_id, "You're already trading.").await;
            }
            Some(Err(reason)) => self.trade_error(player_id, reason).await,
            Some(Ok(id)) => self.broadcast_trade(id).await,
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
            trades.sweep();
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
                Some(Ok(trades.open(*requester_id, *target_id)))
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
        let Some(session_id) = self.live_session(player_id).await else {
            return;
        };
        let partner = {
            let trades = self.player_trades.read().await;
            match trades.sessions.get(&session_id) {
                Some(session) => session.other(player_id),
                None => return,
            }
        };
        if !self.trade_partners_in_range(player_id, &partner).await {
            self.end_trade(session_id, false, "You drifted too far apart.")
                .await;
            return;
        }

        enum Step {
            Stale,
            NotLocked,
            Waiting,
            Execute,
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
                    if both {
                        Step::Execute
                    } else {
                        Step::Waiting
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
            Step::Execute => self.execute_trade(session_id, auth).await,
        }
    }

    /// The session id for a player, or `None` after telling them there isn't
    /// one. Sweeps first, so an idle session is reaped rather than used.
    async fn live_session(&self, player_id: &PlayerId) -> Option<u64> {
        let (ended, session_id) = {
            let mut trades = self.player_trades.write().await;
            let ended = trades.sweep();
            (ended, trades.session_id_of(player_id))
        };
        if !ended.is_empty() {
            self.announce_ended(&ended, false, "The trade timed out.")
                .await;
        }
        session_id
    }

    /// End a session without swapping anything.
    async fn end_trade(&self, session_id: u64, completed: bool, message: &str) {
        let ended = {
            let mut trades = self.player_trades.write().await;
            trades.remove(session_id).into_iter().collect::<Vec<_>>()
        };
        self.announce_ended(&ended, completed, message).await;
    }

    /// Player-initiated cancel.
    pub async fn cancel_player_trade(&self, player_id: &PlayerId) {
        let Some(session_id) = self.live_session(player_id).await else {
            return;
        };
        self.end_trade(session_id, false, "The trade was called off.")
            .await;
    }

    /// Disconnect, death and floor changes all drop any open session. Silent
    /// for the leaver; their partner is told.
    pub(super) async fn drop_player_trade(&self, player_id: &PlayerId, reason: &str) {
        let ended = {
            let mut trades = self.player_trades.write().await;
            trades
                .requests
                .retain(|(from, to), _| from != player_id && to != player_id);
            match trades.session_id_of(player_id) {
                Some(id) => trades.remove(id).into_iter().collect::<Vec<_>>(),
                None => Vec::new(),
            }
        };
        self.announce_ended(&ended, false, reason).await;
    }

    /// The swap itself: validated and mutated under one pair of locks, then
    /// made durable before either side hears that it worked.
    async fn execute_trade(&self, session_id: u64, auth: &AuthService) {
        let Some((a_id, b_id, a_items, b_items, a_copper, b_copper)) = ({
            let trades = self.player_trades.read().await;
            trades.sessions.get(&session_id).map(|session| {
                (
                    session.a.player_id,
                    session.b.player_id,
                    session.a.items.clone(),
                    session.b.items.clone(),
                    session.a.copper,
                    session.b.copper,
                )
            })
        }) else {
            return;
        };

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
            self.end_trade(session_id, false, "The trade could not be completed.")
                .await;
            return;
        };

        let outcome = {
            let mut gold = self.player_gold.write().await;
            let mut inventories = self.inventories.write().await;

            let a_gold_before = gold.get(&a_id).copied().unwrap_or(0);
            let b_gold_before = gold.get(&b_id).copied().unwrap_or(0);
            if a_gold_before < a_copper || b_gold_before < b_copper {
                Err("Someone no longer has the coin they offered.")
            } else if !inventories.contains_key(&a_id) || !inventories.contains_key(&b_id) {
                Err("The trade could not be completed.")
            } else {
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
                    resolve_offer(inv, &slots, &self.item_defs, tip_hats.contains_key(id))
                        .is_ok_and(|resolved| {
                            resolved.len() == items.len()
                                && resolved.iter().zip(items).all(|(now, then)| {
                                    now.item_def_id == then.item_def_id
                                        && now.enchant == then.enchant
                                        && now.quantity == then.quantity
                                })
                        })
                };
                if !revalidate(&a_id, &a_items) || !revalidate(&b_id, &b_items) {
                    Err("Someone no longer has what they offered.")
                } else {
                    drop(tip_hats);
                    let a_out = offer_weight(&a_items, &self.item_defs);
                    let b_out = offer_weight(&b_items, &self.item_defs);
                    let a_after = self.calc_total_weight(&inventories[&a_id]) - a_out + b_out;
                    let b_after = self.calc_total_weight(&inventories[&b_id]) - b_out + a_out;
                    if a_after > a_capacity || b_after > b_capacity {
                        Err("Someone can't carry that much.")
                    } else {
                        let mut next_id = reserved_ids;
                        let mut move_items =
                            |from: &PlayerId, to: &PlayerId, items: &[PlayerTradeItem]| {
                                for item in items {
                                    if let Some(inv) = inventories.get_mut(from) {
                                        take_from_bag(inv, item.instance_id, item.quantity);
                                    }
                                    if let Some(inv) = inventories.get_mut(to) {
                                        next_id += stack_into_bag(
                                            &mut inv.bag,
                                            BagInsert {
                                                stackable: self
                                                    .item_defs
                                                    .stackable(&item.item_def_id),
                                                item_def_id: &item.item_def_id,
                                                enchant: item.enchant,
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
                    }
                }
            }
        };

        let (a_gold_before, a_gold_after, b_gold_before, b_gold_after, a_rows, b_rows) =
            match outcome {
                Ok(values) => values,
                Err(reason) => {
                    self.end_trade(session_id, false, reason).await;
                    return;
                }
            };

        self.mark_dirty(&a_id).await;
        self.mark_dirty(&b_id).await;
        self.mark_inventory_dirty(&a_id).await;
        self.mark_inventory_dirty(&b_id).await;

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

        if let Some(characters) = save_data {
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
            let saved = tokio::task::spawn_blocking(move || {
                auth.commit_trade(&characters, &inventories, &ledger)
            })
            .await;
            match saved {
                Ok(Ok(())) => {}
                // The swap already happened in memory and both sides are
                // marked dirty, so the periodic flush retries it. Loud,
                // because the ledger row is what went missing.
                Ok(Err(err)) => error!("trade commit failed, left to the periodic flush: {err}"),
                Err(err) => error!("trade commit task failed: {err}"),
            }
        }

        self.end_trade(session_id, true, "The trade is done.").await;
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
