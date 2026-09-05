mod announcements;
mod api_auth;
mod auth;
mod banned_names;
mod bgm_defs;
mod cape_texture;
mod cape_texture_routes;
mod conn_limit;
mod connection;
mod debuff_defs;
mod dungeon_defs;
mod game;
mod game_state;
mod google_auth;
mod housing;
mod item_defs;
mod land_grades;
mod merchant_defs;
mod monster_defs;
mod npc_defs;
mod npc_schedule;
mod semicolon_list;
mod terrain;
#[cfg(test)]
mod test_util;
mod title_defs;
mod types;
mod world_config;
mod world_drop_defs;

use announcements::{announcements_router, AnnouncementStore};
use auth::AuthService;
use cape_texture::CapeTextureStore;
use cape_texture_routes::cape_texture_router;
use clap::Parser;
use conn_limit::ConnectLimiter;
use connection::{handle_connection, AuthContext, ServerContext};
use futures_util::FutureExt;
use game_state::GameState;
use google_auth::GoogleAuthVerifier;
use housing::routes::housing_router;
use housing::HousingIO;
use npc_schedule::routes::npc_router;
use npc_schedule::NpcIO;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use terrain::io::TerrainIO;
use terrain::routes::terrain_router;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio::time::{Duration, Instant};
use tower_http::compression::CompressionLayer;
use tracing::{error, info, warn};
use types::GameDateTime;

const SHUTDOWN_NOTICE: &str = "The server is shutting down. Please reconnect shortly.";
const SHUTDOWN_NOTICE_DURATION: Duration = Duration::from_secs(2);

fn load_initial_game_time(auth_service: &AuthService) -> Result<GameDateTime, auth::AuthError> {
    let (source, datetime) = match auth_service.load_world_time()? {
        Some(saved) => ("Loaded game time from DB", saved),
        None => {
            let initial = GameState::default_start_datetime();
            if let Err(err) = auth_service.save_world_time(&initial) {
                warn!("Failed to persist initial game time: {}", err);
            }
            ("Initialized game time", initial)
        }
    };
    info!("{}: {}", source, datetime);
    Ok(datetime)
}

/// Catches and logs a panic from one tick round so the loop task survives.
async fn guard_tick(name: &str, tick: impl std::future::Future<Output = ()>) {
    if std::panic::AssertUnwindSafe(tick)
        .catch_unwind()
        .await
        .is_err()
    {
        error!("{name} tick panicked; loop continues");
    }
}

/// Runs `tick` every `period` until shutdown fires. Owning the loop is what
/// makes every background task drainable: a task spawned any other way would
/// never break, and the shutdown drain would hang on it forever.
async fn run_ticks<F, Fut>(
    name: &'static str,
    period: Duration,
    mut shutdown: watch::Receiver<()>,
    mut tick: F,
) where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let mut interval = tokio::time::interval(period);
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            _ = interval.tick() => guard_tick(name, tick()).await,
        }
    }
}

async fn time_sync_tick(game_state: &GameState, auth_service: &Arc<AuthService>, tick_count: u64) {
    // Regenerate player health every 2 ticks (16 seconds)
    if tick_count.is_multiple_of(2) {
        game_state.tick_regeneration().await;
    }

    // Count down trade-window holds; releases an NPC ~32s (4 ticks)
    // after a customer opened its window, even if still open.
    game_state.tick_shop_holds().await;

    // Close player trades both sides went quiet on.
    game_state.sweep_player_trades().await;

    // Disconnect clients grinding walls their build invented (stale dungeon
    // layout). Queued by the movement tick, which has no AuthService.
    game_state.drain_stale_layout_kicks(auth_service).await;

    // Pay NPC trader salaries on game-day rollover (economy phase 3)
    game_state.tick_npc_salaries().await;

    // Merchants' price meeting on Serin's dark evening (doc/PRICING.md).
    game_state.tick_pricing_meeting(auth_service).await;

    // Sunset closes the dungeon day: evict occupants, wake the guardians.
    game_state.tick_dungeon_reset().await;

    // Batch-save dirty character states and inventories every 4 ticks (32s)
    if tick_count.is_multiple_of(4) {
        game_state.flush_dirty_saves(auth_service).await;
        game_state.tick_gold_snapshot(auth_service).await;
    }

    let datetime = game_state.broadcast_game_time();
    if let Err(err) = auth_service.save_world_time(&datetime) {
        warn!("Failed to persist game time: {}", err);
    }
}

