//! Orchestrator: manages multiple NPC connections in parallel.
//!
//! Each NPC gets its own WebSocket connection and session loop, but they share
//! terrain data (HeightSampler) and world cache (PassabilityCache + houses).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use onlinerpg_shared::monster_ai::BehaviorTree;
use onlinerpg_shared::{
    Character, CharacterAttributes, CharacterClass, ClientMessage, Gender, ServerMessage,
};
use onlinerpg_terrain::height::HeightSampler;
use serde::Deserialize;
use tokio::sync::{mpsc, Mutex};
use tracing::{error, info, warn};

use crate::claude::{self, ClaudeConfig};
use crate::codex::{self, CodexConfig};
use crate::driver;
use crate::google_auth::GoogleAuth;
use crate::llm_scheduler::{LlmPriority, LlmScheduler, TimeoutBackend};
use crate::openai::{self, OpenAiConfig};
use crate::openrouter::{self, OpenRouterConfig};
use crate::state::{SharedState, WorldCache};
use crate::ws;
use crate::LlmType;

use onlinerpg_shared::schedule::{parse_conditions, ScheduleEntry};

/// Wrapper for deserializing a schedule file.
#[derive(Debug, Deserialize)]
struct ScheduleFile {
    schedule: Vec<ScheduleEntry>,
}

/// Wrapper for deserializing a visit-spot file (sickroom.json, tables.json).
#[derive(Debug, Deserialize)]
struct SpotsFile {
    spots: Vec<driver::VisitSpot>,
}

/// Read and parse one optional JSON data file. A missing path yields None
/// silently; an unreadable or malformed file logs and yields None, so the
/// NPC still runs without it.
fn load_json_file<T: serde::de::DeserializeOwned>(path: Option<&str>, label: &str) -> Option<T> {
    let path = path?;
    match std::fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str::<T>(&content) {
            Ok(v) => Some(v),
            Err(e) => {
                error!("[{label}] Failed to parse {path}: {e}");
                None
            }
        },
        Err(e) => {
            error!("[{label}] Failed to read {path}: {e}");
            None
        }
    }
}

/// Per-NPC configuration. Deployment-only values (account, llm backend,
/// timing) live here; everything describing *who the NPC is* comes from the
/// game-data registry via `id` and can merely be overridden here.
#[derive(Debug, Clone, Deserialize)]
pub struct NpcConfig {
    /// Registry id in `data-src/npcs.csv`. When set, `character_name`,
    /// `character_class`, the prompt files and the schedule are derived
    /// from the registry row and the `data/npcs/{id}/` directory
    /// convention (see `resolve_from_registry` in main.rs); the explicit
    /// fields below act as overrides.
    pub id: Option<String>,
    /// NPC account name; required for `npc_token` auth, ignored (and better
    /// omitted) under Google sign-in, where the token decides the account.
    pub account: Option<String>,
    #[serde(default)]
    pub llm: LlmType,
    #[serde(default = "super::default_min_interval_secs")]
    pub min_interval_secs: u64,
    /// Floor between prompts while an urgent event waits (player chat, a hit
    /// landing). Shorter than `min_interval_secs` so replies feel prompt.
    #[serde(default = "super::default_urgent_min_interval_secs")]
    pub urgent_min_interval_secs: u64,
    #[serde(default = "super::default_debounce_secs")]
    pub debounce_secs: u64,
    #[serde(default = "super::default_idle_interval_secs")]
    pub idle_interval_secs: u64,
    #[serde(default = "super::default_activity_window_secs")]
    pub activity_window_secs: u64,
    /// Keep thinking with no human player in sight. On for agents a player
    /// runs — those spend the runner's own LLM quota, so there is nothing for
    /// the project to save, and an agent that stops the moment it walks out of
    /// sight can never hunt a dungeon alone. Registry NPCs (`id = "..."`)
    /// default to off: they run on the operator's budget, and an NPC nobody
    /// can see has no one to act for. See `always_active()`.
    pub always_active: Option<bool>,
    /// Sprint on every walk while well fed. Off makes the agent walk
    /// everywhere, which costs it far less satiation. Registry NPCs
    /// (`id = "..."`) default to off: they are hunger-exempt, so nothing
    /// would ever price or gate their sprinting, and the whole town would
    /// visibly run everywhere for free. See `always_sprint()`.
    pub always_sprint: Option<bool>,
    #[serde(default)]
    pub claude: ClaudeConfig,
    #[serde(default)]
    pub openrouter: OpenRouterConfig,
    #[serde(default)]
    pub codex: CodexConfig,
    #[serde(default)]
    pub openai: OpenAiConfig,

    // --- Auto-provisioning ---
    /// Character name to create if no characters exist on this account.
    pub character_name: Option<String>,
    /// Character class for auto-creation (e.g. "merchant"). Defaults to "knight".
    pub character_class: Option<String>,
    /// Character gender for auto-creation. Defaults to male when omitted.
    pub gender: Option<Gender>,

    // --- 3-tier prompt system ---
    /// Path to template prompt file (role-specific behavior rules).
    /// When set, overrides backend-specific system_prompt_file.
    pub template_prompt: Option<String>,
    /// Path to instance prompt file (individual NPC personality).
    pub instance_prompt: Option<String>,
    /// Path to memory file (accumulated experiences, auto-updated by LLM).
    pub memory_file: Option<String>,
    /// Path to favor file (per-player accumulated favor, auto-updated by LLM).
    pub favor_file: Option<String>,
    /// Path to schedule file (time-based positioning).
    pub schedule_file: Option<String>,
    /// Path to sick-room file (bedside spots for respawn greetings).
    pub sickroom_file: Option<String>,
    /// Walk over to guests who sit down on a nearby chair and take their
    /// order (the inn maid). Defaults to false.
    pub serve_tables: Option<bool>,
    /// Path to tables file (curated order-taking spots per chair).
    pub tables_file: Option<String>,
}

