use super::KickNotice;
use crate::auth::{AuthError, AuthService, CharacterSaveData, ItemRow};
use crate::types::{CharacterAttributes, Player, PlayerId, Position, ServerMessage};
use crate::world_config::world_config;
use bytes::Bytes;
use onlinerpg_shared::housing::MAX_FLOOR_LEVEL;
use onlinerpg_shared::inventory::{EquipSlot, PlayerInventory};
use onlinerpg_shared::{
    shortest_world_delta_x, wrap_world_x, MAX_MOVE_TARGET_DISTANCE, PLAYER_MOVE_SPEED,
};
use std::collections::{HashMap, HashSet, VecDeque};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

/// Most queued waypoints per player; legit smoothed paths stay well under.
const MAX_QUEUED_WAYPOINTS: usize = 32;

/// Arrival ring radius (m) around a teleport's center (`arrival_beside`);
/// golden-angle spacing keeps simultaneous arrivals apart.
const ARRIVAL_RING_RADIUS: f32 = 1.6;

/// Golden angle in radians, spreading arrival spots around the ring.
const GOLDEN_ANGLE_RAD: f32 = 2.399_963;

/// Keep the shared housing limit representable on the signed wire.
const _: () = assert!(MAX_FLOOR_LEVEL <= i8::MAX as u8);

fn exceeds_positive_floor_limit(floor_level: i8) -> bool {
    floor_level > MAX_FLOOR_LEVEL as i8
}

/// Keep legacy out-of-range rows from re-entering live state.
pub(crate) fn restored_floor_level(saved: i8) -> i8 {
    if exceeds_positive_floor_limit(saved) {
        0
    } else {
        saved
    }
}

/// Least time between `PositionCorrected` snaps for one player. Long enough
/// that a client which cannot act on them is not yanked repeatedly, short
/// enough that a real desync is pulled back before it skews AOI or attack range.
/// Also paces the refused-move warn, which rides the same gate.
const POSITION_CORRECTION_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(2);

/// Whether `player_id` may be snapped now, stamping it when so. Deliberately
/// not re-stamped when it suppresses a snap: that would hold a player who trips
/// every tick past the cooldown forever, and they are exactly who needs the
/// next one.
fn correction_due(
    last: &mut HashMap<PlayerId, std::time::Instant>,
    player_id: &PlayerId,
    now: std::time::Instant,
) -> bool {
    match last.get(player_id) {
        Some(at) if now.duration_since(*at) < POSITION_CORRECTION_COOLDOWN => false,
        _ => {
            last.insert(*player_id, now);
            true
        }
    }
}

/// Where a refusal landed, for streak comparison.
#[derive(Clone, Copy)]
struct GrindSite<'a> {
    key: &'a str,
    cell: (i32, i32),
    intent_floor: i8,
}

/// The streak site a refusal belongs to, or `None` when the detector ignores
/// it: surface geometry ships with the terrain and cannot drift this way, and
/// a sealed player is being rescued rather than refused.
fn grind_site(r: &RefusedMove) -> Option<GrindSite<'_>> {
    (onlinerpg_shared::dungeon::is_dungeon_cache_key(&r.block_key) && !r.sealed).then(|| {
        GrindSite {
            key: &r.block_key,
            cell: (r.step_x.round() as i32, r.step_z.round() as i32),
            intent_floor: r.intent_floor,
        }
    })
}

/// How far from where a streak started still counts as the same wall. One
/// cell: a client pushing a wall jitters across the cell boundary between
/// corrections (real refusals alternate (-1456,4702) and (-1457,4702)), and an
/// exact match restarts the count on every flip. The anchor never moves, so
/// this stays a 3x3 patch rather than following someone along a corridor.
const LAYOUT_GRIND_RADIUS: i32 = 1;

/// Advance a player's grind streak, returning its length on the one correction
/// that reaches `LAYOUT_GRIND_LIMIT`. `count` only climbs, so that is once per
/// streak: a player who cannot be kicked does not re-fire every cooldown.
///
/// `site` is `None` for a refusal this detector ignores, which ends any streak:
/// a player who moved on is not stuck.
fn record_grind(
    grinds: &mut HashMap<PlayerId, LayoutGrind>,
    player_id: &PlayerId,
    site: Option<GrindSite<'_>>,
    now: std::time::Instant,
) -> Option<u32> {
    let Some(site) = site else {
        grinds.remove(player_id);
        return None;
    };
    let grind = grinds
        .entry(*player_id)
        .or_insert_with(|| LayoutGrind::new(site, now));
    let near_anchor = (grind.cell.0 - site.cell.0).abs() <= LAYOUT_GRIND_RADIUS
        && (grind.cell.1 - site.cell.1).abs() <= LAYOUT_GRIND_RADIUS;
    if grind.key != site.key
        || !near_anchor
        || grind.intent_floor != site.intent_floor
        || now.duration_since(grind.at) >= LAYOUT_GRIND_TTL
    {
        *grind = LayoutGrind::new(site, now);
    }
    // The anchor stays where the streak began; only the count moves.
    grind.count += 1;
    grind.at = now;
    (grind.count == LAYOUT_GRIND_LIMIT).then_some(grind.count)
}

/// How many corrections at one dungeon wall mean the client is drawing a maze
/// this server does not have. Each one costs a full
/// `POSITION_CORRECTION_COOLDOWN`, so this is ~20 seconds of a player walking
/// into the same cell — deliberately far past a stuck path or a held key.
const LAYOUT_GRIND_LIMIT: u32 = 10;

/// Streaks stop counting once the player stops feeding them; anything older is
/// a finished episode and gets pruned.
const LAYOUT_GRIND_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// One player's run of refusals at a single dungeon cell.
pub(super) struct LayoutGrind {
    /// What blocked them, e.g. `dungeon:old_crypt`.
    key: String,
    /// Where the streak started, rounded to a cell.
    cell: (i32, i32),
    intent_floor: i8,
    count: u32,
    at: std::time::Instant,
}

impl LayoutGrind {
    fn new(site: GrindSite<'_>, now: std::time::Instant) -> Self {
        Self {
            key: site.key.to_string(),
            cell: site.cell,
            intent_floor: site.intent_floor,
            count: 0,
            at: now,
        }
    }
}

/// Client-requested destination the server walks the player toward at capped
/// speed.
#[derive(Clone)]
pub(super) struct MoveIntent {
    pub(super) target: Position,
    rotation: f32,
    pub(super) floor_level: i8,
    /// NPC connections are exempt: schedule force-moves may legitimately
    /// cross closed doors.
    check_collision: bool,
    sprinting: bool,
}

/// FIFO of client-validated legs. `append: false` PlayerMoves replace the
/// whole queue; `append: true` extends it so the server walks the client's
/// polyline instead of beelining (and corner-cutting) to the newest target.
pub(super) type MoveQueue = VecDeque<MoveIntent>;

/// A refused step: where the server has the player (for the correction) plus the
/// diagnostics for the warn, both emitted together in `correct_refused_positions`.
struct RefusedMove {
    player_id: PlayerId,
    position: Position,
    rotation: f32,
    floor_level: i8,
    step_x: f32,
    step_z: f32,
    step_y: f32,
    step_floor: u8,
    intent_floor: i8,
    /// No legal step in any direction, so this one is a rescue rather than a
    /// refusal — see `free_sealed_players`.
    sealed: bool,
    /// Owned: `BlockInfo::key` borrows the passability cache, which is unlocked
    /// before the warn is emitted.
    block_key: String,
    stairwell: bool,
    consulted: usize,
}

/// Wire fields of a client PlayerMove (see shared messages).
pub(crate) struct MoveCommand {
    pub(crate) position: Position,
    pub(crate) rotation: f32,
    pub(crate) floor_level: i8,
    pub(crate) append: bool,
    pub(crate) sprinting: bool,
}

/// Run a blocking DB save on the blocking pool and report whether it committed.
async fn flush_save<F>(op: F, what: &str) -> bool
where
    F: FnOnce() -> Result<(), AuthError> + Send + 'static,
{
    match tokio::task::spawn_blocking(op).await {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            error!("Failed to save {}: {}", what, e);
            false
        }
        Err(e) => {
            error!("spawn_blocking panicked while saving {}: {}", what, e);
            false
        }
    }
}

pub(super) fn build_save_data(
    player: &Player,
    character_id: i64,
    xp: u64,
    gold: i64,
    satiation: u32,
) -> CharacterSaveData {
    CharacterSaveData {
        character_id,
        x: player.position.x,
        y: player.position.y,
        z: player.position.z,
        rotation: player.rotation,
        xp,
        level: player.level,
        max_hp: player.max_health,
        health: player.health,
        floor_level: player.floor_level,
        gold,
        satiation,
    }
}

/// One pass over `items`: which left the mover's AOI and which entered it,
/// judged against `EVENT_DELIVERY_RADIUS` on the item's own floor.
fn aoi_diff<T>(
    items: impl Iterator<Item = T>,
    place: impl Fn(&T) -> (Position, i8),
    old: (&Position, i8),
    new: (&Position, i8),
) -> (Vec<T>, Vec<T>) {
    let radius_sq = super::EVENT_DELIVERY_RADIUS * super::EVENT_DELIVERY_RADIUS;
    let mut left = Vec::new();
    let mut entered = Vec::new();
    for item in items {
        let (position, floor) = place(&item);
        let was = floor == old.1 && old.0.dist_xz_sq(&position) <= radius_sq;
        let now = floor == new.1 && new.0.dist_xz_sq(&position) <= radius_sq;
        match (was, now) {
            (true, false) => left.push(item),
            (false, true) => entered.push(item),
            _ => {}
        }
    }
    (left, entered)
}

impl super::GameState {
    pub async fn get_or_assign_player_number(&self, player_id: &PlayerId) -> u32 {
        let mut id_state = self.id_state.write().await;
        if let Some(player_number) = id_state.player_numbers.get(player_id).copied() {
            player_number
        } else {
            id_state.next_player_number = id_state.next_player_number.saturating_add(1);
            let player_number = id_state.next_player_number;
            id_state.player_numbers.insert(*player_id, player_number);
            player_number
        }
    }