async fn wait_for_shutdown(mut shutdown: watch::Receiver<()>) {
    let _ = shutdown.changed().await;
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(signal) => signal,
        Err(e) => {
            error!("Failed to install SIGTERM handler: {}", e);
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };

    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(e) = result {
                error!("Failed to listen for Ctrl+C: {}", e);
            }
        }
        _ = terminate.recv() => {}
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        error!("Failed to listen for Ctrl+C: {}", e);
    }
}

async fn drain(tasks: &mut JoinSet<()>, name: &str) {
    while let Some(result) = tasks.join_next().await {
        if let Err(e) = result {
            error!("{} task failed during shutdown: {}", name, e);
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "onlinerpg-server")]
#[command(about = "MMORPG Game Server", long_about = None)]
struct Args {
    /// Port number to listen on
    #[arg(short, long, default_value_t = 10006)]
    port: u16,

    /// Port for terrain REST API (default: game port + 1)
    #[arg(long)]
    terrain_port: Option<u16>,

    /// Bind address for the game WebSocket port. Loopback by default;
    /// 0.0.0.0 exposes the port with no TLS and no proxy in front.
    #[arg(long, default_value = "127.0.0.1")]
    bind: String,

    /// Bind address for the REST API. Loopback by default: the vite proxy
    /// and local bots are the only intended callers.
    #[arg(long, default_value = "127.0.0.1")]
    api_bind: String,

    /// Comma-separated Google emails allowed to use REST write endpoints
    /// (map editor)
    #[arg(long, env = "ADMIN_EMAILS", default_value = "")]
    admin_emails: String,

    /// Directory for terrain data files
    #[arg(long, env = "TERRAIN_DIR", default_value = "./data/terrain")]
    terrain_dir: String,

    /// Directory for mutable server state: the SQLite DB, the NPC auth token,
    /// housing files and operator announcements. Defaults to the layout the
    /// systemd units expect (CWD = repo root), so existing deploys are
    /// unaffected when the flag is omitted.
    #[arg(long, env = "STATE_DIR", default_value = "./data")]
    state_dir: PathBuf,

    /// Directory holding per-NPC schedule files. The map editor writes here
    /// over REST, so it is server-owned state even though it lives under
    /// `agent-client/` in a source checkout.
    #[arg(long, env = "NPC_DATA_DIR", default_value = "./agent-client/data/npcs")]
    npc_data_dir: PathBuf,

    /// Google OAuth client ID used to verify browser sign-in tokens
    #[arg(long, env = "GOOGLE_CLIENT_ID")]
    google_client_id: Option<String>,

    /// Google OAuth client ID for headless sign-in (agent-client's device
    /// flow). Separate client, so its tokens carry a different `aud`.
    #[arg(long, env = "GOOGLE_CLI_CLIENT_ID")]
    google_cli_client_id: Option<String>,

    /// Shared secret for headless NPC clients (default: <state-dir>/npc_token,
    /// generated on first run). Blank is treated as unset.
    #[arg(long, env = "NPC_AUTH_TOKEN")]
    npc_token: Option<String>,
}

/// Treat blank CLI/env values as absent: compose `.env` files spell an unset
/// optional key as `KEY=`, which clap reports as `Some("")`.
fn optional_value(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|value| !value.is_empty())
}

/// Paths to the mutable server state that `--state-dir` owns.
///
/// Kept as a pure function so the default layout can be asserted in tests:
/// a silent drift here would point a fresh deploy at an empty database.
struct StatePaths {
    db: PathBuf,
    npc_token: PathBuf,
    housing: PathBuf,
    announcements: PathBuf,
    cape_textures: PathBuf,
    banned_names: PathBuf,
}

fn state_paths(state_dir: &Path) -> StatePaths {
    StatePaths {
        db: state_dir.join("game_data.db"),
        npc_token: state_dir.join(onlinerpg_shared::NPC_TOKEN_FILENAME),
        housing: state_dir.join("housing"),
        announcements: state_dir.join("announcements"),
        cape_textures: state_dir.join("cape-textures"),
        banned_names: state_dir.join("banned_names.txt"),
    }
}