impl NpcConfig {
    /// Whether to prompt the LLM with no human player nearby. Defaults to
    /// true for anything but a registry NPC — see `always_active`.
    pub fn always_active(&self) -> bool {
        self.always_active.unwrap_or(self.id.is_none())
    }

    /// Whether walks sprint by default. True for anything but a registry
    /// NPC — see the field doc.
    pub fn always_sprint(&self) -> bool {
        self.always_sprint.unwrap_or(self.id.is_none())
    }

    /// Log label: the account when there is one, else the character it plays.
    pub fn label(&self) -> &str {
        self.account
            .as_deref()
            .or(self.character_name.as_deref())
            .unwrap_or("agent")
    }

    /// Whether this agent busks. The one gate for everything bard: the
    /// songbook and tip prompt sections, and the `[Tip]` events in state —
    /// so an agent is never instructed about tips it will not receive.
    pub fn plays_music(&self) -> bool {
        self.character_class.as_deref() == Some("bard")
            || self
                .template_prompt
                .as_deref()
                .is_some_and(|path| path.ends_with("bard.txt"))
    }
}

/// Resources shared across all NPC connections.
pub struct SharedResources {
    pub height_sampler: Arc<HeightSampler>,
    pub splat_sampler: Arc<crate::splat::SplatSampler>,
    pub world_cache: Arc<std::sync::RwLock<WorldCache>>,
    pub behavior_trees: Arc<HashMap<String, BehaviorTree>>,
    pub type_mapping: Arc<HashMap<String, String>>,
    pub movement_speeds: Arc<HashMap<String, crate::monster_ai::MonsterMovement>>,
    pub scheduler: LlmScheduler,
    pub codex_app_server: codex::CodexAppServer,
    /// One claim board for the process, so co-located NPCs (the two inn
    /// maids) don't both answer the same bedside or table call.
    pub claims: Arc<driver::VisitClaims>,
    /// Character names of the NPCs that wait tables: staff feed themselves,
    /// so they never count as anyone's guest.
    pub maid_names: HashSet<String>,
    /// `None` when `transcript_dir` is empty; the summary line is logged either way.
    pub transcript: Option<Arc<crate::transcript::Transcript>>,
    pub auth: AuthSource,
    /// `None` when the spectator panel is off, so nothing is recorded for it.
    pub watch: Option<Arc<crate::watch::WatchHub>>,
}

/// How sessions prove who they are. Operator NPCs share one secret; a
/// user-run agent signs in as its own Google account and mints a fresh ID
/// token per connection (they expire in an hour, sessions outlive that).
pub enum AuthSource {
    NpcToken(String),
    Google(GoogleAuth),
}

impl AuthSource {
    async fn authenticate_message(&self, account: Option<&str>) -> anyhow::Result<ClientMessage> {
        match self {
            AuthSource::NpcToken(token) => Ok(ClientMessage::AuthenticateNpc {
                account_name: account
                    .ok_or_else(|| anyhow::anyhow!("[[npcs]] account is required"))?
                    .to_string(),
                npc_token: token.clone(),
            }),
            AuthSource::Google(google) => Ok(ClientMessage::Authenticate {
                google_id_token: google.id_token().await?,
            }),
        }
    }

    /// Whether the agent may delete characters that don't match its config.
    /// Only an npc_token account exists solely for the fixture; a Google
    /// account belongs to a person (`doc/REMOTE_AGENT_CLIENT.md`).
    fn may_delete_mismatches(&self) -> bool {
        match self {
            AuthSource::NpcToken(_) => true,
            AuthSource::Google(_) => false,
        }
    }
}

/// The character a `[[npcs]]` entry asks for; `None` fields are "don't care".
struct Desired {
    name: Option<String>,
    class: Option<CharacterClass>,
    gender: Option<Gender>,
}

impl Desired {
    fn differs_beyond_name(&self, c: &Character) -> bool {
        self.class.as_ref().is_some_and(|d| c.class != *d)
            || self.gender.is_some_and(|gender| c.gender != gender)
    }

    /// Split the account's characters into this entry's and the rest.
    ///
    /// Without deletion the name alone decides: names are globally unique
    /// server-side, so disowning a name match with the wrong class would leave
    /// the entry unable to either enter it or create a replacement.
    fn partition(
        &self,
        may_delete: bool,
        characters: Vec<Character>,
    ) -> (Vec<Character>, Vec<Character>) {
        characters.into_iter().partition(|c| {
            self.name.as_deref().is_none_or(|n| c.name == n)
                && !(may_delete && self.differs_beyond_name(c))
        })
    }
}

/// Run the orchestrator: spawn all NPC sessions in parallel.
pub async fn run_orchestrator(
    server_url: String,
    npcs: Vec<NpcConfig>,
    shared: Arc<SharedResources>,
) -> anyhow::Result<()> {
    info!(
        "Orchestrator starting with {} NPC connection(s)",
        npcs.len()
    );

    let mut handles = Vec::new();
    for (i, npc) in npcs.into_iter().enumerate() {
        let url = server_url.clone();
        let shared = Arc::clone(&shared);
        let handle = tokio::spawn(async move {
            info!("[NPC {}] Starting session loop for '{}'", i, npc.label());
            run_npc_loop(&url, i, &npc, &shared).await;
        });
        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.await;
    }

    Ok(())
}

