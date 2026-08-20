//! Hunger, campfires and grilling (doc/HUNGER.md).
//!
//! Satiation is private to its owner (like gold), so it lives in its own map
//! keyed by player id rather than on the broadcast `Player`. Official NPCs
//! never get an entry — that absence is the exemption, and it covers the
//! debuffs stored alongside (debuff.rs). All decay and band judgement are
//! server-side; clients only render `HungerUpdate`, which is sent on
//! transitions and eating, never per tick.

use onlinerpg_shared::hunger::{
    can_sprint, effective_multipliers, hunger_state, Campfire, CAMPFIRE_GRILL_RADIUS,
    FOOD_REGEN_DURATION_SECS, GRILL_CAST_MS, MOVEMENT_DRAIN_INTERVAL_SECS, NORMAL_MIN,
    SPRINT_DRAIN_INTERVAL_SECS, SUSTENANCE_DRAIN_MULT,
};
use onlinerpg_shared::inventory::PlayerInventory;
use onlinerpg_shared::{PlayerId, Position, ServerMessage};
use std::time::Duration;
use tokio::time::Instant;

pub(crate) struct HungerData {
    pub satiation: u32,
    pub debuffs: Vec<super::debuff::ActiveDebuff>,
    movement_seconds: f32,
    sprint_seconds: f32,
    /// Equipment's drain factor, cached so the movement tick never has to
    /// reach into `inventories` (`refresh_hunger_gear_drain`).
    gear_drain_mult: f32,
}

pub(crate) struct FoodRegeneration {
    total: u32,
    delivered: u32,
    ticks_elapsed: u8,
}

pub(crate) struct CampfireEntry {
    pub campfire: Campfire,
    pub expires_at: Instant,
}

pub(crate) struct GrillSession {
    pub until: Instant,
    pub campfire_id: u64,
    pub instance_id: u64,
    pub grilled_def_id: String,
}

/// What to persist for a player without a hunger entry (official NPCs).
pub(super) fn satiation_for_save(
    hunger: &std::collections::HashMap<PlayerId, HungerData>,
    player_id: &PlayerId,
) -> u32 {
    hunger
        .get(player_id)
        .map(|d| d.satiation)
        .unwrap_or(onlinerpg_shared::hunger::SATIATION_START)
}

pub(crate) fn hunger_update_msg(satiation: u32, debuff_mults: (f32, f32, f32)) -> ServerMessage {
    let (move_mult, attack_mult, carry_mult) = effective_multipliers(satiation, debuff_mults);
    ServerMessage::HungerUpdate {
        satiation,
        state: hunger_state(satiation),
        move_mult,
        attack_mult,
        carry_mult,
    }
}

impl HungerData {
    pub(super) fn hunger_msg(&self, now: Instant) -> ServerMessage {
        hunger_update_msg(self.satiation, self.debuff_mults(now))
    }
}

impl super::GameState {
    /// Seed hunger at login.
    pub(crate) async fn register_hunger(&self, player_id: &PlayerId, satiation: u32) {
        let mut hunger = self.hunger.write().await;
        hunger.insert(
            *player_id,
            HungerData {
                satiation,
                debuffs: Vec::new(),
                movement_seconds: 0.0,
                sprint_seconds: 0.0,
                gear_drain_mult: 1.0,
            },
        );
    }

    /// Recache the equipment drain factor from the caller's own snapshot.
    /// Every item move lands here while almost none changes the factor, so
    /// it write-locks only on a real change.
    pub(crate) async fn refresh_hunger_gear_drain(
        &self,
        player_id: &PlayerId,
        inv: &PlayerInventory,
    ) {
        let mult = if self.equipped_defs(inv).any(|def| def.has_sustenance()) {
            SUSTENANCE_DRAIN_MULT
        } else {
            1.0
        };
        if self
            .hunger
            .read()
            .await
            .get(player_id)
            .is_none_or(|data| data.gear_drain_mult == mult)
        {
            return;
        }
        if let Some(data) = self.hunger.write().await.get_mut(player_id) {
            data.gear_drain_mult = mult;
        }
    }

    pub(crate) async fn forget_hunger(&self, player_id: &PlayerId) {
        self.hunger.write().await.remove(player_id);
        self.food_regeneration.write().await.remove(player_id);
    }

    #[cfg(test)]
    pub(crate) async fn hunger_satiation(&self, player_id: &PlayerId) -> Option<u32> {
        self.hunger.read().await.get(player_id).map(|d| d.satiation)
    }

