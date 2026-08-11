//! Local spectator panel: a small HTTP server that exposes each NPC's live
//! game state (map, entities, chat/combat feed, LLM turns) to a browser page.
//! Read-only and bound to 127.0.0.1 — it observes the agent, never drives it.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use axum::extract::{Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Json, Response};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::state::SharedState;
use onlinerpg_terrain::io::TerrainIO;

const FEED_CAP: usize = 300;
const PROMPT_TEXT_CAP: usize = 6000;

#[derive(Clone, serde::Serialize)]
pub struct FeedItem {
    /// Monotonic per-NPC sequence. The page appends only what it has not
    /// drawn, so a rotating ring never forces a full re-render.
    pub s: u64,
    /// Unix ms.
    pub t: u64,
    /// Kind: chat | combat | trade | system | agent | llm-prompt | llm-response | llm-error.
    pub k: &'static str,
    pub m: String,
    /// Backend call ms, excluding scheduler queue wait (LLM turns only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub d: Option<u64>,
    /// Scheduler queue wait ms — time spent waiting for a slot, so a backlog at
    /// `max_concurrent` is not mistaken for a slow model (LLM turns only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub q: Option<u64>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Default)]
struct Feed {
    items: VecDeque<FeedItem>,
    next_seq: u64,
}

/// Per-NPC watch handle. Outlives sessions, so the feed survives reconnects.
pub struct NpcWatch {
    feed: StdMutex<Feed>,
    state: StdMutex<Option<Arc<Mutex<SharedState>>>>,
    connected: AtomicBool,
}

impl NpcWatch {
    fn new() -> Self {
        Self {
            feed: StdMutex::new(Feed::default()),
            state: StdMutex::new(None),
            connected: AtomicBool::new(false),
        }
    }

    pub fn push(&self, kind: &'static str, text: String) {
        self.push_item(kind, text, None, None)
    }

    /// Truncated so one runaway prompt cannot dominate the feed's memory.
    pub fn llm_prompt(&self, prompt: &str) {
        let mut text = prompt.chars().take(PROMPT_TEXT_CAP).collect::<String>();
        if prompt.len() > text.len() {
            text.push_str("\n… (truncated)");
        }
        self.push("llm-prompt", text);
    }

    pub fn llm_result(&self, kind: &'static str, text: String, invoke_ms: u64, queue_ms: u64) {
        self.push_item(kind, text, Some(invoke_ms), Some(queue_ms))
    }

    fn push_item(&self, kind: &'static str, text: String, d: Option<u64>, q: Option<u64>) {
        let mut feed = self.feed.lock().unwrap();
        let s = feed.next_seq;
        feed.next_seq += 1;
        feed.items.push_back(FeedItem {
            s,
            t: now_ms(),
            k: kind,
            m: text,
            d,
            q,
        });
        while feed.items.len() > FEED_CAP {
            feed.items.pop_front();
        }
    }

    pub fn set_state(&self, state: Arc<Mutex<SharedState>>) {
        *self.state.lock().unwrap() = Some(state);
        self.connected.store(true, Ordering::Relaxed);
    }

    pub fn set_disconnected(&self) {
        self.connected.store(false, Ordering::Relaxed);
    }

    /// The last session's state is deliberately kept after a disconnect so the
    /// map still shows where the agent was when it dropped.
    fn current_state(&self) -> Option<Arc<Mutex<SharedState>>> {
        self.state.lock().unwrap().clone()
    }

