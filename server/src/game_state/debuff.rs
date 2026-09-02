//! Timed debuffs on players (doc/DEBUFF.md): food poisoning, bleeding, wet.
//!
//! Active debuffs ride in `HungerData` so one lock answers "what multipliers
//! apply to this player" and official NPCs stay exempt for free. Rolls,
//! expiry and damage ticks are server-side; the owner gets `DebuffUpdate` on
//! change only.

use crate::debuff_defs::{debuff_def, DebuffDef};
use futures_util::{stream, StreamExt};
use onlinerpg_shared::debuff::ActiveDebuffState;
use onlinerpg_shared::{PlayerId, ServerMessage};
use rand::Rng;
use std::time::Duration;
use tokio::time::Instant;

use super::hunger::HungerData;

pub(crate) const WET_DEBUFF_ID: &str = "wet";
/// Drinks within this window count together toward the next stage.
const ALCOHOL_WINDOW: Duration = Duration::from_secs(10 * 60);
/// One unit lifts, two slow, three and beyond stagger — a beer is one unit,
/// a wine two (`items.csv` `alcohol`).
const ALCOHOL_STAGES: [&str; 3] = ["tipsy", "drunk", "wasted"];
/// Water this deep at a step's end soaks the walker: ankle-deep puddles and
/// the shallowest river margins don't count.
const WET_DEPTH_M: f32 = 0.4;
/// A soaked mover is only re-sampled once their remaining time drops below
/// this, so wading costs one terrain sample per player per refresh window
/// instead of one per movement tick.
const WET_REFRESH_BELOW: Duration = Duration::from_secs(300);
/// Movement ticks (200 ms) per water-check round: every mover is checked on
/// one of them, so an unsoaked crowd samples terrain at ~1 Hz each.
const WATER_CHECK_TICKS: u64 = 5;
/// Depth samples in flight at once. Warm tiles make each one microseconds, so
/// this only matters on a cold cache — where it turns a reconnect wave's
/// first-touch tile reads from a serial stall into a bounded fan-out.
const WATER_CHECK_CONCURRENCY: usize = 16;
/// Seconds burned off the soaking per second spent by a lit campfire, drying
/// a full soaking (450 s) in 45 s of sitting.
const CAMPFIRE_DRY_SECS_PER_SEC: u32 = 10;

pub(crate) struct ActiveDebuff {
    pub def: &'static DebuffDef,
    pub until: Instant,
}

impl HungerData {
    /// Inclusive so a sweep landing exactly on `until` still ticks once
    /// before `tick_debuffs` drops it.
    fn active(&self, now: Instant) -> impl Iterator<Item = &ActiveDebuff> {
        self.debuffs.iter().filter(move |d| d.until >= now)
    }

    pub(super) fn debuff_mults(&self, now: Instant) -> (f32, f32, f32) {
        self.active(now).fold((1.0, 1.0, 1.0), |(m, a, c), d| {
            (
                m * d.def.move_mult,
                a * d.def.attack_mult,
                c * d.def.carry_mult,
            )
        })
    }

    pub(super) fn debuff_drain_mult(&self, now: Instant) -> f32 {
        self.active(now).map(|d| d.def.drain_mult).product()
    }

    pub(super) fn debuff_blocks_regen(&self, now: Instant) -> bool {
        self.active(now).any(|d| d.def.blocks_regen)
    }

    /// One entry per id, so the first match is the answer.
    fn remaining(&self, id: &str, now: Instant) -> Duration {
        self.active(now)
            .find(|d| d.def.id == id)
            .map_or(Duration::ZERO, |d| d.until.saturating_duration_since(now))
    }

    fn armor_weight_mult(&self, now: Instant) -> f32 {
        self.active(now).map(|d| d.def.armor_weight_mult).product()
    }

    fn carries(&self, id: &str, now: Instant) -> bool {
        self.active(now).any(|d| d.def.id == id)
    }

    fn debuff_dps(&self, now: Instant) -> u32 {
        self.active(now).map(|d| d.def.dps).sum()
    }

    fn damaging_debuff_ids(&self, now: Instant) -> String {
        self.active(now)
            .filter(|d| d.def.dps > 0)
            .map(|d| d.def.id.as_str())
            .collect::<Vec<_>>()
            .join("+")
    }

    fn debuff_msg(&self, now: Instant) -> ServerMessage {
        ServerMessage::DebuffUpdate {
            debuffs: self
                .active(now)
                .map(|d| ActiveDebuffState {
                    id: d.def.id.clone(),
                    remaining_ms: d.until.saturating_duration_since(now).as_millis() as u64,
                })
                .collect(),
        }
    }

    /// Both owner-only status messages: debuffs also move the multipliers.
    fn status_msgs(&self, now: Instant) -> [ServerMessage; 2] {
        [self.debuff_msg(now), self.hunger_msg(now)]
    }
}

