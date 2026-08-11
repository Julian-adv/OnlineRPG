use crate::auth::AuthService;
use crate::conn_limit::{resolve_client_ip, ConnectLimiter};
use crate::game::character_attributes::roll_character_attributes;
use crate::game::character_hp::{level_one_max_hp, DEFAULT_CHARACTER_RACE};
use crate::game_state::{
    encode_server_msg, parse_admin_command, parse_notice_command, restored_floor_level,
    DirectMessage, GameState,
};
use crate::google_auth::GoogleAuthVerifier;
use crate::types::{
    new_player, Character, CharacterAttributes, CharacterClass, ClientKind, ClientMessage,
    PlayerId, Position, ServerMessage,
};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use onlinerpg_shared::deserialize_client_msg;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_tungstenite::{
    accept_hdr_async_with_config,
    tungstenite::{
        handshake::server::{ErrorResponse, Request, Response},
        protocol::{frame::coding::CloseCode, CloseFrame, WebSocketConfig},
        Message,
    },
};
use tracing::{debug, error, info, warn};

const FALLBACK_DEFAULT_MAX_HP: u32 = 13;

/// Credential checkers shared by every connection and the REST API.
pub struct AuthContext {
    /// None when the server was started without a Google client id; browser
    /// logins are rejected until it is configured.
    pub google: Option<GoogleAuthVerifier>,
    pub npc_token: String,
    /// Google account emails allowed to call REST write endpoints.
    pub admin_emails: Vec<String>,
}

impl AuthContext {
    /// Verified-email allowlist check shared by REST writes and in-game
    /// debug/cheat commands.
    pub fn is_admin(&self, claims: &crate::google_auth::GoogleClaims) -> bool {
        claims.email_verified == Some(true)
            && claims.email.as_deref().is_some_and(|email| {
                self.admin_emails
                    .iter()
                    .any(|a| a.eq_ignore_ascii_case(email))
            })
    }
}