/// Reconnect loop for a single NPC. `index` is its position in the config,
/// which is how the spectator panel keys feeds — labels can collide.
async fn run_npc_loop(server_url: &str, index: usize, npc: &NpcConfig, shared: &SharedResources) {
    let label = npc.label();
    let watch = shared.watch.as_ref().and_then(|h| h.handle_at(index));
    let mut attempt = 0u32;
    loop {
        match run_npc_session(server_url, npc, shared, watch.as_ref()).await {
            Ok(uptime) => {
                // A kicked session (duplicate login) ends "cleanly" seconds
                // after entry; resetting on it pins the delay at the base and
                // two clients on one account kick each other forever.
                if uptime >= ws::HEALTHY_SESSION {
                    attempt = 0;
                }
                info!("[{label}] Session ended cleanly.");
            }
            Err(e) => {
                // A refused login stays refused: reconnecting would just spin
                // and bury the reason (e.g. "protocol vN required, update").
                if let Some(rejection) = e.downcast_ref::<ws::AuthRejected>() {
                    error!("[{label}] {rejection} — giving up");
                    if let Some(w) = &watch {
                        w.push("system", format!("{rejection} — giving up"));
                    }
                    return;
                }
                warn!("[{label}] Session failed: {e}");
            }
        }
        let delay = ws::retry_delay(attempt);
        attempt = attempt.saturating_add(1);
        info!("[{label}] Reconnecting in {:.1}s...", delay.as_secs_f32());
        if let Some(w) = &watch {
            w.push(
                "system",
                format!(
                    "Connection lost — reconnecting in {:.1}s",
                    delay.as_secs_f32()
                ),
            );
        }
        tokio::time::sleep(delay).await;
    }
}

