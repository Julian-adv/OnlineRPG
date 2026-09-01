//! NPC LLM driver: receive game events, prompt an LLM, parse the response,
//! and translate the chosen actions into game-server commands. The
//! top-level loop (`llm_driver`) owns timing — debounce, min-interval,
//! per-tick combat — and delegates the heavy lifting to submodules.
//!
//! Submodule layout:
//! - `action`: the JSON shape of an LLM response and conversion to
//!   `ClientMessage`.
//! - `prompt`: format server events and the active schedule context into
//!   the prompt string sent to the LLM.
//! - `combat`: face the monster and send the attack tick.
//! - `walk`: the one walker — A* to a place or a moving target, doors,
//!   position corrections, and the step pacing they all share.
//! - `movement`: schedule transitions and the housing-data prefetch that
//!   lets pathfinding avoid buildings.
//! - `execute`: parse a response and run each action; returns the
//!   monster_id of the final attack so the loop can take over chasing it.

mod action;
mod combat;
mod execute;
mod movement;
mod outcome;
mod prompt;
mod walk;

pub(crate) use prompt::format_event;

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::llm_scheduler::{LlmPriority, LlmScheduler};
use crate::state::SharedState;
use onlinerpg_shared::schedule::ScheduleEntry;
use onlinerpg_shared::ClientMessage;

pub(crate) use action::wants_reroll;
use combat::{load_attack_cooldown, tick_combat};
use execute::{append_memory, handle_response};
use movement::{
    check_schedule_transition, coverage_positions, fetch_furniture_around, fetch_houses_around,
    resolve_due_schedule,
};
use prompt::{build_prompt, record_conversation};

/// Trait for LLM backends that can send a prompt and return a text response.
#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn send_message(&self, content: &str) -> anyhow::Result<String>;
}

/// Load a system prompt layer from file. A `{{ACTIONS}}` slot is filled with
/// the action reference rendered from the parser's own spec table
/// (`action::ACTION_SPECS`), so the documented actions cannot drift from what
/// the parser accepts.
pub fn load_system_prompt(path: &str) -> anyhow::Result<String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read system prompt from {path}: {e}"))?;
    Ok(text.replace("{{ACTIONS}}", &action::action_reference()))
}

/// Render the surface-terrain summary without holding the state lock: a tile
/// cache miss hits disk or HTTP, and the lock also gates the NPC's
/// message-processing loop. The brief relock only snapshots the inputs.
async fn rendered_terrain_summary(state: &Arc<Mutex<SharedState>>) -> Option<String> {
    let job = state.lock().await.terrain_summary_job();
    match job {
        Some(job) => Some(job.render().await),
        None => None,
    }
}

/// Empty working directory for CLI backends — prompts are untrusted, so the
/// spawned CLI must not see the shared temp dir or the repo. Removed on drop.
pub struct RunDir(std::path::PathBuf);