/// Constant-time equality so the NPC token can't be probed byte by byte.
pub fn token_matches(provided: &str, expected: &str) -> bool {
    provided.len() == expected.len()
        && provided
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

/// How many seconds without a heartbeat before we consider the client dead.
const HEARTBEAT_TIMEOUT_SECS: u64 = 30;

/// Grace period before an unauthenticated connection is dropped. Measured
/// from connect time (not heartbeats — those are accepted pre-auth) so idle
/// sockets can't hold server resources without ever authenticating.
const UNAUTH_TIMEOUT_SECS: u64 = 60;

/// Caps tungstenite's 64 MiB default; legit client messages are a few KB.
const MAX_WS_MESSAGE_BYTES: usize = 64 * 1024;

/// Per-connection read buffer; the 128 KiB default is oversized for game packets.
const WS_READ_BUFFER_BYTES: usize = 16 * 1024;

/// Tighter caps until auth succeeds; legit pre-auth traffic is just auth attempts.
const UNAUTH_MAX_MESSAGE_BYTES: usize = 8 * 1024;
const UNAUTH_MAX_MESSAGES: u32 = 30;

/// A refused client retries, so one stale build can bury the log in identical
/// lines. Log the first of each window in full and fold the rest into its tail.
const REFUSAL_LOG_WINDOW: Duration = Duration::from_secs(60);

struct LogWindow {
    started: Instant,
    suppressed: u32,
}

/// One throttle per reason, so a flood of one kind can't hide the first
/// occurrence of another.
struct LogThrottle(Mutex<Option<LogWindow>>);

impl LogThrottle {
    const fn new() -> Self {
        Self(Mutex::new(None))
    }

    /// `Some(suffix)` when this event should be logged — the suffix names how
    /// many were folded in since the last line. `None` while a window is open.
    fn claim(&self) -> Option<String> {
        let now = Instant::now();
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match &mut *guard {
            Some(window) if now.duration_since(window.started) < REFUSAL_LOG_WINDOW => {
                window.suppressed += 1;
                None
            }
            slot => {
                let suppressed = slot.as_ref().map_or(0, |w| w.suppressed);
                *slot = Some(LogWindow {
                    started: now,
                    suppressed: 0,
                });
                Some(match suppressed {
                    0 => String::new(),
                    n => format!(" [+{n} more in the last {}s]", REFUSAL_LOG_WINDOW.as_secs()),
                })
            }
        }
    }
}

static PROTOCOL_REFUSAL_LOG: LogThrottle = LogThrottle::new();
static EARLY_MESSAGE_LOG: LogThrottle = LogThrottle::new();
static RATE_LIMIT_LOG: LogThrottle = LogThrottle::new();

fn close_frame(code: u16, reason: &'static str) -> Message {
    Message::Close(Some(CloseFrame {
        code: CloseCode::Library(code),
        reason: reason.into(),
    }))
}

struct ConnectionState {
    /// Address the client is held accountable for; see `resolve_client_ip`.
    client_ip: IpAddr,
    /// Client program reported in `ClientInfo`. `None` until the handshake
    /// arrives, which is what gates every other message.
    client_kind: Option<ClientKind>,
    /// Set when the connection must be dropped right after its pending
    /// responses are flushed (protocol mismatch).
    must_close: bool,
    account_name: Option<String>,
    account_session_id: Option<u64>,
    account_rx: Option<mpsc::UnboundedReceiver<ServerMessage>>,
    player_id: Option<PlayerId>,
    /// Entered character's name, kept here so disconnect-path logs can name the
    /// player after `GameState` has already dropped the record.
    character_name: Option<String>,
    direct_rx: Option<mpsc::UnboundedReceiver<DirectMessage>>,
    pending_character_attributes: Option<CharacterAttributes>,
    connected_at: std::time::Instant,
    last_heartbeat: std::time::Instant,
    is_official_npc: bool,
    /// Account email is on the admin allowlist.
    admin_eligible: bool,
    /// admin_eligible && the entered character's admin_role > 0.
    is_admin: bool,
    /// Last answered positions poll (spam clamp); dies with the connection.
    last_party_positions_poll: Option<Instant>,
    /// Last answered friend-presence poll (spam clamp).
    last_friends_online_poll: Option<Instant>,
    /// An `EnvReport` was already logged; later ones are dropped (spam clamp).
    env_reported: bool,
}

/// Positions snapshot requests inside this window are dropped. Steady-state
/// data rides the push tick; a client only asks on map open, so this is
/// purely a spam brake.
const PARTY_POSITIONS_MIN_INTERVAL: Duration = Duration::from_secs(2);

/// Friend-presence polls inside this window are dropped. The web client polls
/// every 15s with its panel open and every 60s without; this only bounds what
/// a rewritten client can ask for.
const FRIENDS_ONLINE_MIN_INTERVAL: Duration = Duration::from_secs(5);

/// True at most once per clamp window; a clamped poll does not refresh the
/// window, so spam cannot starve refreshes.
fn poll_due(last_poll: &mut Option<Instant>, min_interval: Duration) -> bool {
    let now = Instant::now();
    if last_poll.is_some_and(|last| now.duration_since(last) < min_interval) {
        return false;
    }
    *last_poll = Some(now);
    true
}

impl ConnectionState {
    fn new(client_ip: IpAddr) -> Self {
        Self {
            client_ip,
            client_kind: None,
            must_close: false,
            account_name: None,
            account_session_id: None,
            account_rx: None,
            player_id: None,
            character_name: None,
            direct_rx: None,
            pending_character_attributes: None,
            connected_at: std::time::Instant::now(),
            last_heartbeat: std::time::Instant::now(),
            is_official_npc: false,
            admin_eligible: false,
            is_admin: false,
            last_party_positions_poll: None,
            last_friends_online_poll: None,
            env_reported: false,
        }
    }

    fn party_positions_poll_due(&mut self) -> bool {
        poll_due(
            &mut self.last_party_positions_poll,
            PARTY_POSITIONS_MIN_INTERVAL,
        )
    }

    fn friends_online_poll_due(&mut self) -> bool {
        poll_due(
            &mut self.last_friends_online_poll,
            FRIENDS_ONLINE_MIN_INTERVAL,
        )
    }

    fn require_auth(&self, action: &str) -> Result<String, Vec<ServerMessage>> {
        match &self.account_name {
            Some(name) => Ok(name.clone()),
            None => {
                warn!("{} requested by unauthenticated client", action);
                Err(vec![ServerMessage::CharacterError {
                    message: "Authenticate first".to_string(),
                }])
            }
        }
    }

    /// Merchant and Guard are reserved for operator-run NPCs. The web client
    /// simply does not offer them, so this catches hand-rolled clients —
    /// agent-clients included, which is the point.
    fn require_selectable_class(&self, class: &CharacterClass) -> Result<(), Vec<ServerMessage>> {
        if self.is_official_npc || class.is_player_selectable() {
            return Ok(());
        }
        warn!(
            "Rejected {:?} character for account {:?}: class is operator-only",
            class, self.account_name
        );
        Err(vec![ServerMessage::CharacterError {
            message: format!("The {} class is not available", class.as_str()),
        }])
    }

    fn require_not_in_game(&self, action: &str) -> Result<(), Vec<ServerMessage>> {
        if self.player_id.is_some() {
            warn!("{} ignored because client is already in game", action);
            Err(vec![ServerMessage::CharacterError {
                message: format!("Cannot {} while in game", action),
            }])
        } else {
            Ok(())
        }
    }
}

/// Per-server services every connection needs, bundled so the accept loop
/// clones one `Arc` per connection instead of four.
pub struct ServerContext {
    pub game_state: Arc<GameState>,
    pub auth_service: Arc<AuthService>,
    pub auth_ctx: Arc<AuthContext>,
    pub connect_limiter: ConnectLimiter,
}

// `ErrorResponse` is a large http::Response; the shape is tungstenite's
// handshake-callback signature, not ours.
#[allow(clippy::result_large_err)]
pub async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    ctx: Arc<ServerContext>,
    shutdown_started: watch::Receiver<()>,
    mut shutdown: watch::Receiver<()>,
) {
    let ServerContext {
        game_state,
        auth_service,
        auth_ctx,
        connect_limiter,
    } = &*ctx;
    let ws_config = WebSocketConfig::default()
        .max_message_size(Some(MAX_WS_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_WS_MESSAGE_BYTES))
        .read_buffer_size(WS_READ_BUFFER_BYTES);

    // `X-Real-IP` rides the upgrade request and is gone once the handshake
    // future resolves, so catch it in the callback.
    let forwarded_ip: OnceLock<IpAddr> = OnceLock::new();
    let on_request = |req: &Request, res: Response| -> Result<Response, ErrorResponse> {
        if let Some(ip) = req
            .headers()
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<IpAddr>().ok())
        {
            let _ = forwarded_ip.set(ip);
        }
        Ok(res)
    };

    let handshake = tokio::select! {
        biased;
        _ = shutdown.changed() => {
            info!("Closing pending connection for server shutdown");
            return;
        }
        result = accept_hdr_async_with_config(stream, on_request, Some(ws_config)) => result,
    };
    let ws_stream = match handshake {
        Ok(ws) => ws,
        Err(e) => {
            error!("WebSocket handshake failed from {}: {}", peer.ip(), e);
            return;
        }
    };

    let client_ip = resolve_client_ip(peer, forwarded_ip.get().copied());

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // Charged here rather than at accept: behind nginx the peer address is
    // useless, and this is the first point where the real one is known.
    // Connections that skipped nginx were already charged in the accept loop.
    if peer.ip().is_loopback() && !connect_limiter.allow(client_ip) {
        if let Some(tail) = RATE_LIMIT_LOG.claim() {
            warn!("Rate-limiting client {client_ip}: too many connections{tail}");
        }
        let _ = ws_sender
            .send(close_frame(
                onlinerpg_shared::CLOSE_CODE_RATE_LIMITED,
                "too many connections",
            ))
            .await;
        return;
    }

    debug!("New WebSocket connection established from {client_ip}");

    let mut game_receiver = game_state.subscribe();
    let mut state = ConnectionState::new(client_ip);

    let mut heartbeat_check = tokio::time::interval(std::time::Duration::from_secs(10));
    let mut unauth_message_count: u32 = 0;

    // Built once and polled by reference: rebuilding it per iteration would
    // re-register a waker on tokio's shared notify shards for every message.
    let shutdown = async move {
        let _ = shutdown.changed().await;
    };
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            biased;

            _ = &mut shutdown => {
                info!("Closing connection for server shutdown");
                break;
            }

            // A replacement login wins over buffered traffic from the stale socket.
            account_msg = async {
                match state.account_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(msg) = account_msg {
                    if let Some(bytes) = encode_server_msg(&msg) {
                        let _ = ws_sender.send(Message::Binary(bytes)).await;
                    }
                    info!(
                        "Account {:?} kicked by a replacement login",
                        state.account_name
                    );
                    break;
                }
            }

            // Periodic timeout checks: unauth grace period, in-game heartbeat
            _ = heartbeat_check.tick() => {
                if state.account_name.is_none()
                    && state.connected_at.elapsed().as_secs() > UNAUTH_TIMEOUT_SECS
                {
                    warn!("Dropping connection: unauthenticated after {}s", UNAUTH_TIMEOUT_SECS);
                    let _ = ws_sender.send(close_frame(
                        onlinerpg_shared::CLOSE_CODE_IDLE_TIMEOUT,
                        "login did not complete in time",
                    )).await;
                    break;
                }
                if state.account_name.is_none() {
                    // World broadcasts are withheld until auth, so this ping is
                    // the only thing telling a client mid-login that the socket
                    // is still alive. Costs nothing at rest: a connection is
                    // unauthenticated for at most UNAUTH_TIMEOUT_SECS.
                    let _ = ws_sender.send(Message::Ping(Bytes::new())).await;
                }
                if state.player_id.is_some()
                    && state.last_heartbeat.elapsed().as_secs() > HEARTBEAT_TIMEOUT_SECS
                {
                    warn!("Heartbeat timeout for player {:?}", state.character_name);
                    let _ = ws_sender.send(close_frame(
                        onlinerpg_shared::CLOSE_CODE_IDLE_TIMEOUT,
                        "no heartbeat",
                    )).await;
                    break;
                }
                continue;
            }

            // Handle incoming messages from client
            msg = ws_receiver.next() => {
                // Metered before the type match so Text/Ping can't bypass it
                if state.account_name.is_none() {
                    if let Some(Ok(m)) = &msg {
                        unauth_message_count += 1;
                        if m.len() > UNAUTH_MAX_MESSAGE_BYTES
                            || unauth_message_count > UNAUTH_MAX_MESSAGES
                        {
                            warn!(
                                "Dropping unauthenticated connection: pre-auth limits exceeded ({} bytes, message #{})",
                                m.len(),
                                unauth_message_count
                            );
                            break;
                        }
                    }
                }
                match msg {
                    Some(Ok(Message::Binary(bytes))) => {
                        match handle_client_message(
                            &bytes,
                            game_state,
                            auth_service,
                            auth_ctx,
                            &mut state,
                        )
                        .await
                        {
                            Ok(responses) => {
                                // Send all direct responses to this client
                                for response in responses {
                                    let Some(bytes) = encode_server_msg(&response) else {
                                        continue;
                                    };
                                    if let Err(e) = ws_sender.send(Message::Binary(bytes)).await {
                                        error!("Failed to send direct response to client: {}", e);
                                    }
                                }
                                if state.must_close {
                                    debug!("Closing connection: failed protocol handshake");
                                    // Short reason: 123-byte cap. The full
                                    // hint went out as the AuthError above.
                                    let _ = ws_sender
                                        .send(close_frame(
                                            onlinerpg_shared::CLOSE_CODE_PROTOCOL_MISMATCH,
                                            "protocol mismatch",
                                        ))
                                        .await;
                                    break;
                                }
                            }
                            Err(e) => {
                                error!("Error handling client message: {}", e);
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("Client requested close");
                        break;
                    }
                    Some(Err(e)) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                    None => {
                        info!("WebSocket stream ended");
                        break;
                    }
                    _ => {}
                }
            }

            // Handle game state broadcasts
            broadcast_msg = game_receiver.recv() => {
                match broadcast_msg {
                    // Drop world state for unauthenticated sockets (info leak)
                    Ok(_) if state.account_name.is_none() => {}
                    Ok(msg) => {
                        if let Err(e) = ws_sender.send(Message::Binary(msg.bytes.clone())).await {
                            error!("Failed to send message to client: {}", e);
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("Game state broadcast channel closed");
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("Client lagged behind, skipped {} messages", skipped);
                    }
                }
            }

            // Handle direct messages to this player
            direct_msg = async {
                match state.direct_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match direct_msg {
                    Some(DirectMessage::Shared(bytes)) => {
                        let _ = ws_sender.send(Message::Binary(bytes)).await;
                    }
                    Some(DirectMessage::Typed(msg)) => {
                        let is_kicked = matches!(msg, ServerMessage::Kicked { .. });
                        if let Some(bytes) = encode_server_msg(&msg) {
                            let _ = ws_sender.send(Message::Binary(bytes)).await;
                        }
                        if is_kicked {
                            info!("Player {:?} kicked", state.character_name);
                            break;
                        }
                    }
                    None => {}
                }
            }
        }
    }

    // Once the drain starts, `persist_shutdown_snapshot` owns persistence for
    // every connected player and needs these maps left populated to see them.
    if !shutdown_started.has_changed().unwrap_or(true) {
        if let (Some(account_name), Some(session_id)) =
            (state.account_name.as_deref(), state.account_session_id)
        {
            game_state
                .end_account_session(account_name, session_id, auth_service)
                .await;
        }
    }

    match &state.character_name {
        Some(name) => info!("Session ended for {name} ({})", state.client_ip),
        None => debug!("Connection handler finished"),
    }
}