    /// Off-baseline movement profiles for the given movers; an absent id means
    /// (1.0, sprint allowed) — the well-baselined majority never enters the map.
    pub(super) async fn hunger_movement_profiles_for(
        &self,
        ids: &[PlayerId],
    ) -> std::collections::HashMap<PlayerId, (f32, bool)> {
        let now = Instant::now();
        let hunger = self.hunger.read().await;
        ids.iter()
            .filter_map(|pid| {
                let d = hunger.get(pid)?;
                let (m, _, _) = effective_multipliers(d.satiation, d.debuff_mults(now));
                let sprint_allowed = can_sprint(d.satiation);
                (m != 1.0 || !sprint_allowed).then_some((*pid, (m, sprint_allowed)))
            })
            .collect()
    }

    async fn hunger_mults(&self, player_id: &PlayerId) -> (f32, f32, f32) {
        let hunger = self.hunger.read().await;
        hunger
            .get(player_id)
            .map(|d| effective_multipliers(d.satiation, d.debuff_mults(Instant::now())))
            .unwrap_or((1.0, 1.0, 1.0))
    }

    pub(super) async fn hunger_carry_mult(&self, player_id: &PlayerId) -> f32 {
        self.hunger_mults(player_id).await.2
    }

    pub(super) async fn hunger_attack_mult(&self, player_id: &PlayerId) -> f32 {
        self.hunger_mults(player_id).await.1
    }

    /// Untracked (NPC) movers never sprint.
    pub(super) async fn hunger_sprint_allowed(&self, player_id: &PlayerId) -> bool {
        let hunger = self.hunger.read().await;
        hunger
            .get(player_id)
            .is_some_and(|d| can_sprint(d.satiation))
    }

    /// Weak and `blocksRegen`-debuffed players never regen; Hungry players
    /// only when `include_hungry` (alternate regen ticks — the ×0.5).
    pub(super) async fn hunger_regen_ready(
        &self,
        candidates: &[PlayerId],
        include_hungry: bool,
    ) -> std::collections::HashSet<PlayerId> {
        let now = Instant::now();
        let hunger = self.hunger.read().await;
        candidates
            .iter()
            .filter(|pid| {
                let Some(d) = hunger.get(pid) else {
                    return true;
                };
                if d.debuff_blocks_regen(now) {
                    return false;
                }
                match hunger_state(d.satiation) {
                    onlinerpg_shared::hunger::HungerState::Weak => false,
                    onlinerpg_shared::hunger::HungerState::Hungry => include_hungry,
                    onlinerpg_shared::hunger::HungerState::Normal => true,
                }
            })
            .copied()
            .collect()
    }

    /// Clear transient hunger state on respawn without changing satiation
    /// (debuffs already went with the death).
    pub(crate) async fn reset_hunger_on_respawn(&self, player_id: &PlayerId) {
        let mut hunger = self.hunger.write().await;
        if let Some(data) = hunger.get_mut(player_id) {
            data.movement_seconds = 0.0;
            data.sprint_seconds = 0.0;
        }
    }

    pub(super) async fn record_movement_activity(&self, activities: &[(PlayerId, f32, bool)]) {
        if activities.is_empty() {
            return;
        }
        fn take_points(accumulated: &mut f32, interval: f32) -> u32 {
            let points = (*accumulated / interval).floor() as u32;
            *accumulated -= points as f32 * interval;
            points
        }
        let now = Instant::now();
        let mut updates = Vec::new();
        let mut dirty = Vec::new();
        {
            let mut hunger = self.hunger.write().await;
            for (pid, active_seconds, sprinting) in activities {
                let Some(data) = hunger.get_mut(pid) else {
                    continue;
                };
                let old_satiation = data.satiation;
                let old_state = hunger_state(old_satiation);
                let drained_seconds =
                    active_seconds.max(0.0) * data.debuff_drain_mult(now) * data.gear_drain_mult;

                data.movement_seconds += drained_seconds;
                let movement_points =
                    take_points(&mut data.movement_seconds, MOVEMENT_DRAIN_INTERVAL_SECS);

                if *sprinting && can_sprint(data.satiation) {
                    data.sprint_seconds += drained_seconds;
                }
                // Sprint drain floors at the Normal minimum: sprinting alone
                // never pushes a player into Hungry.
                let sprint_points =
                    take_points(&mut data.sprint_seconds, SPRINT_DRAIN_INTERVAL_SECS)
                        .min(data.satiation.saturating_sub(NORMAL_MIN));
                data.satiation -= sprint_points;
                data.satiation = data.satiation.saturating_sub(movement_points);

                let new_state = hunger_state(data.satiation);
                // A sprint-gate flip inside the Normal band (satiation
                // reaching exactly NORMAL_MIN) is invisible to the band check
                // but changes whether steps sprint — clients mirror the gate.
                if old_state != new_state || can_sprint(old_satiation) != can_sprint(data.satiation)
                {
                    updates.push((*pid, data.hunger_msg(now)));
                    dirty.push(*pid);
                }
            }
        }
        if !dirty.is_empty() {
            self.dirty_players.write().await.extend(dirty);
        }
        for (pid, msg) in updates {
            self.send_direct_message(&pid, msg).await;
        }
    }