/// Run a single game session for one NPC: connect, authenticate, enter game,
/// run until disconnected. Returns the in-game uptime — time spent connecting
/// or authenticating (with their own retries) doesn't count as session health.
async fn run_npc_session(
    server_url: &str,
    npc: &NpcConfig,
    shared: &SharedResources,
    watch: Option<&Arc<crate::watch::NpcWatch>>,
) -> anyhow::Result<Duration> {
    let label = npc.label();
    let watch = watch.cloned();
    let ws_stream = ws::connect_ws(server_url, label).await;
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    ws::send_client_info(&mut ws_tx).await?;

    // --- Authentication (server auto-creates the account on first use) ---
    let auth = shared
        .auth
        .authenticate_message(npc.account.as_deref())
        .await?;

    // Not fatal on its own: a refused handshake may already have closed us,
    // and then the reason is waiting on the read side for `wait_for_auth`.
    if let Err(e) = ws::send(&mut ws_tx, &auth).await {
        warn!("[{label}] Auth send failed ({e}); reading the server's reason");
    }

    let characters = ws::wait_for_auth(&mut ws_rx, label).await?;

    // --- Resolve which of the account's characters this NPC entry owns ---
    let desired = Desired {
        name: npc.character_name.clone(),
        class: npc
            .character_class
            .as_deref()
            .map(|c| {
                c.parse::<CharacterClass>().map_err(|_| {
                    anyhow::anyhow!("invalid character_class {c:?} in config for [{}]", label)
                })
            })
            .transpose()?,
        gender: npc.gender,
    };

    let may_delete = shared.auth.may_delete_mismatches();
    let (mut characters, others) = desired.partition(may_delete, characters);

    if may_delete {
        for c in &others {
            info!(
                "[{}] Deleting character '{}' (id={}, {:?}, {:?}) — mismatch (want name={:?}, class={:?}, gender={:?})",
                label, c.name, c.id, c.class, c.gender, desired.name, desired.class, desired.gender
            );
            ws::send(
                &mut ws_tx,
                &ClientMessage::DeleteCharacter { character_id: c.id },
            )
            .await?;
            ws::wait_for_msg(&mut ws_rx, label, "CharacterDeleted", |msg| {
                matches!(
                    msg,
                    ServerMessage::CharacterDeleted { .. } | ServerMessage::CharacterError { .. }
                )
            })
            .await?;
        }
    } else {
        if !others.is_empty() {
            info!(
                "[{}] Leaving {} other character(s) on this account untouched",
                label,
                others.len()
            );
        }
        if let Some(c) = characters
            .first()
            .filter(|c| desired.differs_beyond_name(c))
        {
            warn!(
                "[{}] '{}' is {:?}/{:?} but config wants {:?}/{:?} — entering as-is",
                label, c.name, c.class, c.gender, desired.class, desired.gender
            );
        }
    }

    // --- Auto-create character if needed ---
    if characters.is_empty() {
        if let Some(ref char_name) = npc.character_name {
            let class = desired.class.unwrap_or(CharacterClass::Knight);
            let gender = desired.gender.unwrap_or_default();

            info!(
                "[{}] No characters found. Creating '{}' ({:?}, {:?})...",
                label, char_name, class, gender
            );

            // Registry NPCs reroll by a fixed class heuristic instead of
            // asking their LLM; only a user's own character shops around
            // by taste. Either way it is the same reroll button a player
            // gets — the server never hands out custom numbers.
            let agent = npc
                .id
                .is_none()
                .then(|| build_llm_backend(npc, watch.clone(), shared))
                .flatten();
            roll_stats_with_agent(
                &mut ws_tx,
                &mut ws_rx,
                label,
                &class,
                gender,
                agent.as_ref(),
                &shared.scheduler,
            )
            .await?;

            // Create character
            ws::send(
                &mut ws_tx,
                &ClientMessage::CreateCharacter {
                    character_name: char_name.clone(),
                    character_class: class,
                    gender,
                },
            )
            .await?;
            let created = ws::wait_for_msg(&mut ws_rx, label, "CharacterCreated", |msg| {
                matches!(
                    msg,
                    ServerMessage::CharacterCreated { .. } | ServerMessage::CharacterError { .. }
                )
            })
            .await?;
            match created {
                ServerMessage::CharacterCreated { character } => {
                    info!(
                        "[{}] Created character '{}' (id={}, {:?}, {:?})",
                        label, character.name, character.id, character.class, character.gender
                    );
                    characters.push(character);
                }
                ServerMessage::CharacterError { message } => {
                    anyhow::bail!("[{}] Failed to create character: {message}", label);
                }
                _ => unreachable!(),
            }
        }
    }

    let llm_enabled = npc.llm != LlmType::None;
    let entering = llm_enabled
        .then(|| characters.first())
        .flatten()
        .map(|c| (c.id, c.name.clone()));

    let buffered = match entering {
        Some((char_id, name)) => {
            enter_game(
                &mut ws_tx,
                &mut ws_rx,
                label,
                char_id,
                &name,
                npc.character_name.as_deref(),
            )
            .await?
        }
        None => Vec::new(),
    };

    let entered = Instant::now();
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<ClientMessage>(32);
    let state = Arc::new(Mutex::new(SharedState::new(
        characters,
        cmd_tx,
        Arc::clone(&shared.height_sampler),
        Arc::clone(&shared.splat_sampler),
        Arc::clone(&shared.world_cache),
        watch.clone(),
    )));
    {
        let mut s = state.lock().await;
        s.always_sprint = npc.always_sprint();
        s.plays_music = npc.plays_music();
        s.keepsake_ids = npc
            .id
            .as_deref()
            .and_then(crate::shop_info::npc_by_id)
            .map(|row| row.offerable_keepsake_ids().map(String::from).collect())
            .unwrap_or_default();
    }
    if let Some(w) = &watch {
        w.set_state(Arc::clone(&state));
        w.push("system", "Session connected".to_string());
    }

    let account_for_tx = label.to_string();
    let tx_task = tokio::spawn(async move {
        while let Some(msg) = cmd_rx.recv().await {
            if let Err(e) = ws::send(&mut ws_tx, &msg).await {
                error!("[{}] Failed to send command: {e}", account_for_tx);
                break;
            }
        }
    });

    let state_for_rx = Arc::clone(&state);
    let account_for_rx = label.to_string();
    let rx_task = tokio::spawn(async move {
        for msg in buffered {
            handle_incoming(&state_for_rx, &account_for_rx, msg).await;
        }
        loop {
            match ws::recv(&mut ws_rx).await {
                Ok(msg) => handle_incoming(&state_for_rx, &account_for_rx, msg).await,
                Err(e) => {
                    error!("[{}] Connection lost: {e}", account_for_rx);
                    break;
                }
            }
        }
    });

    let llm_task = spawn_llm_task(npc, &state, shared, server_url, watch.clone());

    // Monster AI tick task (1Hz)
    let state_for_ai = Arc::clone(&state);
    let trees_for_ai = Arc::clone(&shared.behavior_trees);
    let mapping_for_ai = Arc::clone(&shared.type_mapping);
    let movement_for_ai = Arc::clone(&shared.movement_speeds);
    let ai_task = tokio::spawn(async move {
        let tick_interval = Duration::from_secs(1);
        let mut interval = tokio::time::interval(tick_interval);
        let delta_ms = 1000.0_f32;

        {
            let mut s = state_for_ai.lock().await;
            s.monster_ai.set_behavior_trees((*trees_for_ai).clone());
            s.monster_ai.set_type_mapping((*mapping_for_ai).clone());
            s.monster_ai.set_movement_speeds((*movement_for_ai).clone());
        }

        loop {
            interval.tick().await;
            let mut s = state_for_ai.lock().await;
            if !s.in_game {
                continue;
            }
            s.check_music_finished();

            // Clone Arc to avoid borrow conflict: world_cache (immutable) vs monster_ai (mutable).
            // Must drop the RwLockReadGuard before any .await (not Send).
            let (commands, pending) = {
                let wc = Arc::clone(&s.world_cache);
                let world = wc.read().unwrap();
                // Deliberately the plain conversion, not `passability_floor()`
                // (stair-shaft aware): brains set `path_floor` with the same
                // plain mapping, and the gate must match it.
                let self_pass_floor =
                    onlinerpg_shared::dungeon::passability_floor_for_level(s.self_floor_level);
                // A force-move burst leaves our optimistic position pointing
                // at the destination; hide self until the server catches up.
                let pose_settled = s.self_pose_settled();
                let SharedState {
                    ref nearby_players,
                    ref nearby_monsters,
                    ref self_player,
                    ref mut monster_ai,
                    ..
                } = *s;
                let cmds = monster_ai.tick_all(
                    delta_ms,
                    nearby_players,
                    nearby_monsters,
                    self_player.as_ref().filter(|_| pose_settled),
                    self_pass_floor,
                    world.passability_cache(),
                );
                drop(world);
                let pending = s.drain_pending_commands();
                (cmds, pending)
            };

            for cmd in commands.into_iter().chain(pending) {
                if let Err(e) = s.send_background_command(cmd).await {
                    tracing::warn!("Monster AI command failed: {e}");
                    break;
                }
            }
        }
    });

    if llm_enabled {
        info!("[{}] Running in LLM-driven mode", label);
    } else {
        info!("[{}] Running in direct mode", label);
    }

    // Wait until the WebSocket reader dies (connection lost)
    let _ = rx_task.await;

    tx_task.abort();
    ai_task.abort();
    if let Some(t) = llm_task {
        t.abort();
    }
    if let Some(w) = &watch {
        w.set_disconnected();
    }

    Ok(entered.elapsed())
}