    pub async fn register_connection_channel(
        &self,
        player_id: &PlayerId,
    ) -> mpsc::UnboundedReceiver<Bytes> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut channels = self.direct_channels.write().await;
        channels.insert(*player_id, tx);
        rx
    }

    pub async fn unregister_connection_channel(&self, player_id: &PlayerId) {
        let mut channels = self.direct_channels.write().await;
        channels.remove(player_id);
    }

    pub async fn send_direct_message(&self, player_id: &PlayerId, msg: ServerMessage) {
        let Some(bytes) = super::encode_server_msg(&msg) else {
            return;
        };
        let channels = self.direct_channels.read().await;
        if let Some(tx) = channels.get(player_id) {
            let _ = tx.send(bytes);
        }
    }

    /// Private system chat line for one player (command replies, action
    /// feedback).
    pub async fn send_system_message(&self, player_id: &PlayerId, message: impl Into<String>) {
        self.send_direct_message(
            player_id,
            ServerMessage::SystemMessage {
                message: message.into(),
            },
        )
        .await;
    }

    pub async fn send_direct_message_to_players(
        &self,
        player_ids: &[PlayerId],
        msg: ServerMessage,
    ) {
        self.send_direct_message_to_players_except(player_ids, msg, None)
            .await;
    }

    /// Serializes once and shares the bytes: every connection would otherwise
    /// re-encode the same message, which adds up on move fanout at scale.
    pub async fn send_direct_message_to_players_except(
        &self,
        player_ids: &[PlayerId],
        msg: ServerMessage,
        skip_player_id: Option<&PlayerId>,
    ) {
        let is_skipped = |id: &PlayerId| skip_player_id.is_some_and(|skip_id| skip_id == id);
        if !player_ids.iter().any(|id| !is_skipped(id)) {
            return;
        }
        let Some(bytes) = super::encode_server_msg(&msg) else {
            return;
        };
        let channels = self.direct_channels.read().await;
        for player_id in player_ids {
            if is_skipped(player_id) {
                continue;
            }
            if let Some(tx) = channels.get(player_id) {
                let _ = tx.send(bytes.clone());
            }
        }
    }

    /// Deliver `msg` to every player within `radius` (XZ) of `position` that
    /// is also on `floor_level`. The floor gate keeps events from leaking
    /// between stacked floors that share the same XZ footprint (a dungeon
    /// depth sits directly under the overworld, house upper floors over the
    /// ground floor), so e.g. a surface guard never perceives — and never
    /// reacts to — monsters fighting on the dungeon floor beneath it.
    pub async fn send_direct_message_to_players_within_position(
        &self,
        position: &Position,
        floor_level: i8,
        radius: f32,
        msg: ServerMessage,
        skip_player_id: Option<&PlayerId>,
    ) {
        let player_ids = self
            .player_ids_within_position(position, floor_level, radius)
            .await;
        self.send_direct_message_to_players_except(&player_ids, msg, skip_player_id)
            .await;
    }

    pub async fn register_player_character(
        &self,
        player_id: &PlayerId,
        character_id: i64,
        xp: u64,
        attributes: CharacterAttributes,
        gold: i64,
        satiation: Option<u32>,
    ) {
        {
            let mut map = self.player_characters.write().await;
            map.insert(*player_id, (character_id, xp, attributes));
        }
        {
            let mut gold_map = self.player_gold.write().await;
            gold_map.insert(*player_id, gold);
        }
        // None = official NPC (exempt).
        if let Some(satiation) = satiation {
            self.register_hunger(player_id, satiation).await;
        }
    }

    pub async fn unregister_player_character(&self, player_id: &PlayerId) {
        {
            let mut map = self.player_characters.write().await;
            map.remove(player_id);
        }
        {
            let mut gold_map = self.player_gold.write().await;
            gold_map.remove(player_id);
        }
        self.remove_player_blocks(player_id).await;
        self.remove_player_friends(player_id).await;
        self.remove_player_titles(player_id).await;
        self.forget_whisper_partner(player_id).await;
        self.forget_player_skills(player_id).await;
        self.remove_dungeon_discoveries(player_id).await;
        self.forget_hunger(player_id).await;
    }

    /// Serializes account replacement and character deletion with game entry.
    pub(crate) async fn lock_character_sessions(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.character_session_lock.lock().await
    }

    #[cfg(test)]
    pub(crate) async fn register_account_session(
        &self,
        account_name: &str,
        kick_tx: mpsc::UnboundedSender<KickNotice>,
        auth: &AuthService,
    ) -> u64 {
        let _sessions = self.character_session_lock.lock().await;
        self.register_account_session_locked(account_name, kick_tx, auth)
            .await
    }

    /// Body of `register_account_session` for callers already holding
    /// `lock_character_sessions` — the mutex is not reentrant, so a login that
    /// checks a ban under the lock has to register through this.
    pub(crate) async fn register_account_session_locked(
        &self,
        account_name: &str,
        kick_tx: mpsc::UnboundedSender<KickNotice>,
        auth: &AuthService,
    ) -> u64 {
        use std::sync::atomic::Ordering;

        let session_id = self.next_account_session.fetch_add(1, Ordering::Relaxed);
        let key = account_name.to_ascii_lowercase();
        let replaced = self.account_sessions.write().await.insert(
            key,
            super::AccountSession {
                id: session_id,
                player_id: None,
                kick_tx,
            },
        );

        if let Some(replaced) = replaced {
            info!("Replacing active session for account '{}'", account_name);
            let _ = replaced.kick_tx.send(KickNotice {
                message: ServerMessage::Kicked {
                    player_id: replaced.player_id.unwrap_or(PlayerId::from(0)),
                    reason: "Another session logged in with the same account".to_string(),
                },
                close_code: None,
            });
            if let Some(player_id) = replaced.player_id {
                self.cleanup_player_session(&player_id, auth).await;
            }
        }

        session_id
    }

    pub(crate) async fn is_current_account_session(
        &self,
        account_name: &str,
        session_id: u64,
    ) -> bool {
        self.account_sessions
            .read()
            .await
            .get(&account_name.to_ascii_lowercase())
            .is_some_and(|session| session.id == session_id)
    }

    pub(crate) async fn attach_player_to_account_session(
        &self,
        account_name: &str,
        session_id: u64,
        player_id: PlayerId,
    ) -> bool {
        let mut sessions = self.account_sessions.write().await;
        let Some(session) = sessions.get_mut(&account_name.to_ascii_lowercase()) else {
            return false;
        };
        if session.id != session_id {
            return false;
        }
        session.player_id = Some(player_id);
        true
    }

    pub(crate) async fn end_account_session(
        &self,
        account_name: &str,
        session_id: u64,
        auth: &AuthService,
    ) {
        let _sessions = self.character_session_lock.lock().await;
        let key = account_name.to_ascii_lowercase();
        let ended = {
            let mut sessions = self.account_sessions.write().await;
            if sessions
                .get(&key)
                .is_some_and(|session| session.id == session_id)
            {
                sessions.remove(&key)
            } else {
                None
            }
        };
        if let Some(player_id) = ended.and_then(|session| session.player_id) {
            self.cleanup_player_session(&player_id, auth).await;
        }
    }

    /// Deletes an inactive character, or returns `false` if it is registered.
    pub(crate) async fn delete_character_if_inactive(
        &self,
        auth: &AuthService,
        account_name: &str,
        character_id: i64,
    ) -> Result<bool, AuthError> {
        let _sessions = self.character_session_lock.lock().await;
        if self
            .player_characters
            .read()
            .await
            .values()
            .any(|(active_id, _, _)| *active_id == character_id)
        {
            return Ok(false);
        }
        auth.delete_character(account_name, character_id)?;
        Ok(true)
    }

    pub async fn get_player_gold(&self, player_id: &PlayerId) -> i64 {
        let gold_map = self.player_gold.read().await;
        gold_map.get(player_id).copied().unwrap_or(0)
    }

    /// Online player id for a typed name, ignoring ASCII case — names are
    /// unique ignoring case, so at most one player matches. The caller still
    /// validates the id against `players` under its own lock.
    pub(crate) async fn player_id_by_name(&self, name: &str) -> Option<PlayerId> {
        let names = self.player_ids_by_name.read().await;
        names.get(&name.to_ascii_lowercase()).copied()
    }

    /// Character name for logs and player-facing text. Falls back to the raw
    /// id when the player is gone, so log sites stay useful on the paths that
    /// fire precisely because the lookup missed.
    pub(crate) async fn player_name_of(&self, player_id: &PlayerId) -> String {
        let players = self.players.read().await;
        players
            .get(player_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| player_id.to_string())
    }

    /// Pose snapshot (position, rotation, floor, name) in one lock read.
    pub(crate) async fn player_pose(
        &self,
        player_id: &PlayerId,
    ) -> Option<(Position, f32, i8, String)> {
        let players = self.players.read().await;
        players
            .get(player_id)
            .map(|p| (p.position, p.rotation, p.floor_level, p.name.clone()))
    }

    async fn cleanup_player_session(&self, player_id: &PlayerId, auth: &AuthService) {
        self.cancel_concentration_if_active(player_id).await;
        self.persist_and_detach_player(player_id, auth).await;
        self.unregister_connection_channel(player_id).await;
        self.unregister_player_character(player_id).await;
        self.remove_player(player_id).await;
    }

    /// Force-disconnect an online player (admin `/kick`). Ends the account
    /// session the way a replacement login would: the `kick_tx` message makes
    /// the connection loop close the socket, and removing the session first
    /// keeps the disconnect path from cleaning up a second time.
    ///
    /// `close_code` turns the kick into an instruction the client acts on
    /// (reload rather than reconnect); `None` just leaves the reason on screen.
    pub(crate) async fn kick_player(
        &self,
        player_id: &PlayerId,
        reason: &str,
        close_code: Option<u16>,
        auth: &AuthService,
    ) {
        let _sessions = self.character_session_lock.lock().await;
        match self.account_of_player(player_id).await {
            Some(account) => {
                self.evict_account_session_locked(&account, reason, close_code, auth)
                    .await
            }
            // No session row to close, but per-player state still has to go.
            None => self.cleanup_player_session(player_id, auth).await,
        }
    }

    /// Account behind an online player id, read from the session map. `None`
    /// once the session is gone.
    pub(crate) async fn account_of_player(&self, player_id: &PlayerId) -> Option<String> {
        self.account_sessions
            .read()
            .await
            .iter()
            .find_map(|(key, session)| (session.player_id == Some(*player_id)).then(|| key.clone()))
    }

    /// Force-disconnect a whole account, whichever character it is playing — or
    /// none at all, as at character select. The `kick_tx` message makes the
    /// connection loop close the socket, and removing the session first keeps
    /// the disconnect path from cleaning up a second time.
    ///
    /// Assumes `lock_character_sessions` is held, so `/ban` can serialize its
    /// write with the login path.
    pub(crate) async fn evict_account_session_locked(
        &self,
        account_name: &str,
        reason: &str,
        close_code: Option<u16>,
        auth: &AuthService,
    ) {
        let session = self
            .account_sessions
            .write()
            .await
            .remove(&account_name.to_ascii_lowercase());
        let Some(session) = session else {
            return;
        };
        let _ = session.kick_tx.send(KickNotice {
            message: ServerMessage::Kicked {
                player_id: session.player_id.unwrap_or(PlayerId::from(0)),
                reason: reason.to_string(),
            },
            close_code,
        });
        // Only a session that reached the game has per-player state to clear.
        if let Some(player_id) = session.player_id {
            self.cleanup_player_session(&player_id, auth).await;
        }
    }

    /// Synchronously write a player's character row and inventory to the DB,
    /// detaching the in-memory inventory. Shared by the disconnect path and by
    /// session replacement (kick), which relies on the inventory being flushed
    /// before the replacement login loads from the DB (F-015).
    ///
    /// Must run *before* `unregister_player_character`: both the character-state
    /// and inventory snapshots resolve the character id through
    /// `player_characters`, so unregistering first would silently skip both
    /// saves while still detaching the inventory.
    pub async fn persist_and_detach_player(&self, player_id: &PlayerId, auth: &AuthService) {
        let _persistence = self.persistence_lock.lock().await;

        let mut characters = Vec::new();
        if let Some(save_data) = self.get_player_save_data(player_id).await {
            self.remove_dirty(player_id).await;
            characters.push(save_data);
        }
        let inventories = Vec::from_iter(self.take_player_inventory(player_id).await);
        let skills = Vec::from_iter(self.take_player_skills(player_id).await);

        let auth = auth.clone();
        flush_save(
            move || auth.save_batch(&characters, &inventories, &skills, &[], None),
            "player state",
        )
        .await;
    }

    /// Persist every connected player plus the world clock in one transaction.
    /// Used by the shutdown drain, where connections skip their own teardown so
    /// 5,000 logouts don't become 5,000 commits.
    pub async fn persist_shutdown_snapshot(&self, auth: &AuthService) {
        let (characters, inventories) = self.collect_shutdown_snapshot().await;
        let skills = self.collect_all_skill_states().await;
        let discoveries = self.take_pending_discovery_saves().await;
        let character_count = characters.len();
        let inventory_count = inventories.len();
        let datetime = self.current_game_datetime();
        let auth = auth.clone();
        flush_save(
            move || {
                auth.save_batch(
                    &characters,
                    &inventories,
                    &skills,
                    &discoveries,
                    Some(&datetime),
                )?;
                info!(
                    "Saved shutdown snapshot: {} character(s), {} inventory/inventories",
                    character_count, inventory_count
                );
                Ok(())
            },
            "shutdown snapshot",
        )
        .await;
    }

    async fn collect_shutdown_snapshot(
        &self,
    ) -> (Vec<CharacterSaveData>, Vec<(i64, Vec<ItemRow>)>) {
        let _persistence = self.persistence_lock.lock().await;
        let players = self.players.read().await;
        let player_characters = self.player_characters.read().await;
        let player_gold = self.player_gold.read().await;
        let hunger = self.hunger.read().await;
        let inventories = self.inventories.read().await;

        let mut characters = Vec::with_capacity(player_characters.len());
        let mut inventory_rows = Vec::with_capacity(player_characters.len());

        for (player_id, (character_id, xp, _)) in player_characters.iter() {
            if let Some(player) = players.get(player_id) {
                characters.push(build_save_data(
                    player,
                    *character_id,
                    *xp,
                    player_gold.get(player_id).copied().unwrap_or(0),
                    super::hunger::satiation_for_save(&hunger, player_id),
                ));
            }
            if let Some(inventory) = inventories.get(player_id) {
                inventory_rows.push((
                    *character_id,
                    super::inventory::serialize_inventory(inventory),
                ));
            }
        }

        characters.sort_by_key(|state| state.character_id);
        inventory_rows.sort_by_key(|(character_id, _)| *character_id);
        (characters, inventory_rows)
    }

    /// Write every dirty character state and inventory. Takes the same lock as
    /// `persist_and_detach_player` so a periodic flush cannot interleave with a
    /// logout's save.
    pub async fn flush_dirty_saves(&self, auth: &AuthService) {
        let _persistence = self.persistence_lock.lock().await;

        let (dirty_player_ids, dirty_states) = self.collect_dirty_character_states().await;
        let (dirty_inventory_ids, dirty_inventories) = self.collect_dirty_inventory_states().await;
        let (dirty_skill_ids, dirty_skills) = self.collect_dirty_skill_states().await;
        let dirty_discoveries = self.take_pending_discovery_saves().await;
        if dirty_states.is_empty()
            && dirty_inventories.is_empty()
            && dirty_skills.is_empty()
            && dirty_discoveries.is_empty()
        {
            return;
        }

        let character_count = dirty_states.len();
        let inventory_count = dirty_inventories.len();
        let auth = auth.clone();
        let discoveries = dirty_discoveries.clone();
        let saved = flush_save(
            move || {
                auth.save_batch(
                    &dirty_states,
                    &dirty_inventories,
                    &dirty_skills,
                    &discoveries,
                    None,
                )?;
                info!(
                    "Batch-saved {} character state(s), {} inventory/inventories",
                    character_count, inventory_count
                );
                Ok(())
            },
            "dirty state",
        )
        .await;
        if !saved {
            self.restore_dirty_players(dirty_player_ids).await;
            self.restore_dirty_inventories(dirty_inventory_ids).await;
            self.restore_dirty_skills(dirty_skill_ids).await;
            self.restore_pending_discovery_saves(dirty_discoveries)
                .await;
        }
    }

    /// Registers the player and returns the messages that materialize the
    /// surroundings for them: the visible-state snapshot, plus any
    /// performances already underway in earshot (delivered mid-track).
    pub async fn add_player(&self, mut player: Player) -> Vec<ServerMessage> {
        // Normalize persisted legacy positions before they enter the spatial
        // index or are sent to clients.
        player.position.x = onlinerpg_shared::wrap_world_x(player.position.x);
        let player_id = player.id;
        let player_name = player.name.clone();
        let player_number = self.get_or_assign_player_number(&player_id).await;
        let player_position = player.position;
        let player_floor = player.floor_level;

        {
            let mut players = self.players.write().await;
            players.insert(player_id, player.clone());
        }
        {
            let mut names = self.player_ids_by_name.write().await;
            names.insert(player_name.to_ascii_lowercase(), player_id);
        }
        self.insert_player_spatial_cell(&player_id, &player_position)
            .await;

        info!(
            "Player {} ({}) joined the game [#{}]",
            player_name, player_id, player_number
        );

        let nearby_player_ids = self
            .player_ids_within(&player_id, super::EVENT_DELIVERY_RADIUS)
            .await;
        let nearby_player_set: HashSet<_> = nearby_player_ids.iter().cloned().collect();
        self.send_direct_message_to_players_except(
            &nearby_player_ids,
            ServerMessage::PlayerJoined {
                player: player.clone(),
            },
            Some(&player_id),
        )
        .await;

        // Return visible game_state to be sent directly to the new player only
        let current_players = self.players.read().await;
        let other_players: Vec<Player> = current_players
            .iter()
            .filter(|(id, _)| nearby_player_set.contains(*id) && *id != &player_id)
            .map(|(_, player)| player.clone())
            .collect();

        let monsters: HashMap<String, crate::types::Monster> = self
            .monsters
            .read()
            .await
            .values()
            .filter(|monster| {
                monster.floor_level == player_floor
                    && monster.position.dist_xz_sq(&player_position)
                        <= super::EVENT_DELIVERY_RADIUS * super::EVENT_DELIVERY_RADIUS
            })
            .map(|monster| (monster.id.clone(), monster.clone()))
            .collect();
        let ground_items: Vec<_> = self
            .ground_items
            .read()
            .await
            .values()
            .filter(|sgi| {
                sgi.item.floor_level == player_floor
                    && sgi.item.position.dist_xz_sq(&player_position)
                        <= super::EVENT_DELIVERY_RADIUS * super::EVENT_DELIVERY_RADIUS
            })
            .map(|sgi| sgi.item.clone())
            .collect();
        let campfires: Vec<_> = self
            .campfires
            .read()
            .await
            .values()
            .filter(|e| {
                e.campfire.floor_level == player_floor
                    && e.campfire.position.dist_xz_sq(&player_position)
                        <= super::EVENT_DELIVERY_RADIUS * super::EVENT_DELIVERY_RADIUS
            })
            .map(|e| e.campfire.clone())
            .collect();
        let stalls: Vec<_> = self
            .stalls
            .read()
            .await
            .values()
            .filter(|s| {
                s.floor_level == player_floor
                    && s.position.dist_xz_sq(&player_position)
                        <= super::EVENT_DELIVERY_RADIUS * super::EVENT_DELIVERY_RADIUS
            })
            .cloned()
            .collect();
        let tip_hats: Vec<_> = self
            .tip_hats
            .read()
            .await
            .values()
            .filter(|h| {
                h.floor_level == player_floor
                    && h.position.dist_xz_sq(&player_position)
                        <= super::EVENT_DELIVERY_RADIUS * super::EVENT_DELIVERY_RADIUS
            })
            .cloned()
            .collect();

        let mut msgs = Vec::new();
        if !other_players.is_empty()
            || !monsters.is_empty()
            || !ground_items.is_empty()
            || !campfires.is_empty()
            || !stalls.is_empty()
            || !tip_hats.is_empty()
        {
            msgs.push(ServerMessage::GameState {
                players: other_players,
                monsters,
                ground_items,
                campfires,
                stalls,
                tip_hats,
            });
        }

        let performances = self.music_performances.read().await;
        if !performances.is_empty() {
            for id in &nearby_player_ids {
                if *id != player_id {
                    if let Some(entry) = performances.get(id) {
                        msgs.push(super::chat::music_started_msg(*id, entry));
                    }
                }
            }
        }

        msgs
    }

    pub async fn remove_player(&self, player_id: &PlayerId) {
        self.movement_intents.write().await.remove(player_id);
        self.music_performances.write().await.remove(player_id);
        self.remove_player_stall(player_id).await;
        self.remove_player_tip_hat(player_id).await;
        self.drop_player_trade(player_id, "They left.").await;
        self.last_player_attacks.write().await.remove(player_id);
        // A player disconnecting inside a dungeon leaves its floor first,
        // so its monsters get reassigned (or despawned) instead of being
        // dropped by remove_monsters_by_owner below.
        let dungeon_exit = {
            let players = self.players.read().await;
            players
                .get(player_id)
                .filter(|p| p.floor_level < 0)
                .map(|p| (p.floor_level, p.position))
        };
        if let Some((floor, position)) = dungeon_exit {
            self.handle_player_floor_change(player_id, floor, 0, &position, &position)
                .await;
        }

        self.remove_monsters_by_owner(player_id).await;

        // Release any trade-window holds: this player may have been shopping
        // with NPCs (free them if it was their last customer) or be a trading
        // NPC itself (forget its entry).
        self.clear_shops_for_player(player_id).await;

        let removed_player_number = {
            let mut id_state = self.id_state.write().await;
            let removed = id_state.player_numbers.remove(player_id);
            if let Some(player_number) = removed {
                id_state.owner_spawn_counts.remove(&player_number);
            }
            removed
        };

        let nearby_player_ids = self
            .player_ids_within(player_id, super::EVENT_DELIVERY_RADIUS)
            .await;
        let removed_player = {
            let mut players = self.players.write().await;
            players.remove(player_id)
        };
        if let Some(player) = &removed_player {
            let key = player.name.to_ascii_lowercase();
            let mut names = self.player_ids_by_name.write().await;
            // Guarded so a same-name replacement session keeps its entry.
            if names.get(&key) == Some(player_id) {
                names.remove(&key);
            }
        }

        // After the roster removal on purpose: party mutations hold the
        // players lock, so an in-flight accept lands before this sweep and
        // gets cleaned up instead of leaving a ghost member.
        self.clear_party_for_player(player_id).await;

        if let Some(player) = removed_player {
            self.remove_player_spatial_cell(player_id, &player.position)
                .await;
            info!(
                "Player {} ({}) left the game{}",
                player.name,
                player_id,
                removed_player_number
                    .map(|n| format!(" [#{}]", n))
                    .unwrap_or_default()
            );
            self.send_direct_message_to_players_except(
                &nearby_player_ids,
                ServerMessage::PlayerLeft {
                    player_id: *player_id,
                },
                Some(player_id),
            )
            .await;
        } else {
            warn!("Attempted to remove non-existent player: {}", player_id);
        }
    }

    pub async fn mark_world_ready(&self, player_id: &PlayerId) {
        let mut players = self.players.write().await;
        if let Some(player) = players.get_mut(player_id) {
            player.ready_at = 0;
        }
    }

    /// Handle a client PlayerMove. The client sends destinations (waypoints),
    /// so the position becomes a MoveIntent that `tick_player_movement` walks
    /// toward.
    pub async fn update_player_position(
        &self,
        player_id: &PlayerId,
        cmd: MoveCommand,
        is_official_npc: bool,
    ) {
        let MoveCommand {
            position: mut new_position,
            rotation: new_rotation,
            floor_level,
            append,
            sprinting,
        } = cmd;
        if exceeds_positive_floor_limit(floor_level) {
            self.reject_out_of_range_floor(player_id, floor_level, "move")
                .await;
            return;
        }
        if !(new_position.is_finite() && new_rotation.is_finite()) {
            if self.snap_refused_move_back(player_id).await {
                warn!(
                    "Rejected non-finite move from player {}",
                    self.player_name_of(player_id).await
                );
            }
            return;
        }
        new_position.x = wrap_world_x(new_position.x);
        let (current_floor, current_position, health) = {
            let players = self.players.read().await;
            match players.get(player_id) {
                Some(p) => (p.floor_level, p.position, p.health),
                None => {
                    warn!("Attempted to move non-existent player: {}", player_id);
                    return;
                }
            }
        };

        // Appended legs chain off the queue tail, so both the floor change and
        // the distance guard judge the new leg, not the path from the player.
        let (leg_start, leg_floor) = if append {
            self.movement_intents
                .read()
                .await
                .get(player_id)
                .and_then(|q| q.back())
                .map(|w| (w.target, w.floor_level))
                .unwrap_or((current_position, current_floor))
        } else {
            (current_position, current_floor)
        };

        // Dungeon floors (negative) are held to the entrance registry, the
        // stairs and the floor's ground height, which replaces the reported Y;
        // house storeys to their stairwells, and the Y to the ground the
        // server models (`surface_ground_y`, below the cheap refusals).
        let declared_floor = floor_level;
        let floor_level = if floor_level < 0 || leg_floor < 0 {
            let verdict = self
                .validated_dungeon_floor(
                    player_id,
                    leg_floor,
                    floor_level,
                    &leg_start,
                    &new_position,
                )
                .await;
            new_position.y = verdict.y;
            verdict.floor
        } else {
            self.validated_house_floor(
                player_id,
                leg_floor,
                floor_level,
                &leg_start,
                &new_position,
                is_official_npc,
            )
            .await
        };

        // A coerced floor means the declared (position, floor) pair failed
        // floor validation (which already warned why), and the mover never
        // sees its own PlayerMoved — snap so its floor resyncs instead of
        // diverging silently.
        if floor_level != declared_floor {
            self.snap_refused_move_back(player_id).await;
            return;
        }
        // The tick's health backstop would only drop the queue silently; the
        // client predicts every move it sends, so refuse here with a snap or
        // its phantom walks off without the body.
        if health == 0 {
            self.snap_refused_move_back(player_id).await;
            return;
        }

        let dist_sq = leg_start.dist_xz_sq(&new_position);
        if dist_sq > MAX_MOVE_TARGET_DISTANCE * MAX_MOVE_TARGET_DISTANCE {
            if self.snap_refused_move_back(player_id).await {
                warn!(
                    "Rejected move target {:.0}m away from player {}",
                    dist_sq.sqrt(),
                    self.player_name_of(player_id).await
                );
            }
            return;
        }
        if floor_level >= 0 {
            new_position.y = self
                .surface_ground_y(floor_level as u8, &new_position, leg_start.y)
                .await;
        }
        let mut queues = self.movement_intents.write().await;
        let queue = queues.entry(*player_id).or_default();
        if !append {
            queue.clear();
        } else if queue.len() >= MAX_QUEUED_WAYPOINTS {
            // Drop the oldest leg, not the newest: losing path fidelity in the
            // middle beats never reaching where the client actually is. The
            // resulting current-position→new-head beeline is the same risk as
            // a replace and is still collision-checked while walking.
            queue.pop_front();
            warn!(
                "Waypoint queue full for player {}, dropped oldest",
                self.player_name_of(player_id).await
            );
        }
        queue.push_back(MoveIntent {
            target: new_position,
            rotation: new_rotation,
            floor_level,
            check_collision: !is_official_npc,
            sprinting,
        });
    }

    /// Advance every pending move queue by `dt` seconds of walking and
    /// broadcast the results. A tick's budget can span several short legs;
    /// consumed waypoints are popped in place, finished queues dropped.
    pub async fn tick_player_movement(&self, dt: f32) {
        // Exactly the client's speed: headroom ran the sim to the leg end
        // ahead of the client, and monsters swung at that empty spot.
        let base_step = PLAYER_MOVE_SPEED * dt.max(0.0);
        let mut moved: Vec<(PlayerId, Position, i8, Player, bool)> = Vec::new();
        let mut steps: Vec<super::ambient_spawn::MoveStep> = Vec::new();

        let mut activities: Vec<(PlayerId, f32, bool)> = Vec::new();
        let mut refused: Vec<RefusedMove> = Vec::new();
        {
            let mut queues = self.movement_intents.write().await;
            if queues.is_empty() {
                return;
            }
            let mover_ids: Vec<PlayerId> = queues.keys().copied().collect();
            let hunger_profiles = self.hunger_movement_profiles_for(&mover_ids).await;
            let mut players = self.players.write().await;
            let cache = self.passability_read();
            queues.retain(|player_id, waypoints| {
                let Some(player) = players.get_mut(player_id) else {
                    return false;
                };
                // Backstop: combat clears the queue on death.
                if player.health == 0 {
                    return false;
                }

                let (hunger_mult, sprint_allowed) = hunger_profiles
                    .get(player_id)
                    .copied()
                    .unwrap_or((1.0, true));
                let sprinting = waypoints
                    .front()
                    .is_some_and(|intent| intent.sprinting && sprint_allowed);
                let max_step =
                    base_step * hunger_mult * onlinerpg_shared::hunger::sprint_move_mult(sprinting);
                let old_position = player.position;
                let old_floor = player.floor_level;
                let old_rotation = player.rotation;
                let mut budget = max_step;
                let mut blocked = false;
                while let Some(intent) = waypoints.front() {
                    let target = &intent.target;
                    let dx = shortest_world_delta_x(player.position.x, target.x);
                    let dz = target.z - player.position.z;
                    let dist = (dx * dx + dz * dz).sqrt();
                    let snap = dist <= budget;
                    // Step in unwrapped X so a seam-crossing move stays a
                    // short local sweep for the collision query.
                    let (step_x, step_y, step_z) = if snap {
                        (player.position.x + dx, target.y, target.z)
                    } else {
                        let t = budget / dist;
                        (
                            player.position.x + dx * t,
                            player.position.y + (target.y - player.position.y) * t,
                            player.position.z + dz * t,
                        )
                    };
                    // Edge-crossing subset of the client's continuous-mover
                    // check (no body radius, on the leg's own floor), including
                    // its wall slide. Only a step both axes refuse stops the
                    // player and drops the queue.
                    if intent.check_collision {
                        let step_floor =
                            super::passability::authoritative_floor(&cache, &player.position);
                        match super::passability::resolve_step(
                            &cache,
                            player.position.x,
                            player.position.z,
                            step_x,
                            step_z,
                            step_floor,
                            player.position.y,
                        ) {
                            super::passability::StepOutcome::Clear => {}
                            super::passability::StepOutcome::Slid(slid_x, slid_z) => {
                                // The leg is unfinished: keep it queued so the
                                // next tick resumes from the slid position.
                                player.rotation = intent.rotation;
                                player.position = Position {
                                    x: wrap_world_x(slid_x),
                                    y: step_y,
                                    z: slid_z,
                                };
                                break;
                            }
                            super::passability::StepOutcome::Blocked(info) => {
                                // A blocked step never moves the player, so the
                                // position/rotation here are what the correction
                                // sends.
                                refused.push(RefusedMove {
                                    player_id: *player_id,
                                    position: player.position,
                                    rotation: player.rotation,
                                    floor_level: player.floor_level,
                                    step_x,
                                    step_z,
                                    step_y,
                                    step_floor,
                                    intent_floor: intent.floor_level,
                                    sealed: super::passability::sealed_in(
                                        &cache,
                                        &player.position,
                                        step_floor,
                                    ),
                                    block_key: info.key.to_string(),
                                    stairwell: info.stairwell,
                                    consulted: info.consulted,
                                });
                                blocked = true;
                                break;
                            }
                        }
                    }
                    player.rotation = intent.rotation;
                    if snap {
                        player.position = *target;
                        player.floor_level = intent.floor_level;
                        budget -= dist;
                        waypoints.pop_front();
                    } else {
                        player.position = Position {
                            x: wrap_world_x(step_x),
                            y: step_y,
                            z: step_z,
                        };
                        break;
                    }
                }
                let position_changed = player.position.x != old_position.x
                    || player.position.y != old_position.y
                    || player.position.z != old_position.z;
                // Rotation-only updates are not activity: only actual
                // displacement burns satiation (doc/HUNGER.md).
                if position_changed {
                    activities.push((*player_id, dt.max(0.0), sprinting));
                    steps.push(super::ambient_spawn::MoveStep {
                        player_id: *player_id,
                        from: old_position,
                        to: player.position,
                        floor_level: player.floor_level,
                        is_official_npc: player.is_official_npc,
                    });
                }
                if position_changed
                    || player.floor_level != old_floor
                    || player.rotation != old_rotation
                {
                    moved.push((
                        *player_id,
                        old_position,
                        old_floor,
                        player.clone(),
                        sprinting,
                    ));
                }
                !blocked && !waypoints.is_empty()
            });
        }

        let refused = self.free_sealed_players(refused).await;
        self.correct_refused_positions(refused).await;
        self.record_movement_activity(&activities).await;

        for (player_id, old_position, old_floor, moved_player, sprinting) in moved {
            let update_msg = ServerMessage::PlayerMoved {
                player_id,
                position: moved_player.position,
                rotation: moved_player.rotation,
                floor_level: moved_player.floor_level,
                sprinting,
            };
            self.finish_position_update(
                &player_id,
                old_position,
                old_floor,
                moved_player,
                update_msg,
            )
            .await;
        }

        // Last: placement samples terrain, and nothing should delay the move
        // fanout. Distance walked is what grants ambient monsters
        // (doc/REPEAT_FARMING.md).
        self.spawn_along_movement(&steps).await;
        self.soak_movers(&steps).await;
    }

    /// Step players sealed into their own cell out to an adjoining one, and
    /// return the refusals still worth a correction — see
    /// `passability::escape_from_sealed_cell` for what seals a cell under a
    /// standing player. A rescued player must not keep their refusal: the
    /// correction carries the position they were refused *at*, which would snap
    /// them straight back inside the wall the teleport just took them out of.
    ///
    /// Teleport rather than a correction: the mover is out of the world's
    /// geometry rather than out of sync with it, and unlike a correction it
    /// reaches the other players watching too.
    async fn free_sealed_players(&self, refused: Vec<RefusedMove>) -> Vec<RefusedMove> {
        let mut kept = Vec::with_capacity(refused.len());
        for r in refused {
            let out = if r.sealed {
                self.sealed_player_escape(&r.position, r.step_floor).await
            } else {
                None
            };
            let Some(out) = out else {
                kept.push(r);
                continue;
            };
            warn!(
                "Freeing player {} sealed at ({:.1},{:.1}) -> ({:.1},{:.1})",
                r.player_id, r.position.x, r.position.z, out.x, out.z
            );
            self.teleport_player(&r.player_id, out, r.rotation, r.floor_level)
                .await;
        }
        kept
    }

    /// Tell players whose step was refused where the server actually has them.
    /// Nothing else reconciles the two sims — the client ignores its own
    /// `PlayerMoved` — so a refusal would otherwise strand the two apart.
    ///
    /// Throttled per player, and the warn rides the same gate: one line per
    /// snap actually sent, not one per refused tick, so a player grinding a
    /// wall no longer floods the journal.
    async fn correct_refused_positions(&self, refused: Vec<RefusedMove>) {
        if refused.is_empty() {
            return;
        }
        let now = std::time::Instant::now();
        let due: Vec<_> = {
            let mut last = self.last_position_correction.write().await;
            // Only prune site: one pass per tick, not one per snapped player.
            last.retain(|_, at| now.duration_since(*at) < POSITION_CORRECTION_COOLDOWN);
            refused
                .into_iter()
                .filter(|r| correction_due(&mut last, &r.player_id, now))
                .collect()
        };
        // One pass each over the batch, rather than a lock per refusal: names
        // for the warn, then streaks for the desync detector.
        let profiles: Vec<(String, bool)> = {
            let players = self.players.read().await;
            due.iter()
                .map(|r| match players.get(&r.player_id) {
                    Some(p) => (p.name.clone(), p.is_official_npc),
                    None => (r.player_id.to_string(), false),
                })
                .collect()
        };
        let fires: Vec<usize> = {
            let mut grinds = self.stale_layout_grinds.write().await;
            // Only prune site, like the correction map above.
            grinds.retain(|_, g| now.duration_since(g.at) < LAYOUT_GRIND_TTL);
            due.iter()
                .enumerate()
                .filter_map(|(i, r)| {
                    record_grind(&mut grinds, &r.player_id, grind_site(r), now).map(|_| i)
                })
                .collect()
        };
        let mut kicks = Vec::new();
        for i in fires {
            let (name, is_npc) = &profiles[i];
            let r = &due[i];
            // Our own NPCs ship with the server, so their layout is ours by
            // construction: one of them stuck here is a bug on our side, and
            // disconnecting it would only feed a systemd restart loop.
            if *is_npc {
                warn!(
                    "Layout grind by NPC {name} at {} floor {} x{LAYOUT_GRIND_LIMIT} — our bug, not a stale build",
                    r.block_key, r.intent_floor
                );
                continue;
            }
            warn!(
                "Layout grind: {name} pushed {} floor {} x{LAYOUT_GRIND_LIMIT} — disconnecting, its layout is not ours",
                r.block_key, r.intent_floor
            );
            kicks.push(r.player_id);
        }
        if !kicks.is_empty() {
            self.pending_layout_kicks.write().await.extend(kicks);
        }

        for (r, (name, _)) in due.iter().zip(&profiles) {
            // Stays at warn: with the slide in place a hit means client and
            // server disagree — a bug signal, not an expected outcome.
            warn!(
                "Blocked move for player {}: ({:.1},{:.1}) -> ({:.1},{:.1}) y={:.1}->{:.1} \
                 floor={} (intent {}) by {} stairwell={} consulted={}",
                name,
                r.position.x,
                r.position.z,
                r.step_x,
                r.step_z,
                r.position.y,
                r.step_y,
                r.step_floor,
                r.intent_floor,
                r.block_key,
                r.stairwell,
                r.consulted,
            );
            self.send_direct_message(
                &r.player_id,
                ServerMessage::PositionCorrected {
                    position: r.position,
                    rotation: r.rotation,
                    floor_level: r.floor_level,
                },
            )
            .await;
        }
    }

    /// Disconnect the players the grind detector flagged. Runs from the time
    /// sync tick because the movement tick has no `AuthService` to persist
    /// them with.
    pub async fn drain_stale_layout_kicks(&self, auth: &AuthService) {
        let pending = std::mem::take(&mut *self.pending_layout_kicks.write().await);
        if pending.is_empty() {
            return;
        }
        {
            let mut grinds = self.stale_layout_grinds.write().await;
            for player_id in &pending {
                grinds.remove(player_id);
            }
        }
        for player_id in pending {
            self.kick_player(
                &player_id,
                "Your client and this server disagree about the dungeon's walls. \
                 Reloading — if this repeats, update your client.",
                Some(onlinerpg_shared::CLOSE_CODE_CLIENT_DESYNC),
                auth,
            )
            .await;
        }
    }

    /// Snap a client whose move was refused outright (too far, dead) back to
    /// the authoritative pose. Nothing else reconciles the two sims — the
    /// mover never receives its own `PlayerMoved` — so a silent drop leaves
    /// the client's prediction walking a path the server never accepted.
    /// Rides the shared correction throttle; returns whether a snap was sent.
    async fn snap_refused_move_back(&self, player_id: &PlayerId) -> bool {
        let now = std::time::Instant::now();
        let due = {
            let mut last = self.last_position_correction.write().await;
            correction_due(&mut last, player_id, now)
        };
        if !due {
            return false;
        }
        let Some((position, rotation, floor_level)) = self.get_player_position(player_id).await
        else {
            return false;
        };
        self.send_direct_message(
            player_id,
            ServerMessage::PositionCorrected {
                position,
                rotation,
                floor_level,
            },
        )
        .await;
        true
    }

    /// Refuse a client-reported floor above the housing limit and snap the
    /// client back. It reads its own floor from local geometry and so keeps
    /// resending the rejected value: without a snap every later move packet is
    /// dropped too and the player is stranded with no signal. Rides the
    /// refused-move throttle, warn included.
    async fn reject_out_of_range_floor(&self, player_id: &PlayerId, floor_level: i8, source: &str) {
        if self.snap_refused_move_back(player_id).await {
            warn!(
                "Rejected {} with out-of-range floor {} from player {}",
                source, floor_level, player_id
            );
        }
    }

    /// Store a position immediately (trusted server-side path) and run the
    /// shared bookkeeping/fanout.
    async fn apply_player_position(
        &self,
        player_id: &PlayerId,
        new_position: Position,
        new_rotation: f32,
        floor_level: i8,
        update_msg: ServerMessage,
    ) {
        self.movement_intents.write().await.remove(player_id);
        let (old_position, old_floor, moved_player) = {
            let mut players = self.players.write().await;
            let Some(player) = players.get_mut(player_id) else {
                warn!("Attempted to move non-existent player: {}", player_id);
                return;
            };
            let old_position = player.position;
            let old_floor = player.floor_level;
            player.position = new_position;
            player.rotation = new_rotation;
            player.floor_level = floor_level;
            (old_position, old_floor, player.clone())
        };
        self.finish_position_update(player_id, old_position, old_floor, moved_player, update_msg)
            .await;
    }

    /// Shared bookkeeping after a position write: spatial cell, dirty flag,
    /// floor-change handling and AOI fanout of `update_msg`. Every relocation
    /// funnels through here — client moves, tick walking, teleports, floor
    /// changes — so this is also where movement breaks fishing. Turning in
    /// place is exempt: the cast itself faces the water with a rotation-only
    /// move.
    async fn finish_position_update(
        &self,
        player_id: &PlayerId,
        old_position: Position,
        old_floor: i8,
        moved_player: Player,
        update_msg: ServerMessage,
    ) {
        if old_position != moved_player.position || old_floor != moved_player.floor_level {
            self.cancel_concentration_if_active(player_id).await;
        }
        let new_position = moved_player.position;
        let floor_level = moved_player.floor_level;
        self.move_player_spatial_cell(player_id, &old_position, &new_position)
            .await;
        self.mark_dirty(player_id).await;
        // Rotation-only moves are exempt: markers draw x/z and floor.
        if old_position.x != new_position.x
            || old_position.z != new_position.z
            || old_floor != floor_level
        {
            self.mark_party_position_dirty(player_id).await;
        }
        if old_position != new_position {
            self.check_dungeon_discovery(player_id, &new_position).await;
        }
        if old_floor != floor_level {
            self.handle_player_floor_change(
                player_id,
                old_floor,
                floor_level,
                &old_position,
                &new_position,
            )
            .await;
        }
        self.fanout_player_position_update(
            player_id,
            &old_position,
            old_floor,
            &moved_player,
            update_msg,
        )
        .await;
    }

    /// Apply a floor change reported between waypoints, leaving the move queue
    /// alone. Keeps AOI membership tracking where the player visually is while
    /// they walk a stairwell, which is one uninterrupted leg.
    pub async fn update_player_floor(&self, player_id: &PlayerId, floor_level: i8) {
        if exceeds_positive_floor_limit(floor_level) {
            self.reject_out_of_range_floor(player_id, floor_level, "floor change")
                .await;
            return;
        }
        let (current_floor, position, is_official_npc) = {
            let players = self.players.read().await;
            match players.get(player_id) {
                Some(p) => (p.floor_level, p.position, p.is_official_npc),
                None => return,
            }
        };

        let (settled_floor, snapped_y) = if floor_level < 0 || current_floor < 0 {
            let verdict = self
                .validated_dungeon_floor(
                    player_id,
                    current_floor,
                    floor_level,
                    &position,
                    &position,
                )
                .await;
            (verdict.floor, verdict.y)
        } else {
            let floor = self
                .validated_house_floor(
                    player_id,
                    current_floor,
                    floor_level,
                    &position,
                    &position,
                    is_official_npc,
                )
                .await;
            let y = self
                .surface_ground_y(floor as u8, &position, position.y)
                .await;
            (floor, y)
        };
        // Refused claims must be told, or the client re-announces forever.
        if settled_floor != floor_level {
            self.snap_refused_move_back(player_id).await;
        }
        let floor_level = settled_floor;
        if floor_level == current_floor {
            return;
        }

        // Queued legs carry the floor they were sent with, and snapping to one
        // re-applies it. Without this the explicit change is clobbered the
        // moment the leg finishes, flickering the player back out of view.
        // Legs are appended one at a time as each waypoint is reached, so every
        // pending one belongs to the floor we just moved to.
        {
            let mut queues = self.movement_intents.write().await;
            if let Some(queue) = queues.get_mut(player_id) {
                for intent in queue.iter_mut() {
                    intent.floor_level = floor_level;
                }
            }
        }

        let moved_player = {
            let mut players = self.players.write().await;
            let Some(player) = players.get_mut(player_id) else {
                return;
            };
            player.floor_level = floor_level;
            // Snap Y to the claimed floor's ground: leaving the departing
            // floor's Y would misplace AOI and targeting until the next move.
            player.position.y = snapped_y;
            player.clone()
        };

        let update_msg = ServerMessage::PlayerMoved {
            player_id: *player_id,
            position: moved_player.position,
            rotation: moved_player.rotation,
            floor_level,
            sprinting: false,
        };
        self.finish_position_update(player_id, position, current_floor, moved_player, update_msg)
            .await;
    }

    /// A walkable spot on a small ring beside `center` for a teleport
    /// arrival, golden-angle-seeded by the mover's id so simultaneous
    /// arrivals don't stack.
    pub(crate) fn arrival_beside(&self, mover: &PlayerId, center: &Position) -> Position {
        let angle = (mover.get() % 360) as f32 * GOLDEN_ANGLE_RAD;
        self.open_spot_beside(center, angle, ARRIVAL_RING_RADIUS)
    }

    /// A walkable spot at `angle` from `center`, X wrapped to the canonical
    /// range (callers store it directly). A blocked spot (dungeon walls run
    /// 1m from a corridor's center) retries at half radius and finally lands
    /// on `center` itself — a walkable cell by construction.
    pub(crate) fn open_spot_beside(&self, center: &Position, angle: f32, radius: f32) -> Position {
        let cache = self.passability_read();
        let cell_floor = super::passability::authoritative_floor(&cache, center);
        for radius in [radius, radius * 0.5] {
            let candidate = Position {
                x: wrap_world_x(center.x + angle.cos() * radius),
                y: center.y,
                z: center.z + angle.sin() * radius,
            };
            if super::passability::wrapped_block_info(
                &cache,
                center.x,
                center.z,
                candidate.x,
                candidate.z,
                cell_floor,
                center.y,
            )
            .is_none()
            {
                return candidate;
            }
        }
        *center
    }

    pub async fn teleport_player(
        &self,
        player_id: &PlayerId,
        mut new_position: Position,
        new_rotation: f32,
        new_floor_level: i8,
    ) {
        // A NaN position would poison the SQLite save batch for everyone.
        if !(new_position.is_finite() && new_rotation.is_finite()) {
            warn!("Rejected non-finite teleport for player {player_id}");
            return;
        }
        new_position.x = wrap_world_x(new_position.x);
        self.apply_player_position(
            player_id,
            new_position,
            new_rotation,
            new_floor_level,
            ServerMessage::PlayerTeleported {
                player_id: *player_id,
                position: new_position,
                rotation: new_rotation,
                floor_level: new_floor_level,
            },
        )
        .await;
        self.void_summons_aimed_at(player_id).await;
    }

    /// Put a player at the world spawn on the surface.
    pub async fn teleport_to_town(&self, player_id: &PlayerId) {
        let spawn = &world_config().spawn_position;
        self.teleport_player(player_id, spawn.position(), spawn.rotation, 0)
            .await;
    }

    pub async fn respawn_player(&self, player_id: &PlayerId) {
        self.movement_intents.write().await.remove(player_id);
        let respawned_player = {
            let mut players = self.players.write().await;
            if let Some(player) = players.get_mut(player_id) {
                if player.health > 0 {
                    info!(
                        "Ignored respawn request for alive player {} ({}) HP: {}/{}",
                        player.name, player.id, player.health, player.max_health
                    );
                    return;
                }
                player.health = player.max_health;
                let old_floor = player.floor_level;
                let old_position = player.position;
                let spawn = &world_config().spawn_position;
                player.position = spawn.position();
                player.rotation = spawn.rotation;
                // Death always returns to the surface — clears dungeon
                // depths and stale housing floors alike.
                player.floor_level = 0;
                Some((old_floor, old_position, player.clone()))
            } else {
                None
            }
        };

        if let Some((old_floor, old_position, player)) = respawned_player {
            info!("Player {} ({}) respawned", player.name, player.id);
            let update_msg = ServerMessage::PlayerRespawned {
                player: player.clone(),
            };
            self.finish_position_update(player_id, old_position, old_floor, player, update_msg)
                .await;
            self.reset_hunger_on_respawn(player_id).await;
            self.mark_party_vitals_dirty(player_id).await;
        } else {
            warn!("Attempted to respawn non-existent player: {}", player_id);
        }
    }

    /// Revive a defeated player where they fell with `hp_percent` of their
    /// max HP. Position, floor and the already-applied death penalty stay, so
    /// this only announces the new HP — no AOI move. Returns false when the
    /// player is alive or unknown.
    pub async fn revive_in_place(&self, player_id: &PlayerId, hp_percent: u32) -> bool {
        let revived = {
            let mut players = self.players.write().await;
            let Some(player) = players.get_mut(player_id).filter(|p| p.health == 0) else {
                return false;
            };
            player.health = (player.max_health * hp_percent / 100).max(1);
            player.clone()
        };
        info!("Player {} ({}) revived in place", revived.name, revived.id);
        self.mark_dirty(player_id).await;
        self.mark_party_vitals_dirty(player_id).await;
        let (position, floor_level) = (revived.position, revived.floor_level);
        self.send_direct_message_to_players_within_position(
            &position,
            floor_level,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::PlayerRespawned { player: revived },
            None,
        )
        .await;
        true
    }

    pub async fn get_player_position(&self, player_id: &PlayerId) -> Option<(Position, f32, i8)> {
        let players = self.players.read().await;
        players
            .get(player_id)
            .map(|p| (p.position, p.rotation, p.floor_level))
    }

    pub async fn set_player_torch(&self, player_id: &PlayerId, enabled: bool) {
        let position = {
            let mut players = self.players.write().await;
            if let Some(player) = players.get_mut(player_id) {
                if player.torch_on == enabled {
                    return;
                }
                player.torch_on = enabled;
                Some((player.position, player.floor_level))
            } else {
                None
            }
        };

        if let Some((position, floor_level)) = position {
            self.send_direct_message_to_players_within_position(
                &position,
                floor_level,
                super::EVENT_DELIVERY_RADIUS,
                ServerMessage::PlayerTorchToggled {
                    player_id: *player_id,
                    enabled,
                },
                Some(player_id),
            )
            .await;
        }
    }

    /// Update the gear nearby clients render and tell them what changed. Both
    /// slots are compared under one write lock: every bag mutation routes
    /// through here and almost none of them touch gear, so taking the global
    /// players lock once per snapshot rather than once per slot matters.
    /// Reads what shows from the inventory itself rather than taking a widening
    /// list of `Option<String>`s. The cape's dye and texture are part of it:
    /// neither changes the def id, so both have to count as a change or the
    /// new look never reaches anyone else.
    pub async fn set_player_gear(&self, player_id: &PlayerId, inventory: &PlayerInventory) {
        let main_hand = inventory.equipped_def_id(EquipSlot::MainHand);
        let back = inventory.equipped_def_id(EquipSlot::Back);
        let back_color = inventory.equipped_cape_color();
        let back_texture = inventory.equipped_cape_texture();
        let changed = {
            let mut players = self.players.write().await;
            let Some(player) = players.get_mut(player_id) else {
                return;
            };
            let mut messages = Vec::new();
            if player.main_hand != main_hand {
                player.main_hand = main_hand.clone();
                messages.push(ServerMessage::PlayerMainHandChanged {
                    player_id: *player_id,
                    item_def_id: main_hand,
                });
            }
            if player.back != back
                || player.back_color != back_color
                || player.back_texture != back_texture
            {
                player.back = back.clone();
                player.back_color = back_color.clone();
                player.back_texture = back_texture.clone();
                messages.push(ServerMessage::PlayerBackChanged {
                    player_id: *player_id,
                    item_def_id: back,
                    cape_color: back_color,
                    cape_texture: back_texture,
                });
            }
            if messages.is_empty() {
                return;
            }
            (player.position, player.floor_level, messages)
        };

        let (position, floor_level, messages) = changed;
        for message in messages {
            self.send_direct_message_to_players_within_position(
                &position,
                floor_level,
                super::EVENT_DELIVERY_RADIUS,
                message,
                Some(player_id),
            )
            .await;
        }
    }

    pub async fn set_player_interaction(
        &self,
        player_id: &PlayerId,
        object_type: Option<String>,
        object_id: Option<u32>,
    ) {
        let rejected_or_position = {
            let mut players = self.players.write().await;

            // Reject if the specific object is already occupied
            if object_id.is_some_and(|fid| {
                players
                    .values()
                    .any(|p| p.id != *player_id && p.object_id == Some(fid))
            }) {
                Err(())
            } else if let Some(player) = players.get_mut(player_id) {
                player.object_type = object_type.clone();
                player.object_id = object_id;
                Ok(Some((player.position, player.floor_level)))
            } else {
                Ok(None)
            }
        };

        if rejected_or_position.is_err() {
            self.send_direct_message(
                player_id,
                ServerMessage::InteractionRejected {
                    reason: "occupied".to_string(),
                },
            )
            .await;
        } else if let Ok(Some((position, floor_level))) = rejected_or_position {
            if object_type.as_deref() != Some(onlinerpg_shared::messages::MUSIC_EMOTE) {
                self.music_performances.write().await.remove(player_id);
            }
            self.send_direct_message_to_players_within_position(
                &position,
                floor_level,
                super::EVENT_DELIVERY_RADIUS,
                ServerMessage::PlayerInteractionChanged {
                    player_id: *player_id,
                    object_type,
                },
                None,
            )
            .await;
        }
    }

    pub async fn mark_dirty(&self, player_id: &PlayerId) {
        let mut dirty = self.dirty_players.write().await;
        dirty.insert(*player_id);
    }

    async fn restore_dirty_players(&self, ids: Vec<PlayerId>) {
        if !ids.is_empty() {
            self.dirty_players.write().await.extend(ids);
        }
    }

    pub async fn remove_dirty(&self, player_id: &PlayerId) {
        let mut dirty = self.dirty_players.write().await;
        dirty.remove(player_id);
    }

    pub async fn collect_dirty_character_states(&self) -> (Vec<PlayerId>, Vec<CharacterSaveData>) {
        let dirty_ids: Vec<PlayerId> = {
            let mut dirty = self.dirty_players.write().await;
            dirty.drain().collect()
        };

        if dirty_ids.is_empty() {
            return (Vec::new(), Vec::new());
        }

        let players = self.players.read().await;
        let player_chars = self.player_characters.read().await;
        let gold_map = self.player_gold.read().await;
        let hunger = self.hunger.read().await;

        let mut result = Vec::with_capacity(dirty_ids.len());
        for pid in &dirty_ids {
            if let (Some(player), Some((char_id, xp, _))) =
                (players.get(pid), player_chars.get(pid))
            {
                let gold = gold_map.get(pid).copied().unwrap_or(0);
                let satiation = super::hunger::satiation_for_save(&hunger, pid);
                result.push(build_save_data(player, *char_id, *xp, gold, satiation));
            }
        }

        (dirty_ids, result)
    }

    pub async fn get_player_save_data(&self, player_id: &PlayerId) -> Option<CharacterSaveData> {
        let players = self.players.read().await;
        let player_chars = self.player_characters.read().await;
        let gold_map = self.player_gold.read().await;
        let hunger = self.hunger.read().await;

        let player = players.get(player_id)?;
        let (char_id, xp, _) = player_chars.get(player_id)?;
        let gold = gold_map.get(player_id).copied().unwrap_or(0);
        let satiation = super::hunger::satiation_for_save(&hunger, player_id);

        Some(build_save_data(player, *char_id, *xp, gold, satiation))
    }

    async fn insert_player_spatial_cell(&self, player_id: &PlayerId, position: &Position) {
        self.player_spatial_cells
            .write()
            .await
            .insert(*player_id, position);
    }

    async fn remove_player_spatial_cell(&self, player_id: &PlayerId, position: &Position) {
        self.player_spatial_cells
            .write()
            .await
            .remove(player_id, position);
    }

    async fn move_player_spatial_cell(
        &self,
        player_id: &PlayerId,
        old_position: &Position,
        new_position: &Position,
    ) {
        // Most moves stay inside one cell. Checking before taking the lock keeps
        // them off the write guard every mover on the server shares.
        if super::SpatialCell::from_position(old_position)
            == super::SpatialCell::from_position(new_position)
        {
            return;
        }
        self.player_spatial_cells
            .write()
            .await
            .moved(player_id, old_position, new_position);
    }

    async fn fanout_player_position_update(
        &self,
        player_id: &PlayerId,
        old_position: &Position,
        old_floor: i8,
        player: &Player,
        update_msg: ServerMessage,
    ) {
        // Visibility is per-floor: the old set is who could see the player on
        // the floor it left, the new set is who can see it on the floor it is
        // on now. For a same-floor move both use the same floor; for a stair /
        // teleport / respawn floor change the diff naturally turns into
        // disappear-from-old-floor + appear-on-new-floor.
        let new_floor = player.floor_level;
        let old_visible: HashSet<PlayerId> = self
            .player_ids_within_position(old_position, old_floor, super::EVENT_DELIVERY_RADIUS)
            .await
            .into_iter()
            .filter(|id| id != player_id)
            .collect();
        let new_visible: HashSet<PlayerId> = self
            .player_ids_within_position(&player.position, new_floor, super::EVENT_DELIVERY_RADIUS)
            .await
            .into_iter()
            .filter(|id| id != player_id)
            .collect();

        let left: Vec<_> = old_visible.difference(&new_visible).cloned().collect();
        let entered: Vec<_> = new_visible.difference(&old_visible).cloned().collect();
        let stayed: Vec<_> = new_visible.intersection(&old_visible).cloned().collect();

        for other_id in &left {
            self.send_direct_message(
                player_id,
                ServerMessage::PlayerDisappeared {
                    player_id: *other_id,
                },
            )
            .await;
            self.send_direct_message(
                other_id,
                ServerMessage::PlayerDisappeared {
                    player_id: *player_id,
                },
            )
            .await;
        }

        let entered_players = {
            let players = self.players.read().await;
            entered
                .iter()
                .filter_map(|id| players.get(id).cloned())
                .collect::<Vec<_>>()
        };

        // Coming into earshot of a running performance delivers it mid-track,
        // in either direction. Collected here, sent after the appearances so
        // the receiver already knows the performer.
        let music_msgs: Vec<(PlayerId, ServerMessage)> = if entered_players.is_empty() {
            Vec::new()
        } else {
            let performances = self.music_performances.read().await;
            let mut msgs = Vec::new();
            let mut push = |to: PlayerId, performer: PlayerId| {
                if let Some(entry) = performances.get(&performer) {
                    msgs.push((to, super::chat::music_started_msg(performer, entry)));
                }
            };
            for other in &entered_players {
                push(*player_id, other.id);
                push(other.id, *player_id);
            }
            msgs
        };

        for other in entered_players {
            self.send_direct_message(
                player_id,
                ServerMessage::PlayerAppeared {
                    player: other.clone(),
                },
            )
            .await;
            self.send_direct_message(
                &other.id,
                ServerMessage::PlayerAppeared {
                    player: player.clone(),
                },
            )
            .await;
        }
        for (to, msg) in music_msgs {
            self.send_direct_message(&to, msg).await;
        }

        let (monsters_left, monsters_entered) = {
            let monsters = self.monsters.read().await;
            let (left, entered) = aoi_diff(
                // Not `values()`: this runs on every accepted move packet, and
                // the registry holds up to 135k.
                monsters.near_either(old_position, &player.position),
                |m| (m.position, m.floor_level),
                (old_position, old_floor),
                (&player.position, new_floor),
            );
            (
                left.into_iter()
                    .map(|m| {
                        (
                            m.id.clone(),
                            m.position,
                            m.floor_level,
                            m.lifecycle,
                            m.owner_id,
                        )
                    })
                    .collect::<Vec<_>>(),
                entered.into_iter().cloned().collect::<Vec<_>>(),
            )
        };

        // Leaving AOI is mere visibility for a watcher, but for the owner it
        // ends the simulation — its client reads MonsterRemoved as "drop the
        // brain". Owned monsters are released instead: transferred, despawned
        // or parked on the spot, each branch delivering the owner's removal.
        let mut abandoned = Vec::new();
        for (monster_id, position, floor_level, lifecycle, owner_id) in monsters_left {
            if owner_id == Some(*player_id) {
                abandoned.push((monster_id, position, floor_level, lifecycle));
            } else {
                self.send_direct_message(player_id, ServerMessage::MonsterRemoved { monster_id })
                    .await;
            }
        }
        if !abandoned.is_empty() {
            self.release_monsters_left_behind(player_id, abandoned)
                .await;
        }
        for monster in &monsters_entered {
            self.send_direct_message(
                player_id,
                ServerMessage::MonsterSpawned {
                    monster: self.wire_monster(monster),
                },
            )
            .await;
        }
        self.adopt_unattended_monsters(player_id, &monsters_entered)
            .await;

        let (items_left, items_entered) = {
            let ground_items = self.ground_items.read().await;
            let (left, entered) = aoi_diff(
                ground_items.values(),
                |sgi| (sgi.item.position, sgi.item.floor_level),
                (old_position, old_floor),
                (&player.position, new_floor),
            );
            (
                left.into_iter()
                    .map(|sgi| sgi.item.instance_id)
                    .collect::<Vec<_>>(),
                entered
                    .into_iter()
                    .map(|sgi| sgi.item.clone())
                    .collect::<Vec<_>>(),
            )
        };

        for instance_id in items_left {
            self.send_direct_message(
                player_id,
                ServerMessage::GroundItemRemoved {
                    instance_id,
                    picked_up_by: None,
                },
            )
            .await;
        }
        for item in items_entered {
            self.send_direct_message(player_id, ServerMessage::GroundItemAppeared { item })
                .await;
        }

        let (fires_left, fires_entered) = {
            let campfires = self.campfires.read().await;
            let (left, entered) = aoi_diff(
                campfires.values(),
                |e| (e.campfire.position, e.campfire.floor_level),
                (old_position, old_floor),
                (&player.position, new_floor),
            );
            (
                left.into_iter().map(|e| e.campfire.id).collect::<Vec<_>>(),
                entered
                    .into_iter()
                    .map(|e| e.campfire.clone())
                    .collect::<Vec<_>>(),
            )
        };
        for campfire_id in fires_left {
            self.send_direct_message(player_id, ServerMessage::CampfireRemoved { campfire_id })
                .await;
        }
        for campfire in fires_entered {
            self.send_direct_message(player_id, ServerMessage::CampfireAppeared { campfire })
                .await;
        }

        let (stalls_left, stalls_entered) = {
            let stalls = self.stalls.read().await;
            let (left, entered) = aoi_diff(
                stalls.values(),
                |s| (s.position, s.floor_level),
                (old_position, old_floor),
                (&player.position, new_floor),
            );
            (
                left.into_iter().map(|s| s.id).collect::<Vec<_>>(),
                entered.into_iter().cloned().collect::<Vec<_>>(),
            )
        };
        for stall_id in stalls_left {
            self.send_direct_message(player_id, ServerMessage::StallRemoved { stall_id })
                .await;
        }
        for stall in stalls_entered {
            self.send_direct_message(player_id, ServerMessage::StallAppeared { stall })
                .await;
        }

        // The mover's own leash is read here too, off the one lock this pass
        // already takes.
        let (hats_left, hats_entered, strayed) = {
            let tip_hats = self.tip_hats.read().await;
            let (left, entered) = aoi_diff(
                tip_hats.values(),
                |h| (h.position, h.floor_level),
                (old_position, old_floor),
                (&player.position, new_floor),
            );
            (
                left.into_iter().map(|h| h.id).collect::<Vec<_>>(),
                entered.into_iter().cloned().collect::<Vec<_>>(),
                tip_hats
                    .get(player_id)
                    .is_some_and(|h| h.strayed_from(&player.position, new_floor)),
            )
        };
        for tip_hat_id in hats_left {
            self.send_direct_message(player_id, ServerMessage::TipHatRemoved { tip_hat_id })
                .await;
        }
        for tip_hat in hats_entered {
            self.send_direct_message(player_id, ServerMessage::TipHatAppeared { tip_hat })
                .await;
        }
        if strayed {
            self.pack_up_strayed_tip_hat(player_id).await;
        }

        // The mover hears its own update through the same fanout, so the
        // message is serialized once for everyone.
        let mut recipients = stayed;
        recipients.push(*player_id);
        self.send_direct_message_to_players(&recipients, update_msg)
            .await;
    }

    pub async fn player_ids_within_position(
        &self,
        position: &Position,
        floor_level: i8,
        radius: f32,
    ) -> Vec<PlayerId> {
        self.players_within_position(position, floor_level, radius, None)
            .await
            .into_iter()
            .map(|(player_id, _)| player_id)
            .collect()
    }

    /// As `player_ids_within_position`, but keeping the squared distance and
    /// skipping one player — what an ownership handoff needs to pick the
    /// nearest candidate that is not the one leaving.
    pub(super) async fn players_within_position(
        &self,
        position: &Position,
        floor_level: i8,
        radius: f32,
        skip: Option<&PlayerId>,
    ) -> Vec<(PlayerId, f32)> {
        let radius_sq = radius * radius;
        let players = self.players.read().await;
        let cells = self.player_spatial_cells.read().await;
        let mut found: HashMap<PlayerId, f32> = HashMap::new();

        for player_id in cells.keys_near(position, radius) {
            if skip == Some(player_id) {
                continue;
            }
            let Some(player) = players.get(player_id) else {
                continue;
            };

            let dist_sq = position.dist_xz_sq(&player.position);
            if player.floor_level == floor_level && dist_sq <= radius_sq {
                found.insert(*player_id, dist_sq);
            }
        }

        found.into_iter().collect()
    }

    pub async fn player_ids_within(&self, player_id: &PlayerId, radius: f32) -> Vec<PlayerId> {
        let (position, floor_level) = {
            let players = self.players.read().await;
            let Some(player) = players.get(player_id) else {
                return Vec::new();
            };
            (player.position, player.floor_level)
        };

        self.player_ids_within_position(&position, floor_level, radius)
            .await
    }

    #[allow(dead_code)]
    pub async fn get_player_count(&self) -> usize {
        self.players.read().await.len()
    }

    #[allow(dead_code)]
    pub async fn get_all_players(&self) -> HashMap<PlayerId, Player> {
        self.players.read().await.clone()
    }
}