    /// One kill costs one drain point (× debuff drain), funneled through the
    /// activity path as one movement interval's worth of effort.
    pub(super) async fn drain_hunger_for_kill(&self, player_id: &PlayerId) {
        self.record_movement_activity(&[(*player_id, MOVEMENT_DRAIN_INTERVAL_SECS, false)])
            .await;
    }

    /// Apply an `Eat`'s satiation side; the food's `useDebuff` (raw fish →
    /// food poisoning) is rolled by the caller via `inflict_debuff`.
    pub(super) async fn apply_eat(
        &self,
        player_id: &PlayerId,
        nutrition: u32,
    ) -> Option<ServerMessage> {
        let now = Instant::now();
        let mut hunger = self.hunger.write().await;
        // No hunger entry (official NPC): heal and decrement still proceed.
        let data = hunger.get_mut(player_id)?;
        data.satiation = onlinerpg_shared::hunger::apply_nutrition(data.satiation, nutrition);
        Some(data.hunger_msg(now))
    }

    pub(super) async fn start_food_regeneration(&self, player_id: &PlayerId, amount: u32) {
        if amount == 0 {
            return;
        }
        let mut regeneration = self.food_regeneration.write().await;
        let entry = regeneration.entry(*player_id).or_insert(FoodRegeneration {
            total: 0,
            delivered: 0,
            ticks_elapsed: 0,
        });
        // A new meal folds the undelivered remainder into a fresh window.
        entry.total = entry
            .total
            .saturating_sub(entry.delivered)
            .saturating_add(amount);
        entry.delivered = 0;
        entry.ticks_elapsed = 0;
    }

    pub(super) async fn cancel_food_regeneration(&self, player_id: &PlayerId) {
        self.food_regeneration.write().await.remove(player_id);
    }

    pub async fn tick_food_regeneration(&self) {
        if self.food_regeneration.read().await.is_empty() {
            return;
        }
        let portions: Vec<(PlayerId, u32)> = {
            let mut regeneration = self.food_regeneration.write().await;
            let mut portions = Vec::with_capacity(regeneration.len());
            regeneration.retain(|pid, regen| {
                regen.ticks_elapsed += 1;
                let target = regen.total.saturating_mul(u32::from(regen.ticks_elapsed))
                    / u32::from(FOOD_REGEN_DURATION_SECS);
                let amount = target.saturating_sub(regen.delivered);
                regen.delivered = target;
                if amount > 0 {
                    portions.push((*pid, amount));
                }
                regen.ticks_elapsed < FOOD_REGEN_DURATION_SECS
            });
            portions
        };
        if portions.is_empty() {
            return;
        }

        let mut messages = Vec::new();
        {
            let mut players = self.players.write().await;
            for (pid, amount) in portions {
                let Some(player) = players.get_mut(&pid) else {
                    continue;
                };
                if player.health == 0 || player.health >= player.max_health {
                    continue;
                }
                player.health = player.health.saturating_add(amount).min(player.max_health);
                messages.push((
                    pid,
                    player.position,
                    player.floor_level,
                    player.health,
                    player.max_health,
                ));
            }
        }
        let healed: Vec<PlayerId> = messages.iter().map(|(pid, ..)| *pid).collect();
        if !healed.is_empty() {
            self.dirty_players.write().await.extend(healed.iter());
            self.party_vitals_dirty.write().await.extend(healed);
        }
        for (pid, position, floor, health, max_health) in messages {
            self.send_direct_message_to_players_within_position(
                &position,
                floor,
                super::EVENT_DELIVERY_RADIUS,
                ServerMessage::PlayerHealthUpdate {
                    player_id: pid,
                    health,
                    max_health,
                },
                None,
            )
            .await;
        }
    }

    // ---- Campfires ----

    /// The nearest campfire within `radius`, if any.
    pub(super) async fn nearby_campfire(
        &self,
        position: &Position,
        floor_level: i8,
        radius: f32,
    ) -> Option<u64> {
        let campfires = self.campfires.read().await;
        campfires
            .values()
            .filter(|e| e.campfire.floor_level == floor_level)
            .map(|e| (e.campfire.id, e.campfire.position.dist_xz_sq(position)))
            .filter(|(_, d2)| *d2 <= radius * radius)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }

    /// Light a campfire for `duration_ms` and announce it to the area.
    pub(super) async fn spawn_campfire(
        &self,
        position: Position,
        floor_level: i8,
        duration_ms: u64,
    ) -> Campfire {
        let id = self.next_instance_id().await;
        let campfire = Campfire {
            id,
            position,
            floor_level,
        };
        {
            let mut campfires = self.campfires.write().await;
            campfires.insert(
                id,
                CampfireEntry {
                    campfire: campfire.clone(),
                    expires_at: Instant::now() + Duration::from_millis(duration_ms),
                },
            );
        }
        self.send_direct_message_to_players_within_position(
            &campfire.position,
            campfire.floor_level,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::CampfireSpawned {
                campfire: campfire.clone(),
            },
            None,
        )
        .await;
        campfire
    }