/// One server message into shared state. The entry handshake reads a few
/// before the reader task starts, so both paths go through here.
async fn handle_incoming(state: &Arc<Mutex<SharedState>>, label: &str, msg: ServerMessage) {
    let mut s = state.lock().await;
    if matches!(msg, ServerMessage::GameTimeSync { .. }) {
        let _ = s.send_background_command(ClientMessage::Heartbeat).await;
        s.push_event(msg);
        return;
    }

    // Relocations land on a configured Y, not the terrain's. Asked before
    // `push_event`, which is where `JoinSuccess` sets `self_player_id`.
    let needs_height_sync = match &msg {
        ServerMessage::JoinSuccess { .. } => true,
        ServerMessage::PlayerRespawned { player } => s.self_player_id == Some(player.id),
        ServerMessage::PlayerTeleported { player_id, .. } => s.self_player_id == Some(*player_id),
        _ => false,
    };

    s.push_event(msg);

    if needs_height_sync {
        if let Err(e) = s.sync_height().await {
            warn!("[{label}] Failed to sync height after relocation: {e}");
        }
    }
}

/// The name to answer a rename demand with. Deriving one from the refused name
/// would only be a way around the ban, so the operator's configured
/// `character_name` is the only answer — and None means there is none.
fn rename_target<'a>(current: &str, configured: Option<&'a str>) -> Option<&'a str> {
    configured.filter(|name| !name.eq_ignore_ascii_case(current))
}

/// Enter the game and settle what the server answers with: `JoinSuccess`, or a
/// demand to rename a banned character first. Anything else that lands
/// meanwhile belongs to the session, so it comes back buffered for the reader
/// to replay in order rather than being dropped.
async fn enter_game(
    ws_tx: &mut ws::WsTx,
    ws_rx: &mut ws::WsRx,
    label: &str,
    char_id: i64,
    current_name: &str,
    configured_name: Option<&str>,
) -> anyhow::Result<Vec<ServerMessage>> {
    let enter = ClientMessage::EnterGame {
        character_id: char_id,
    };
    ws::send(ws_tx, &enter).await?;
    info!("[{label}] Entering game with character {char_id}...");

    let mut buffered = Vec::new();
    let mut renamed = false;
    loop {
        let msg = ws::recv(ws_rx).await?;
        match msg {
            ServerMessage::JoinSuccess { .. } => {
                // No scene to compile here, so the entry grace ends at once.
                ws::send(ws_tx, &ClientMessage::WorldReady).await?;
                buffered.push(msg);
                return Ok(buffered);
            }
            ServerMessage::CharacterRenameRequired { character_id } => {
                if renamed {
                    // One rename per connection: a second demand means the new
                    // name is refused too, and asking again would spin.
                    return Err(ws::AuthRejected(
                        "renamed the character, but entry was refused again".to_string(),
                    )
                    .into());
                }
                let Some(new_name) = rename_target(current_name, configured_name) else {
                    return Err(ws::AuthRejected(format!(
                        "'{current_name}' cannot be used on this server; set a different \
                         character_name in config.toml and restart"
                    ))
                    .into());
                };
                warn!("[{label}] Entry refused as '{current_name}' — renaming to '{new_name}'");
                ws::send(
                    ws_tx,
                    &ClientMessage::RenameCharacter {
                        character_id,
                        new_name: new_name.to_string(),
                    },
                )
                .await?;
                renamed = true;
            }
            ServerMessage::CharacterRenamed { name, .. } => {
                info!("[{label}] Renamed to '{name}'");
                ws::send(ws_tx, &enter).await?;
            }
            ServerMessage::CharacterError { message } => {
                return Err(ws::AuthRejected(format!("entry refused: {message}")).into());
            }
            other => buffered.push(other),
        }
    }
}

impl NpcConfig {
    /// Get the backend-specific system_prompt_file path.
    fn system_prompt_file(&self) -> Option<&str> {
        match &self.llm {
            LlmType::Claude => Some(&self.claude.system_prompt_file),
            LlmType::Openrouter => Some(&self.openrouter.system_prompt_file),
            LlmType::Codex => Some(&self.codex.system_prompt_file),
            LlmType::Openai => Some(&self.openai.system_prompt_file),
            LlmType::None => None,
        }
    }
}

/// Ceiling on stat rolls at character creation. A backstop against a prompt
/// that never says yes — how picky to be is the agent's own business, and
/// belongs in its prompt.
const MAX_STAT_ROLLS: u32 = 20;