/// Shared tail of both auth paths: load characters, replace any account
/// session, and build the AuthSuccess reply.
async fn finish_auth(
    game_state: &GameState,
    auth_service: &AuthService,
    state: &mut ConnectionState,
    account_name: String,
    is_official_npc: bool,
) -> Vec<ServerMessage> {
    let character_records = match auth_service.list_characters(&account_name) {
        Ok(characters) => characters,
        Err(err) => {
            warn!(
                "Failed to load character list for account '{}': {}",
                account_name, err
            );
            return vec![ServerMessage::AuthError {
                message: err.client_message().to_string(),
            }];
        }
    };

    let characters = character_records
        .into_iter()
        .map(character_record_to_shared)
        .collect::<Vec<Character>>();

    let (kick_tx, kick_rx) = mpsc::unbounded_channel();

    // The single gate both login paths pass through: a ban stops the session
    // here, not at character select, because the account carries it. Checked
    // and registered under the one lock `/ban` also takes, so a ban landing
    // in between cannot admit a session it can no longer see to evict.
    let account_session_id = {
        let _sessions = game_state.lock_character_sessions().await;
        let ban = {
            let auth = auth_service.clone();
            let account = account_name.clone();
            crate::game_state::auth_db(move || auth.active_ban(&account)).await
        };
        match ban {
            Ok(Some(ban)) => {
                info!("Rejected banned account '{}'", account_name);
                return vec![ServerMessage::AuthError {
                    message: ban.message(),
                }];
            }
            Ok(None) => {}
            Err(err) => {
                // Fail closed: an unreadable ban table must not become a way in.
                error!("Ban check failed for '{}': {}", account_name, err);
                return vec![ServerMessage::AuthError {
                    message: "Could not verify the account. Try again shortly.".to_string(),
                }];
            }
        }
        game_state
            .register_account_session_locked(&account_name, kick_tx, auth_service)
            .await
    };

    state.account_name = Some(account_name.clone());
    state.account_session_id = Some(account_session_id);
    state.account_rx = Some(kick_rx);
    state.is_official_npc = is_official_npc;
    state.pending_character_attributes = None;

    info!(
        "Account '{}' authenticated successfully with {} character(s)",
        account_name,
        characters.len()
    );
    vec![ServerMessage::AuthSuccess {
        account_name,
        characters,
    }]
}

/// Debug/cheat messages every new Debug* variant must be added to; anything
/// listed here is dropped before dispatch unless the connection is admin.
fn requires_admin(msg: &ClientMessage) -> bool {
    match msg {
        ClientMessage::DebugTeleport { .. }
        | ClientMessage::DebugDropItem { .. }
        | ClientMessage::DebugSetTime { .. }
        | ClientMessage::DebugResetDungeonProps { .. } => true,
        ClientMessage::ChatMessage { message } => {
            message.starts_with("/give ")
                || parse_notice_command(message).is_some()
                || parse_admin_command(message).is_some()
        }
        _ => false,
    }
}

/// What a refused client should do. Carried in the rejection text because that
/// message is the only thing an out-of-date client can still show, and it has
/// to serve both audiences: a browser holding a cached bundle (reload) and an
/// agent-client binary on someone else's machine (download a new one).
const CLIENT_UPDATE_HINT: &str =
    "reload the page, or update agent-client (https://github.com/Julian-adv/OpenMMO)";

/// Protocol handshake gate, run before every other message.
///
/// `ClientInfo` must be the first message on a connection and must carry this
/// server's exact `PROTOCOL_VERSION`; anything else is refused with an
/// `AuthError` naming the mismatch, and the connection is closed. Deliberately
/// strict: version-straddling code is the kind that only breaks on the clients
/// we cannot redeploy. See `doc/REMOTE_AGENT_CLIENT.md`.
///
/// Returns `Some(responses)` when the message was consumed (handshake or
/// rejection), `None` when the caller should keep handling it.
fn handle_handshake(
    client_msg: &ClientMessage,
    state: &mut ConnectionState,
) -> Option<Vec<ServerMessage>> {
    if let ClientMessage::ClientInfo {
        protocol_version,
        client_kind,
        client_version,
    } = client_msg
    {
        if state.client_kind.is_some() {
            warn!("Duplicate ClientInfo ignored");
            return Some(vec![]);
        }
        if *protocol_version != onlinerpg_shared::PROTOCOL_VERSION {
            if let Some(tail) = PROTOCOL_REFUSAL_LOG.claim() {
                warn!(
                    "Refusing client: protocol v{} (server speaks v{}) ip={} kind={client_kind} version={client_version}{tail}",
                    protocol_version,
                    onlinerpg_shared::PROTOCOL_VERSION,
                    state.client_ip,
                );
            }
            state.must_close = true;
            return Some(vec![ServerMessage::AuthError {
                message: format!(
                    "Protocol v{} required, you sent v{} — {CLIENT_UPDATE_HINT}",
                    onlinerpg_shared::PROTOCOL_VERSION,
                    protocol_version
                ),
            }]);
        }
        let kind = ClientKind::from_reported(client_kind);
        info!(
            "Client handshake: kind={} version={client_version} ip={}",
            kind.as_str(),
            state.client_ip
        );
        state.client_kind = Some(kind);
        return Some(vec![]);
    }

    if state.client_kind.is_none() {
        if let Some(tail) = EARLY_MESSAGE_LOG.claim() {
            let ip = state.client_ip;
            warn!("Refusing client {ip}: message arrived before ClientInfo{tail}");
        }
        state.must_close = true;
        return Some(vec![ServerMessage::AuthError {
            message: format!("Send ClientInfo first — {CLIENT_UPDATE_HINT}"),
        }]);
    }
    None
}