impl super::GameState {
    /// Roll `def_id`'s chance and apply (or refresh) it. `force` pins the
    /// roll for tests. False when it didn't land or the player is exempt.
    pub(crate) async fn inflict_debuff(
        &self,
        player_id: &PlayerId,
        def_id: &str,
        force: Option<bool>,
    ) -> bool {
        let Some(def) = debuff_def(def_id) else {
            return false;
        };
        // thread_rng is !Send — roll before any await.
        if !force.unwrap_or_else(|| rand::thread_rng().gen_range(0..100) < def.chance) {
            return false;
        }
        let now = Instant::now();
        let msgs = {
            let mut hunger = self.hunger.write().await;
            let Some(data) = hunger.get_mut(player_id) else {
                return false;
            };
            let until = now + Duration::from_secs(def.duration_secs);
            match data.debuffs.iter_mut().find(|d| d.def.id == def.id) {
                // A refresh moves no multiplier, so the hunger side stays quiet.
                Some(active) => {
                    active.until = until;
                    vec![data.debuff_msg(now)]
                }
                None => {
                    if let Some(group) = &def.group {
                        data.debuffs.retain(|d| d.def.group.as_ref() != Some(group));
                    }
                    data.debuffs.push(ActiveDebuff { def, until });
                    data.status_msgs(now).into()
                }
            }
        };
        for msg in msgs {
            self.send_direct_message(player_id, msg).await;
        }
        // Mirrored here rather than at the call site: every way of applying
        // `wet` comes through this function, and the flag must not depend on
        // which one did.
        if def.id == WET_DEBUFF_ID {
            self.set_player_wet(player_id, true).await;
        }
        true
    }

    /// `units` of drink just went down: add them to the recent ones and move
    /// the player to that stage (the `alcohol` group makes it exclusive).
    /// No hunger entry (official NPC) means no drinking either.
    pub(crate) async fn apply_alcohol(&self, player_id: &PlayerId, units: u32) {
        if units == 0 {
            return;
        }
        let now = Instant::now();
        let total: u32 = {
            let mut hunger = self.hunger.write().await;
            let Some(data) = hunger.get_mut(player_id) else {
                return;
            };
            data.recent_drinks
                .retain(|(at, _)| now.duration_since(*at) < ALCOHOL_WINDOW);
            data.recent_drinks.push((now, units));
            data.recent_drinks.iter().map(|(_, u)| u).sum()
        };
        let stage = ALCOHOL_STAGES[(total as usize).min(ALCOHOL_STAGES.len()) - 1];
        self.inflict_debuff(player_id, stage, Some(true)).await;
    }

    /// Death drops every debuff.
    pub(crate) async fn clear_debuffs(&self, player_id: &PlayerId) {
        let now = Instant::now();
        let (msgs, was_wet) = {
            let mut hunger = self.hunger.write().await;
            match hunger.get_mut(player_id) {
                Some(data) if !data.debuffs.is_empty() => {
                    let was_wet = data.carries(WET_DEBUFF_ID, now);
                    data.debuffs.clear();
                    (data.status_msgs(now), was_wet)
                }
                _ => return,
            }
        };
        for msg in msgs {
            self.send_direct_message(player_id, msg).await;
        }
        if was_wet {
            self.set_player_wet(player_id, false).await;
        }
    }

    /// 1s sweep: deal each active debuff's dps, then drop the expired ones.
    /// Read-locked bail-out first: only a due tick or expiry may write-lock
    /// the 5,000 entries — a lingering dps-less debuff (food poisoning)
    /// must not.
    pub async fn tick_debuffs(&self) {
        let now = Instant::now();
        {
            let hunger = self.hunger.read().await;
            let due = hunger
                .values()
                .flat_map(|d| d.debuffs.iter())
                .any(|d| d.until <= now || d.def.dps > 0);
            if !due {
                return;
            }
        }
        let mut damage: Vec<(PlayerId, u32, String)> = Vec::new();
        let mut updates: Vec<(PlayerId, [ServerMessage; 2])> = Vec::new();
        let mut dried: Vec<PlayerId> = Vec::new();
        {
            let mut hunger = self.hunger.write().await;
            for (pid, data) in hunger.iter_mut() {
                if data.debuffs.is_empty() {
                    continue;
                }
                let dps = data.debuff_dps(now);
                if dps > 0 {
                    damage.push((*pid, dps, data.damaging_debuff_ids(now)));
                }
                // The raw list, not `active()`: a timer already run down (or
                // pulled into the past by a campfire) is still listed until
                // the retain below drops it, and that drop is the drying.
                if data
                    .debuffs
                    .iter()
                    .any(|d| d.def.id == WET_DEBUFF_ID && d.until <= now)
                {
                    dried.push(*pid);
                }
                let before = data.debuffs.len();
                data.debuffs.retain(|d| d.until > now);
                if data.debuffs.len() != before {
                    updates.push((*pid, data.status_msgs(now)));
                }
            }
        }
        for (pid, msgs) in updates {
            for msg in msgs {
                self.send_direct_message(&pid, msg).await;
            }
        }
        self.clear_wet_flags(dried).await;
        if !damage.is_empty() {
            self.apply_debuff_damage(damage).await;
        }
    }