impl RunDir {
    pub fn create() -> anyhow::Result<Self> {
        let dir = std::env::temp_dir().join(format!(
            "agent-client-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir(&dir)
            .map_err(|e| anyhow::anyhow!("Failed to create run dir {}: {e}", dir.display()))?;
        Ok(Self(dir))
    }

    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for RunDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A curated standing spot beside one furniture piece — a sick-room bed, a
/// dining chair — keyed by the piece's placement id (arrives as `object_id`
/// on the respawn / sitting broadcast).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct VisitSpot {
    #[serde(alias = "bed_object_id", alias = "chair_object_id")]
    pub object_id: u32,
    pub pos: [f32; 3],
    #[serde(default)]
    pub rotation: f32,
    #[serde(default)]
    pub floor_level: u8,
}

/// One answerable call: a sick-room bed or a seated guest.
#[derive(PartialEq, Eq, Hash)]
pub enum ClaimKey {
    Bed(u32),
    Guest(onlinerpg_shared::PlayerId),
}

/// Process-wide claim board so two maids sharing an inn don't answer the
/// same call — the first to claim a bedside or a seated guest wins, the
/// other falls through to the next pending event. Best effort: a lost
/// race sending both is acceptable.
#[derive(Default)]
pub struct VisitClaims(std::sync::Mutex<std::collections::HashMap<ClaimKey, (String, Instant)>>);

impl VisitClaims {
    /// True when `who` may hold `key` for `ttl`: unclaimed, expired, or
    /// already theirs (which refreshes the hold).
    pub fn try_claim(&self, key: ClaimKey, who: &str, ttl: Duration) -> bool {
        let now = Instant::now();
        let mut m = self.0.lock().unwrap();
        m.retain(|_, (_, until)| now < *until);
        match m.get(&key) {
            Some((owner, _)) if owner != who => false,
            _ => {
                m.insert(key, (who.to_string(), now + ttl));
                true
            }
        }
    }
}

/// Configuration for the LLM driver loop.
pub struct DriverConfig {
    pub label: String,
    pub memory_file: Option<String>,
    /// JSON map of player name -> accumulated favor, loaded at startup and
    /// rewritten whenever the LLM's `favor` deltas change it.
    pub favor_file: Option<String>,
    pub min_interval: Duration,
    /// Floor between prompts once an urgent event is pending — a player
    /// talking to us shouldn't wait out the routine cadence.
    pub urgent_min_interval: Duration,
    pub debounce: Duration,
    pub idle_interval: Duration,
    pub activity_window: Duration,
    /// Think even with nobody watching. Off only for operator-run registry
    /// NPCs, whose LLM calls come out of the project's budget.
    pub always_active: bool,
    pub schedule: Vec<ScheduleEntry>,
    /// Bedside spots in the inn's sick room; a respawn in one of these beds
    /// sends the NPC to stand at the matching spot and greet the woken.
    pub sickroom: Vec<VisitSpot>,
    /// Walk over to guests who sit down on a chair nearby and take their
    /// order. For NPCs that wait tables (the inn maid).
    pub serve_tables: bool,
    /// Curated order-taking spots for known chairs.
    pub tables: Vec<VisitSpot>,
    /// Shared with the process's other NPCs so only one answers each call.
    pub claims: Arc<VisitClaims>,
    /// HTTP base URL for the game server API (e.g. "http://127.0.0.1:10007").
    pub api_base_url: String,
}

/// Synthetic schedule entry for a visit — reuses the schedule walk
/// (waypointless, exact final rotation, cross-floor force move).
fn visit_entry(spot: &VisitSpot, label: &str) -> ScheduleEntry {
    ScheduleEntry {
        pos: spot.pos,
        rotation: spot.rotation,
        floor_level: spot.floor_level,
        label: Some(label.to_string()),
        ..Default::default()
    }
}

/// The seated guest's name and a schedule entry standing beside them,
/// facing the seat. `None` when the guest is gone, off their chair, on
/// another floor, or beyond serving range.
fn table_entry(
    s: &SharedState,
    guest_id: &onlinerpg_shared::PlayerId,
    tables: &[VisitSpot],
) -> Option<(String, ScheduleEntry)> {
    let me = s.self_player.as_ref()?;
    let p = s.nearby_players.get(guest_id)?;
    if p.object_type.as_deref() != Some(crate::state::SIT_OBJECT_TYPE)
        || p.floor_level != me.floor_level
        || p.floor_level < 0
    {
        return None;
    }
    // The broadcast position is where the guest stood when they clicked the
    // chair (the sit never re-sends a move), so the chair itself comes from
    // the placement.
    let (seat_x, seat_z) = p
        .object_id
        .and_then(|id| {
            s.furniture_position(
                crate::state::SIT_OBJECT_TYPE,
                id,
                p.position.x,
                p.position.z,
            )
        })
        .unwrap_or((p.position.x, p.position.z));
    let to_seat = crate::geom::PlanarDelta::to_xz(&me.position, seat_x, seat_z);
    if to_seat.dist > TABLE_SERVICE_RADIUS {
        return None;
    }
    let label = "the guest's table";
    // A curated spot for this chair wins over the computed one.
    if let Some(t) = tables.iter().find(|t| p.object_id == Some(t.object_id)) {
        return Some((p.name.clone(), visit_entry(t, label)));
    }
    // The nearest open cell of the four flanking the chair; solid
    // neighbours (the table, sibling chairs) are sealed and drop out.
    let floor = p.floor_level as u8;
    let (ccx, ccz) = (seat_x.floor() + 0.5, seat_z.floor() + 0.5);
    let d2 = |(x, z): (f32, f32)| (x - me.position.x).powi(2) + (z - me.position.z).powi(2);
    let (x, z) = onlinerpg_shared::pathfinding::DIRS
        .iter()
        .map(|&(dx, dz)| (ccx + dx as f32, ccz + dz as f32))
        .filter(|&(x, z)| s.cell_open(x, z, floor))
        .min_by(|&a, &b| d2(a).total_cmp(&d2(b)))
        // Every flank is solid: the nearest open neighbour of the seat.
        .unwrap_or_else(|| s.walkable_near(seat_x, seat_z, floor));
    let rotation = crate::geom::PlanarDelta::xz(x, z, seat_x, seat_z)
        .rotation()
        .to_degrees();
    let entry = ScheduleEntry {
        pos: [x, p.position.y, z],
        rotation,
        floor_level: floor,
        label: Some(label.to_string()),
        ..Default::default()
    };
    Some((p.name.clone(), entry))
}

/// The main LLM agent driver loop. Runs as a tokio task.
///
/// Ticks every ATTACK_COOLDOWN to send attack packets when there's an active
/// target. LLM calls are submitted to the shared scheduler so they don't block
/// combat and respect the global concurrency limit.
pub async fn llm_driver(
    state: Arc<Mutex<SharedState>>,
    invoker: Arc<dyn LlmBackend>,
    scheduler: LlmScheduler,
    config: DriverConfig,
) {
    let DriverConfig {
        label,
        memory_file,
        favor_file,
        min_interval,
        urgent_min_interval,
        debounce,
        idle_interval,
        activity_window,
        always_active,
        schedule,
        sickroom,
        serve_tables,
        tables,
        claims,
        api_base_url,
    } = config;
    let urgent_notify = {
        let s = state.lock().await;
        Arc::clone(&s.urgent_notify)
    };

    // Wait until we're in the game
    loop {
        {
            let s = state.lock().await;
            if s.in_game {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    info!("[{label}] LLM driver: in game, ready.");

    // Operator announcements — what a web player reads once on the login
    // screen. Delivered as one-shot events for the same reason: the next
    // prompt carries them, then they drain away instead of riding every
    // world state.
    {
        let notices = fetch_announcements(&api_base_url).await;
        if !notices.is_empty() {
            info!("[{label}] {} server announcement(s) loaded", notices.len());
            let mut s = state.lock().await;
            for notice in notices {
                s.push_agent_event_quiet(format!("[Notice] {notice}"));
            }
        }
    }

    // Favor persists across sessions; the map feeds prompt rendering and
    // the keepsake gate from the first turn.
    if let Some(path) = &favor_file {
        match std::fs::read_to_string(path) {
            Ok(txt) if !txt.trim().is_empty() => match serde_json::from_str(&txt) {
                Ok(map) => state.lock().await.favor = map,
                Err(e) => warn!("[{label}] Ignoring unreadable favor file {path}: {e}"),
            },
            _ => {}
        }
    }

    let attack_cooldown = load_attack_cooldown();

    // Stagger idle polls: random offset so NPCs don't all poll at the same time
    let idle_stagger = {
        use rand::Rng;
        let secs = idle_interval.as_secs().max(1);
        Duration::from_secs(rand::thread_rng().gen_range(0..secs))
    };
    let mut last_prompt_at = Instant::now() - idle_stagger;
    let mut attack_target: Option<(String, Option<bool>)> = None;
    let mut last_attack_at = Instant::now() - attack_cooldown;
    let mut llm_in_flight: Option<tokio::task::JoinHandle<anyhow::Result<String>>> = None;
    let mut prompt_pending_since: Option<Instant> = None;
    // Track last chat/combat activity to decide polling interval
    let mut last_activity_at = Instant::now() - idle_interval;
    // Track the highest urgency since the last prompt
    let mut pending_urgency = LlmPriority::Idle;
    let mut active_schedule: (Option<usize>, Option<u32>) = (None, None);
    // Deadline for the LLM's wrap-up turn before a due schedule move; the
    // bool marks that a prompt containing the wrap-up notice was submitted.
    let mut wrapup: Option<(Instant, bool)> = None;
    let mut dead_since: Option<Instant> = None;
    // Visit (bedside or guest's table) in progress: when to give the spot
    // up and resume the schedule.
    let mut visit_until: Option<Instant> = None;
    // Submit the pending prompt without waiting out the debounce or the
    // pacing floor — a single human-facing event (a guest sitting down)
    // that must not queue behind batching heuristics. Also exempts that
    // prompt from the audience check: the visit's human (a sleeper woken
    // upstairs) may not be on our floor yet. Cleared on submit.
    let mut force_prompt = false;
    // A table walk runs as a background task so the [Guest] prompt's LLM
    // round trip overlaps the walk instead of waiting for arrival.
    let mut visit_walk: Option<tokio::task::JoinHandle<()>> = None;

    // Fetch housing + furniture data so pathfinding avoids buildings and solid
    // props the same way the browser client does. An agent without a schedule
    // (a plain player agent) has no route to prefetch along, so cover where it
    // stands — otherwise its A* knows of no buildings at all and it walks
    // straight through the town it spawned in.
    let mut world_data_at: Option<(f32, f32)> = {
        let (world_cache, around) = {
            let s = state.lock().await;
            (
                Arc::clone(&s.world_cache),
                s.self_player.as_ref().map(|p| p.position),
            )
        };
        let area = coverage_positions(&schedule, around);
        // Different endpoints, disjoint data — no reason to wait for one before
        // asking for the other.
        tokio::join!(
            fetch_houses_around(&world_cache, &area, &api_base_url, &label),
            fetch_furniture_around(&world_cache, &area, &api_base_url, &label),
        );
        around.map(|p| (p.x, p.z))
    };

    // Execute initial schedule move (go to correct position for current time)
    if !schedule.is_empty() {
        // Wait for first GameTimeSync to arrive (up to 10s)
        for _ in 0..20 {
            let has_time = { state.lock().await.is_night.is_some() };
            if has_time {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        let due = resolve_due_schedule(&state, &schedule).await;
        active_schedule =
            check_schedule_transition(&state, &schedule, active_schedule, due, &label).await;
    }

    // Send initial world state unless the NPC is asleep, or it only thinks
    // with an audience and has none.
    let is_sleeping = active_schedule.0.is_some_and(|i| schedule[i].is_sleeping());
    {
        let mut s = state.lock().await;
        if is_sleeping {
            discard_turn(&mut s);
            info!("[{label}] LLM driver: NPC is sleeping, skipping initial prompt");
        } else if always_active || s.has_nearby_human_players() {
            drop(s);
            // File I/O and tile sampling outside the state lock.
            let terrain = rendered_terrain_summary(&state).await;
            let memory = load_memory_tail(&memory_file);
            let mut s = state.lock().await;
            let agent_events = s.drain_agent_events();
            let initial_prompt = build_prompt(
                &s,
                &[],
                &agent_events,
                &schedule,
                active_schedule.0,
                memory.as_deref(),
                terrain.as_deref(),
            );
            drop(s);
            info!("[{label}] LLM driver: sending initial world state");
            match scheduler
                .submit(
                    &label,
                    LlmPriority::Routine,
                    initial_prompt,
                    Arc::clone(&invoker),
                )
                .await
            {
                Ok(response) => {
                    let has_action = active_schedule
                        .0
                        .is_some_and(|i| schedule[i].action.is_some());
                    let skip_movement = {
                        let s = state.lock().await;
                        has_action || s.trade_busy || s.self_fishing
                    };
                    attack_target = handle_response(
                        &state,
                        &response,
                        &memory_file,
                        &favor_file,
                        skip_movement,
                    )
                    .await;
                    last_prompt_at = Instant::now();
                }
                Err(e) => {
                    error!("[{label}] LLM initial prompt failed: {e}");
                }
            }
        } else {
            discard_turn(&mut s);
            info!("[{label}] LLM driver: no human players nearby, skipping initial prompt");
        }
    }

    loop {
        decline_lapsed_trade(&state, &label).await;

        // Housing data was fetched around the start position; the initial
        // prefetch covers ±96m (the chunk and its neighbors). An exploring
        // agent walks out of that in minutes, after which buildings vanish
        // from pathfinding, the terrain map and the watch page — so refetch
        // around the new position whenever we've moved a chunk away. Moving is
        // only the trigger; the world cache decides what is actually asked for.
        {
            let (world_cache, pos) = {
                let s = state.lock().await;
                (
                    Arc::clone(&s.world_cache),
                    s.self_player.as_ref().map(|p| p.position),
                )
            };
            if let Some(p) = pos {
                let moved_a_chunk = world_data_at.is_none_or(|(x, z)| {
                    let (dx, dz) = (p.x - x, p.z - z);
                    dx * dx + dz * dz > 64.0 * 64.0
                });
                if moved_a_chunk {
                    let area = [(p.x, p.z)];
                    tokio::join!(
                        fetch_houses_around(&world_cache, &area, &api_base_url, &label),
                        fetch_furniture_around(&world_cache, &area, &api_base_url, &label),
                    );
                    world_data_at = Some((p.x, p.z));
                }
            }
        }

        // Tick interval: ATTACK_COOLDOWN when in combat, otherwise 1s (responsive to events)
        let tick_duration = if attack_target.is_some() {
            attack_cooldown.saturating_sub(last_attack_at.elapsed())
        } else {
            Duration::from_secs(1)
        };

        tokio::select! {
            _ = urgent_notify.notified() => {
                // Sets both the rate-limit floor below and the queue priority.
                let woken_by = LlmPriority::from(state.lock().await.take_wake_urgency());
                debug!("[{label}] LLM driver: woken by a {woken_by:?} event");
                last_activity_at = Instant::now();
                pending_urgency = pending_urgency.min(woken_by);
                // Start the debounce window now, even mid-call: it then runs
                // alongside the in-flight prompt instead of only starting once
                // that one lands. Double-submission is already ruled out below.
                if prompt_pending_since.is_none() {
                    prompt_pending_since = Some(Instant::now());
                }
            }
            _ = tokio::time::sleep(tick_duration) => {}
        }

        // === Auto-respawn ===
        // Respawn as an LLM action needs a turn, and turns are skipped with
        // no audience — a dead NPC would otherwise stay dead forever. The
        // driver revives itself after a short delay.
        let self_dead = {
            let s = state.lock().await;
            s.in_game && s.self_player.as_ref().is_some_and(|p| p.health == 0)
        };
        if self_dead && dead_since.is_none() {
            info!(
                "[{label}] LLM driver: agent is dead, auto-respawn in {}s",
                RESPAWN_DELAY.as_secs()
            );
        }
        if respawn_due(self_dead, &mut dead_since, Instant::now()) {
            request_respawn(&state, &memory_file, &label).await;
        }
        if self_dead {
            attack_target = None;
            continue;
        }

        // === Combat tick ===
        if let Some((monster_id, sprint)) = attack_target.clone() {
            if last_attack_at.elapsed() >= attack_cooldown {
                if !tick_combat(&state, &monster_id, sprint).await {
                    attack_target = None;
                }
                last_attack_at = Instant::now();
            }
        }

        let sleeping_now = active_schedule.0.is_some_and(|i| schedule[i].is_sleeping());

        // === Visits: sick-room bedsides and guest tables ===
        // Both queue the same shape: the ambient event goes out before the
        // walk, the walk runs in the background, and the prompt is forced
        // out at once — the LLM round trip overlaps the walk, so the
        // greeting lands around arrival instead of a full round trip after.
        let mut visit: Option<(String, ScheduleEntry)> = None;
        if !sickroom.is_empty() && attack_target.is_none() {
            // Drain even while asleep so stale respawns don't pile up.
            let respawns = { state.lock().await.drain_recent_respawns() };
            // Claiming while asleep would leave the bed unanswered.
            let woken = if sleeping_now {
                None
            } else {
                respawns.into_iter().rev().find_map(|(name, bed_id)| {
                    sickroom
                        .iter()
                        .find(|s| s.object_id == bed_id)
                        .filter(|_| claims.try_claim(ClaimKey::Bed(bed_id), &label, VISIT_DWELL))
                        .map(|s| (name, s.clone()))
                })
            };
            if let Some((name, spot)) = woken {
                info!("[{label}] {name} woke in the sick room — going to the bedside");
                visit = Some((
                    format!(
                        "[SickRoom] {name} just came back to their senses in the \
                         sick room — they fell out there and were carried up. You \
                         are on your way to their bedside; no move action needed. \
                         Welcome them back with warm relief in your own words and \
                         offer something hot to eat. If they have already left \
                         the room, let them go — don't shout after them."
                    ),
                    visit_entry(&spot, "the sick-room bedside"),
                ));
            }
        }
        if serve_tables && attack_target.is_none() {
            // Drain every tick so stale seatings don't pile up. A bedside
            // visit outranks table service; an ongoing visit keeps its spot.
            let seated = {
                let mut s = state.lock().await;
                let seatings = s.drain_recent_seatings();
                if visit.is_some() || sleeping_now || visit_until.is_some() {
                    None
                } else {
                    seatings.into_iter().rev().find_map(|pid| {
                        table_entry(&s, &pid, &tables)
                            .filter(|_| claims.try_claim(ClaimKey::Guest(pid), &label, VISIT_DWELL))
                    })
                }
            };
            if let Some((name, entry)) = seated {
                info!("[{label}] {name} took a seat nearby — going over for the order");
                visit = Some((
                    format!(
                        "[Guest] {name} just took a seat at one of your tables and \
                         you are on your way over — no move action needed. Greet \
                         them in your own words and take their order — they buy by \
                         clicking you to open your shop. If they have already left \
                         the seat, let it go."
                    ),
                    entry,
                ));
            }
        }
        if let Some((event, entry)) = visit {
            // A visit walk still in flight would fight this one for the body.
            if let Some(h) = visit_walk.take() {
                h.abort();
            }
            // The same leave-taking a schedule transition does: a visit
            // must not walk off mid-follow or leave a stall behind.
            movement::stop_current_entry(&state, &schedule, active_schedule.0, &label).await;
            state.lock().await.push_ambient_event_quiet(event);
            // One event, not a burst: no debounce, no pacing floor.
            prompt_pending_since = Some(Instant::now());
            pending_urgency = LlmPriority::Urgent;
            force_prompt = true;
            last_activity_at = Instant::now();
            let walk_state = Arc::clone(&state);
            visit_walk = Some(tokio::spawn(async move {
                movement::execute_schedule_move(&walk_state, &entry).await;
            }));
            visit_until = Some(Instant::now() + VISIT_DWELL);
        }

        if visit_until.is_some_and(|until| Instant::now() >= until) {
            // Visit over: forget the active entry so the schedule walks
            // us back to our regular post.
            visit_until = None;
            if let Some(h) = visit_walk.take() {
                h.abort();
            }
            active_schedule = (None, None);
        }

        // === Check schedule transitions ===
        if !schedule.is_empty() && attack_target.is_none() && visit_until.is_none() {
            let due = resolve_due_schedule(&state, &schedule).await;
            if due == active_schedule {
                wrapup = None;
            } else {
                // The move blocks this loop for the whole walk, so on a real
                // departure announce it first and hold the move until the LLM
                // had a turn to wrap up (pack a stall, end a song) or the
                // grace runs out. A recurring entry re-triggering on the same
                // spot has nothing to wrap up and moves right away.
                let transition_now = match (wrapup, due.0) {
                    _ if due.0 == active_schedule.0 => true,
                    (_, None) => true,
                    (Some((deadline, prompted)), _) => {
                        (prompted && llm_in_flight.is_none()) || Instant::now() >= deadline
                    }
                    (None, Some(i)) => {
                        let sleeping_now =
                            active_schedule.0.is_some_and(|j| schedule[j].is_sleeping());
                        let mut start_now = true;
                        if !sleeping_now {
                            let mut s = state.lock().await;
                            if always_active || s.has_nearby_human_players() {
                                s.push_ambient_event(format!(
                                    "[Schedule] Time to head to {} — you set off in a moment. \
                                     Wrap up what you are doing now (pack up, finish your \
                                     tune, say goodbye).",
                                    schedule[i].display_label()
                                ));
                                wrapup = Some((Instant::now() + SCHEDULE_WRAPUP_GRACE, false));
                                start_now = false;
                            }
                        }
                        start_now
                    }
                };
                if transition_now {
                    wrapup = None;
                    active_schedule =
                        check_schedule_transition(&state, &schedule, active_schedule, due, &label)
                            .await;
                }
            }
        }
        let is_sleeping = active_schedule.0.is_some_and(|i| schedule[i].is_sleeping());
        let has_scheduled_action = active_schedule
            .0
            .is_some_and(|i| schedule[i].action.is_some());

        // === Check if LLM response arrived ===
        if llm_in_flight.as_ref().is_some_and(|h| h.is_finished()) {
            let handle = llm_in_flight.take().unwrap();
            last_prompt_at = Instant::now();
            if let Some(response) = await_llm_response(handle, &label).await {
                // A live visit walk owns the body; the LLM turn it triggered
                // may talk but not move.
                let walking_to_visit = visit_walk.as_ref().is_some_and(|h| !h.is_finished());
                let skip_movement = {
                    let s = state.lock().await;
                    has_scheduled_action || s.trade_busy || s.self_fishing || walking_to_visit
                };
                let new_target =
                    handle_response(&state, &response, &memory_file, &favor_file, skip_movement)
                        .await;
                if new_target.is_some() {
                    attack_target = new_target;
                }
            }
        }

        // === Maybe start a new LLM prompt ===
        if llm_in_flight.is_some() {
            continue;
        }

        // Periodic prompt — use short interval only when recently active (chat/combat)
        let active = attack_target.is_some() || last_activity_at.elapsed() < activity_window;
        // A pending urgent event (a player talking to us, a hit landing) gets
        // a shorter floor, so a reply isn't held back by the routine cadence.
        let floor = if pending_urgency == LlmPriority::Urgent {
            urgent_min_interval
        } else {
            min_interval
        };
        let effective_interval = if active { floor } else { idle_interval };
        if prompt_pending_since.is_none() && last_prompt_at.elapsed() >= effective_interval {
            prompt_pending_since = Some(Instant::now());
            if pending_urgency == LlmPriority::Idle && active {
                pending_urgency = LlmPriority::Routine;
            }
        }

        // Debounce: wait at least `debounce` after the trigger before actually prompting
        let ready_to_prompt =
            prompt_pending_since.is_some_and(|t| force_prompt || t.elapsed() >= debounce);

        if !ready_to_prompt {
            continue;
        }

        // Also ensure the floor since last prompt (keep pending state so we retry next tick)
        if !force_prompt && last_prompt_at.elapsed() < floor {
            continue;
        }
        prompt_pending_since = None;
        let forced = std::mem::take(&mut force_prompt);

        // Drain first; the prompt build (and its memory-file read) only
        // happens when something actually needs answering.
        let (events, agent_events, priority) = {
            let mut s = state.lock().await;

            // Skip the LLM while asleep, and — for an operator-run NPC — while
            // nobody is around to see it. Events still drain so they can't pile
            // up unbounded; what was said still lands in the conversation
            // history, so a waking NPC knows what it heard. A forced visit
            // prompt is exempt from the audience check: its human (a sleeper
            // woken upstairs) may not be on our floor yet.
            if is_sleeping || !(always_active || forced || s.has_nearby_human_players()) {
                discard_turn(&mut s);
                pending_urgency = LlmPriority::Idle;
                continue;
            }

            let events = s.drain_events();
            let agent_events = s.drain_agent_events();

            // Determine priority from the most urgent event (lower = more urgent)
            let max_urgency = events
                .iter()
                .map(|e| LlmPriority::from(s.classify_event(e)))
                .fold(pending_urgency, std::cmp::min);
            (events, agent_events, max_urgency)
        };
        pending_urgency = LlmPriority::Idle; // reset for next cycle

        if events.is_empty() && agent_events.is_empty() {
            continue;
        }

        // File I/O and tile sampling outside the state lock.
        let memory = load_memory_tail(&memory_file);
        let terrain = rendered_terrain_summary(&state).await;
        let prompt = {
            let mut s = state.lock().await;
            // Armed one turn early so the closing turn reads it under EVENTS.
            if s.meeting_turn() {
                let closing = prompt::meeting_closing_event(
                    s.meeting_host,
                    s.pricing.as_ref().map_or(0, |p| p.last_change_pct),
                );
                s.push_ambient_event(closing);
            }
            // History is recorded after the build: this prompt shows the
            // batch under EVENTS, the next one under RECENT CONVERSATION.
            let prompt = build_prompt(
                &s,
                &events,
                &agent_events,
                &schedule,
                active_schedule.0,
                memory.as_deref(),
                terrain.as_deref(),
            );
            record_conversation(&mut s, &events);
            prompt
        };

        // Submit to scheduler as background task (doesn't block combat ticks)
        info!(
            "[{label}] LLM driver: submitting {:?} prompt ({} chars)",
            priority,
            prompt.len()
        );
        let sched = scheduler.clone();
        let inv = Arc::clone(&invoker);
        let lbl = label.clone();
        llm_in_flight = Some(tokio::spawn(async move {
            sched.submit(&lbl, priority, prompt, inv).await
        }));
        if let Some((_, prompted)) = &mut wrapup {
            *prompted = true;
        }
    }
}

/// Grace period between noticing our own death and requesting respawn.
const RESPAWN_DELAY: Duration = Duration::from_secs(5);

/// How long a due schedule move waits for the LLM's wrap-up turn. Covers the
/// prompt debounce plus one LLM round trip; the move starts as soon as that
/// turn lands, so the full grace is only spent when the LLM stays silent.
/// 30 real seconds ≈ 4 game minutes.
const SCHEDULE_WRAPUP_GRACE: Duration = Duration::from_secs(30);

/// How long a visit (sick-room bedside, guest's table) holds the NPC there
/// before the regular schedule pulls it back.
const VISIT_DWELL: Duration = Duration::from_secs(90);

/// A guest sitting down within this range of the NPC counts as one of its
/// tables. Same floor only.
const TABLE_SERVICE_RADIUS: f32 = 10.0;

/// Death watch: fires once `dead` has held for RESPAWN_DELAY, then re-arms
/// so a lost request is retried after another delay. Revival clears it.
fn respawn_due(dead: bool, dead_since: &mut Option<Instant>, now: Instant) -> bool {
    if !dead {
        *dead_since = None;
        return false;
    }
    match *dead_since {
        None => {
            *dead_since = Some(now);
            false
        }
        Some(t) if now.duration_since(t) >= RESPAWN_DELAY => {
            *dead_since = None;
            true
        }
        Some(_) => false,
    }
}

/// Ask the server to revive us (heal + teleport to spawn + hunger reset)
/// and note the death in memory so the LLM learns it died even though it
/// never got a turn while dead.
async fn request_respawn(
    state: &Arc<Mutex<SharedState>>,
    memory_file: &Option<String>,
    label: &str,
) {
    info!("[{label}] LLM driver: requesting respawn");
    if let Some(path) = memory_file {
        append_memory(path, "I was killed and woke up back at the spawn point.");
    }
    let mut s = state.lock().await;
    if let Err(e) = s.send_command(ClientMessage::RequestRespawn).await {
        error!("[{label}] Respawn request failed: {e}");
    }
}

/// Wave off a trade offer nobody answered, like the web client's toast timing
/// out, so the merchant stops pushing windows at us.
async fn decline_lapsed_trade(state: &Arc<Mutex<SharedState>>, label: &str) {
    let mut s = state.lock().await;
    if let Some(offer) = s.pushed_trade.take_if(|t| !t.is_live()) {
        let cmd = ClientMessage::DeclineTrade {
            merchant_player_id: offer.merchant_id,
        };
        if let Err(e) = s.send_command(cmd).await {
            error!("[{label}] Failed to decline a lapsed trade offer: {e}");
        }
    }
}

/// Drop a turn without prompting: events drain (so they cannot pile up)
/// but what was said still lands in the conversation history.
fn discard_turn(s: &mut SharedState) {
    let events = s.drain_events();
    record_conversation(s, &events);
    s.drain_agent_events();
}

/// Cap on memory.txt and on the per-prompt MEMORIES tail: `append_memory`
/// rewrites the file to its newest lines, and `load_memory_tail` guards
/// against legacy files still longer than the cap.
const MEMORY_LINES: usize = 50;

/// Tail of the NPC's memory file. Re-read per prompt — the file is tiny —
/// so notes written this session reach a stateless backend without a
/// restart (the file used to be baked into the system prompt at startup).
fn load_memory_tail(memory_file: &Option<String>) -> Option<String> {
    let path = memory_file.as_ref()?;
    let content = std::fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return None;
    }
    let start = lines.len().saturating_sub(MEMORY_LINES);
    Some(lines[start..].join("\n"))
}

/// Await a finished LLM submission and unwrap the join/scheduler result.
/// Logs the failure and returns `None` for both join panics and scheduler
/// errors so the caller can collapse three error arms into one branch.
async fn await_llm_response(
    handle: tokio::task::JoinHandle<anyhow::Result<String>>,
    label: &str,
) -> Option<String> {
    match handle.await {
        Ok(Ok(response)) => Some(response),
        Ok(Err(e)) => {
            error!("[{label}] LLM prompt failed: {e}");
            None
        }
        Err(e) => {
            error!("[{label}] LLM task panicked: {e}");
            None
        }
    }
}

/// Fetch the operator announcements a web player reads on the login screen.
/// Best-effort: served notices are login-screen content, so a failure means
/// an empty list, not an error the driver should stall on.
async fn fetch_announcements(api_base_url: &str) -> Vec<String> {
    let url = format!("{api_base_url}/api/announcements");
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(client) => client,
        Err(_) => return Vec::new(),
    };
    let resp = match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => resp,
        _ => return Vec::new(),
    };
    let Ok(list) = resp.json::<Vec<serde_json::Value>>().await else {
        return Vec::new();
    };
    list.iter()
        .take(3)
        .filter_map(|a| {
            let date = a.get("date").and_then(|d| d.as_str()).unwrap_or("");
            let translations = a.get("translations")?;
            // Prefer Korean (the server's default locale), fall back to any.
            let t = translations
                .get("ko")
                .or_else(|| translations.as_object()?.values().next())?;
            let title = t.get("title").and_then(|s| s.as_str())?;
            let body = t.get("body").and_then(|s| s.as_str()).unwrap_or("");
            let mut line = format!("[{date}] {title}");
            let body = body.trim();
            if !body.is_empty() {
                let short: String = body.chars().take(200).collect();
                line.push_str(" — ");
                line.push_str(&short);
                if body.chars().count() > 200 {
                    line.push('…');
                }
            }
            Some(line)
        })
        .collect()
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::state::tests::{test_player, test_state};

    /// A visit forces its prompt out at once, so the driver consumes the
    /// ambient event into a real prompt — the backend captures prompts for
    /// the tests to scan.
    struct Capture(Arc<std::sync::Mutex<Vec<String>>>);
    #[async_trait]
    impl LlmBackend for Capture {
        async fn send_message(&self, p: &str) -> anyhow::Result<String> {
            self.0.lock().unwrap().push(p.to_string());
            Ok("{}".to_string())
        }
    }

    /// Wait until `marker` shows up in a captured prompt — or in the event
    /// queue, covering the race where this loop drains the event before the
    /// driver does.
    async fn await_marker(
        state: &Arc<Mutex<SharedState>>,
        prompts: &Arc<std::sync::Mutex<Vec<String>>>,
        marker: &str,
    ) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let drained = state.lock().await.drain_agent_events();
                if drained.iter().any(|e| e.contains(marker))
                    || prompts.lock().unwrap().iter().any(|p| p.contains(marker))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("the visit never surfaced the {marker} event"));
    }

    #[test]
    fn respawn_fires_only_after_the_delay_and_rearms_while_still_dead() {
        let mut since = None;
        let t0 = Instant::now();
        assert!(!respawn_due(true, &mut since, t0));
        assert!(!respawn_due(true, &mut since, t0 + RESPAWN_DELAY / 2));
        assert!(respawn_due(true, &mut since, t0 + RESPAWN_DELAY));
        assert!(!respawn_due(true, &mut since, t0 + RESPAWN_DELAY));
        assert!(respawn_due(true, &mut since, t0 + RESPAWN_DELAY * 2));
        assert!(!respawn_due(false, &mut since, t0 + RESPAWN_DELAY * 2));
        assert!(since.is_none());
    }

    #[tokio::test]
    async fn request_respawn_sends_the_request_and_notes_the_death() {
        let (mut s, mut rx) = test_state();
        let mut me = test_player(0.0, 0.0);
        me.health = 0;
        s.self_player = Some(me);
        let state = Arc::new(Mutex::new(s));
        let dir = RunDir::create().unwrap();
        let memory = dir.path().join("memory.txt").to_str().unwrap().to_string();

        request_respawn(&state, &Some(memory.clone()), "test").await;

        assert!(matches!(rx.try_recv(), Ok(ClientMessage::RequestRespawn)));
        assert!(std::fs::read_to_string(&memory)
            .unwrap()
            .contains("I was killed"));
    }

    #[tokio::test]
    async fn a_sickroom_respawn_sends_the_npc_to_the_bedside() {
        let (mut s, mut rx) = test_state();
        let me = test_player(-1443.9, 4748.9);
        s.in_game = true;
        s.is_night = Some(true);
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);

        // Queued before the driver starts: the respawn buffer must survive
        // startup and be picked up by the loop's first drain.
        let mut woken = test_player(-1451.5, 4754.05);
        woken.id = onlinerpg_shared::PlayerId::from(7);
        woken.object_id = Some(52);
        s.push_event(onlinerpg_shared::ServerMessage::PlayerRespawned { player: woken });

        let state = Arc::new(Mutex::new(s));

        // Keep the outgoing command channel drained so walking can't block.
        tokio::spawn(async move { while rx.recv().await.is_some() {} });

        let prompts: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();

        // One live entry on the NPC's serving spot, so the visit leaves a
        // real active schedule the way Mira's does.
        let mut entry = ScheduleEntry {
            at: "night".to_string(),
            pos: [-1443.9, 1.3, 4748.9],
            rotation: -40.9,
            floor_level: 0,
            label: Some("serving tables".to_string()),
            ..Default::default()
        };
        entry.parse_condition().unwrap();

        let never = Duration::from_secs(3600);
        let config = DriverConfig {
            label: "test_maid".to_string(),
            memory_file: None,
            favor_file: None,
            min_interval: never,
            urgent_min_interval: never,
            debounce: never,
            idle_interval: never,
            activity_window: never,
            always_active: false,
            schedule: vec![entry],
            sickroom: vec![VisitSpot {
                object_id: 52,
                pos: [-1450.3, 4.4, 4754.7],
                rotation: -104.6,
                floor_level: 1,
            }],
            serve_tables: false,
            tables: Vec::new(),
            claims: Arc::default(),
            api_base_url: "http://127.0.0.1:9".to_string(),
        };
        let scheduler = LlmScheduler::new(1, Duration::from_secs(5));
        let driver = tokio::spawn(llm_driver(
            Arc::clone(&state),
            Arc::new(Capture(Arc::clone(&prompts))),
            scheduler,
            config,
        ));

        await_marker(&state, &prompts, "[SickRoom]").await;
        driver.abort();
    }

    #[test]
    fn first_claimant_holds_a_visit_claim() {
        let c = VisitClaims::default();
        assert!(c.try_claim(ClaimKey::Bed(52), "mira", VISIT_DWELL));
        assert!(!c.try_claim(ClaimKey::Bed(52), "coco", VISIT_DWELL));
        assert!(c.try_claim(ClaimKey::Bed(52), "mira", VISIT_DWELL));
        assert!(c.try_claim(ClaimKey::Bed(53), "coco", VISIT_DWELL));
    }

    #[tokio::test]
    async fn a_seated_guest_sends_the_npc_to_the_table() {
        let (mut s, mut rx) = test_state();
        let me = test_player(-1443.9, 4748.9);
        s.in_game = true;
        s.is_night = Some(true);
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);

        // A guest in sight sits down before the driver starts: the seating
        // buffer must survive startup like the respawn one does.
        let mut guest = test_player(-1447.0, 4750.5);
        guest.id = onlinerpg_shared::PlayerId::from(8);
        guest.name = "Jake".to_string();
        let guest_id = guest.id;
        s.nearby_players.insert(guest_id, guest);
        s.push_event(onlinerpg_shared::ServerMessage::PlayerInteractionChanged {
            player_id: guest_id,
            object_type: Some(crate::state::SIT_OBJECT_TYPE.to_string()),
            object_id: Some(46),
        });

        let state = Arc::new(Mutex::new(s));
        tokio::spawn(async move { while rx.recv().await.is_some() {} });

        let prompts: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();

        let never = Duration::from_secs(3600);
        let config = DriverConfig {
            label: "test_maid".to_string(),
            memory_file: None,
            favor_file: None,
            min_interval: never,
            urgent_min_interval: never,
            debounce: never,
            idle_interval: never,
            activity_window: never,
            always_active: false,
            schedule: Vec::new(),
            sickroom: Vec::new(),
            serve_tables: true,
            tables: Vec::new(),
            claims: Arc::default(),
            api_base_url: "http://127.0.0.1:9".to_string(),
        };
        let scheduler = LlmScheduler::new(1, Duration::from_secs(5));
        let driver = tokio::spawn(llm_driver(
            Arc::clone(&state),
            Arc::new(Capture(Arc::clone(&prompts))),
            scheduler,
            config,
        ));

        await_marker(&state, &prompts, "[Guest] Jake").await;
        driver.abort();
    }

