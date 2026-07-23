//! Fishing sessions: cast → wait → bite → hook → caught/escaped
//! (design: `doc/FISHING.md`). The server owns every timer and roll; clients
//! only render what the broadcasts describe and answer with `FishingRespond`.
//! Timers are `tokio::time::Instant`s advanced by the 250 ms fishing tick, so
//! tests drive the whole machine with `start_paused` + `time::advance` — the
//! tick period only bounds how late a transition can fire, and every player
//! deadline already carries `LATENCY_GRACE_MS` of slack beyond it.

use onlinerpg_shared::fishing::{
    struggle_rounds, struggle_window_ms, tension_miss_penalty, FishState, FishingAction,
    FishingOutcome, BITE_WINDOW_MS, CAST_MS, CATCH_XP_PER_RARITY_SQ, ESCAPE_XP, LATENCY_GRACE_MS,
    MAX_CAST_DISTANCE_METERS, TENSION_CORRECT_RELIEF, TENSION_MAX, WAIT_MAX_MS, WAIT_MIN_MS,
};
use onlinerpg_shared::inventory::EquipSlot;
use onlinerpg_shared::skills::SkillId;
use onlinerpg_shared::Position;
use rand::Rng;
use std::time::Duration;
use tokio::time::Instant;
use tracing::warn;

use super::GameState;
use crate::types::{PlayerId, ServerMessage};

/// Sea level: terrain below this is water (doc/WATER_SYSTEM.md — the client
/// generates water meshes for tiles under Y=0; this is the server-side twin
/// of that rule and the server's first gameplay use of terrain height).
const SEA_LEVEL: f32 = 0.0;

/// Casts are only valid on the overworld floor — no fishing in dungeons or
/// on house upper floors, whose "water" would be a terrain-height fiction.
const OVERWORLD_FLOOR: i8 = 0;

pub(crate) enum FishingPhase {
    /// Rod is swinging; the bobber lands when this elapses.
    Casting { until: Instant },
    /// Bobber is floating; the fish bites at `bite_at`.
    Waiting { bite_at: Instant },
    /// Bobber dipped at `since`; `Hook` must arrive before
    /// `since + BITE_WINDOW_MS + LATENCY_GRACE_MS`.
    Bite { since: Instant },
    /// Hooked — the fight is on. One round at a time: answer `fish_state`
    /// with its correct action before `deadline` (+ grace) or take tension.
    Struggle {
        round: u32,
        total_rounds: u32,
        fish_state: FishState,
        deadline: Instant,
        /// The round was answered — the tick's reaper must not also miss it.
        responded: bool,
        tension: u32,
    },
}

/// What bit the line. Rolled when the bite fires — not at resolution — so a
/// future "line tension hints at the catch" broadcast stays honest, but only
/// revealed to the player on a successful catch.
pub(crate) struct RolledFish {
    pub item_def_id: String,
    pub rarity: u32,
    pub size_cm: u16,
    pub trophy: bool,
}

pub(crate) struct FishingSession {
    pub bobber: Position,
    pub phase: FishingPhase,
    pub rolled_fish: Option<RolledFish>,
    pub skill_level: u32,
}

/// Pure catch-table entry, split out so the weighting is unit-testable
/// without a `GameState`.
pub(crate) struct CatchCandidate {
    pub item_def_id: String,
    pub rarity: u32,
    pub catch_weight: u32,
}

/// Effective weight of one species for a given fishing level: rarer fish
/// gain `rarity` weight per level, so skill shifts the table toward the top
/// without ever emptying the bottom.
pub(crate) fn effective_weight(candidate: &CatchCandidate, skill_level: u32) -> u64 {
    u64::from(candidate.catch_weight) + u64::from(skill_level) * u64::from(candidate.rarity)
}

/// Weighted pick over the catch table. `roll` is a uniform draw in
/// `[0, total_weight)`; separating the draw from the pick keeps this pure.
pub(crate) fn pick_catch(
    candidates: &[CatchCandidate],
    skill_level: u32,
    mut roll: u64,
) -> Option<usize> {
    for (index, candidate) in candidates.iter().enumerate() {
        let weight = effective_weight(candidate, skill_level);
        if roll < weight {
            return Some(index);
        }
        roll -= weight;
    }
    None
}

