mod announcements;
mod api_auth;
mod auth;
mod celestial;
mod conn_limit;
mod connection;
mod dungeon_defs;
mod game;
mod game_state;
mod google_auth;
mod housing;
mod item_defs;
mod merchant_defs;
mod monster_defs;
mod npc_defs;
mod npc_schedule;
mod semicolon_list;
mod terrain;
#[cfg(test)]
mod test_util;
mod types;
mod world_config;
mod world_drop_defs;

use announcements::{announcements_router, AnnouncementStore};
use auth::AuthService;
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
use std::sync::Arc;
use terrain::io::TerrainIO;
use terrain::routes::terrain_router;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio::time::{Duration, Instant};
use tower_http::compression::CompressionLayer;
use tracing::{error, info, warn};

const SHUTDOWN_NOTICE: &str = "The server is shutting down. Please reconnect shortly.";
const SHUTDOWN_NOTICE_DURATION: Duration = Duration::from_secs(2);

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

    // Pay NPC trader salaries on game-day rollover (economy phase 3)
    game_state.tick_npc_salaries().await;

    // Batch-save dirty character states and inventories every 4 ticks (32s)
    if tick_count.is_multiple_of(4) {
        game_state.flush_dirty_saves(auth_service).await;
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
    #[arg(long, default_value = "./data/terrain")]
    terrain_dir: String,

    /// Google OAuth client ID used to verify browser sign-in tokens
    #[arg(long, env = "GOOGLE_CLIENT_ID")]
    google_client_id: Option<String>,

    /// Google OAuth client ID for headless sign-in (agent-client's device
    /// flow). Separate client, so its tokens carry a different `aud`.
    #[arg(long, env = "GOOGLE_CLI_CLIENT_ID")]
    google_cli_client_id: Option<String>,

    /// Shared secret for headless NPC clients (default: data/npc_token,
    /// generated on first run)
    #[arg(long, env = "NPC_AUTH_TOKEN")]
    npc_token: Option<String>,
}

/// Read the NPC token file, generating a random one on first run so local
/// bots work with zero config (they read the same file).
fn load_or_create_npc_token() -> std::io::Result<String> {
    let path = std::path::Path::new(onlinerpg_shared::NPC_TOKEN_PATH_FROM_ROOT);
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

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    world_config::log_world_config();
    let monster_defs = monster_defs::MonsterDefs::load();
    let item_defs = item_defs::ItemDefs::load();
    let dungeon_defs = dungeon_defs::DungeonDefs::load(&item_defs);
    let world_drop_defs = world_drop_defs::WorldDropDefs::load(&item_defs);
    let auth_service = match AuthService::new(AuthService::default_db_path()) {
        Ok(service) => Arc::new(service),
        Err(e) => {
            error!("Failed to initialize auth service: {}", e);
            return;
        }
    };

    let google_client_ids: Vec<String> = [&args.google_client_id, &args.google_cli_client_id]
        .into_iter()
        .flatten()
        .cloned()
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
    let npc_token = match args.npc_token.clone() {
        Some(token) => token,
        None => match load_or_create_npc_token() {
            Ok(token) => token,
            Err(e) => {
                error!("failed to load/create NPC token: {e}");
                return;
            }
        },
    };
    if npc_token.trim().len() < MIN_NPC_TOKEN_LEN {
        error!(
            "NPC token is shorter than {MIN_NPC_TOKEN_LEN} chars; refusing to start. \
             Unset --npc-token / NPC_AUTH_TOKEN to auto-generate a secure one."
        );
        return;
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
    let initial_game_time = match auth_service.load_world_time() {
        Ok(Some(saved)) => {
            info!(
                "Loaded game time from DB: {:04}-{:02}-{:02} {:02}:{:02}",
                saved.year, saved.month, saved.day, saved.hour, saved.minute
            );
            saved
        }
        Ok(None) => {
            let initial = GameState::default_start_datetime();
            if let Err(err) = auth_service.save_world_time(&initial) {
                warn!("Failed to persist initial game time: {}", err);
            }
            info!(
                "Initialized game time: {:04}-{:02}-{:02} {:02}:{:02}",
                initial.year, initial.month, initial.day, initial.hour, initial.minute
            );
            initial
        }
        Err(err) => {
            warn!("Failed to load game time from DB, using default: {}", err);
            GameState::default_start_datetime()
        }
    };

    let housing_io = Arc::new(HousingIO::new(std::path::PathBuf::from("./data/housing")));
    let npc_io = Arc::new(NpcIO::new(std::path::PathBuf::from(
        "./agent-client/data/npcs",
    )));
    let terrain_io = Arc::new(TerrainIO::new(std::path::PathBuf::from(&args.terrain_dir)));
    let announcement_store = Arc::new(AnnouncementStore::new(std::path::PathBuf::from(
        "./data/announcements",
    )));
    announcement_store.warm().await;

    // Load no-spawn zones (towns) from per-region zone files. Monster spawn
    // areas come from world.json `ambientSpawns`, not per-region rectangles.
    let no_spawn_zones = world_config::load_no_spawn_zones_from_regions(&terrain_io).await;

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
    ));
    // Server-side collision data for the movement sim: houses, solid
    // furniture and dungeon layouts, mirroring what clients build.
    game_state.init_passability(&args.terrain_dir).await;
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

    // Every 10s, top up each player's ambient monsters toward their caps.
    let game_state_for_spawns = Arc::clone(&game_state);
    background.spawn(run_ticks(
        "monster spawn",
        Duration::from_secs(10),
        drain_shutdown.clone(),
        move || {
            let game_state = Arc::clone(&game_state_for_spawns);
            async move { game_state.tick_monster_spawns().await }
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

    // Fishing session timers (cast → wait → bite → expiry). 250 ms is far
    // inside every player-facing window; deadlines carry their own grace.
    let game_state_for_fishing = Arc::clone(&game_state);
    background.spawn(run_ticks(
        "fishing",
        Duration::from_millis(250),
        drain_shutdown.clone(),
        move || {
            let game_state = Arc::clone(&game_state_for_fishing);
            async move { game_state.tick_fishing().await }
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
            return;
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
        .merge(npc_router(npc_io))
        .merge(announcements_router(announcement_store))
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&auth_ctx),
            api_auth::require_admin_for_writes,
        ))
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
            return;
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

    let shutdown_notice_deadline = loop {
        tokio::select! {
            biased;
            _ = &mut signal => {
                info!("Shutdown signal received; draining server");
                game_state
                    .set_server_notice(Some(SHUTDOWN_NOTICE.to_string()))
                    .await;
                break Instant::now() + SHUTDOWN_NOTICE_DURATION;
            }
            result = api_task.join_next(), if !api_task.is_empty() => {
                // A dead REST API behind a live game listener is a partial
                // outage systemd cannot see; drain and exit so it restarts us.
                error!("Terrain REST API stopped unexpectedly ({result:?}); draining for restart");
                game_state
                    .set_server_notice(Some(SHUTDOWN_NOTICE.to_string()))
                    .await;
                break Instant::now() + SHUTDOWN_NOTICE_DURATION;
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
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn guard_tick_swallows_panic() {
        super::guard_tick("test", async { panic!("boom") }).await;
    }
}