/// Read the NPC token file, generating a random one on first run so local
/// bots work with zero config (they read the same file).
fn load_or_create_npc_token(path: &Path) -> std::io::Result<String> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        let existing = existing.trim().to_string();
        if !existing.is_empty() {
            return Ok(existing);
        }
    }

    let token = uuid::Uuid::new_v4().simple().to_string();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &token)?;
    // Shared secret: keep it owner-only so other local users can't read it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    info!("Generated NPC auth token at {}", path.display());
    Ok(token)
}

/// Minimum length for an operator-supplied NPC token; the auto-generated one
/// is 32 hex chars.
const MIN_NPC_TOKEN_LEN: usize = 16;

/// The fantasy tiles live outside git, so a fresh deploy that skips
/// `terrain-gen render-map-world` would silently serve the legacy minimap art.
fn warn_if_fantasy_map_missing(terrain_dir: &Path) {
    let family = onlinerpg_terrain::coords::MinimapFamily::Fantasy;
    let fantasy_dir = terrain_dir.join(family.dir());
    let has_tiles = std::fs::read_dir(&fantasy_dir)
        .map(|entries| {
            entries
                .flatten()
                .any(|e| e.path().extension().is_some_and(|ext| ext == family.ext()))
        })
        .unwrap_or(false);
    if !has_tiles {
        warn!(
            "No fantasy world-map tiles at {} — the world map will fall back to \
             legacy minimap PNGs. Run `terrain-gen render-map-world` to bake them.",
            fantasy_dir.display()
        );
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    world_config::log_world_config();
    let monster_defs = monster_defs::MonsterDefs::load();
    let item_defs = item_defs::item_defs().clone();
    title_defs::validate(&monster_defs, &item_defs);
    let dungeon_defs = dungeon_defs::DungeonDefs::load(&item_defs, &monster_defs);
    let world_drop_defs = world_drop_defs::WorldDropDefs::load(&item_defs);
    let paths = state_paths(&args.state_dir);
    let auth_service = match AuthService::new(paths.db.clone()) {
        Ok(service) => Arc::new(service.with_banned_names(banned_names::load(&paths.banned_names))),
        Err(e) => {
            error!("Failed to initialize auth service: {}", e);
            return ExitCode::FAILURE;
        }
    };

    let google_client_ids: Vec<String> = [&args.google_client_id, &args.google_cli_client_id]
        .into_iter()
        .filter_map(|value| optional_value(value.as_deref()))
        .map(str::to_string)
        .collect();
    let google = if google_client_ids.is_empty() {
        warn!("No --google-client-id / GOOGLE_CLIENT_ID set: Google sign-in disabled");
        None
    } else {
        info!(
            "Google sign-in accepts {} client id(s)",
            google_client_ids.len()
        );
        Some(GoogleAuthVerifier::new(google_client_ids))
    };
    let npc_token = match optional_value(args.npc_token.as_deref()) {
        Some(token) => token.to_string(),
        None => match load_or_create_npc_token(&paths.npc_token) {
            Ok(token) => token,
            Err(e) => {
                error!("failed to load/create NPC token: {e}");
                return ExitCode::FAILURE;
            }
        },
    };
    if npc_token.trim().len() < MIN_NPC_TOKEN_LEN {
        error!(
            "NPC token is shorter than {MIN_NPC_TOKEN_LEN} chars; refusing to start. \
             Unset --npc-token / NPC_AUTH_TOKEN to auto-generate a secure one."
        );
        return ExitCode::FAILURE;
    }
    let admin_emails: Vec<String> = args
        .admin_emails
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if admin_emails.is_empty() {
        warn!("No --admin-emails / ADMIN_EMAILS set: REST writes require the NPC token");
    }
    let auth_ctx = Arc::new(AuthContext {
        google,
        npc_token,
        admin_emails,
    });
    let initial_game_time = match load_initial_game_time(&auth_service) {
        Ok(initial) => initial,
        Err(err) => {
            error!(
                "Failed to load game time from DB; refusing to start: {}",
                err
            );
            return ExitCode::FAILURE;
        }
    };

    let housing_io = Arc::new(HousingIO::new(paths.housing));
    let npc_io = Arc::new(NpcIO::new(args.npc_data_dir.clone()));
    let terrain_io = Arc::new(TerrainIO::new(PathBuf::from(&args.terrain_dir)));
    warn_if_fantasy_map_missing(std::path::Path::new(&args.terrain_dir));
    let announcement_store = Arc::new(AnnouncementStore::new(paths.announcements));
    announcement_store.warm().await;

    // Load no-spawn zones (towns) from per-region zone files. Monster spawn
    // areas come from world.json `ambientSpawns`, not per-region rectangles.
    let no_spawn_zones = match world_config::load_no_spawn_zones_from_regions(&terrain_io).await {
        Ok(zones) => zones,
        Err(err) => {
            error!("Failed to load no-spawn zones; refusing to start: {}", err);
            return ExitCode::FAILURE;
        }
    };

    // Extra TerrainIO handles on the same directory: the samplers want
    // ownership and TerrainIO is only a path handle. Fishing's water check
    // reads both — the heightmap (terrain bed) and the water field (sea +
    // river surface).
    let height_sampler = Arc::new(onlinerpg_terrain::height::HeightSampler::new(
        TerrainIO::new(std::path::PathBuf::from(&args.terrain_dir)),
    ));
    let water_sampler = Arc::new(onlinerpg_terrain::water::WaterSampler::new(TerrainIO::new(
        std::path::PathBuf::from(&args.terrain_dir),
    )));

    let splat_sampler = Arc::new(onlinerpg_terrain::splat::SplatSampler::new(TerrainIO::new(
        std::path::PathBuf::from(&args.terrain_dir),
    )));

    let cape_textures = match CapeTextureStore::new(paths.cape_textures.clone()) {
        Ok(store) => Arc::new(store),
        Err(e) => {
            error!(
                "Failed to open cape texture store at {}: {}",
                paths.cape_textures.display(),
                e
            );
            return ExitCode::FAILURE;
        }
    };

    let game_state = Arc::new(GameState::new(
        monster_defs,
        item_defs,
        world_drop_defs,
        initial_game_time,
        Arc::clone(&housing_io),
        no_spawn_zones,
        dungeon_defs,
        height_sampler,
        water_sampler,
        splat_sampler,
        Arc::clone(&cape_textures),
    ));
    game_state.load_npc_schedules(&npc_io).await;
    game_state.load_pricing(&auth_service).await;
    // Server-side collision data for the movement sim: houses, solid
    // furniture and dungeon layouts, mirroring what clients build.
    if let Err(err) = game_state.init_passability(&terrain_io).await {
        error!(
            "Failed to build passability cache; refusing to start: {}",
            err
        );
        return ExitCode::FAILURE;
    }
    // Stops the listeners, the REST API and every periodic task; connections
    // outlive it so players still see the shutdown notice.
    let (drain_shutdown_tx, drain_shutdown) = watch::channel(());
    let (connection_shutdown_tx, connection_shutdown) = watch::channel(());
    let mut background = JoinSet::new();

    // Player movement simulation: walks pending move intents toward their
    // targets at capped speed (server-authoritative positions, F-006).
    let game_state_for_movement = Arc::clone(&game_state);
    let mut last = std::time::Instant::now();
    background.spawn(run_ticks(
        "player movement",
        Duration::from_millis(200),
        drain_shutdown.clone(),
        move || {
            let now = std::time::Instant::now();
            let dt = (now - last).as_secs_f32();
            last = now;
            let game_state = Arc::clone(&game_state_for_movement);
            async move { game_state.tick_player_movement(dt).await }
        },
    ));

    // Push party positions to members whose party relocated since the last
    // tick; 3s matches the freshness of the world map's former poll.
    let game_state_for_party_positions = Arc::clone(&game_state);
    background.spawn(run_ticks(
        "party positions",
        Duration::from_secs(3),
        drain_shutdown.clone(),
        move || {
            let game_state = Arc::clone(&game_state_for_party_positions);
            async move { game_state.tick_party_positions().await }
        },
    ));

    // Faster than positions: a party HP bar lagging mid-fight reads as wrong.
    let game_state_for_party_vitals = Arc::clone(&game_state);
    background.spawn(run_ticks(
        "party vitals",
        Duration::from_secs(1),
        drain_shutdown.clone(),
        move || {
            let game_state = Arc::clone(&game_state_for_party_vitals);
            async move { game_state.tick_party_vitals().await }
        },
    ));

    // Server-driven monster brains (doc/SERVER_SIDE_MONSTER_AI.md); same
    // cadence as player movement.
    let game_state_for_monster_ai = Arc::clone(&game_state);
    background.spawn(run_ticks(
        "monster ai",
        Duration::from_millis(200),
        drain_shutdown.clone(),
        move || {
            let game_state = Arc::clone(&game_state_for_monster_ai);
            async move { game_state.tick_monster_ai().await }
        },
    ));

    // Safety net behind the event-driven ownership handoff (AOI diff, monster
    // move fanout, adoption on sight): repairs what a race strands and frees
    // monsters nobody can see. The events do the real-time work, so this only
    // needs to be frequent enough that a stranded monster is not one for long.
    let game_state_for_abandoned = Arc::clone(&game_state);
    background.spawn(run_ticks(
        "monster ownership reconcile",
        Duration::from_secs(60),
        drain_shutdown.clone(),
        move || {
            let game_state = Arc::clone(&game_state_for_abandoned);
            async move { game_state.tick_monster_ownership().await }
        },
    ));

    // Dungeon spawn-slot refill tick (respawns on occupied floors)
    let game_state_for_dungeons = Arc::clone(&game_state);
    background.spawn(run_ticks(
        "dungeon refill",
        Duration::from_secs(30),
        drain_shutdown.clone(),
        move || {
            let game_state = Arc::clone(&game_state_for_dungeons);
            async move { game_state.tick_dungeons().await }
        },
    ));

    let game_state_for_ground = Arc::clone(&game_state);
    background.spawn(run_ticks(
        "ground item despawn",
        Duration::from_secs(30),
        drain_shutdown.clone(),
        move || {
            let game_state = Arc::clone(&game_state_for_ground);
            async move { game_state.tick_ground_item_despawn().await }
        },
    ));

    // Reclaim buyback entries for characters who never trade again; trades
    // filter expiry inline, so this only has to beat memory growth.
    let game_state_for_buybacks = Arc::clone(&game_state);
    background.spawn(run_ticks(
        "buyback expiry",
        game_state::BUYBACK_SWEEP_PERIOD,
        drain_shutdown.clone(),
        move || {
            let game_state = Arc::clone(&game_state_for_buybacks);
            async move { game_state.tick_buyback_expiry().await }
        },
    ));

    // Evict terrain tiles idle for a full period so memory tracks the
    // live working set.
    let game_state_for_terrain_sweep = Arc::clone(&game_state);
    background.spawn(run_ticks(
        "terrain cache sweep",
        onlinerpg_terrain::TILE_CACHE_SWEEP_PERIOD,
        drain_shutdown.clone(),
        move || {
            let game_state = Arc::clone(&game_state_for_terrain_sweep);
            async move {
                let evicted = game_state.sweep_terrain_caches().await;
                if evicted > 0 {
                    tracing::debug!("terrain cache sweep evicted {evicted} idle tiles");
                }
            }
        },
    ));

    // Fishing session timers (cast → wait → bite → expiry). 250 ms is far
    // inside every player-facing window; deadlines carry their own grace.
    let game_state_for_fishing = Arc::clone(&game_state);
    let auth_for_fishing = Arc::clone(&auth_service);
    background.spawn(run_ticks(
        "fishing",
        Duration::from_millis(250),
        drain_shutdown.clone(),
        move || {
            let game_state = Arc::clone(&game_state_for_fishing);
            let auth = Arc::clone(&auth_for_fishing);
            async move { game_state.tick_fishing(Some(&auth)).await }
        },
    ));

    // Hunger (doc/HUNGER.md): grills every 250ms; campfires, debuffs and
    // food regeneration every second. Activity drain rides movement/kills.
    let game_state_for_hunger = Arc::clone(&game_state);
    let mut hunger_tick_count = 0u64;
    background.spawn(run_ticks(
        "hunger",
        Duration::from_millis(250),
        drain_shutdown.clone(),
        move || {
            hunger_tick_count = hunger_tick_count.wrapping_add(1);
            let game_state = Arc::clone(&game_state_for_hunger);
            let count = hunger_tick_count;
            async move {
                game_state.tick_grills().await;
                if count.is_multiple_of(4) {
                    game_state.tick_campfires().await;
                    game_state.tick_meals().await;
                    game_state
                        .tick_campfire_drying(Duration::from_secs(1))
                        .await;
                    game_state.tick_debuffs().await;
                    game_state.tick_food_regeneration().await;
                }
            }
        },
    ));

    let game_state_for_time_sync = Arc::clone(&game_state);
    let auth_service_for_time_sync = Arc::clone(&auth_service);
    let mut tick_count = 0u64;
    background.spawn(run_ticks(
        "time sync",
        Duration::from_secs(8),
        drain_shutdown.clone(),
        move || {
            tick_count = tick_count.wrapping_add(1);
            let game_state = Arc::clone(&game_state_for_time_sync);
            let auth = Arc::clone(&auth_service_for_time_sync);
            async move { time_sync_tick(&game_state, &auth, tick_count).await }
        },
    ));

    let addr = format!("{}:{}", args.bind, args.port);
    let listener = match TcpListener::bind(addr.as_str()).await {
        Ok(listener) => {
            info!("MMORPG Server listening on: {}", addr);
            listener
        }
        Err(e) => {
            error!("Failed to bind to {}: {}", addr, e);
            return ExitCode::FAILURE;
        }
    };

    // Start terrain REST API server. No CORS layer on purpose: browsers only
    // reach this API same-origin through the vite proxy.
    let terrain_port = args.terrain_port.unwrap_or(args.port + 1);
    let terrain_app = terrain_router(Arc::clone(&terrain_io), Arc::clone(&game_state))
        .merge(housing_router(
            Arc::clone(&housing_io),
            terrain_io,
            Arc::clone(&game_state),
        ))
        .merge(npc_router(npc_io, Arc::clone(&game_state)))
        .merge(announcements_router(announcement_store))
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&auth_ctx),
            api_auth::require_admin_for_writes,
        ))
        // Merged after the admin layer on purpose: uploading a cape texture is
        // a player action, authorised by the session token the server hands
        // out at login rather than by the admin allowlist.
        .merge(cape_texture_router(cape_textures, Arc::clone(&auth_ctx)))
        .layer(CompressionLayer::new());
    let terrain_addr = format!("{}:{}", args.api_bind, terrain_port);
    let mut api_task = JoinSet::new();
    match TcpListener::bind(&terrain_addr).await {
        Ok(terrain_listener) => {
            info!("Terrain REST API listening on: {}", terrain_addr);
            let api_shutdown = drain_shutdown.clone();
            api_task.spawn(async move {
                if let Err(e) = axum::serve(terrain_listener, terrain_app)
                    .with_graceful_shutdown(wait_for_shutdown(api_shutdown))
                    .await
                {
                    error!("Terrain API server error: {}", e);
                }
            });
        }
        Err(e) => {
            error!("Failed to bind terrain API to {}: {}", terrain_addr, e);
            return ExitCode::FAILURE;
        }
    }

    info!("🎮 MMORPG Server started successfully!");
    info!("📡 WebSocket server ready for connections");
    info!("🌐 Connect clients to: ws://{}", addr);

    let mut connections = JoinSet::new();
    let conn_ctx = Arc::new(ServerContext {
        game_state: Arc::clone(&game_state),
        auth_service: Arc::clone(&auth_service),
        auth_ctx: Arc::clone(&auth_ctx),
        connect_limiter: ConnectLimiter::default(),
    });
    let signal = shutdown_signal();
    tokio::pin!(signal);

    let exit_code = loop {
        tokio::select! {
            biased;
            _ = &mut signal => {
                info!("Shutdown signal received; draining server");
                break ExitCode::SUCCESS;
            }
            result = api_task.join_next(), if !api_task.is_empty() => {
                // A dead REST API behind a live game listener is a partial
                // outage systemd cannot see; drain and exit non-zero so
                // Restart=on-failure restarts us.
                error!("Terrain REST API stopped unexpectedly ({result:?}); draining for restart");
                break ExitCode::FAILURE;
            }
            result = listener.accept() => match result {
                Ok((stream, addr)) => {
                    // Only bites with --bind 0.0.0.0; loopback peers are
                    // charged after the upgrade reveals their real address.
                    if !conn_ctx.connect_limiter.allow(addr.ip()) {
                        continue;
                    }
                    let ctx = Arc::clone(&conn_ctx);
                    let shutdown_started = drain_shutdown.clone();
                    let shutdown = connection_shutdown.clone();

                    connections.spawn(async move {
                        handle_connection(stream, addr, ctx, shutdown_started, shutdown).await;
                    });
                }
                Err(e) => error!("Failed to accept connection: {}", e),
            },
            result = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(e)) = result {
                    error!("Connection task failed: {}", e);
                }
            }
        }
    };

    game_state
        .set_server_notice(Some(SHUTDOWN_NOTICE.to_string()))
        .await;
    let shutdown_notice_deadline = Instant::now() + SHUTDOWN_NOTICE_DURATION;

    // Quiesce background writers, hold the notice for its full window, then
    // stop all player mutations before taking the final snapshot.
    drop(listener);
    let _ = drain_shutdown_tx.send(());
    drain(&mut background, "Background").await;
    tokio::time::sleep_until(shutdown_notice_deadline).await;

    let _ = connection_shutdown_tx.send(());
    drain(&mut connections, "Connection").await;

    game_state.persist_shutdown_snapshot(&auth_service).await;

    drain(&mut api_task, "Terrain API").await;

    info!("Graceful shutdown complete");
    exit_code
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn temp_auth(name: &str) -> (AuthService, std::path::PathBuf) {
        let db_path = test_util::unique_temp_dir(name).join("auth.db");
        (AuthService::new(db_path.clone()).unwrap(), db_path)
    }

    /// The systemd units run with CWD = repo root and pass no path flags, so
    /// these defaults are the compatibility contract with the existing deploy.
    /// Drift here would silently point a live server at an empty database.
    #[test]
    fn default_flags_reproduce_the_pre_flag_layout() {
        let args = Args::parse_from(["onlinerpg-server"]);
        let paths = state_paths(&args.state_dir);

        assert_eq!(args.state_dir, PathBuf::from("./data"));
        assert_eq!(paths.db, PathBuf::from("./data/game_data.db"));
        assert_eq!(paths.npc_token, PathBuf::from("./data/npc_token"));
        assert_eq!(paths.housing, PathBuf::from("./data/housing"));
        assert_eq!(paths.announcements, PathBuf::from("./data/announcements"));
        assert_eq!(
            args.npc_data_dir,
            PathBuf::from("./agent-client/data/npcs"),
            "npcs live outside --state-dir and need their own flag"
        );
        assert_eq!(args.terrain_dir, "./data/terrain");
    }

    /// Every state path must follow --state-dir. Missing one would leave that
    /// single file behind in ./data while the rest moved to the volume.
    #[test]
    fn custom_state_dir_moves_every_derived_path() {
        let args = Args::parse_from([
            "onlinerpg-server",
            "--state-dir",
            "/srv/state",
            "--npc-data-dir",
            "/srv/npcs",
        ]);
        let paths = state_paths(&args.state_dir);

        assert_eq!(paths.db, PathBuf::from("/srv/state/game_data.db"));
        assert_eq!(paths.npc_token, PathBuf::from("/srv/state/npc_token"));
        assert_eq!(paths.housing, PathBuf::from("/srv/state/housing"));
        assert_eq!(
            paths.announcements,
            PathBuf::from("/srv/state/announcements")
        );
        assert_eq!(args.npc_data_dir, PathBuf::from("/srv/npcs"));
    }

    #[test]
    fn blank_optional_values_are_treated_as_absent() {
        assert_eq!(optional_value(None), None);
        assert_eq!(optional_value(Some("")), None);
        assert_eq!(optional_value(Some("   ")), None);
        assert_eq!(optional_value(Some("  token  ")), Some("token"));
    }

    #[test]
    fn npc_token_is_generated_once_and_reread() {
        let path = test_util::unique_temp_dir("npc_token_roundtrip").join("npc_token");

        let created = load_or_create_npc_token(&path).unwrap();
        assert!(created.len() >= MIN_NPC_TOKEN_LEN);
        assert_eq!(load_or_create_npc_token(&path).unwrap(), created);
    }

    /// The token is a shared secret; a readable file would leak it to every
    /// other local user on a multi-tenant host.
    #[cfg(unix)]
    #[test]
    fn npc_token_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let path = test_util::unique_temp_dir("npc_token_perms").join("npc_token");
        load_or_create_npc_token(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// A blank file (truncated write, manual edit) must not become the token:
    /// MIN_NPC_TOKEN_LEN would reject it later and refuse to start.
    #[test]
    fn empty_npc_token_file_is_regenerated() {
        let path = test_util::unique_temp_dir("npc_token_empty").join("npc_token");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "   \n").unwrap();

        let token = load_or_create_npc_token(&path).unwrap();

        assert!(token.trim().len() >= MIN_NPC_TOKEN_LEN);
    }

    /// Guards acceptance criterion 3: a flag-free server must touch exactly
    /// the paths it touched before --state-dir existed. The expected set is a
    /// hand-audited constant of the pre-flag behaviour, not a snapshot of the
    /// current code, so a regression cannot silently rewrite the baseline.
    ///
    /// Only AuthService::new and load_or_create_npc_token create anything;
    /// HousingIO/NpcIO/AnnouncementStore construct lazily.
    #[test]
    fn flag_free_startup_touches_only_the_legacy_paths() {
        let root = test_util::unique_temp_dir("state_dir_compat");
        let state_dir = root.join("data");
        let paths = state_paths(&state_dir);

        AuthService::new(paths.db.clone()).unwrap();
        load_or_create_npc_token(&paths.npc_token).unwrap();
        let _ = HousingIO::new(paths.housing.clone());
        let _ = NpcIO::new(root.join("agent-client/data/npcs"));
        let _ = AnnouncementStore::new(paths.announcements.clone());

        let mut created: Vec<String> = walk_relative(&root, &root);
        created.sort();

        assert_eq!(
            created,
            vec![
                "data".to_string(),
                "data/game_data.db".to_string(),
                "data/npc_token".to_string(),
            ],
            "startup side effects drifted from the pre-flag layout"
        );
        assert!(!paths.housing.exists(), "housing must stay lazy");
        assert!(
            !paths.announcements.exists(),
            "announcements must stay lazy"
        );
    }

    fn walk_relative(base: &Path, dir: &Path) -> Vec<String> {
        let mut found = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return found;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // SQLite may leave -wal/-shm siblings; they are not layout.
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.ends_with("-wal") || name.ends_with("-shm") {
                continue;
            }
            // Normalize to forward slashes so expectations hold on Windows.
            found.push(
                path.strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
            if path.is_dir() {
                found.extend(walk_relative(base, &path));
            }
        }
        found
    }

    #[tokio::test]
    async fn guard_tick_swallows_panic() {
        guard_tick("test", async { panic!("boom") }).await;
    }

    #[test]
    fn initial_game_time_preserves_stored_value() {
        let (auth, _) = temp_auth("time_stored");
        let stored = GameDateTime {
            year: 42,
            month: 7,
            day: 9,
            hour: 18,
            minute: 23,
        };
        auth.save_world_time(&stored).unwrap();

        assert_eq!(load_initial_game_time(&auth).unwrap(), stored);
    }

    #[test]
    fn initial_game_time_persists_default_when_absent() {
        let (auth, _) = temp_auth("time_absent");

        let initial = load_initial_game_time(&auth).unwrap();

        assert_eq!(auth.load_world_time().unwrap().unwrap(), initial);
    }

    #[test]
    fn initial_game_time_propagates_read_failure() {
        let (auth, db_path) = temp_auth("time_failed");
        Connection::open(db_path)
            .unwrap()
            .execute("DROP TABLE world_time", [])
            .unwrap();

        assert!(load_initial_game_time(&auth).is_err());
    }
}