/// Bite wait for a given skill level: uniform in the shared range, shortened
/// 2% per level, floored at the range minimum.
pub(crate) fn roll_wait_ms(skill_level: u32, rng: &mut impl Rng) -> u64 {
    let base = rng.gen_range(u64::from(WAIT_MIN_MS)..=u64::from(WAIT_MAX_MS));
    let shortened = base * u64::from(100u32.saturating_sub(skill_level * 2)) / 100;
    shortened.max(u64::from(WAIT_MIN_MS) / 2)
}

impl GameState {
    /// Handle `ClientMessage::FishingCast`: validate everything the design
    /// requires (rod, floor, range, water, liveness) and open the session.
    pub async fn start_fishing(&self, player_id: &PlayerId, target: Position) {
        if self.fishing_sessions.read().await.contains_key(player_id) {
            self.send_fishing_error(player_id, "You are already fishing.")
                .await;
            return;
        }

        let (player_pos, player_floor, alive) = {
            let players = self.players.read().await;
            let Some(p) = players.get(player_id) else {
                return;
            };
            (p.position, p.floor_level, p.health > 0)
        };
        if !alive {
            self.send_fishing_error(player_id, "You cannot fish while defeated.")
                .await;
            return;
        }
        if player_floor != OVERWORLD_FLOOR {
            self.send_fishing_error(player_id, "You can only fish outdoors.")
                .await;
            return;
        }

        let has_rod = self
            .get_player_inventory(player_id)
            .await
            .and_then(|inv| inv.equipped.get(&EquipSlot::MainHand).cloned())
            .and_then(|item| {
                self.item_defs
                    .get(&item.item_def_id)
                    .map(|def| def.is_fishing_rod())
            })
            .unwrap_or(false);
        if !has_rod {
            self.send_fishing_error(player_id, "You need a fishing rod in your main hand.")
                .await;
            return;
        }

        let dx = onlinerpg_shared::shortest_world_delta_x(target.x, player_pos.x);
        let dz = target.z - player_pos.z;
        if dx * dx + dz * dz > MAX_CAST_DISTANCE_METERS * MAX_CAST_DISTANCE_METERS {
            self.send_fishing_error(player_id, "That water is out of casting range.")
                .await;
            return;
        }

        // The one async terrain read in the flow, deliberately in the cast
        // handler (first-touch tile IO) and never in the tick.
        match self
            .height_sampler
            .sample_height(onlinerpg_shared::wrap_world_x(target.x), target.z)
            .await
        {
            Ok(height) if height < SEA_LEVEL => {}
            Ok(_) => {
                self.send_fishing_error(player_id, "You can only cast into water.")
                    .await;
                return;
            }
            Err(err) => {
                warn!("start_fishing: height sample failed: {err}");
                self.send_fishing_error(player_id, "You can only cast into water.")
                    .await;
                return;
            }
        }

        let skill_level = self
            .get_player_skills(player_id)
            .await
            .get(SkillId::Fishing)
            .level;
        let bobber = Position {
            x: onlinerpg_shared::wrap_world_x(target.x),
            y: SEA_LEVEL,
            z: target.z,
        };
        {
            let mut sessions = self.fishing_sessions.write().await;
            sessions.insert(
                *player_id,
                FishingSession {
                    bobber,
                    phase: FishingPhase::Casting {
                        until: Instant::now() + Duration::from_millis(u64::from(CAST_MS)),
                    },
                    rolled_fish: None,
                    skill_level,
                },
            );
        }
        self.broadcast_fishing(
            &bobber,
            ServerMessage::FishingCasted {
                player_id: *player_id,
                position: bobber,
            },
        )
        .await;
    }

