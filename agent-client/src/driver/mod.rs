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
//! - `combat`: chase a monster (A* + repath on monster shift), face it,
//!   send the attack tick.
//! - `movement`: A*-driven walks, schedule transitions, and the
//!   housing-data prefetch that lets pathfinding avoid buildings.
//! - `execute`: parse a response and run each action; returns the
//!   monster_id of the final attack so the loop can take over chasing it.

mod action;
mod combat;
mod execute;
mod movement;
mod prompt;

pub(crate) use prompt::format_event;

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;
use tracing::{debug, error, info};

use crate::llm_scheduler::{LlmPriority, LlmScheduler};
use crate::orchestrator::ScheduleEntry;
use crate::state::SharedState;

pub(crate) use action::wants_reroll;
use combat::{load_attack_cooldown, tick_combat};
use execute::handle_response;
use movement::{
    check_schedule_transition, coverage_positions, fetch_furniture_around, fetch_houses_around,
};
use prompt::build_prompt;

/// Trait for LLM backends that can send a prompt and return a text response.
#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn send_message(&self, content: &str) -> anyhow::Result<String>;
}

/// Load system prompt from file.
pub fn load_system_prompt(path: &str) -> anyhow::Result<String> {
    std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read system prompt from {path}: {e}"))
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

    let attack_cooldown = load_attack_cooldown();

    // Stagger idle polls: random offset so NPCs don't all poll at the same time
    let idle_stagger = {
        use rand::Rng;
        let secs = idle_interval.as_secs().max(1);
        Duration::from_secs(rand::thread_rng().gen_range(0..secs))
    };
    let mut last_prompt_at = Instant::now() - idle_stagger;
    let mut attack_target: Option<String> = None;
    let mut last_attack_at = Instant::now() - attack_cooldown;
    let mut llm_in_flight: Option<tokio::task::JoinHandle<anyhow::Result<String>>> = None;
    let mut prompt_pending_since: Option<Instant> = None;
    // Track last chat/combat activity to decide polling interval
    let mut last_activity_at = Instant::now() - idle_interval;
    // Track the highest urgency since the last prompt
    let mut pending_urgency = LlmPriority::Idle;
    let mut active_schedule: (Option<usize>, Option<u32>) = (None, None);

    // Fetch housing + furniture data so pathfinding avoids buildings and solid
    // props the same way the browser client does. An agent without a schedule
    // (a plain player agent) has no route to prefetch along, so cover where it
    // stands — otherwise its A* knows of no buildings at all and it walks
    // straight through the town it spawned in.
    {
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
    }

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

        active_schedule =
            check_schedule_transition(&state, &schedule, active_schedule, &label).await;
    }

    // Send initial world state unless the NPC is asleep, or it only thinks
    // with an audience and has none.
    let is_sleeping = active_schedule.0.is_some_and(|i| schedule[i].is_sleeping());
    {
        let mut s = state.lock().await;
        if is_sleeping {
            s.drain_events();
            s.drain_agent_events();
            info!("[{label}] LLM driver: NPC is sleeping, skipping initial prompt");
        } else if always_active || s.has_nearby_human_players() {
            let agent_events = s.drain_agent_events();
            let initial_prompt = build_prompt(&s, &[], &agent_events, &schedule, active_schedule.0);
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
                    attack_target =
                        handle_response(&state, &response, &memory_file, skip_movement).await;
                    last_prompt_at = Instant::now();
                }
                Err(e) => {
                    error!("[{label}] LLM initial prompt failed: {e}");
                }
            }
        } else {
            s.drain_events();
            s.drain_agent_events();
            info!("[{label}] LLM driver: no human players nearby, skipping initial prompt");
        }
    }

    loop {
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

        // === Combat tick ===
        if attack_target.is_some() && last_attack_at.elapsed() >= attack_cooldown {
            attack_target = tick_combat(&state, attack_target.unwrap()).await;
            last_attack_at = Instant::now();
        }

        // === Check schedule transitions ===
        if !schedule.is_empty() && attack_target.is_none() {
            active_schedule =
                check_schedule_transition(&state, &schedule, active_schedule, &label).await;
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
                    handle_response(&state, &response, &memory_file, skip_movement).await;
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

        // Drain events and build prompt, determine priority from events
        let (prompt, has_events, priority) = {
            let mut s = state.lock().await;

            // Skip the LLM while asleep, and — for an operator-run NPC — while
            // nobody is around to see it. Events still drain so they can't pile
            // up unbounded.
            if is_sleeping || !(always_active || s.has_nearby_human_players()) {
                s.drain_events();
                s.drain_agent_events();
                pending_urgency = LlmPriority::Idle;
                continue;
            }

            let events = s.drain_events();
            let agent_events = s.drain_agent_events();
            let has_events = !events.is_empty() || !agent_events.is_empty();

            // Determine priority from the most urgent event (lower = more urgent)
            let max_urgency = events
                .iter()
                .map(|e| LlmPriority::from(s.classify_event(e)))
                .fold(pending_urgency, std::cmp::min);

            let prompt = build_prompt(&s, &events, &agent_events, &schedule, active_schedule.0);
            (prompt, has_events, max_urgency)
        };
        pending_urgency = LlmPriority::Idle; // reset for next cycle

        if !has_events {
            continue;
        }

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
    }
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