/// Roll starting stats, letting the agent accept or reroll each result the
/// way a human works the web client's reroll button. Whatever is on the table
/// when it accepts — or when the rolls run out — is what it plays. Without an
/// LLM a fixed class heuristic does the accepting instead.
///
/// The decisions go through the scheduler like any other call, so a fleet
/// starting up with fresh accounts still honours `max_concurrent`.
async fn roll_stats_with_agent(
    ws_tx: &mut ws::WsTx,
    ws_rx: &mut ws::WsRx,
    label: &str,
    class: &CharacterClass,
    gender: Gender,
    agent: Option<&Arc<dyn driver::LlmBackend>>,
    scheduler: &LlmScheduler,
) -> anyhow::Result<()> {
    for attempt in 1..=MAX_STAT_ROLLS {
        ws::send(
            ws_tx,
            &ClientMessage::RollCharacterStats {
                character_class: class.clone(),
                gender,
            },
        )
        .await?;
        let rolled = ws::wait_for_msg(ws_rx, label, "CharacterStatsRolled", |msg| {
            matches!(msg, ServerMessage::CharacterStatsRolled { .. })
        })
        .await?;
        let ServerMessage::CharacterStatsRolled {
            attributes: a,
            max_hp,
        } = rolled
        else {
            unreachable!("wait_for_msg only returns CharacterStatsRolled here");
        };
        info!(
            "[{label}] Roll {attempt}/{MAX_STAT_ROLLS}: STR {} DEX {} CON {} INT {} WIS {} CHA {} \
             guard {} HP {max_hp}",
            a.r#str, a.dex, a.con, a.int, a.wis, a.cha, a.guard
        );

        // Agentless: the loop bound keeps the last roll if none ever fits.
        let Some(agent) = agent else {
            if roll_fits_class(&a, class, gender) {
                return Ok(());
            }
            continue;
        };
        let left = MAX_STAT_ROLLS - attempt;
        if left == 0 {
            warn!("[{label}] Out of rerolls — keeping this roll");
            return Ok(());
        }

        let question = format!(
            "[CharacterCreation] You are rolling up your {class:?}, before entering the world \
             for the first time. This roll: STR {} DEX {} CON {} INT {} WIS {} CHA {}, \
             guard {}, max HP {max_hp}. The six stats always sum to 72, and once you enter \
             the world they are fixed for good. Decide what to do with this roll; you may \
             be offered {left} more.",
            a.r#str, a.dex, a.con, a.int, a.wis, a.cha, a.guard
        );
        let decision = scheduler
            .submit(label, LlmPriority::Routine, question, Arc::clone(agent))
            .await;
        match decision {
            Ok(reply) if driver::wants_reroll(&reply) => continue,
            Ok(_) => {
                info!("[{label}] Agent accepted roll {attempt}");
                return Ok(());
            }
            Err(e) => {
                warn!("[{label}] Could not ask about the roll ({e}) — keeping it");
                return Ok(());
            }
        }
    }
    Ok(())
}

