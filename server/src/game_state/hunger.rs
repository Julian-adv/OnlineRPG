//! Hunger, food poisoning, campfires and grilling (doc/HUNGER.md).
//!
//! Satiation is private to its owner (like gold), so it lives in its own map
//! keyed by player id rather than on the broadcast `Player`. Official NPCs
//! never get an entry — that absence is the exemption. All decay, band
//! judgement and the poison roll are server-side; clients only render
//! `HungerUpdate`, which is sent on transitions and eating, never per tick.

use onlinerpg_shared::hunger::{
    effective_multipliers, hunger_state, Campfire, CAMPFIRE_GRILL_RADIUS, FOOD_POISONING_MS,
    FOOD_POISONING_PCT, FOOD_REGEN_DURATION_SECS, GRILL_CAST_MS, MOVEMENT_DRAIN_INTERVAL_SECS,
    NORMAL_MIN, POISON_DRAIN_MULT, SPRINT_DRAIN_INTERVAL_SECS,
};
use onlinerpg_shared::{PlayerId, Position, ServerMessage};
use rand::Rng;
use std::time::Duration;
use tokio::time::Instant;

pub(crate) struct HungerData {
    pub satiation: u32,
    pub poisoned_until: Option<Instant>,
    movement_seconds: f32,
    sprint_seconds: f32,
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

fn remaining_ms(until: Option<Instant>, now: Instant) -> u64 {
    until
        .map(|u| u.saturating_duration_since(now).as_millis() as u64)
        .unwrap_or(0)
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

pub(crate) fn hunger_update_msg(satiation: u32, poisoned_ms: u64) -> ServerMessage {
    let (move_mult, attack_mult, carry_mult) = effective_multipliers(satiation, poisoned_ms > 0);
    ServerMessage::HungerUpdate {
        satiation,
        state: hunger_state(satiation),
        move_mult,
        attack_mult,
        carry_mult,
        poisoned_ms,
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
                poisoned_until: None,
                movement_seconds: 0.0,
                sprint_seconds: 0.0,
            },
        );
    }

    pub(crate) async fn forget_hunger(&self, player_id: &PlayerId) {
        self.hunger.write().await.remove(player_id);
        self.food_regeneration.write().await.remove(player_id);
    }

    #[cfg(test)]
    pub(crate) async fn hunger_satiation(&self, player_id: &PlayerId) -> Option<u32> {
        self.hunger.read().await.get(player_id).map(|d| d.satiation)
    }

    /// Push the owner's current hunger snapshot (join, respawn, eating).
    pub(crate) async fn send_hunger_update(&self, player_id: &PlayerId) {
        let msg = {
            let now = Instant::now();
            let hunger = self.hunger.read().await;
            let Some(data) = hunger.get(player_id) else {
                return;
            };
            hunger_update_msg(data.satiation, remaining_ms(data.poisoned_until, now))
        };
        self.send_direct_message(player_id, msg).await;
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
                let poisoned = d.poisoned_until.is_some_and(|u| u > now);
                let (m, _, _) = effective_multipliers(d.satiation, poisoned);
                let sprint_allowed = d.satiation > NORMAL_MIN;
                (m != 1.0 || !sprint_allowed).then_some((*pid, (m, sprint_allowed)))
            })
            .collect()
    }

    async fn hunger_mults(&self, player_id: &PlayerId) -> (f32, f32, f32) {
        let hunger = self.hunger.read().await;
        hunger
            .get(player_id)
            .map(|d| {
                let poisoned = d.poisoned_until.is_some_and(|u| u > Instant::now());
                effective_multipliers(d.satiation, poisoned)
            })
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
            .is_some_and(|d| d.satiation > NORMAL_MIN)
    }

    /// Weak and poisoned players never regen; Hungry players only when
    /// `include_hungry` (alternate regen ticks — the ×0.5).
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
                if d.poisoned_until.is_some_and(|u| u > now) {
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

    /// Clear transient hunger state on respawn without changing satiation.
    pub(crate) async fn reset_hunger_on_respawn(&self, player_id: &PlayerId) {
        let poison_cleared = {
            let mut hunger = self.hunger.write().await;
            match hunger.get_mut(player_id) {
                Some(data) => {
                    data.movement_seconds = 0.0;
                    data.sprint_seconds = 0.0;
                    data.poisoned_until.take().is_some()
                }
                None => false,
            }
        };
        if poison_cleared {
            self.send_hunger_update(player_id).await;
        }
    }

    /// Push poison expiry the moment it lapses. Read-locked bail-out first:
    /// poison is rare, so the 1s sweep must not write-lock 5,000 entries.
    pub async fn tick_hunger_effects(&self) {
        let now = Instant::now();
        {
            let hunger = self.hunger.read().await;
            if !hunger
                .values()
                .any(|d| d.poisoned_until.is_some_and(|u| u <= now))
            {
                return;
            }
        }
        let mut updates: Vec<(PlayerId, ServerMessage)> = Vec::new();
        {
            let mut hunger = self.hunger.write().await;
            for (pid, data) in hunger.iter_mut() {
                if data.poisoned_until.is_some_and(|u| u <= now) {
                    data.poisoned_until = None;
                    updates.push((*pid, hunger_update_msg(data.satiation, 0)));
                }
            }
        }
        for (pid, msg) in updates {
            self.send_direct_message(&pid, msg).await;
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
                let poisoned = data.poisoned_until.is_some_and(|u| u > now);
                let drain_mult = if poisoned { POISON_DRAIN_MULT } else { 1 };
                let old_state = hunger_state(data.satiation);
                let drained_seconds = active_seconds.max(0.0) * drain_mult as f32;

                data.movement_seconds += drained_seconds;
                let movement_points =
                    take_points(&mut data.movement_seconds, MOVEMENT_DRAIN_INTERVAL_SECS);

                if *sprinting && data.satiation > NORMAL_MIN {
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
                let sprint_depleted = *sprinting && data.satiation == NORMAL_MIN;
                if old_state != new_state || sprint_depleted {
                    updates.push((
                        *pid,
                        hunger_update_msg(data.satiation, remaining_ms(data.poisoned_until, now)),
                    ));
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

    /// One kill costs one drain point (×4 poisoned), funneled through the
    /// activity path as one movement interval's worth of effort.
    pub(super) async fn drain_hunger_for_kill(&self, player_id: &PlayerId) {
        self.record_movement_activity(&[(*player_id, MOVEMENT_DRAIN_INTERVAL_SECS, false)])
            .await;
    }

    /// Apply an `Eat`'s satiation/poison side. `force_poison` pins the
    /// raw-fish roll for tests.
    pub(super) async fn apply_eat(
        &self,
        player_id: &PlayerId,
        nutrition: u32,
        raw_fish: bool,
        force_poison: Option<bool>,
    ) -> Option<ServerMessage> {
        // thread_rng is !Send — roll before any await.
        let rolled_poison = raw_fish
            && force_poison
                .unwrap_or_else(|| rand::thread_rng().gen_range(0..100) < FOOD_POISONING_PCT);

        let now = Instant::now();
        let mut hunger = self.hunger.write().await;
        // No hunger entry (official NPC): heal and decrement still proceed.
        let data = hunger.get_mut(player_id)?;
        data.satiation = onlinerpg_shared::hunger::apply_nutrition(data.satiation, nutrition);
        if rolled_poison {
            // Reinfection refreshes the clock rather than stacking.
            data.poisoned_until = Some(now + Duration::from_millis(FOOD_POISONING_MS));
        }
        Some(hunger_update_msg(
            data.satiation,
            remaining_ms(data.poisoned_until, now),
        ))
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