    /// Items newer than `since`, or the whole ring on a first poll. Sequences
    /// only increase, so this is a suffix — the page re-fetching what it has
    /// already drawn would dominate the response at 1 Hz.
    fn feed_since(&self, since: Option<u64>) -> Vec<FeedItem> {
        let feed = self.feed.lock().unwrap();
        feed.items
            .iter()
            .skip_while(|i| since.is_some_and(|s| i.s <= s))
            .cloned()
            .collect()
    }
}

/// Wraps any LLM backend to record its turns on the panel. Every backend is
/// built through one funnel, so decorating there covers claude / codex /
/// openrouter without the scheduler or the driver knowing the panel exists.
pub struct WatchedBackend {
    inner: Arc<dyn crate::driver::LlmBackend>,
    watch: Arc<NpcWatch>,
}

impl WatchedBackend {
    pub fn wrap(
        inner: Arc<dyn crate::driver::LlmBackend>,
        watch: Option<Arc<NpcWatch>>,
    ) -> Arc<dyn crate::driver::LlmBackend> {
        match watch {
            Some(watch) => Arc::new(Self { inner, watch }),
            None => inner,
        }
    }
}

#[async_trait::async_trait]
impl crate::driver::LlmBackend for WatchedBackend {
    async fn send_message(&self, content: &str) -> anyhow::Result<String> {
        // We run inside the task the scheduler dispatched, so the prompt lands
        // on the feed when the model actually starts, and the queue wait it
        // scoped in is readable here.
        self.watch.llm_prompt(content);
        let queue_ms = crate::llm_scheduler::queue_wait()
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let started = std::time::Instant::now();
        let result = self.inner.send_message(content).await;
        let ms = started.elapsed().as_millis() as u64;
        match &result {
            Ok(text) => self
                .watch
                .llm_result("llm-response", text.clone(), ms, queue_ms),
            Err(e) => self
                .watch
                .llm_result("llm-error", e.to_string(), ms, queue_ms),
        }
        result
    }
}

/// All NPC watch handles, in orchestrator order.
pub struct WatchHub {
    npcs: Vec<(String, Arc<NpcWatch>)>,
}

impl WatchHub {
    /// Display names are forced unique: `NpcConfig::label()` falls back to a
    /// shared default, so two NPCs can legitimately both answer to "agent" and
    /// a name lookup would hand them the same feed.
    pub fn new(labels: Vec<String>) -> Self {
        let mut seen: HashMap<String, usize> = HashMap::new();
        let npcs = labels
            .into_iter()
            .map(|label| {
                let n = seen.entry(label.clone()).or_insert(0);
                *n += 1;
                let name = if *n == 1 {
                    label
                } else {
                    format!("{label}#{n}")
                };
                (name, Arc::new(NpcWatch::new()))
            })
            .collect();
        Self { npcs }
    }

    pub fn handle_at(&self, index: usize) -> Option<Arc<NpcWatch>> {
        self.npcs.get(index).map(|(_, w)| Arc::clone(w))
    }
}

struct AppState {
    hub: Arc<WatchHub>,
    minimap: MinimapSource,
    /// Changes on every process start; the page reloads itself when it
    /// sees a new value, so a redeploy never needs a manual refresh.
    boot_id: u64,
}

/// Where the panel reads baked region minimaps from. These are the same PNGs
/// the web client's world map draws, so the spectator view cannot drift from
/// the game's own colours the way a second elevation ramp would.
pub enum MinimapSource {
    Local(TerrainIO),
    Http {
        base_url: String,
        http: reqwest::Client,
    },
}

impl MinimapSource {
    /// Mirrors `create_height_sampler`: a local terrain tree when the agent
    /// sits on the game server, the public tile API otherwise.
    pub fn new(terrain: &str) -> Self {
        if terrain.starts_with("http://") || terrain.starts_with("https://") {
            Self::Http {
                base_url: terrain.trim_end_matches('/').to_string(),
                http: reqwest::Client::new(),
            }
        } else {
            Self::Local(TerrainIO::new(std::path::PathBuf::from(terrain)))
        }
    }

    async fn read(&self, rx: i32, rz: i32) -> anyhow::Result<Option<Vec<u8>>> {
        match self {
            Self::Local(io) => Ok(io.read_minimap(rx, rz).await?),
            Self::Http { base_url, http } => {
                let url = format!("{base_url}/api/terrain/minimap/{rx}/{rz}");
                let response = http.get(&url).send().await?;
                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    return Ok(None);
                }
                Ok(Some(response.error_for_status()?.bytes().await?.to_vec()))
            }
        }
    }
}

#[derive(Deserialize)]
struct NpcQuery {
    npc: Option<String>,
    /// Highest feed sequence the page already holds.
    since: Option<u64>,
}

/// Loopback-only listener, so any other `Host` is a DNS-rebind attempt.
fn host_is_local(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or_default(),
        None => host.split(':').next().unwrap_or_default(),
    };
    matches!(name, "127.0.0.1" | "localhost" | "::1")
}