/// The agentless acceptance bar: every attribute the class favours (positive
/// stat adjustment) reached 14. A guard rerolls until STR and CON can carry
/// a fight; a class with no favourites takes the first roll.
fn roll_fits_class(a: &CharacterAttributes, class: &CharacterClass, gender: Gender) -> bool {
    const KEY_STAT_MIN: u8 = 14;
    let values = [a.r#str, a.dex, a.con, a.int, a.wis, a.cha];
    class
        .stat_adjustments(gender)
        .iter()
        .zip(values)
        .filter(|(adj, _)| **adj > 0)
        .all(|(_, value)| value >= KEY_STAT_MIN)
}

/// Role prompt for agents with no class template — the plain player agent's
/// own layer, mirroring `data/templates/{class}.txt` for registry NPCs.
/// Optional: agents run fine on the shared prompt alone.
const USER_PROMPT_FILE: &str = "data/user_prompt.txt";

/// The songs a bard may call up, straight from the registry so the prompt
/// cannot drift from what `/play_music` will resolve.
fn songbook_prompt() -> String {
    let mut section = String::from(
        "## Your Songbook\nEvery tune you know. Use a title exactly as written here:\n",
    );
    for title in crate::bgm_defs::songbook() {
        section.push_str(&format!("- {title}\n"));
    }
    section
}

/// Build the system prompt for an NPC by layering, outermost first.
///
/// Every agent starts from the shared prompt — the JSON schema and the action
/// types they all speak — so a new action is documented in one place. Its role
/// goes on top (a registry NPC's class template, else `user_prompt.txt`),
/// then its personality, shop knowledge and memories.
fn build_system_prompt(npc: &NpcConfig) -> anyhow::Result<String> {
    let role = npc.template_prompt.as_deref().or_else(|| {
        std::path::Path::new(USER_PROMPT_FILE)
            .exists()
            .then_some(USER_PROMPT_FILE)
    });
    let files: Vec<&str> = npc
        .system_prompt_file()
        .into_iter()
        .chain(role)
        .chain(npc.instance_prompt.as_deref())
        .collect();

    let mut parts = files
        .iter()
        .map(|path| driver::load_system_prompt(path))
        .collect::<anyhow::Result<Vec<_>>>()?;

    // Merchants get their catalog and prices for roleplay; the server
    // re-validates every trade and haggle. Resident traders get their
    // wishlist per turn instead (driver/prompt.rs) so it can satiate.
    if let Some(shop) = npc
        .character_name
        .as_deref()
        .and_then(crate::shop_info::merchant_prompt_for)
    {
        parts.push(shop);
    }
    // A bard announces the song before playing it, so it needs the titles in
    // front of it — both to pick one and to match a listener's request.
    if npc.plays_music() {
        parts.push(songbook_prompt());
    }
    // Memories are deliberately NOT baked in here: the driver re-reads the
    // memory file into every prompt (load_memory_tail), so notes written
    // mid-session reach even a stateless backend.

    info!("[{}] Prompt layers: {}", npc.label(), files.join(" + "));
    Ok(parts.join("\n\n"))
}

/// Build the configured LLM backend, already carrying the layered system
/// prompt. `None` when the agent runs without an LLM, or when the provider
/// could not be set up.
fn build_llm_backend(
    npc: &NpcConfig,
    watch: Option<Arc<crate::watch::NpcWatch>>,
    shared: &SharedResources,
) -> Option<Arc<dyn driver::LlmBackend>> {
    let label = npc.label();
    let system_prompt = match build_system_prompt(npc) {
        Ok(p) => p,
        Err(e) => {
            error!("[{label}] Failed to build system prompt: {e}");
            return None;
        }
    };

    let (provider, model, invoker) = match npc.llm {
        LlmType::Claude => (
            "Claude CLI",
            &npc.claude.model,
            claude::ClaudeInvoker::new(&npc.claude, system_prompt)
                .map(|i| Arc::new(i) as Arc<dyn driver::LlmBackend>),
        ),
        LlmType::Openrouter => (
            "OpenRouter API",
            &npc.openrouter.model,
            openrouter::invoker(&npc.openrouter, system_prompt)
                .map(|i| Arc::new(i) as Arc<dyn driver::LlmBackend>),
        ),
        LlmType::Codex => (
            "Codex app-server",
            &npc.codex.model,
            codex::CodexInvoker::new(&npc.codex, system_prompt, shared.codex_app_server.clone())
                .map(|i| Arc::new(i) as Arc<dyn driver::LlmBackend>),
        ),
        LlmType::Openai => (
            "OpenAI-compatible API",
            &npc.openai.model,
            npc.openai.endpoint().map(|ep| {
                Arc::new(openai::OpenAiInvoker::new(ep, system_prompt))
                    as Arc<dyn driver::LlmBackend>
            }),
        ),
        LlmType::None => return None,
    };

    match invoker {
        Ok(inv) => {
            info!("[{label}] {provider} integration enabled (model={model})");
            // Timeout under the watcher, so a giving-up call still lands on the
            // panel as an error instead of a prompt with no answer.
            let inv = TimeoutBackend::wrap(inv, shared.scheduler.request_timeout());
            let inv =
                crate::transcript::TranscriptBackend::wrap(inv, shared.transcript.clone(), label);
            Some(crate::watch::WatchedBackend::wrap(inv, watch))
        }
        Err(e) => {
            error!("[{label}] Failed to create {provider} invoker: {e}");
            None
        }
    }
}

/// WS URL → REST base URL: an explicit port means game port + 1; otherwise
/// same origin with the path dropped (the reverse proxy routes `/api/*`).
fn api_base_url(server_url: &str) -> String {
    let (scheme, rest) = server_url.split_once("://").unwrap_or(("ws", server_url));
    let scheme = if scheme == "wss" { "https" } else { "http" };
    let authority = rest.split('/').next().unwrap_or(rest);
    match authority
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().ok().map(|p| (host, p)))
    {
        Some((host, port)) => format!("{scheme}://{host}:{}", port + 1),
        None => format!("{scheme}://{authority}"),
    }
}