    #[tokio::test]
    async fn a_curated_table_spot_wins_over_the_computed_one() {
        let (mut s, _rx) = test_state();
        let me = test_player(-1443.9, 4748.9);
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);

        let mut guest = test_player(-1446.75, 4751.75);
        guest.id = onlinerpg_shared::PlayerId::from(8);
        guest.object_type = Some(crate::state::SIT_OBJECT_TYPE.to_string());
        guest.object_id = Some(46);
        let guest_id = guest.id;
        s.nearby_players.insert(guest_id, guest);

        let tables = vec![VisitSpot {
            object_id: 46,
            pos: [-1446.4, 1.3, 4752.5],
            rotation: 145.9,
            floor_level: 0,
        }];
        let (name, entry) = table_entry(&s, &guest_id, &tables).expect("guest is seated in range");
        assert_eq!(name, "Me");
        assert_eq!(entry.pos, [-1446.4, 1.3, 4752.5]);
        assert_eq!(entry.rotation, 145.9);

        // A seat away from every curated chair falls back to a computed spot.
        let (_, entry) = table_entry(&s, &guest_id, &[]).expect("fallback still serves");
        assert_ne!(entry.pos, [-1446.4, 1.3, 4752.5]);
    }

    #[tokio::test]
    async fn the_computed_spot_flanks_the_chair_placement_not_the_stale_position() {
        let (mut s, _rx) = test_state();
        let me = test_player(-1443.9, 4748.9);
        s.self_player_id = Some(me.id);
        s.self_player = Some(me);

        // The guest's broadcast position is where they stood when they
        // clicked the chair — one cell east of it.
        let mut guest = test_player(-1449.3, 4749.4);
        guest.id = onlinerpg_shared::PlayerId::from(8);
        guest.object_type = Some(crate::state::SIT_OBJECT_TYPE.to_string());
        guest.object_id = Some(39);
        let guest_id = guest.id;
        s.nearby_players.insert(guest_id, guest);

        let chair = onlinerpg_shared::furniture::FurniturePlacement {
            id: 39,
            type_id: "chair".to_string(),
            x: -1450.3,
            y: 1.3,
            z: 4749.05,
            rotation_deg: 0.0,
            floor_level: 0,
        };
        s.world_cache
            .write()
            .unwrap()
            .sync_furniture(-2, 4, vec![chair]);

        let (_, entry) = table_entry(&s, &guest_id, &[]).expect("guest is seated in range");
        // The chair cell's east flank — not the standing cell's.
        assert_eq!(entry.pos[0], -1449.5);
        assert_eq!(entry.pos[2], 4749.5);
    }
}