    /// Handle `ClientMessage::FishingRespond`. Timing is judged here against
    /// the server's own deadlines — a late hook is an escape no matter what
    /// the client believed.
    pub async fn respond_fishing(&self, player_id: &PlayerId, action: FishingAction) {
        let verdict = {
            let mut sessions = self.fishing_sessions.write().await;
            let Some(session) = sessions.get_mut(player_id) else {
                self.send_fishing_error(player_id, "You are not fishing.")
                    .await;
                return;
            };
            match &mut session.phase {
                // Yanking before the bite scares the fish off (any action).
                FishingPhase::Casting { .. } | FishingPhase::Waiting { .. } => Verdict::TooEarly,
                FishingPhase::Bite { since } => {
                    let deadline = *since
                        + Duration::from_millis(u64::from(BITE_WINDOW_MS + LATENCY_GRACE_MS));
                    if Instant::now() > deadline {
                        // The tick will call it escaped; treat the stale
                        // response the same way rather than racing it.
                        Verdict::TooLate
                    } else if action == FishingAction::Hook {
                        Verdict::Hooked
                    } else {
                        // Reeling or giving line before the hook is set:
                        // the fish spits the bait.
                        Verdict::TooEarly
                    }
                }
                FishingPhase::Struggle {
                    fish_state,
                    deadline,
                    responded,
                    tension,
                    ..
                } => {
                    if *responded {
                        // Round already answered; ignore the duplicate.
                        return;
                    }
                    let rarity = rarity_of(&session.rolled_fish);
                    let in_time = Instant::now()
                        <= *deadline + Duration::from_millis(u64::from(LATENCY_GRACE_MS));
                    let correct = in_time && action == fish_state.correct_action();
                    *responded = true;
                    if correct {
                        *tension = tension.saturating_sub(TENSION_CORRECT_RELIEF);
                    } else {
                        *tension += tension_miss_penalty(rarity);
                    }
                    Verdict::RoundAnswered {
                        correct,
                        tension: *tension,
                    }
                }
            }
        };

        match verdict {
            Verdict::Hooked => self.begin_struggle(player_id).await,
            Verdict::TooEarly => {
                self.end_fishing(player_id, FishingOutcome::Escaped, 0)
                    .await;
            }
            Verdict::TooLate => {
                self.end_fishing(player_id, FishingOutcome::Escaped, ESCAPE_XP)
                    .await;
            }
            Verdict::RoundAnswered { correct, tension } => {
                self.broadcast_round_result(player_id, correct, tension)
                    .await;
                self.advance_struggle(player_id).await;
            }
        }
    }

    /// Deliberate reel-in (`ClientMessage::FishingStop`).
    pub async fn stop_fishing(&self, player_id: &PlayerId) {
        self.end_fishing(player_id, FishingOutcome::Aborted, 0)
            .await;
    }

    /// Anything that breaks concentration — movement, combat, disconnect —
    /// lands here. Quiet no-op for the overwhelmingly common case of a
    /// player who isn't fishing.
    pub async fn cancel_fishing_if_active(&self, player_id: &PlayerId) {
        if self.fishing_sessions.read().await.contains_key(player_id) {
            self.end_fishing(player_id, FishingOutcome::Aborted, 0)
                .await;
        }
    }