    /// Movement-coupled soaking: a step ending in water (sea or river)
    /// applies `wet`, and wading refreshes it. A step onto a bridge deck is
    /// exempt by the server's own deck index and Y. Runs after
    /// `tick_player_movement` drops its locks — terrain sampling never
    /// happens under them — and skips anyone already soaked with time to
    /// spare, so only entries and near-expiry refreshes touch a tile.
    pub(super) async fn soak_movers(&self, steps: &[super::ambient_spawn::MoveStep]) {
        if steps.is_empty() {
            return;
        }
        let bucket = self
            .water_check_tick
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % WATER_CHECK_TICKS;
        let now = Instant::now();
        let candidates: Vec<(PlayerId, onlinerpg_shared::Position)> = {
            let hunger = self.hunger.read().await;
            let decks = self.bridge_decks_read();
            steps
                .iter()
                .filter(|s| s.floor_level == 0 && s.player_id.get() % WATER_CHECK_TICKS == bucket)
                // No hunger entry (official NPCs) is the exemption, as
                // everywhere else in this module.
                .filter(|s| {
                    hunger
                        .get(&s.player_id)
                        .is_some_and(|d| d.remaining(WET_DEBUFF_ID, now) < WET_REFRESH_BELOW)
                })
                // The mover's Y is server-derived, so it says whether they
                // are on the deck above or in the river beneath it.
                .filter(|s| {
                    let wx = onlinerpg_shared::wrap_world_x(s.to.x);
                    super::passability::bridge_deck_y(&decks, wx, s.to.z, s.to.y).is_none()
                })
                .map(|s| (s.player_id, s.to))
                .collect()
        };
        // Concurrently, and bounded: a cold tile costs two file reads, and
        // awaiting a tickful of them one by one would stall the movement tick
        // behind the IO that fishing.rs keeps out of ticks for the same reason.
        let soaked: Vec<PlayerId> = stream::iter(candidates)
            .map(|(player_id, at)| async move {
                let wx = onlinerpg_shared::wrap_world_x(at.x);
                let depth = self.water_depth_at(wx, at.z).await.unwrap_or(0.0);
                (depth > WET_DEPTH_M).then_some(player_id)
            })
            .buffer_unordered(WATER_CHECK_CONCURRENCY)
            .filter_map(|hit| async move { hit })
            .collect()
            .await;
        for player_id in soaked {
            self.inflict_debuff(&player_id, WET_DEBUFF_ID, None).await;
        }
    }

    /// The active debuffs' combined armour-weight factor, from the
    /// authoritative hunger map rather than the broadcast mirror. Ranks with
    /// `max_carry_weight`: read it before taking `inventories`, never under it.
    pub(super) async fn armor_weight_mult(&self, player_id: &PlayerId) -> f32 {
        let now = Instant::now();
        self.hunger
            .read()
            .await
            .get(player_id)
            .map_or(1.0, |d| d.armor_weight_mult(now))
    }

    /// Mirror the soaking onto the broadcast `Player` so nearby clients can
    /// draw wet footprints, and tell them when it flips. Compare-and-set like
    /// `set_player_torch`, which is what keeps a wading player's per-step
    /// refresh off the wire.
    async fn set_player_wet(&self, player_id: &PlayerId, wet: bool) {
        let changed = {
            let mut players = self.players.write().await;
            Self::flip_wet(&mut players, player_id, wet)
        };
        if let Some((position, floor_level)) = changed {
            self.announce_wet(player_id, position, floor_level, wet)
                .await;
        }
    }

    /// The batch form: one write lock for a whole sweep's worth of drying.
    async fn clear_wet_flags(&self, player_ids: Vec<PlayerId>) {
        if player_ids.is_empty() {
            return;
        }
        let flipped: Vec<_> = {
            let mut players = self.players.write().await;
            player_ids
                .iter()
                .filter_map(|pid| Some((*pid, Self::flip_wet(&mut players, pid, false)?)))
                .collect()
        };
        for (pid, (position, floor_level)) in flipped {
            self.announce_wet(&pid, position, floor_level, false).await;
        }
    }

    /// Set the broadcast flag; the position to announce from when it moved.
    fn flip_wet(
        players: &mut std::collections::HashMap<PlayerId, onlinerpg_shared::Player>,
        player_id: &PlayerId,
        wet: bool,
    ) -> Option<(onlinerpg_shared::Position, i8)> {
        let player = players.get_mut(player_id)?;
        (player.wet != wet).then(|| {
            player.wet = wet;
            (player.position, player.floor_level)
        })
    }