async fn guard_host(req: Request, next: Next) -> Response {
    let allowed = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(host_is_local);
    if allowed {
        next.run(req).await
    } else {
        StatusCode::FORBIDDEN.into_response()
    }
}

pub async fn serve(hub: Arc<WatchHub>, minimap: MinimapSource, port: u16) {
    let boot_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1);
    let app_state = Arc::new(AppState {
        hub,
        minimap,
        boot_id,
    });
    let app = Router::new()
        .route("/", get(page))
        .route("/api/npcs", get(npcs))
        .route("/api/state", get(state_snapshot))
        .route("/api/minimap/{rx}/{rz}", get(minimap_png))
        .layer(middleware::from_fn(guard_host))
        .with_state(app_state);

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            warn!("Watch panel failed to bind {addr}: {e}");
            return;
        }
    };
    info!("Watch panel: http://{addr}/");
    if let Err(e) = axum::serve(listener, app).await {
        warn!("Watch panel server stopped: {e}");
    }
}

async fn page() -> Html<&'static str> {
    Html(include_str!("watch.html"))
}

async fn npcs(State(app): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let labels: Vec<&str> = app.hub.npcs.iter().map(|(l, _)| l.as_str()).collect();
    Json(json!({ "npcs": labels }))
}

async fn state_snapshot(State(app): State<Arc<AppState>>, Query(q): Query<NpcQuery>) -> Response {
    let entry = match &q.npc {
        Some(label) => app.hub.npcs.iter().find(|(l, _)| l == label),
        None => app.hub.npcs.first(),
    };
    let Some((label, watch)) = entry else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "unknown npc" })),
        )
            .into_response();
    };

    let feed = watch.feed_since(q.since);
    let connected = watch.connected.load(Ordering::Relaxed);

    let Some(state_arc) = watch.current_state() else {
        return Json(json!({
            "npc": label, "connected": false, "feed": feed,
        }))
        .into_response();
    };

    let body = {
        let s = state_arc.lock().await;
        let houses: Vec<serde_json::Value> = {
            let wc = s.world_cache.read().unwrap();
            wc.houses()
                .values()
                .map(|h| {
                    let rooms: Vec<serde_json::Value> = h
                        .rooms
                        .iter()
                        .map(|r| {
                            json!({
                                "x": h.origin.x + r.local_x as f32,
                                "z": h.origin.z + r.local_z as f32,
                                "w": r.size_x, "d": r.size_z, "floor": r.floor_level,
                            })
                        })
                        .collect();
                    json!({ "id": h.id, "rooms": rooms })
                })
                .collect()
        };

        // Underground: ship the real floor layout so the map can draw rock,
        // rooms and stairs instead of the surface terrain.
        let dungeon = if s.self_floor_level < 0 {
            s.dungeon_here().and_then(|d| {
                use onlinerpg_shared::dungeon::{cell_center, dungeon_origin, GRID};
                let depth = s.self_floor_level.unsigned_abs();
                d.layouts().get(depth as usize - 1).map(|layout| {
                    let (ox, oz) = dungeon_origin(d.entrance.x, d.entrance.z);
                    let rows: Vec<String> = (0..GRID)
                        .map(|z| {
                            (0..GRID)
                                .map(|x| {
                                    if layout.carved[(x + z * GRID) as usize] {
                                        '.'
                                    } else {
                                        '#'
                                    }
                                })
                                .collect()
                        })
                        .collect();
                    let up =
                        cell_center(&d.entrance, depth, (layout.up_shaft.x, layout.up_shaft.z));
                    let down = layout.down_shaft.as_ref().map(|sh| {
                        let p = cell_center(&d.entrance, depth, (sh.x, sh.z));
                        json!({ "x": p.x, "z": p.z })
                    });
                    json!({
                        "name": d.name,
                        "origin": { "x": ox, "z": oz },
                        "grid": GRID,
                        "rows": rows,
                        "stairs_up": { "x": up.x, "z": up.z },
                        "stairs_down": down,
                    })
                })
            })
        } else {
            None
        };

        let dungeon_entrances: Vec<serde_json::Value> = {
            let wc = s.world_cache.read().unwrap();
            wc.all_dungeons()
                .iter()
                .map(|d| {
                    json!({
                        "name": d.name,
                        "x": d.entrance.x,
                        "z": d.entrance.z,
                        "floors": d.max_depth(),
                    })
                })
                .collect()
        };
        let ground_items: Vec<serde_json::Value> = s
            .ground_items_in_sight()
            .into_iter()
            .map(|(_, i)| {
                json!({
                    "x": i.position.x,
                    "z": i.position.z,
                    "item": i.item_def_id,
                    "floor": i.floor_level,
                })
            })
            .collect();

        json!({
            "npc": label,
            "boot": app.boot_id,
            "sight": onlinerpg_shared::NPC_SIGHT_RADIUS,
            "connected": connected && s.in_game,
            "self": s.self_player,
            "gold": s.self_gold,
            "floor": s.self_floor_level,
            "dungeon": dungeon,
            "dungeon_entrances": dungeon_entrances,
            "ground_items": ground_items,
            "bag": s.self_bag,
            "time": { "hour": s.game_hour, "minute": s.game_minute, "night": s.is_night },
            "players": s.nearby_players.values().collect::<Vec<_>>(),
            "monsters": s.nearby_monsters.values().collect::<Vec<_>>(),
            "houses": houses,
            "feed": feed,
        })
    };
    Json(body).into_response()
}

