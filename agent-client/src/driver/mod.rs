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
    /// HTTP base URL for the game server API (e.g. "http://127.0.0.1:10007").
    pub api_base_url: String,
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

        // === Check schedule transitions ===
        if !schedule.is_empty() && attack_target.is_none() {
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
                let skip_movement = {
                    let s = state.lock().await;
                    has_scheduled_action || s.trade_busy || s.self_fishing
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
        let ready_to_prompt = prompt_pending_since.is_some_and(|t| t.elapsed() >= debounce);

        if !ready_to_prompt {
            continue;
        }

        // Also ensure the floor since last prompt (keep pending state so we retry next tick)
        if last_prompt_at.elapsed() < floor {
            continue;
        }
        prompt_pending_since = None;

        // Drain first; the prompt build (and its memory-file read) only
        // happens when something actually needs answering.
        let (events, agent_events, priority) = {
            let mut s = state.lock().await;

            // Skip the LLM while asleep, and — for an operator-run NPC — while
            // nobody is around to see it. Events still drain so they can't pile
            // up unbounded; what was said still lands in the conversation
            // history, so a waking NPC knows what it heard.
            if is_sleeping || !(always_active || s.has_nearby_human_players()) {
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
}