    /// The 250 ms fishing tick: advances casts to waits, waits to bites, and
    /// expires bites the angler slept through.
    pub async fn tick_fishing(&self) {
        let now = Instant::now();
        enum Due {
            BobberLanded(PlayerId),
            Bite(PlayerId),
            Expired(PlayerId),
            StruggleMissed(PlayerId),
            PlayerGone(PlayerId),
        }
        let mut due = Vec::new();
        {
            let sessions = self.fishing_sessions.read().await;
            if sessions.is_empty() {
                return;
            }
            let players = self.players.read().await;
            for (player_id, session) in sessions.iter() {
                let alive = players.get(player_id).is_some_and(|p| p.health > 0);
                if !alive {
                    due.push(Due::PlayerGone(*player_id));
                    continue;
                }
                match &session.phase {
                    FishingPhase::Casting { until } if now >= *until => {
                        due.push(Due::BobberLanded(*player_id));
                    }
                    FishingPhase::Waiting { bite_at } if now >= *bite_at => {
                        due.push(Due::Bite(*player_id));
                    }
                    FishingPhase::Bite { since }
                        if now
                            >= *since
                                + Duration::from_millis(u64::from(
                                    BITE_WINDOW_MS + 2 * LATENCY_GRACE_MS,
                                )) =>
                    {
                        // Doubled grace: a response that raced the deadline is
                        // judged in respond_fishing; the tick only reaps
                        // sessions nobody answered for.
                        due.push(Due::Expired(*player_id));
                    }
                    FishingPhase::Struggle {
                        deadline,
                        responded: false,
                        ..
                    } if now
                        >= *deadline + Duration::from_millis(u64::from(2 * LATENCY_GRACE_MS)) =>
                    {
                        // Same doubled-grace contract as the bite reaper.
                        due.push(Due::StruggleMissed(*player_id));
                    }
                    _ => {}
                }
            }
        }

        for entry in due {
            match entry {
                Due::PlayerGone(player_id) => {
                    self.end_fishing(&player_id, FishingOutcome::Aborted, 0)
                        .await;
                }
                Due::BobberLanded(player_id) => {
                    let skill_level = {
                        let sessions = self.fishing_sessions.read().await;
                        let Some(session) = sessions.get(&player_id) else {
                            continue;
                        };
                        session.skill_level
                    };
                    // rand's thread_rng is !Send: keep it inside an
                    // await-free block.
                    let wait_ms = roll_wait_ms(skill_level, &mut rand::thread_rng());
                    let mut sessions = self.fishing_sessions.write().await;
                    if let Some(session) = sessions.get_mut(&player_id) {
                        session.phase = FishingPhase::Waiting {
                            bite_at: now + Duration::from_millis(wait_ms),
                        };
                    }
                }
                Due::Bite(player_id) => {
                    let rolled = self.roll_fish(&player_id).await;
                    let bobber = {
                        let mut sessions = self.fishing_sessions.write().await;
                        let Some(session) = sessions.get_mut(&player_id) else {
                            continue;
                        };
                        match rolled {
                            Some(fish) => {
                                session.rolled_fish = Some(fish);
                                session.phase = FishingPhase::Bite { since: now };
                                Some(session.bobber)
                            }
                            // Empty catch table (no fish defs): nothing can
                            // ever bite, end the session instead of hanging.
                            None => None,
                        }
                    };
                    match bobber {
                        Some(bobber) => {
                            self.broadcast_fishing(
                                &bobber,
                                ServerMessage::FishingBite { player_id },
                            )
                            .await;
                        }
                        None => {
                            self.end_fishing(&player_id, FishingOutcome::Escaped, 0)
                                .await;
                        }
                    }
                }
                Due::Expired(player_id) => {
                    self.end_fishing(&player_id, FishingOutcome::Escaped, ESCAPE_XP)
                        .await;
                }
                Due::StruggleMissed(player_id) => {
                    let tension = {
                        let mut sessions = self.fishing_sessions.write().await;
                        let Some(session) = sessions.get_mut(&player_id) else {
                            continue;
                        };
                        let rarity = rarity_of(&session.rolled_fish);
                        match &mut session.phase {
                            FishingPhase::Struggle {
                                responded, tension, ..
                            } if !*responded => {
                                *responded = true;
                                *tension += tension_miss_penalty(rarity);
                                Some(*tension)
                            }
                            _ => None,
                        }
                    };
                    if let Some(tension) = tension {
                        self.broadcast_round_result(&player_id, false, tension)
                            .await;
                        self.advance_struggle(&player_id).await;
                    }
                }
            }
        }
    }

    /// Roll species + size + trophy for a bite, from the item-def catch
    /// table (`category == "fish"`, weighted by `catchWeight`).
    async fn roll_fish(&self, player_id: &PlayerId) -> Option<RolledFish> {
        let skill_level = {
            let sessions = self.fishing_sessions.read().await;
            sessions.get(player_id)?.skill_level
        };
        let candidates = self.item_defs.fish_catch_table();
        if candidates.is_empty() {
            return None;
        }
        let total: u64 = candidates
            .iter()
            .map(|c| effective_weight(c, skill_level))
            .sum();
        let (index, quality) = {
            let mut rng = rand::thread_rng();
            (
                pick_catch(&candidates, skill_level, rng.gen_range(0..total))?,
                rng.gen_range(1..=20u32),
            )
        };
        let picked = &candidates[index];
        let def = self.item_defs.get(&picked.item_def_id)?;
        let mut size_cm = def
            .size_dice
            .as_deref()
            .map(crate::game::combat::roll_dice)
            .unwrap_or(10) as u16;
        // Natural 20 on the quality roll: a once-in-a-session monster.
        let nat_twenty = quality == 20;
        if nat_twenty {
            size_cm = size_cm.saturating_mul(2);
        }
        let trophy = nat_twenty
            || def
                .trophy_cm
                .is_some_and(|threshold| u32::from(size_cm) >= threshold);
        Some(RolledFish {
            item_def_id: picked.item_def_id.clone(),
            rarity: picked.rarity,
            size_cm,
            trophy,
        })
    }

