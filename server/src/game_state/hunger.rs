//! Hunger, food poisoning, campfires and grilling (doc/HUNGER.md).
//!
//! Satiation is private to its owner (like gold), so it lives in its own map
//! keyed by player id rather than on the broadcast `Player`. Official NPCs
//! never get an entry — that absence is the exemption. All decay, band
//! judgement and the poison roll are server-side; clients only render
//! `HungerUpdate`, which is sent on transitions and eating, never per tick.

use onlinerpg_shared::hunger::{
    effective_multipliers, hunger_state, Campfire, CAMPFIRE_DURATION_MS, CAMPFIRE_GRILL_RADIUS,
    FOOD_POISONING_MS, FOOD_POISONING_PCT, GRILL_CAST_MS, POISON_DRAIN_MULT, SATIATION_RESPAWN,
};
use onlinerpg_shared::{PlayerId, Position, ServerMessage};
use rand::Rng;
use std::time::Duration;
use tokio::time::Instant;

pub(crate) struct HungerData {
    pub satiation: u32,
    pub poisoned_until: Option<Instant>,
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

/// The satiation/poison side of one meal, decoded by `use_eat_item`.
pub(super) enum EatOutcome {
    /// No hunger entry (official NPC): heal and decrement still proceed.
    Untracked,
    TooStuffed,
    Fed(ServerMessage),
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
            },
        );
    }

    pub(crate) async fn forget_hunger(&self, player_id: &PlayerId) {
        self.hunger.write().await.remove(player_id);
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

    /// Off-baseline move multipliers for the given movers; absent id = 1.0
    /// (NPCs and the well-baselined majority never enter the map).
    pub(super) async fn hunger_move_mults_for(
        &self,
        ids: &[PlayerId],
    ) -> std::collections::HashMap<PlayerId, f32> {
        let now = Instant::now();
        let hunger = self.hunger.read().await;
        ids.iter()
            .filter_map(|pid| {
                let d = hunger.get(pid)?;
                let poisoned = d.poisoned_until.is_some_and(|u| u > now);
                let (m, _, _) = effective_multipliers(d.satiation, poisoned);
                (m != 1.0).then_some((*pid, m))
            })
            .collect()
    }

    pub(super) async fn hunger_carry_mult(&self, player_id: &PlayerId) -> f32 {
        let hunger = self.hunger.read().await;
        hunger
            .get(player_id)
            .map(|d| {
                let poisoned = d.poisoned_until.is_some_and(|u| u > Instant::now());
                effective_multipliers(d.satiation, poisoned).2
            })
            .unwrap_or(1.0)
    }

    /// Weak or food-poisoned players don't regenerate (doc/HUNGER.md).
    /// Snapshot for the regen loop; untracked players (NPCs) regen freely.
    pub(super) async fn hunger_regen_blocked(&self) -> std::collections::HashSet<PlayerId> {
        let now = Instant::now();
        let hunger = self.hunger.read().await;
        hunger
            .iter()
            .filter_map(|(pid, d)| {
                let poisoned = d.poisoned_until.is_some_and(|u| u > now);
                let weak = hunger_state(d.satiation) == onlinerpg_shared::hunger::HungerState::Weak;
                (weak || poisoned).then_some(*pid)
            })
            .collect()
    }

    /// Reset to the Well-Fed floor on respawn so death never compounds
    /// starvation; poison does not survive death either.
    pub(crate) async fn reset_hunger_on_respawn(&self, player_id: &PlayerId) {
        let updated = {
            let mut hunger = self.hunger.write().await;
            match hunger.get_mut(player_id) {
                Some(data) => {
                    data.satiation = SATIATION_RESPAWN;
                    data.poisoned_until = None;
                    true
                }
                None => false,
            }
        };
        if updated {
            self.mark_dirty(player_id).await;
            self.send_hunger_update(player_id).await;
        }
    }

    /// The 20s decay bucket: one point per tick, four while poisoned. O(n)
    /// integer work; `HungerUpdate` goes out only on a band transition or
    /// poison expiry, so 5,000 players cost a handful of messages a day.
    pub async fn tick_hunger_decay(&self) {
        let mut updates: Vec<(PlayerId, ServerMessage)> = Vec::new();
        let mut newly_dirty: Vec<PlayerId> = Vec::new();
        {
            let now = Instant::now();
            let mut hunger = self.hunger.write().await;
            for (pid, data) in hunger.iter_mut() {
                let was_poisoned = data.poisoned_until.is_some();
                let still_poisoned = data.poisoned_until.is_some_and(|u| u > now);
                if was_poisoned && !still_poisoned {
                    data.poisoned_until = None;
                }

                let drain = if still_poisoned { POISON_DRAIN_MULT } else { 1 };
                let old_state = hunger_state(data.satiation);
                data.satiation = data.satiation.saturating_sub(drain);
                let new_state = hunger_state(data.satiation);

                if old_state != new_state || (was_poisoned && !still_poisoned) {
                    updates.push((
                        *pid,
                        hunger_update_msg(data.satiation, remaining_ms(data.poisoned_until, now)),
                    ));
                }
                if old_state != new_state {
                    newly_dirty.push(*pid);
                }
            }
        }
        if !newly_dirty.is_empty() {
            self.dirty_players.write().await.extend(newly_dirty);
        }
        for (pid, msg) in updates {
            self.send_direct_message(&pid, msg).await;
        }
    }

    /// Apply an `Eat`'s satiation/poison side. `force_poison` pins the
    /// raw-fish roll for tests.
    pub(super) async fn apply_eat(
        &self,
        player_id: &PlayerId,
        nutrition: u32,
        raw_fish: bool,
        force_poison: Option<bool>,
    ) -> EatOutcome {
        // thread_rng is !Send — roll before any await.
        let rolled_poison = raw_fish
            && force_poison
                .unwrap_or_else(|| rand::thread_rng().gen_range(0..100) < FOOD_POISONING_PCT);

        let now = Instant::now();
        let mut hunger = self.hunger.write().await;
        let Some(data) = hunger.get_mut(player_id) else {
            return EatOutcome::Untracked;
        };
        if data.satiation >= onlinerpg_shared::hunger::SATIATION_MAX {
            return EatOutcome::TooStuffed;
        }
        data.satiation = onlinerpg_shared::hunger::apply_nutrition(data.satiation, nutrition);
        if rolled_poison {
            // Reinfection refreshes the clock rather than stacking.
            data.poisoned_until = Some(now + Duration::from_millis(FOOD_POISONING_MS));
        }
        EatOutcome::Fed(hunger_update_msg(
            data.satiation,
            remaining_ms(data.poisoned_until, now),
        ))
    }

    // ---- Campfires ----

    /// The nearest campfire within grilling range, if any.
    async fn nearby_campfire(&self, position: &Position, floor_level: i8) -> Option<u64> {
        let campfires = self.campfires.read().await;
        campfires
            .values()
            .filter(|e| e.campfire.floor_level == floor_level)
            .map(|e| (e.campfire.id, e.campfire.position.dist_xz_sq(position)))
            .filter(|(_, d2)| *d2 <= CAMPFIRE_GRILL_RADIUS * CAMPFIRE_GRILL_RADIUS)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }

    /// Light a campfire and announce it to the area.
    pub(super) async fn spawn_campfire(&self, position: Position, floor_level: i8) -> Campfire {
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
                    expires_at: Instant::now() + Duration::from_millis(CAMPFIRE_DURATION_MS),
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
        let Some(campfire_id) = self.nearby_campfire(position, floor_level).await else {
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
