//! Timed debuffs on players (doc/DEBUFF.md): food poisoning, bleeding.
//!
//! Active debuffs ride in `HungerData` so one lock answers "what multipliers
//! apply to this player" and official NPCs stay exempt for free. Rolls,
//! expiry and damage ticks are server-side; the owner gets `DebuffUpdate` on
//! change only.

use crate::debuff_defs::{debuff_def, DebuffDef};
use onlinerpg_shared::debuff::ActiveDebuffState;
use onlinerpg_shared::{PlayerId, ServerMessage};
use rand::Rng;
use std::time::Duration;
use tokio::time::Instant;

use super::hunger::HungerData;

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
                    data.debuffs.push(ActiveDebuff { def, until });
                    data.status_msgs(now).into()
                }
            }
        };
        for msg in msgs {
            self.send_direct_message(player_id, msg).await;
        }
        true
    }

    /// Death drops every debuff.
    pub(crate) async fn clear_debuffs(&self, player_id: &PlayerId) {
        let msgs = {
            let mut hunger = self.hunger.write().await;
            match hunger.get_mut(player_id) {
                Some(data) if !data.debuffs.is_empty() => {
                    data.debuffs.clear();
                    data.status_msgs(Instant::now())
                }
                _ => return,
            }
        };
        for msg in msgs {
            self.send_direct_message(player_id, msg).await;
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
        if !damage.is_empty() {
            self.apply_debuff_damage(damage).await;
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
        let hits: Vec<Hit> = {
            let mut players = self.players.write().await;
            damage
                .into_iter()
                .filter_map(|(pid, amount, cause)| {
                    let player = players.get_mut(&pid)?;
                    if player.health == 0 {
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