    /// Successful hook: the fight begins. Roll the first round's state and
    /// announce it; `advance_struggle` runs the rest.
    async fn begin_struggle(&self, player_id: &PlayerId) {
        let announce = {
            let mut sessions = self.fishing_sessions.write().await;
            let Some(session) = sessions.get_mut(player_id) else {
                return;
            };
            let rarity = rarity_of(&session.rolled_fish);
            let total_rounds = struggle_rounds(rarity);
            let window_ms = struggle_window_ms(rarity, session.skill_level);
            let fish_state = roll_fish_state();
            session.phase = FishingPhase::Struggle {
                round: 1,
                total_rounds,
                fish_state,
                deadline: Instant::now() + Duration::from_millis(u64::from(window_ms)),
                responded: false,
                tension: 0,
            };
            (session.bobber, total_rounds, fish_state, window_ms)
        };
        let (bobber, total_rounds, fish_state, window_ms) = announce;
        self.broadcast_fishing(
            &bobber,
            ServerMessage::FishingStruggleRound {
                player_id: *player_id,
                round: 1,
                total_rounds,
                fish_state,
                respond_within_ms: window_ms,
                tension_pct: 0,
            },
        )
        .await;
    }

    /// After a round resolved (answered or reaped): escape at max tension,
    /// catch when every round is survived, otherwise open the next round.
    async fn advance_struggle(&self, player_id: &PlayerId) {
        enum Next {
            Escaped,
            Caught,
            Round {
                bobber: Position,
                round: u32,
                total_rounds: u32,
                fish_state: FishState,
                window_ms: u32,
                tension: u32,
            },
        }
        let next = {
            let mut sessions = self.fishing_sessions.write().await;
            let Some(session) = sessions.get_mut(player_id) else {
                return;
            };
            let rarity = rarity_of(&session.rolled_fish);
            let skill_level = session.skill_level;
            let bobber = session.bobber;
            match &mut session.phase {
                FishingPhase::Struggle {
                    round,
                    total_rounds,
                    fish_state,
                    deadline,
                    responded,
                    tension,
                } => {
                    if *tension >= TENSION_MAX {
                        Next::Escaped
                    } else if *round >= *total_rounds {
                        Next::Caught
                    } else {
                        *round += 1;
                        *fish_state = roll_fish_state();
                        let window_ms = struggle_window_ms(rarity, skill_level);
                        *deadline = Instant::now() + Duration::from_millis(u64::from(window_ms));
                        *responded = false;
                        Next::Round {
                            bobber,
                            round: *round,
                            total_rounds: *total_rounds,
                            fish_state: *fish_state,
                            window_ms,
                            tension: *tension,
                        }
                    }
                }
                _ => return,
            }
        };
        match next {
            Next::Escaped => {
                self.end_fishing(player_id, FishingOutcome::Escaped, ESCAPE_XP)
                    .await;
            }
            Next::Caught => self.finish_fishing_caught(player_id).await,
            Next::Round {
                bobber,
                round,
                total_rounds,
                fish_state,
                window_ms,
                tension,
            } => {
                self.broadcast_fishing(
                    &bobber,
                    ServerMessage::FishingStruggleRound {
                        player_id: *player_id,
                        round,
                        total_rounds,
                        fish_state,
                        respond_within_ms: window_ms,
                        tension_pct: tension,
                    },
                )
                .await;
            }
        }
    }

    async fn broadcast_round_result(&self, player_id: &PlayerId, correct: bool, tension: u32) {
        let bobber = {
            let sessions = self.fishing_sessions.read().await;
            match sessions.get(player_id) {
                Some(session) => session.bobber,
                None => return,
            }
        };
        self.broadcast_fishing(
            &bobber,
            ServerMessage::FishingRoundResult {
                player_id: *player_id,
                correct,
                tension_pct: tension,
            },
        )
        .await;
    }