/// Spawn the appropriate LLM driver task based on NPC config.
fn spawn_llm_task(
    npc: &NpcConfig,
    state: &Arc<Mutex<SharedState>>,
    shared: &SharedResources,
    server_url: &str,
    watch: Option<Arc<crate::watch::NpcWatch>>,
) -> Option<tokio::task::JoinHandle<()>> {
    let scheduler = &shared.scheduler;
    let label = npc.label();
    let min_interval = Duration::from_secs(npc.min_interval_secs);
    let urgent_min_interval = Duration::from_secs(npc.urgent_min_interval_secs);
    let debounce = Duration::from_secs(npc.debounce_secs);
    let idle_interval = Duration::from_secs(npc.idle_interval_secs);
    let activity_window = Duration::from_secs(npc.activity_window_secs);

    let invoker = build_llm_backend(npc, watch, shared)?;

    let state = Arc::clone(state);
    let scheduler = scheduler.clone();
    let schedule = load_json_file::<ScheduleFile>(npc.schedule_file.as_deref(), label)
        .map(|mut f| {
            let errors = parse_conditions(&mut f.schedule);
            for e in &errors {
                error!("[{label}] Schedule entry error: {e}");
            }
            if errors.is_empty() {
                info!("[{label}] Loaded {} schedule entries", f.schedule.len());
                f.schedule
            } else {
                Vec::new()
            }
        })
        .unwrap_or_default();

    let sickroom = load_json_file::<SpotsFile>(npc.sickroom_file.as_deref(), label)
        .map(|f| f.spots)
        .unwrap_or_default();

    let tables = load_json_file::<SpotsFile>(npc.tables_file.as_deref(), label)
        .map(|f| f.spots)
        .unwrap_or_default();

    let api_base_url = api_base_url(server_url);

    let driver_config = driver::DriverConfig {
        label: label.to_string(),
        memory_file: npc.memory_file.clone(),
        favor_file: npc.favor_file.clone(),
        min_interval,
        urgent_min_interval,
        debounce,
        idle_interval,
        activity_window,
        always_active: npc.always_active(),
        schedule,
        sickroom,
        serve_tables: npc.serve_tables.unwrap_or(false),
        maid_names: shared.maid_names.clone(),
        tables,
        claims: Arc::clone(&shared.claims),
        api_base_url,
    };
    Some(tokio::spawn(async move {
        driver::llm_driver(state, invoker, scheduler, driver_config).await;
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use onlinerpg_shared::CharacterAttributes;

    fn schedule(at: &str) -> ScheduleEntry {
        ScheduleEntry {
            at: at.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn schedule_times_stay_within_clock_bounds() {
        for value in ["0:00", "23:59", "*:00", "*:59", "day", "night"] {
            let mut entry = schedule(value);
            assert!(entry.parse_condition().is_ok(), "{value} should be valid");
            assert!(entry.condition.is_some());
        }

        for value in ["24:00", "25:30", "12:60", "*:60", "*:99"] {
            let mut entry = schedule(value);
            assert!(
                entry.parse_condition().is_err(),
                "{value} should be rejected"
            );
            assert!(entry.condition.is_none());
        }
    }

    fn character(name: &str, class: CharacterClass, gender: Gender) -> Character {
        Character {
            id: 1,
            name: name.to_string(),
            created_at: 0,
            level: 1,
            xp: 0,
            max_hp: 10,
            attributes: CharacterAttributes {
                r#str: 10,
                dex: 10,
                con: 10,
                int: 10,
                wis: 10,
                cha: 10,
                guard: 0,
            },
            class,
            gender,
            equipment: Default::default(),
            titles: Vec::new(),
            active_title: None,
        }
    }

    fn desired(name: &str, class: CharacterClass) -> Desired {
        Desired {
            name: Some(name.to_string()),
            class: Some(class),
            gender: None,
        }
    }

    #[test]
    fn a_guard_rerolls_until_str_and_con_carry_a_fight() {
        let attrs = |str_: u8, con: u8| CharacterAttributes {
            r#str: str_,
            dex: 10,
            con,
            int: 10,
            wis: 10,
            cha: 10,
            guard: 10,
        };
        let class = CharacterClass::Guard;
        assert!(roll_fits_class(&attrs(14, 14), &class, Gender::Male));
        assert!(!roll_fits_class(&attrs(15, 13), &class, Gender::Male));
        assert!(!roll_fits_class(&attrs(11, 16), &class, Gender::Male));
    }

    #[test]
    fn npc_token_deletes_every_mismatch() {
        let chars = vec![
            character("Delver", CharacterClass::Rogue, Gender::Male),
            character("Someone", CharacterClass::Rogue, Gender::Male),
            character("Delver", CharacterClass::Knight, Gender::Male),
        ];
        let (mine, others) = desired("Delver", CharacterClass::Rogue).partition(true, chars);
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].class, CharacterClass::Rogue);
        assert_eq!(others.len(), 2);
    }

    #[test]
    fn without_deletion_the_name_decides() {
        let chars = vec![
            character("Ryulamg", CharacterClass::Rogue, Gender::Male),
            character("RyuK", CharacterClass::Ranger, Gender::Male),
        ];
        let (mine, others) = desired("Ryulamg", CharacterClass::Rogue).partition(false, chars);
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].name, "Ryulamg");
        assert_eq!(others.len(), 1);
    }

    /// The name is globally unique, so a wrong class must not send the agent
    /// into a create-fails-forever loop.
    #[test]
    fn a_name_match_with_the_wrong_class_is_still_mine() {
        let want = desired("Ryulamg", CharacterClass::Knight);
        let chars = vec![character("Ryulamg", CharacterClass::Rogue, Gender::Male)];
        let (mine, others) = want.partition(false, chars);
        assert_eq!(mine.len(), 1);
        assert!(others.is_empty());
        assert!(want.differs_beyond_name(&mine[0]));
    }

    #[test]
    fn nothing_of_mine_means_create_one() {
        let chars = vec![character("Someone", CharacterClass::Rogue, Gender::Male)];
        let (mine, others) = desired("Ryulamg", CharacterClass::Rogue).partition(false, chars);
        assert!(mine.is_empty());
        assert_eq!(others.len(), 1);
    }

    #[test]
    fn npc_token_may_delete() {
        assert!(AuthSource::NpcToken("t".to_string()).may_delete_mismatches());
    }

    #[test]
    fn direct_ws_url_bumps_the_port() {
        assert_eq!(
            api_base_url("ws://127.0.0.1:10006"),
            "http://127.0.0.1:10007"
        );
    }

    #[test]
    fn proxied_wss_url_keeps_the_origin_and_drops_the_path() {
        assert_eq!(
            api_base_url("wss://openmmo.example/ws"),
            "https://openmmo.example"
        );
    }

    #[test]
    fn wss_url_with_port_and_path_bumps_the_port() {
        assert_eq!(
            api_base_url("wss://openmmo.example:10006/ws"),
            "https://openmmo.example:10007"
        );
    }

    #[test]
    fn a_rename_demand_is_answered_from_the_configured_name() {
        assert_eq!(rename_target("시스템", Some("인공지능")), Some("인공지능"));
    }

    #[test]
    fn a_config_that_cannot_answer_has_no_target() {
        // Offering the refused name back — in any case — is refused again.
        assert_eq!(rename_target("시스템", None), None);
        assert_eq!(rename_target("시스템", Some("시스템")), None);
        assert_eq!(rename_target("SYSTEM", Some("system")), None);
    }
}