async fn handle_client_message(
    message: &[u8],
    game_state: &Arc<GameState>,
    auth_service: &Arc<AuthService>,
    auth_ctx: &Arc<AuthContext>,
    state: &mut ConnectionState,
) -> Result<Vec<ServerMessage>, Box<dyn std::error::Error + Send + Sync>> {
    let client_msg: ClientMessage = deserialize_client_msg(message)?;

    if let Some(responses) = handle_handshake(&client_msg, state) {
        return Ok(responses);
    }

    if matches!(
        client_msg,
        ClientMessage::Authenticate { .. } | ClientMessage::AuthenticateNpc { .. }
    ) && state.account_name.is_some()
    {
        warn!("Client is already authenticated");
        return Ok(vec![ServerMessage::AuthError {
            message: "Already authenticated".to_string(),
        }]);
    }

    if requires_admin(&client_msg) && !state.is_admin {
        warn!(
            "Admin-only message rejected for account {:?}",
            state.account_name
        );
        return Ok(match &state.player_id {
            Some(_) => vec![ServerMessage::SystemMessage {
                message: "Admin only".to_string(),
            }],
            None => vec![],
        });
    }

    match client_msg {
        ClientMessage::Authenticate { google_id_token } => {
            let Some(verifier) = &auth_ctx.google else {
                warn!("Google login attempted but no --google-client-id is configured");
                return Ok(vec![ServerMessage::AuthError {
                    message: "Google sign-in is not configured on this server".to_string(),
                }]);
            };

            let claims = match verifier.verify(&google_id_token).await {
                Ok(claims) => claims,
                Err(err) => {
                    warn!("Google token verification failed: {}", err);
                    return Ok(vec![ServerMessage::AuthError {
                        message: "Google sign-in verification failed".to_string(),
                    }]);
                }
            };

            let account_name = match auth_service.login_google(&claims.sub) {
                Ok(name) => name,
                Err(err) => {
                    warn!("Google login failed for sub '{}': {}", claims.sub, err);
                    return Ok(vec![ServerMessage::AuthError {
                        message: err.client_message().to_string(),
                    }]);
                }
            };
            info!("Google sub '{}' -> account '{}'", claims.sub, account_name);

            state.admin_eligible = auth_ctx.is_admin(&claims);
            return Ok(finish_auth(game_state, auth_service, state, account_name, false).await);
        }

        ClientMessage::AuthenticateNpc {
            account_name,
            npc_token,
        } => {
            if !token_matches(&npc_token, &auth_ctx.npc_token) {
                warn!("NPC auth rejected for {:?}: bad token", account_name);
                return Ok(vec![ServerMessage::AuthError {
                    message: "Invalid NPC token".to_string(),
                }]);
            }

            let account_name = match auth_service.login_npc(&account_name) {
                Ok(name) => name,
                Err(err) => {
                    warn!("NPC login failed for {:?}: {}", account_name, err);
                    return Ok(vec![ServerMessage::AuthError {
                        message: err.client_message().to_string(),
                    }]);
                }
            };

            return Ok(finish_auth(game_state, auth_service, state, account_name, true).await);
        }

        ClientMessage::CreateCharacter {
            character_name,
            character_class,
            gender,
        } => {
            if let Err(responses) = state.require_not_in_game("CreateCharacter") {
                return Ok(responses);
            }
            let authed_account_name = match state.require_auth("CreateCharacter") {
                Ok(name) => name,
                Err(responses) => return Ok(responses),
            };
            if let Err(responses) = state.require_selectable_class(&character_class) {
                return Ok(responses);
            }

            let Some(rolled_attributes) = state.pending_character_attributes.clone() else {
                warn!(
                    "Character creation requested without rolled stats for account '{}'",
                    authed_account_name
                );
                return Ok(vec![ServerMessage::CharacterError {
                    message: "Roll attributes first".to_string(),
                }]);
            };

            let max_hp = default_character_max_hp(&rolled_attributes, &character_class);
            match auth_service.create_character(
                &authed_account_name,
                &character_name,
                &rolled_attributes,
                max_hp,
                character_class.clone(),
                gender,
            ) {
                Ok(character) => {
                    state.pending_character_attributes = None;
                    info!(
                        "Character '{}' created for account '{}'",
                        character.name, authed_account_name
                    );
                    return Ok(vec![ServerMessage::CharacterCreated {
                        character: character_record_to_shared(character),
                    }]);
                }
                Err(err) => {
                    warn!(
                        "Character create failed for account '{}': {}",
                        authed_account_name, err
                    );
                    return Ok(vec![ServerMessage::CharacterError {
                        message: err.client_message().to_string(),
                    }]);
                }
            }
        }

        ClientMessage::DeleteCharacter { character_id } => {
            if let Err(responses) = state.require_not_in_game("DeleteCharacter") {
                return Ok(responses);
            }
            let authed_account_name = match state.require_auth("DeleteCharacter") {
                Ok(name) => name,
                Err(responses) => return Ok(responses),
            };
            match game_state
                .delete_character_if_inactive(auth_service, &authed_account_name, character_id)
                .await
            {
                Ok(true) => {
                    info!(
                        "Character id={} deleted for account '{}'",
                        character_id, authed_account_name
                    );
                    return Ok(vec![ServerMessage::CharacterDeleted { character_id }]);
                }
                Ok(false) => {
                    warn!(
                        "Character delete rejected for account '{}': id={} is active",
                        authed_account_name, character_id
                    );
                    return Ok(vec![ServerMessage::CharacterError {
                        message: "Cannot delete a character while it is in game".to_string(),
                    }]);
                }
                Err(err) => {
                    warn!(
                        "Character delete failed for account '{}': {}",
                        authed_account_name, err
                    );
                    return Ok(vec![ServerMessage::CharacterError {
                        message: err.client_message().to_string(),
                    }]);
                }
            }
        }

        ClientMessage::RollCharacterStats {
            character_class,
            gender,
        } => {
            if let Err(responses) = state.require_not_in_game("RollCharacterStats") {
                return Ok(responses);
            }
            if let Err(responses) = state.require_auth("RollCharacterStats") {
                return Ok(responses);
            }
            if let Err(responses) = state.require_selectable_class(&character_class) {
                return Ok(responses);
            }

            let attributes = roll_character_attributes(&character_class, gender);
            let max_hp = default_character_max_hp(&attributes, &character_class);
            state.pending_character_attributes = Some(attributes.clone());
            return Ok(vec![ServerMessage::CharacterStatsRolled {
                attributes,
                max_hp,
            }]);
        }

        ClientMessage::EnterGame { character_id } => {
            if state.player_id.is_some() {
                warn!("Client already entered game, ignoring EnterGame request");
                return Ok(vec![]);
            }

            let authed_account_name = match state.require_auth("EnterGame") {
                Ok(name) => name,
                Err(responses) => return Ok(responses),
            };
            let character_sessions = game_state.lock_character_sessions().await;
            let Some(account_session_id) = state.account_session_id else {
                return Ok(vec![]);
            };
            if !game_state
                .is_current_account_session(&authed_account_name, account_session_id)
                .await
            {
                return Ok(vec![]);
            }

            let selected_character =
                match auth_service.get_character_for_account(&authed_account_name, character_id) {
                    Ok(character) => character,
                    Err(err) => {
                        warn!(
                            "EnterGame failed for account '{}': {}",
                            authed_account_name, err
                        );
                        return Ok(vec![ServerMessage::CharacterError {
                            message: err.client_message().to_string(),
                        }]);
                    }
                };

            state.is_admin = state.admin_eligible && selected_character.admin_role > 0;
            if state.is_admin {
                info!(
                    "Account '{}' entering as admin character '{}' (role {})",
                    authed_account_name, selected_character.name, selected_character.admin_role
                );
            }

            // Skills and dungeon history load before any registration: a failed
            // read must refuse the session, or an empty fallback would overwrite
            // trained skills on save and re-grant already-opened chest rewards.
            let (skill_rows, chest_opens, discovered_dungeons) = {
                let auth = Arc::clone(auth_service);
                let loaded = crate::game_state::auth_db(move || {
                    Ok((
                        auth.load_skills(character_id)?,
                        auth.load_dungeon_history(character_id)?,
                    ))
                })
                .await;
                match loaded {
                    Ok((rows, (opens, ids))) => (rows, opens, ids),
                    Err(err) => {
                        warn!(
                            "Failed to load required state for character {}: {} — refusing session",
                            character_id, err
                        );
                        return Ok(vec![ServerMessage::CharacterError {
                            message: err.client_message().to_string(),
                        }]);
                    }
                }
            };

            let max_hp = selected_character.max_hp;
            let character_xp = selected_character.xp;

            let mut player = new_player(
                selected_character.name.clone(),
                selected_character.level,
                max_hp,
                selected_character.class.clone(),
                selected_character.gender,
                Position {
                    x: selected_character.last_x,
                    y: selected_character.last_y,
                    z: selected_character.last_z,
                },
                selected_character.last_rotation,
                state.is_official_npc,
                state.client_kind.unwrap_or_default(),
            );

            // Restore saved health (if available) and floor_level from DB
            if let Some(saved_health) = selected_character.health {
                player.health = saved_health.min(max_hp);
            }
            let saved_floor = selected_character.floor_level;
            player.floor_level = restored_floor_level(saved_floor);
            if player.floor_level != saved_floor {
                let spawn = &crate::world_config::world_config().spawn_position;
                player.position = spawn.position();
                player.rotation = spawn.rotation;
                warn!(
                    "Reset out-of-range stored floor {} to the world spawn for character '{}'",
                    saved_floor, selected_character.name
                );
            }
            // A negative floor means the player logged out inside a
            // dungeon: re-prime that dungeon's runtime, or fall back to
            // the world spawn if the entrance no longer exists.
            if player.floor_level < 0 {
                let ok = game_state
                    .rehydrate_dungeon_player(&player.id, &player.position, player.floor_level)
                    .await;
                if !ok {
                    let spawn = &crate::world_config::world_config().spawn_position;
                    player.position = spawn.position();
                    player.rotation = spawn.rotation;
                    player.floor_level = 0;
                }
            }
            let id = player.id;

            state.direct_rx = Some(game_state.register_connection_channel(&id).await);
            game_state
                .register_player_character(
                    &id,
                    character_id,
                    character_xp,
                    selected_character.attributes.clone(),
                    selected_character.gold,
                    (!state.is_official_npc).then_some(selected_character.satiation),
                )
                .await;
            if !game_state
                .attach_player_to_account_session(&authed_account_name, account_session_id, id)
                .await
            {
                warn!(
                    "Account session changed during game admission for '{}'",
                    authed_account_name
                );
                return Ok(vec![]);
            }
            let auth = Arc::clone(auth_service);
            match crate::game_state::auth_db(move || {
                Ok((
                    auth.load_blocked_names(character_id)?,
                    auth.load_friends(character_id)?,
                ))
            })
            .await
            {
                Ok((blocked, friends)) => {
                    game_state.set_player_blocks(&id, blocked).await;
                    game_state.set_player_friends(&id, friends).await;
                }
                Err(err) => warn!(
                    "Failed to load block/friend lists for character {}: {}",
                    character_id, err
                ),
            }

            game_state.set_chest_opens(character_id, chest_opens).await;
            game_state
                .set_dungeon_discoveries(&id, discovered_dungeons.clone())
                .await;

            // Load inventory from DB
            game_state
                .load_player_inventory(&id, character_id, auth_service)
                .await;
            if state.is_official_npc {
                game_state.seed_npc_loadout(&id, &player.name).await;
                game_state.seed_npc_keepsakes(&id, &player.name).await;
            }

            // Missing rows = never trained.
            let skills = crate::game_state::skills_from_rows(&skill_rows);
            game_state.register_player_skills(&id, skills.clone()).await;

            // The equipped off-hand is the authoritative carried-torch state.
            // Resolve it before add_player builds the late-join GameState snapshot.
            let inventory = game_state.get_player_inventory(&id).await;
            player.torch_on = inventory.as_ref().is_some_and(|inv| inv.is_torch_lit());
            player.main_hand = inventory.as_ref().and_then(|inv| inv.main_hand_def_id());

            let mut responses = vec![ServerMessage::JoinSuccess {
                player: player.clone(),
                is_admin: state.is_admin,
            }];
            let datetime = game_state.current_game_datetime();
            responses.push(ServerMessage::GameTimeSync {
                is_night: GameState::is_night(&datetime),
                datetime,
            });

            // Send no-spawn zones so client can validate spawn positions
            responses.push(ServerMessage::NoSpawnZones {
                zones: game_state.no_spawn_zones().to_vec(),
            });

            // Send inventory state
            if let Some(inv) = inventory {
                responses.push(ServerMessage::InventoryState { inventory: inv });
            }

            responses.push(ServerMessage::GuardUpdated {
                guard: game_state.effective_guard(&id).await,
            });

            responses.push(ServerMessage::GoldUpdate {
                gold: selected_character.gold,
            });

            responses.push(ServerMessage::SkillsUpdate { skills });

            responses.push(ServerMessage::DungeonDiscoveries {
                entrance_ids: discovered_dungeons,
            });

            if !state.is_official_npc {
                responses.push(crate::game_state::hunger::hunger_update_msg(
                    selected_character.satiation,
                    0,
                ));
            }

            if let Some(notice) = game_state.server_notice().await {
                responses.push(ServerMessage::ServerNotice {
                    message: Some(notice),
                });
            }

            let rejoin_floor = player.floor_level;
            let rejoin_pos = player.position;
            responses.extend(game_state.add_player(player).await);

            // After the snapshot on purpose: the client treats `GameState` as
            // the start of a session and clears its friend stores there.
            responses.push(game_state.friend_list_message(&id).await);
            if rejoin_floor < 0 {
                // Rejoining inside a dungeon: enter its floor (occupancy
                // + lazy monster spawn with this player as AI owner).
                game_state
                    .handle_player_floor_change(&id, 0, rejoin_floor, &rejoin_pos, &rejoin_pos)
                    .await;
            }

            state.player_id = Some(id);
            state.character_name = Some(selected_character.name.clone());
            drop(character_sessions);

            info!(
                "Account '{}' entered game as character '{}' with player ID {:?}",
                authed_account_name, selected_character.name, state.player_id
            );
            return Ok(responses);
        }

        ClientMessage::PlayerMove {
            position,
            rotation,
            floor_level,
            append,
            sprinting,
        } => {
            if let Some(id) = &state.player_id {
                game_state
                    .update_player_position(
                        id,
                        crate::game_state::MoveCommand {
                            position,
                            rotation,
                            floor_level,
                            append,
                            sprinting,
                        },
                        state.is_admin,
                        state.is_official_npc,
                    )
                    .await;
            } else {
                warn!("Received move from client that is not in game");
            }
        }

        ClientMessage::PlayerFloorChanged { floor_level } => {
            if let Some(id) = &state.player_id {
                game_state.update_player_floor(id, floor_level).await;
            } else {
                warn!("Received floor change from client that is not in game");
            }
        }

        ClientMessage::ChatMessage { message } => {
            if let Some(id) = &state.player_id {
                game_state
                    .send_chat_message(id, message, auth_service)
                    .await;
            } else {
                warn!("Received chat message from client that is not in game");
            }
        }

        ClientMessage::RequestSpawnMonster {
            monster_type,
            position,
            rotation,
        } => {
            if let Some(id) = &state.player_id {
                let Some(position) = game_state
                    .validate_spawn_request(id, &monster_type, &position, rotation)
                    .await
                else {
                    warn!(
                        "Spawn request rejected: position ({:.1}, {:.1}) rotation {:.1} invalid for {}",
                        position.x, position.z, rotation, monster_type
                    );
                    return Ok(vec![]);
                };
                if !game_state.take_spawn_allowance(id, &monster_type).await {
                    warn!(
                        "Spawn request rejected: no unconsumed allowance for {}",
                        monster_type
                    );
                } else if let Some(monster) = game_state
                    .spawn_monster(monster_type, position, rotation, Some(*id), 0, None, false)
                    .await
                {
                    game_state
                        .send_direct_message(id, ServerMessage::MonsterAssigned { monster })
                        .await;
                }
            } else {
                warn!("Received spawn request from client that is not in game");
            }
        }

        ClientMessage::MonsterMove {
            monster_id,
            position,
            rotation,
            state: monster_state,
            target_position,
        } => {
            if let Some(id) = &state.player_id {
                game_state
                    .update_monster_position(
                        id,
                        monster_id,
                        position,
                        rotation,
                        monster_state,
                        target_position,
                    )
                    .await;
            } else {
                warn!("Received monster move from client that is not in game");
            }
        }

        ClientMessage::PlayerAttack { monster_id } => {
            if let Some(id) = &state.player_id {
                game_state.broadcast_player_attack(id, monster_id).await;
            } else {
                warn!("Received attack from client that is not in game");
            }
        }

        ClientMessage::FishingCast { position } => {
            if let Some(id) = &state.player_id {
                game_state.start_fishing(id, position).await;
            } else {
                warn!("Received fishing cast from client that is not in game");
            }
        }

        ClientMessage::FishingRespond { action } => {
            if let Some(id) = &state.player_id {
                game_state.respond_fishing(id, action).await;
            } else {
                warn!("Received fishing response from client that is not in game");
            }
        }

        ClientMessage::FishingStop => {
            if let Some(id) = &state.player_id {
                game_state.stop_fishing(id).await;
            } else {
                warn!("Received fishing stop from client that is not in game");
            }
        }

        ClientMessage::MonsterAttack {
            monster_id,
            target_player_id,
        } => {
            if let Some(id) = &state.player_id {
                game_state
                    .broadcast_monster_attack(id, &monster_id, &target_player_id)
                    .await;
            } else {
                warn!("Received monster attack from client that is not in game");
            }
        }

        ClientMessage::RequestRespawn => {
            if let Some(id) = &state.player_id {
                game_state.respawn_player(id).await;
            } else {
                warn!("Received respawn request from client that is not in game");
            }
        }

        ClientMessage::OpenDungeonChest { entrance_id } => {
            if let Some(id) = &state.player_id {
                game_state
                    .open_dungeon_chest(id, &entrance_id, auth_service)
                    .await;
            } else {
                warn!("Received chest open from client that is not in game");
            }
        }

        ClientMessage::BreakDungeonProp {
            entrance_id,
            depth,
            prop_id,
        } => {
            if let Some(id) = &state.player_id {
                game_state
                    .break_dungeon_prop(id, &entrance_id, depth, prop_id)
                    .await;
            } else {
                warn!("Received prop break from client that is not in game");
            }
        }

        ClientMessage::OpenDungeonProp {
            entrance_id,
            depth,
            prop_id,
        } => {
            if let Some(id) = &state.player_id {
                game_state
                    .open_dungeon_prop(id, &entrance_id, depth, prop_id)
                    .await;
            } else {
                warn!("Received prop open from client that is not in game");
            }
        }

        ClientMessage::ToggleDungeonDoor {
            entrance_id,
            depth,
            door_id,
        } => {
            if let Some(id) = &state.player_id {
                if let Some(is_open) = game_state
                    .toggle_dungeon_door(id, &entrance_id, depth, door_id)
                    .await
                {
                    game_state
                        .publish_dungeon_door_toggle(id, entrance_id, depth, door_id, is_open)
                        .await;
                }
            }
        }

        ClientMessage::RequestDungeonDoors { entrance_id } => {
            if let Some(id) = &state.player_id {
                let doors = game_state.dungeon_open_doors(&entrance_id).await;
                game_state
                    .send_direct_message(
                        id,
                        ServerMessage::DungeonDoorsState { entrance_id, doors },
                    )
                    .await;
            }
        }

        ClientMessage::DebugTeleport { position } => {
            if let Some(id) = &state.player_id {
                let rotation = game_state
                    .get_player_position(id)
                    .await
                    .map(|(_, rot, _)| rot)
                    .unwrap_or(0.0);
                // Debug teleports can land inside a dungeon; infer the
                // floor from the target Y instead of trusting the old one.
                let floor_level = game_state.dungeon_floor_for_position(&position).await;
                game_state
                    .teleport_player(id, position, rotation, floor_level)
                    .await;
            } else {
                warn!("Received debug teleport from client that is not in game");
            }
        }

        ClientMessage::DebugDropItem { item_def_id } => {
            if let Some(id) = &state.player_id {
                game_state.debug_drop_item(id, &item_def_id).await;
            } else {
                warn!("Received debug drop from client that is not in game");
            }
        }

        ClientMessage::DebugSetTime { hour, minute } => {
            if state.player_id.is_some() {
                let datetime = game_state.debug_set_time(hour, minute);
                info!("Debug time jump to {}", datetime);
            } else {
                warn!("Received debug set time from client that is not in game");
            }
        }

        ClientMessage::DebugResetDungeonProps { entrance_id } => {
            if state.player_id.is_some() {
                game_state.debug_reset_dungeon_props(&entrance_id).await;
            } else {
                warn!("Received debug dungeon prop reset from client that is not in game");
            }
        }

        ClientMessage::TorchToggle { enabled } => {
            if let Some(id) = &state.player_id {
                game_state.set_player_torch(id, enabled).await;
            } else {
                warn!("Received torch toggle from client that is not in game");
            }
        }

        ClientMessage::InteractObject {
            object_type,
            object_id,
        } => {
            if let Some(id) = &state.player_id {
                game_state
                    .set_player_interaction(id, Some(object_type), Some(object_id))
                    .await;
            } else {
                warn!("Received interact object from client that is not in game");
            }
        }

        ClientMessage::StopInteraction => {
            if let Some(id) = &state.player_id {
                game_state.set_player_interaction(id, None, None).await;
            } else {
                warn!("Received stop interaction from client that is not in game");
            }
        }

        ClientMessage::Heartbeat => {
            state.last_heartbeat = std::time::Instant::now();
        }

        ClientMessage::EnvReport(r) => {
            if state.env_reported {
                return Ok(vec![]);
            }
            state.env_reported = true;
            let account = state.account_name.as_deref().unwrap_or("<unauth>");
            let character = state.character_name.as_deref().unwrap_or("<none>");
            // The client clamps these too, but a hostile client could skip that.
            fn clamp(s: &str, max: usize) -> &str {
                s.char_indices().nth(max).map_or(s, |(i, _)| &s[..i])
            }
            info!(
                target: "env_report",
                "account='{account}' character='{character}' backend={} gpu={}/{}/{} desc='{}' quality={}({}) aa={} \
                 pr={:.2} dpr={:.2} viewport={}x{} screen={}x{} ua='{}'",
                clamp(&r.backend, 16),
                clamp(&r.gpu_vendor, 64),
                clamp(&r.gpu_architecture, 64),
                clamp(&r.gpu_device, 64),
                clamp(&r.gpu_description, 128),
                clamp(&r.quality, 16),
                clamp(&r.render_budget, 16),
                r.antialias,
                r.pixel_ratio,
                r.device_pixel_ratio,
                r.viewport_w,
                r.viewport_h,
                r.screen_w,
                r.screen_h,
                clamp(&r.user_agent, 256),
            );
        }

        ClientMessage::PlaceHouse { .. } => {
            warn!("Ignoring client-side PlaceHouse broadcast request; use the housing REST API");
        }

        ClientMessage::ModifyRoom { .. } => {
            // TODO: room modification broadcast
        }

        ClientMessage::RemoveHouse { .. } => {
            warn!("Ignoring client-side RemoveHouse broadcast request; use the housing REST API");
        }

        ClientMessage::ToggleDoor {
            house_id,
            room_index,
            wall_dir,
            segment_index,
        } => {
            // Toggle door is_open and broadcast to all players
            if let Some(ref pid) = state.player_id {
                let toggled = game_state
                    .toggle_door(pid, &house_id, room_index, wall_dir, segment_index)
                    .await;
                if let Some(is_open) = toggled {
                    if let Some((position, _, floor_level)) =
                        game_state.get_player_position(pid).await
                    {
                        game_state
                            .send_direct_message_to_players_within_position(
                                &position,
                                floor_level,
                                crate::game_state::EVENT_DELIVERY_RADIUS,
                                ServerMessage::DoorToggled {
                                    house_id,
                                    room_index,
                                    wall_dir,
                                    segment_index,
                                    is_open,
                                },
                                None,
                            )
                            .await;
                    }
                }
            }
        }

        ClientMessage::EquipItem { instance_id } => {
            if let Some(id) = &state.player_id {
                game_state.equip_item(id, instance_id).await;
            }
        }

        ClientMessage::UnequipItem { slot } => {
            if let Some(id) = &state.player_id {
                game_state.unequip_item(id, slot).await;
            }
        }

        ClientMessage::DropItem { instance_id } => {
            if let Some(id) = &state.player_id {
                game_state.drop_item(id, instance_id).await;
            }
        }

        ClientMessage::DropItems { items } => {
            if let Some(id) = &state.player_id {
                game_state.drop_items(id, items).await;
            }
        }

        ClientMessage::PickupStarted => {
            if let Some(id) = &state.player_id {
                game_state.broadcast_pickup_animation(id).await;
            }
        }

        ClientMessage::PickupItem { instance_id } => {
            if let Some(id) = &state.player_id {
                game_state.pickup_item(id, instance_id).await;
            }
        }

        ClientMessage::UseItem { instance_id } => {
            if let Some(id) = &state.player_id {
                game_state.use_item(id, instance_id).await;
            }
        }

        ClientMessage::OpenShop { merchant_player_id } => {
            if let Some(id) = &state.player_id {
                game_state.open_shop(id, &merchant_player_id, true).await;
            }
        }

        ClientMessage::CloseShop { merchant_player_id } => {
            if let Some(id) = &state.player_id {
                game_state.close_shop(id, &merchant_player_id).await;
            }
        }

        ClientMessage::PartyInvite { target_name } => {
            if let Some(id) = &state.player_id {
                game_state.invite_to_party(id, &target_name).await;
            }
        }

        ClientMessage::PartyRespond { inviter_id, accept } => {
            if let Some(id) = &state.player_id {
                game_state
                    .respond_to_party_invite(id, &inviter_id, accept)
                    .await;
            }
        }

        ClientMessage::PartySummonRespond { caster_id, accept } => {
            if let Some(id) = &state.player_id {
                game_state
                    .respond_to_party_summon(id, &caster_id, accept)
                    .await;
            }
        }

        ClientMessage::PartyLeave => {
            if let Some(id) = &state.player_id {
                game_state.leave_party(id).await;
            }
        }

        ClientMessage::PartyKick { target_id } => {
            if let Some(id) = &state.player_id {
                game_state.kick_from_party(id, &target_id).await;
            }
        }

        ClientMessage::PartyPromote { target_id } => {
            if let Some(id) = &state.player_id {
                game_state.promote_party_leader(id, &target_id).await;
            }
        }

        ClientMessage::PartyChat { message } => {
            if let Some(id) = &state.player_id {
                game_state.send_party_chat(id, message).await;
            }
        }

        ClientMessage::FriendRespond {
            requester_id,
            accept,
        } => {
            if let Some(id) = &state.player_id {
                game_state
                    .respond_to_friend_request(id, &requester_id, accept, auth_service)
                    .await;
            }
        }

        ClientMessage::FriendRemove { name } => {
            if let Some(id) = &state.player_id {
                game_state
                    .remove_friend_by_name(id, &name, auth_service)
                    .await;
            }
        }

        ClientMessage::RequestFriendsOnline => {
            if let Some(id) = state.player_id {
                if state.friends_online_poll_due() {
                    game_state.send_friends_online(&id).await;
                }
            }
        }

        ClientMessage::RequestPartyPositions => {
            if let Some(id) = state.player_id {
                if state.party_positions_poll_due() {
                    game_state.send_party_positions(&id).await;
                }
            }
        }

        ClientMessage::BuyItem {
            merchant_player_id,
            item_def_id,
        } => {
            if let Some(id) = &state.player_id {
                game_state
                    .buy_item(id, &merchant_player_id, &item_def_id)
                    .await;
            }
        }

        ClientMessage::SellItem {
            merchant_player_id,
            instance_id,
        } => {
            if let Some(id) = &state.player_id {
                game_state
                    .sell_item(id, &merchant_player_id, instance_id)
                    .await;
            }
        }

        ClientMessage::BuybackItem {
            merchant_player_id,
            entry_id,
        } => {
            if let Some(id) = &state.player_id {
                game_state
                    .buyback_item(id, &merchant_player_id, entry_id)
                    .await;
            }
        }

        ClientMessage::BuyItems {
            merchant_player_id,
            items,
        } => {
            if let Some(id) = &state.player_id {
                game_state.buy_items(id, &merchant_player_id, items).await;
            }
        }

        ClientMessage::SellItems {
            merchant_player_id,
            items,
        } => {
            if let Some(id) = &state.player_id {
                game_state.sell_items(id, &merchant_player_id, items).await;
            }
        }

        ClientMessage::BuybackItems {
            merchant_player_id,
            entry_ids,
        } => {
            if let Some(id) = &state.player_id {
                game_state
                    .buyback_items(id, &merchant_player_id, entry_ids)
                    .await;
            }
        }

        ClientMessage::OfferDeal {
            target_player_id,
            item_def_id,
            kind,
            modifier_pct,
            reason,
        } => {
            if let Some(id) = &state.player_id {
                game_state
                    .offer_deal(
                        id,
                        &target_player_id,
                        &item_def_id,
                        kind,
                        modifier_pct,
                        &reason,
                    )
                    .await;
            }
        }

        ClientMessage::OpenTrade { target_player_id } => {
            if let Some(id) = &state.player_id {
                game_state.open_trade(id, &target_player_id).await;
            }
        }

        ClientMessage::DeclineTrade { merchant_player_id } => {
            if let Some(id) = &state.player_id {
                game_state.decline_trade(id, &merchant_player_id).await;
            }
        }

        // Consumed by `handle_handshake` above; a repeat never reaches here.
        ClientMessage::ClientInfo { .. } => {}
    }

    Ok(vec![])
}