    /// Burn out expired campfires; a grill cast on a dying fire loses the race
    /// and is cancelled by the next `tick_grills` verification.
    pub async fn tick_campfires(&self) {
        let expired: Vec<CampfireEntry> = {
            let now = Instant::now();
            let mut campfires = self.campfires.write().await;
            if campfires.is_empty() {
                return;
            }
            let dead: Vec<u64> = campfires
                .iter()
                .filter(|(_, e)| e.expires_at <= now)
                .map(|(id, _)| *id)
                .collect();
            dead.into_iter()
                .filter_map(|id| campfires.remove(&id))
                .collect()
        };
        for entry in expired {
            self.send_direct_message_to_players_within_position(
                &entry.campfire.position,
                entry.campfire.floor_level,
                super::EVENT_DELIVERY_RADIUS,
                ServerMessage::CampfireRemoved {
                    campfire_id: entry.campfire.id,
                },
                None,
            )
            .await;
        }
    }

    // ---- Grilling ----

    /// A raw fish used near a burning campfire grills instead of being
    /// eaten. True when the 3s cast started.
    pub(super) async fn try_start_grill(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        def_id: &str,
        position: &Position,
        floor_level: i8,
    ) -> bool {
        let Some(grilled_def_id) = self
            .item_defs
            .get(def_id)
            .and_then(|d| d.grills_into.clone())
        else {
            return false;
        };
        let Some(campfire_id) = self
            .nearby_campfire(position, floor_level, CAMPFIRE_GRILL_RADIUS)
            .await
        else {
            return false;
        };

        {
            let mut sessions = self.grill_sessions.write().await;
            sessions.insert(
                *player_id,
                GrillSession {
                    until: Instant::now() + Duration::from_millis(GRILL_CAST_MS),
                    campfire_id,
                    instance_id,
                    grilled_def_id,
                },
            );
        }
        self.send_direct_message(player_id, ServerMessage::GrillStarted)
            .await;
        let name = self.item_name(def_id);
        self.send_system_message(player_id, format!("You hold the {name} over the fire..."))
            .await;
        true
    }

    /// Resolve due grill casts (250ms sweep, empty-map early-out). The cast
    /// re-verifies its campfire on completion — movement cancels are handled
    /// eagerly by `cancel_grill_if_active`.
    pub async fn tick_grills(&self) {
        let due: Vec<(PlayerId, GrillSession)> = {
            let mut sessions = self.grill_sessions.write().await;
            if sessions.is_empty() {
                return;
            }
            let now = Instant::now();
            let due_ids: Vec<PlayerId> = sessions
                .iter()
                .filter(|(_, s)| s.until <= now)
                .map(|(pid, _)| *pid)
                .collect();
            due_ids
                .into_iter()
                .filter_map(|pid| sessions.remove(&pid).map(|s| (pid, s)))
                .collect()
        };

        for (player_id, session) in due {
            let fire_alive = {
                let campfires = self.campfires.read().await;
                campfires.contains_key(&session.campfire_id)
            };
            // The raw fish may have been dropped or sold mid-cast.
            let fish_in_bag = {
                let inventories = self.inventories.read().await;
                inventories
                    .get(&player_id)
                    .is_some_and(|inv| inv.bag.iter().any(|i| i.instance_id == session.instance_id))
            };
            if !fire_alive || !fish_in_bag {
                self.send_direct_message(
                    &player_id,
                    ServerMessage::GrillEnded {
                        grilled_item_def_id: None,
                    },
                )
                .await;
                continue;
            }

            self.consume_one_and_sync(&player_id, session.instance_id)
                .await;
            // Grilled fish weighs no more than raw, so this always fits.
            self.give_item(&player_id, &session.grilled_def_id).await;
            let name = self.item_name(&session.grilled_def_id);
            self.send_direct_message(
                &player_id,
                ServerMessage::GrillEnded {
                    grilled_item_def_id: Some(session.grilled_def_id.clone()),
                },
            )
            .await;
            self.send_system_message(&player_id, format!("The {name} sizzles over the fire."))
                .await;
        }
    }

    pub(super) async fn cancel_grill_if_active(&self, player_id: &PlayerId) {
        let cancelled = {
            let mut sessions = self.grill_sessions.write().await;
            sessions.remove(player_id).is_some()
        };
        if cancelled {
            self.send_direct_message(
                player_id,
                ServerMessage::GrillEnded {
                    grilled_item_def_id: None,
                },
            )
            .await;
        }
    }
}