    /// Skips the owner like `set_player_torch` does: their own trail rides
    /// `DebuffUpdate`, and the client drops this message for itself anyway.
    async fn announce_wet(
        &self,
        player_id: &PlayerId,
        position: onlinerpg_shared::Position,
        floor_level: i8,
        wet: bool,
    ) {
        self.send_direct_message_to_players_within_position(
            &position,
            floor_level,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::PlayerWetToggled {
                player_id: *player_id,
                wet,
            },
            Some(player_id),
        )
        .await;
    }

    /// A lit campfire dries you off: `elapsed` by the fire burns
    /// `CAMPFIRE_DRY_SECS_PER_SEC`× that much off the soaking. Runs on the
    /// 1 s hunger sweep just before `tick_debuffs`, which then drops the
    /// timer it pulled to zero. Both lists are read-locked and bailed on
    /// first: with no fire lit, or nobody wet, this costs two reads.
    pub async fn tick_campfire_drying(&self, elapsed: Duration) {
        let now = Instant::now();
        let wet: Vec<PlayerId> = {
            let hunger = self.hunger.read().await;
            hunger
                .iter()
                .filter(|(_, d)| d.carries(WET_DEBUFF_ID, now))
                .map(|(pid, _)| *pid)
                .collect()
        };
        if wet.is_empty() {
            return;
        }
        let fires: Vec<(onlinerpg_shared::Position, i8)> = {
            let campfires = self.campfires.read().await;
            campfires
                .values()
                .map(|e| (e.campfire.position, e.campfire.floor_level))
                .collect()
        };
        if fires.is_empty() {
            return;
        }
        let radius_sq = onlinerpg_shared::hunger::CAMPFIRE_GRILL_RADIUS.powi(2);
        let drying: Vec<PlayerId> = {
            let players = self.players.read().await;
            wet.into_iter()
                .filter(|pid| {
                    players.get(pid).is_some_and(|p| {
                        fires.iter().any(|(pos, floor)| {
                            *floor == p.floor_level && pos.dist_xz_sq(&p.position) <= radius_sq
                        })
                    })
                })
                .collect()
        };
        if drying.is_empty() {
            return;
        }
        // The countdown the owner is watching just sped up, so it has to be
        // resent; expiry itself is left to `tick_debuffs`.
        let pulled = elapsed * (CAMPFIRE_DRY_SECS_PER_SEC - 1);
        let updates: Vec<(PlayerId, ServerMessage)> = {
            let mut hunger = self.hunger.write().await;
            drying
                .into_iter()
                .filter_map(|pid| {
                    let data = hunger.get_mut(&pid)?;
                    let active = data
                        .debuffs
                        .iter_mut()
                        .find(|d| d.def.id == WET_DEBUFF_ID)?;
                    active.until = active.until.checked_sub(pulled).unwrap_or(now);
                    Some((pid, data.debuff_msg(now)))
                })
                .collect()
        };
        for (pid, msg) in updates {
            self.send_direct_message(&pid, msg).await;
        }
    }

    async fn apply_debuff_damage(&self, damage: Vec<(PlayerId, u32, String)>) {
        struct Hit {
            pid: PlayerId,
            cause: String,
            position: onlinerpg_shared::Position,
            floor: i8,
            health: u32,
            max_health: u32,
        }
        let now = Self::now_ms();
        let hits: Vec<Hit> = {
            let mut players = self.players.write().await;
            damage
                .into_iter()
                .filter_map(|(pid, amount, cause)| {
                    let player = players.get_mut(&pid)?;
                    if !player.is_damageable(now) {
                        return None;
                    }
                    player.health = player.health.saturating_sub(amount);
                    Some(Hit {
                        pid,
                        cause,
                        position: player.position,
                        floor: player.floor_level,
                        health: player.health,
                        max_health: player.max_health,
                    })
                })
                .collect()
        };
        if hits.is_empty() {
            return;
        }
        let ids: Vec<PlayerId> = hits.iter().map(|h| h.pid).collect();
        self.dirty_players.write().await.extend(ids.iter());
        self.party_vitals_dirty.write().await.extend(ids);
        for hit in hits {
            self.send_direct_message_to_players_within_position(
                &hit.position,
                hit.floor,
                super::EVENT_DELIVERY_RADIUS,
                ServerMessage::PlayerHealthUpdate {
                    player_id: hit.pid,
                    health: hit.health,
                    max_health: hit.max_health,
                },
                None,
            )
            .await;
            if hit.health == 0 {
                self.announce_player_death(&hit.pid, hit.position, hit.floor, &hit.cause)
                    .await;
            }
        }
    }
}