#[cfg(test)]
mod grind_tests {
    use super::*;

    fn site(key: &str, cell: (i32, i32)) -> Option<GrindSite<'_>> {
        Some(GrindSite {
            key,
            cell,
            intent_floor: -4,
        })
    }

    /// Replay a run of refusals, returning the streak length at each fire.
    fn run(sites: &[Option<GrindSite<'_>>]) -> Vec<u32> {
        let mut grinds = HashMap::new();
        let player = PlayerId::from(1);
        let start = std::time::Instant::now();
        sites
            .iter()
            .enumerate()
            .filter_map(|(i, site)| {
                let now = start + POSITION_CORRECTION_COOLDOWN * i as u32;
                record_grind(&mut grinds, &player, *site, now)
            })
            .collect()
    }

    #[test]
    fn one_cell_ground_past_the_limit_fires_once() {
        let wall = site("dungeon:old_crypt", (-1456, 4702));
        let fires = run(&vec![wall; LAYOUT_GRIND_LIMIT as usize * 3]);
        assert_eq!(fires, vec![LAYOUT_GRIND_LIMIT]);
    }

    #[test]
    fn a_streak_short_of_the_limit_never_fires() {
        let wall = site("dungeon:old_crypt", (-1456, 4702));
        assert!(run(&vec![wall; LAYOUT_GRIND_LIMIT as usize - 1]).is_empty());
    }

    #[test]
    fn moving_to_another_wall_starts_over() {
        let mut sites = vec![site("dungeon:old_crypt", (-1456, 4702)); 9];
        // One refusal well clear of the streak: they are walking, not stuck.
        sites.push(site("dungeon:old_crypt", (-1460, 4702)));
        sites.extend(vec![site("dungeon:old_crypt", (-1456, 4702)); 9]);
        assert!(run(&sites).is_empty());
    }

    #[test]
    fn jitter_across_the_cell_boundary_keeps_the_streak() {
        // The shape prod actually logs: the same push rounding to neighbouring
        // cells from one correction to the next.
        let sites: Vec<_> = (0..LAYOUT_GRIND_LIMIT as usize)
            .map(|i| site("dungeon:old_crypt", (-1456 - (i % 2) as i32, 4702)))
            .collect();
        assert_eq!(run(&sites), vec![LAYOUT_GRIND_LIMIT]);
    }

    #[test]
    fn a_streak_cannot_walk_its_anchor_down_a_corridor() {
        // Each step is within a cell of the last, but not of where it began.
        let sites: Vec<_> = (0..LAYOUT_GRIND_LIMIT as usize * 2)
            .map(|i| site("dungeon:old_crypt", (-1456 + i as i32, 4702)))
            .collect();
        assert!(run(&sites).is_empty());
    }

    #[test]
    fn surface_and_furniture_refusals_are_ignored() {
        assert!(run(&vec![None; LAYOUT_GRIND_LIMIT as usize * 2]).is_empty());
    }

    #[test]
    fn a_streak_that_goes_quiet_expires() {
        let wall = site("dungeon:old_crypt", (-1456, 4702));
        let mut grinds = HashMap::new();
        let player = PlayerId::from(1);
        let start = std::time::Instant::now();
        let mut last = start;
        for i in 0..LAYOUT_GRIND_LIMIT - 1 {
            last = start + POSITION_CORRECTION_COOLDOWN * i;
            record_grind(&mut grinds, &player, wall, last);
        }
        // Back long after the last refusal: a fresh episode, not the tail of
        // the old one. The TTL runs from when the streak last moved.
        let later = last + LAYOUT_GRIND_TTL + POSITION_CORRECTION_COOLDOWN;
        assert!(record_grind(&mut grinds, &player, wall, later).is_none());
        assert!(grinds.get(&player).is_some_and(|g| g.count == 1));
    }
}