/// Serve one baked region PNG, straight through from the terrain source.
async fn minimap_png(
    State(app): State<Arc<AppState>>,
    axum::extract::Path((rx, rz)): axum::extract::Path<(i32, i32)>,
) -> Response {
    match app.minimap.read(rx, rz).await {
        Ok(Some(png)) => (
            [
                (header::CONTENT_TYPE, "image/png"),
                // Baked data; only a re-bake changes it.
                (header::CACHE_CONTROL, "max-age=300"),
            ],
            png,
        )
            .into_response(),
        // Outside the baked area — the page just leaves that region blank.
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            warn!("Minimap ({rx}, {rz}) unavailable: {e}");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

/// Feed category for a server message; `None` keeps it off the panel
/// (movement/time spam is visible on the map already).
pub fn feed_kind(msg: &onlinerpg_shared::ServerMessage) -> Option<&'static str> {
    use onlinerpg_shared::ServerMessage as M;
    Some(match msg {
        M::ChatMessage { .. } | M::PartyChatMessage { .. } | M::PlayerMusicStarted { .. } => "chat",
        M::PlayerAttacked { .. }
        | M::MonsterAttackedPlayer { .. }
        | M::MonsterDead { .. }
        | M::PlayerDead { .. }
        | M::PlayerRespawned { .. }
        | M::XpGained { .. } => "combat",
        M::DealResult { .. } | M::TradeNotice { .. } | M::TradeError { .. } => "trade",
        M::JoinSuccess { .. }
        | M::Kicked { .. }
        | M::SystemMessage { .. }
        | M::ServerNotice { .. }
        | M::PlayerJoined { .. }
        | M::PlayerLeft { .. }
        | M::PlayerAppeared { .. }
        | M::PlayerDisappeared { .. }
        | M::PlayerTeleported { .. }
        | M::CharacterCreated { .. }
        | M::CharacterError { .. } => "system",
        _ => return None,
    })
}

/// Message text for feed kinds `format_event` does not cover.
pub fn feed_fallback(msg: &onlinerpg_shared::ServerMessage) -> Option<String> {
    use onlinerpg_shared::ServerMessage as M;
    match msg {
        M::ServerNotice { message } => Some(format!(
            "[Notice] {}",
            message.as_deref().unwrap_or("(cleared)")
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onlinerpg_shared::{PlayerId, Position, ServerMessage as M};

    #[test]
    fn feed_kinds_cover_the_panel_categories() {
        let pid = PlayerId::from(1u64);
        assert_eq!(
            feed_kind(&M::ChatMessage {
                player_id: pid,
                message: "hi".into()
            }),
            Some("chat")
        );
        assert_eq!(feed_kind(&M::PlayerLeft { player_id: pid }), Some("system"));
        assert_eq!(
            feed_kind(&M::PlayerTeleported {
                player_id: pid,
                position: Position {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                rotation: 0.0,
                floor_level: 0,
            }),
            Some("system")
        );
        assert_eq!(
            feed_kind(&M::ServerNotice { message: None }),
            Some("system")
        );
    }

    #[test]
    fn movement_stays_off_the_feed() {
        assert_eq!(
            feed_kind(&M::PlayerMoved {
                player_id: PlayerId::from(1u64),
                position: Position {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                rotation: 0.0,
                floor_level: 0,
                sprinting: false,
            }),
            None
        );
    }

    #[test]
    fn cleared_notice_reads_as_cleared() {
        assert_eq!(
            feed_fallback(&M::ServerNotice { message: None }).as_deref(),
            Some("[Notice] (cleared)")
        );
        assert_eq!(
            feed_fallback(&M::PlayerLeft {
                player_id: PlayerId::from(1u64)
            }),
            None
        );
    }

    #[test]
    fn sequence_keeps_climbing_past_eviction() {
        let w = NpcWatch::new();
        for i in 0..FEED_CAP + 50 {
            w.push("system", format!("line {i}"));
        }
        let snap = w.feed_since(None);
        assert_eq!(snap.len(), FEED_CAP);
        // Strictly increasing, and the ring has dropped the earliest items, so
        // the page can tell "new items" from "the ring rotated".
        assert!(snap.windows(2).all(|p| p[1].s > p[0].s));
        assert_eq!(snap.last().unwrap().s, (FEED_CAP + 49) as u64);
        assert_eq!(snap[0].s, 50);
    }

    #[test]
    fn since_returns_only_what_the_page_has_not_drawn() {
        let w = NpcWatch::new();
        for i in 0..5 {
            w.push("system", format!("line {i}"));
        }
        let fresh = w.feed_since(Some(2));
        assert_eq!(
            fresh.iter().map(|i| i.s).collect::<Vec<_>>(),
            [3, 4],
            "only sequences past `since`"
        );
        assert!(
            w.feed_since(Some(4)).is_empty(),
            "caught up polls are empty"
        );
        // A `since` older than the ring still yields whatever survived.
        assert_eq!(w.feed_since(Some(0)).len(), 4);
    }

    #[test]
    fn oversized_prompts_are_truncated() {
        let w = NpcWatch::new();
        w.llm_prompt(&"박".repeat(PROMPT_TEXT_CAP + 100));
        let snap = w.feed_since(None);
        assert_eq!(snap[0].k, "llm-prompt");
        assert!(snap[0].m.ends_with("… (truncated)"));
        assert_eq!(
            snap[0].m.chars().count(),
            PROMPT_TEXT_CAP + "\n… (truncated)".chars().count()
        );
    }

    #[test]
    fn multibyte_prompts_under_the_cap_are_left_alone() {
        let w = NpcWatch::new();
        let prompt = "한글".repeat(10);
        w.llm_prompt(&prompt);
        assert_eq!(w.feed_since(None)[0].m, prompt);
    }

    #[test]
    fn duplicate_labels_get_distinct_feeds() {
        let hub = WatchHub::new(vec!["agent".into(), "agent".into(), "karl".into()]);
        let names: Vec<&str> = hub.npcs.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(names, ["agent", "agent#2", "karl"]);

        hub.handle_at(0).unwrap().push("system", "first".into());
        assert_eq!(hub.handle_at(0).unwrap().feed_since(None).len(), 1);
        assert!(hub.handle_at(1).unwrap().feed_since(None).is_empty());
        assert!(hub.handle_at(3).is_none());
    }

    #[test]
    fn only_loopback_hosts_are_served() {
        for ok in [
            "127.0.0.1",
            "127.0.0.1:8808",
            "localhost",
            "localhost:8808",
            "[::1]:8808",
        ] {
            assert!(host_is_local(ok), "{ok} should be allowed");
        }
        for bad in [
            "example.com",
            "example.com:8808",
            "192.168.0.17:8808",
            "127.0.0.1.evil.com",
            "",
        ] {
            assert!(!host_is_local(bad), "{bad} should be rejected");
        }
    }
}