    /// Every round survived: award the fish (bag, or ground when overweight),
    /// grant skill XP, end the session with the full catch details.
    async fn finish_fishing_caught(&self, player_id: &PlayerId) {
        let Some(fish) = ({
            let mut sessions = self.fishing_sessions.write().await;
            sessions
                .get_mut(player_id)
                .and_then(|session| session.rolled_fish.take())
        }) else {
            // Bite phase always has a rolled fish; a missing one means the
            // session raced an abort — treat it as gone.
            return;
        };

        self.award_stackable_item(player_id, &fish.item_def_id)
            .await;
        let xp = CATCH_XP_PER_RARITY_SQ * u64::from(fish.rarity) * u64::from(fish.rarity);
        self.add_skill_xp(player_id, SkillId::Fishing, xp).await;
        self.end_fishing(
            player_id,
            FishingOutcome::Caught {
                item_def_id: fish.item_def_id,
                size_cm: fish.size_cm,
                trophy: fish.trophy,
            },
            0,
        )
        .await;
    }

    /// Remove the session (if any) and broadcast how it ended. `escape_xp`
    /// covers the hooked-but-lost consolation; catches grant theirs before
    /// calling in.
    async fn end_fishing(&self, player_id: &PlayerId, outcome: FishingOutcome, escape_xp: u64) {
        let Some(session) = self.fishing_sessions.write().await.remove(player_id) else {
            return;
        };
        if escape_xp > 0 {
            self.add_skill_xp(player_id, SkillId::Fishing, escape_xp)
                .await;
        }
        self.broadcast_fishing(
            &session.bobber,
            ServerMessage::FishingEnded {
                player_id: *player_id,
                outcome,
            },
        )
        .await;
    }

    /// Fishing events go to everyone near the bobber on the overworld floor
    /// — the angler is inside cast range of it by construction.
    async fn broadcast_fishing(&self, bobber: &Position, msg: ServerMessage) {
        self.send_direct_message_to_players_within_position(
            bobber,
            OVERWORLD_FLOOR,
            super::EVENT_DELIVERY_RADIUS,
            msg,
            None,
        )
        .await;
    }

    async fn send_fishing_error(&self, player_id: &PlayerId, message: &str) {
        self.send_direct_message(
            player_id,
            ServerMessage::FishingError {
                message: message.to_string(),
            },
        )
        .await;
    }
}

enum Verdict {
    Hooked,
    TooEarly,
    TooLate,
    RoundAnswered { correct: bool, tension: u32 },
}

fn rarity_of(rolled: &Option<RolledFish>) -> u32 {
    rolled.as_ref().map_or(1, |f| f.rarity)
}

/// Coin-flip the fish's next move. No await between creation and use — see
/// the thread_rng note in `tick_fishing`.
fn roll_fish_state() -> FishState {
    if rand::thread_rng().gen_bool(0.5) {
        FishState::Pulling
    } else {
        FishState::Tiring
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Vec<CatchCandidate> {
        vec![
            CatchCandidate {
                item_def_id: "raw_minnow".into(),
                rarity: 1,
                catch_weight: 50,
            },
            CatchCandidate {
                item_def_id: "golden_sturgeon".into(),
                rarity: 5,
                catch_weight: 1,
            },
        ]
    }

    #[test]
    fn weighting_scales_rarity_with_skill() {
        let t = table();
        assert_eq!(effective_weight(&t[0], 0), 50);
        assert_eq!(effective_weight(&t[1], 0), 1);
        // Level 20: minnow 50+20, carp 1+100 — rare fish gain ground but the
        // commons never vanish.
        assert_eq!(effective_weight(&t[0], 20), 70);
        assert_eq!(effective_weight(&t[1], 20), 101);
    }

    #[test]
    fn pick_walks_cumulative_weights() {
        let t = table();
        assert_eq!(pick_catch(&t, 0, 0), Some(0));
        assert_eq!(pick_catch(&t, 0, 49), Some(0));
        assert_eq!(pick_catch(&t, 0, 50), Some(1));
        // Out-of-range roll (caller bug) picks nothing rather than panicking.
        assert_eq!(pick_catch(&t, 0, 51), None);
    }

    #[test]
    fn wait_shortens_with_skill_but_keeps_a_floor() {
        let mut rng = rand::thread_rng();
        for _ in 0..200 {
            let novice = roll_wait_ms(0, &mut rng);
            assert!((u64::from(WAIT_MIN_MS)..=u64::from(WAIT_MAX_MS)).contains(&novice));
            let master = roll_wait_ms(20, &mut rng);
            // 40% shorter, never below the floor.
            assert!(master >= u64::from(WAIT_MIN_MS) / 2);
            assert!(master <= u64::from(WAIT_MAX_MS) * 60 / 100);
        }
    }
}