fn character_record_to_shared(record: crate::auth::CharacterRecord) -> Character {
    Character {
        id: record.id,
        name: record.name,
        created_at: record.created_at,
        level: record.level,
        xp: record.xp,
        max_hp: record.max_hp,
        attributes: record.attributes,
        class: record.class,
        gender: record.gender,
    }
}

fn default_character_max_hp(
    attributes: &CharacterAttributes,
    character_class: &CharacterClass,
) -> u32 {
    match level_one_max_hp(DEFAULT_CHARACTER_RACE, character_class, attributes.con) {
        Ok(value) => value,
        Err(err) => {
            warn!(
                "Failed to resolve level 1 max HP for race='{}', class='{}', con='{}': {}",
                DEFAULT_CHARACTER_RACE,
                character_class.as_str(),
                attributes.con,
                err
            );
            FALLBACK_DEFAULT_MAX_HP
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn token_matches_requires_exact_token() {
        assert!(token_matches("secret-token", "secret-token"));
        assert!(!token_matches("secret-token", "secret-tokeN"));
        assert!(!token_matches("secret", "secret-token"));
        assert!(!token_matches("", "secret-token"));
    }

    /// The gate both login paths share: a banned account must not reach the
    /// character list, and lifting the ban must let it back in.
    #[tokio::test]
    async fn finish_auth_refuses_a_banned_account() {
        let game_state = crate::game_state::tests::make_test_game_state("auth_ban_gate");
        let auth = crate::game_state::tests::make_test_auth("auth_ban_gate");
        let account = auth.login_npc("npc_ban_gate").unwrap();

        let mut state = ConnectionState::new(Ipv4Addr::LOCALHOST.into());
        let ok = finish_auth(&game_state, &auth, &mut state, account.clone(), true).await;
        assert!(
            matches!(ok.as_slice(), [ServerMessage::AuthSuccess { .. }]),
            "unbanned account authenticates: {ok:?}"
        );

        auth.ban_account(&account, Some("testing"), None).unwrap();
        let mut state = ConnectionState::new(Ipv4Addr::LOCALHOST.into());
        let refused = finish_auth(&game_state, &auth, &mut state, account.clone(), true).await;
        match refused.as_slice() {
            [ServerMessage::AuthError { message }] => {
                assert!(
                    message.contains("testing"),
                    "reason reaches the client: {message}"
                )
            }
            other => panic!("expected an auth error, got {other:?}"),
        }
        assert!(
            state.account_name.is_none(),
            "a refused session must not be left holding the account"
        );

        auth.unban_account(&account).unwrap();
        let mut state = ConnectionState::new(Ipv4Addr::LOCALHOST.into());
        let ok = finish_auth(&game_state, &auth, &mut state, account, true).await;
        assert!(
            matches!(ok.as_slice(), [ServerMessage::AuthSuccess { .. }]),
            "lifting the ban restores access: {ok:?}"
        );
    }

    #[test]
    fn refusal_logging_folds_repeats_into_one_line() {
        let throttle = LogThrottle::new();
        assert_eq!(throttle.claim(), Some(String::new()));
        for _ in 0..5 {
            assert_eq!(throttle.claim(), None);
        }
    }

    fn client_info(protocol_version: u32, kind: &str) -> ClientMessage {
        ClientMessage::ClientInfo {
            protocol_version,
            client_kind: kind.to_string(),
            client_version: "test".to_string(),
        }
    }

    fn is_auth_error(responses: &Option<Vec<ServerMessage>>) -> bool {
        matches!(
            responses.as_deref(),
            Some([ServerMessage::AuthError { .. }])
        )
    }

    #[test]
    fn handshake_accepts_matching_protocol_version() {
        let mut state = ConnectionState::new(Ipv4Addr::LOCALHOST.into());
        let responses = handle_handshake(
            &client_info(onlinerpg_shared::PROTOCOL_VERSION, "cli"),
            &mut state,
        );

        assert!(responses.is_some_and(|r| r.is_empty()));
        assert_eq!(state.client_kind, Some(ClientKind::Cli));
        assert!(!state.must_close);
        // Later messages pass through once the handshake is done.
        assert!(handle_handshake(&ClientMessage::Heartbeat, &mut state).is_none());
    }

    #[test]
    fn handshake_refuses_other_protocol_versions() {
        for version in [
            onlinerpg_shared::PROTOCOL_VERSION - 1,
            onlinerpg_shared::PROTOCOL_VERSION + 1,
        ] {
            let mut state = ConnectionState::new(Ipv4Addr::LOCALHOST.into());
            let responses = handle_handshake(&client_info(version, "cli"), &mut state);

            assert!(is_auth_error(&responses), "v{version} should be refused");
            assert!(state.must_close);
            assert!(state.client_kind.is_none());
        }
    }

    #[test]
    fn handshake_refuses_messages_sent_before_client_info() {
        let mut state = ConnectionState::new(Ipv4Addr::LOCALHOST.into());
        let responses = handle_handshake(
            &ClientMessage::AuthenticateNpc {
                account_name: "npc_x".into(),
                npc_token: "t".into(),
            },
            &mut state,
        );

        assert!(is_auth_error(&responses));
        assert!(state.must_close);
    }

    #[test]
    fn party_positions_poll_clamped_inside_window() {
        let mut state = ConnectionState::new(Ipv4Addr::LOCALHOST.into());
        assert!(state.party_positions_poll_due());
        // Inside the window: dropped, and the drop must not refresh it.
        assert!(!state.party_positions_poll_due());
        state.last_party_positions_poll = Some(Instant::now() - PARTY_POSITIONS_MIN_INTERVAL);
        assert!(state.party_positions_poll_due());
    }

    #[test]
    fn handshake_buckets_unknown_client_kinds() {
        let mut state = ConnectionState::new(Ipv4Addr::LOCALHOST.into());
        handle_handshake(
            &client_info(onlinerpg_shared::PROTOCOL_VERSION, "totally-made-up"),
            &mut state,
        );

        assert_eq!(state.client_kind, Some(ClientKind::Other));
    }

    #[test]
    fn operator_only_classes_are_refused_for_players() {
        let mut state = ConnectionState::new(Ipv4Addr::LOCALHOST.into());
        assert!(state
            .require_selectable_class(&CharacterClass::Merchant)
            .is_err());
        assert!(state
            .require_selectable_class(&CharacterClass::Guard)
            .is_err());
        assert!(state
            .require_selectable_class(&CharacterClass::Ranger)
            .is_ok());

        // Operator NPCs are exactly who those classes exist for.
        state.is_official_npc = true;
        assert!(state
            .require_selectable_class(&CharacterClass::Merchant)
            .is_ok());
    }

    #[test]
    fn requires_admin_classifies_cheat_messages() {
        assert!(requires_admin(&ClientMessage::DebugSetTime {
            hour: 0,
            minute: 0
        }));
        assert!(requires_admin(&ClientMessage::DebugDropItem {
            item_def_id: "x".into()
        }));
        assert!(requires_admin(&ClientMessage::ChatMessage {
            message: "/give sword".into()
        }));
        assert!(requires_admin(&ClientMessage::ChatMessage {
            message: "/notice Server maintenance".into()
        }));
        assert!(requires_admin(&ClientMessage::ChatMessage {
            message: "/notice".into()
        }));
        for admin_command in [
            "/kick Abuser",
            "/mute Abuser 5",
            "/unmute Abuser",
            "/summon Abuser",
            "/goto Abuser",
        ] {
            assert!(
                requires_admin(&ClientMessage::ChatMessage {
                    message: admin_command.into()
                }),
                "{admin_command} must be admin-gated"
            );
        }
        assert!(!requires_admin(&ClientMessage::ChatMessage {
            message: "hello".into()
        }));
        assert!(!requires_admin(&ClientMessage::ChatMessage {
            message: "/who".into()
        }));
        assert!(!requires_admin(&ClientMessage::Heartbeat));
    }
}
