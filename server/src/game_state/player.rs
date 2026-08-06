use crate::auth::{AuthError, AuthService, CharacterSaveData, ItemRow};
use crate::types::{CharacterAttributes, Player, PlayerId, Position, ServerMessage};
use crate::world_config::world_config;
use onlinerpg_shared::housing::MAX_FLOOR_LEVEL;
use onlinerpg_shared::{shortest_world_delta_x, wrap_world_x, PLAYER_MOVE_SPEED};
use std::collections::{HashMap, HashSet, VecDeque};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

/// Headroom over walk speed so the sim absorbs network jitter and catches up.
pub(super) const MOVE_SPEED_SLACK: f32 = 1.15;
/// Longest accepted move target; the farthest in-view click is ~42m.
const MAX_MOVE_TARGET_DISTANCE: f32 = 60.0;

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

/// Client-requested destination the server walks the player toward at capped
/// speed.
#[derive(Clone)]
pub(super) struct MoveIntent {
    target: Position,
    rotation: f32,
    floor_level: i8,
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

fn build_save_data(
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
    ) -> mpsc::UnboundedReceiver<super::DirectMessage> {
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
        let channels = self.direct_channels.read().await;
        if let Some(tx) = channels.get(player_id) {
            let _ = tx.send(super::DirectMessage::Typed(msg));
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
                let _ = tx.send(super::DirectMessage::Shared(bytes.clone()));
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
        kick_tx: mpsc::UnboundedSender<ServerMessage>,
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
        kick_tx: mpsc::UnboundedSender<ServerMessage>,
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
            let _ = replaced.kick_tx.send(ServerMessage::Kicked {
                player_id: replaced.player_id.unwrap_or(PlayerId::from(0)),
                reason: "Another session logged in with the same account".to_string(),
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
    pub(crate) async fn kick_player(&self, player_id: &PlayerId, reason: &str, auth: &AuthService) {
        let _sessions = self.character_session_lock.lock().await;
        match self.account_of_player(player_id).await {
            Some(account) => {
                self.evict_account_session_locked(&account, reason, auth)
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
        let _ = session.kick_tx.send(ServerMessage::Kicked {
            player_id: session.player_id.unwrap_or(PlayerId::from(0)),
            reason: reason.to_string(),
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

    pub async fn add_player(&self, mut player: Player) -> Option<ServerMessage> {
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
            .iter()
            .filter(|(_, monster)| {
                monster.floor_level == player_floor
                    && monster.position.dist_xz_sq(&player_position)
                        <= super::EVENT_DELIVERY_RADIUS * super::EVENT_DELIVERY_RADIUS
            })
            .map(|(id, monster)| (id.clone(), monster.clone()))
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

        if !other_players.is_empty()
            || !monsters.is_empty()
            || !ground_items.is_empty()
            || !campfires.is_empty()
        {
            return Some(ServerMessage::GameState {
                players: other_players,
                monsters,
                ground_items,
                campfires,
            });
        }

        None
    }

    pub async fn remove_player(&self, player_id: &PlayerId) {
        self.movement_intents.write().await.remove(player_id);
        self.ambient_spawn_allowances
            .write()
            .await
            .retain(|(owner_id, _), _| owner_id != player_id);
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

    /// Handle a client PlayerMove. The client sends destinations (waypoints),
    /// so the position becomes a MoveIntent that `tick_player_movement` walks
    /// toward; trusted (admin) connections apply immediately.
    pub async fn update_player_position(
        &self,
        player_id: &PlayerId,
        cmd: MoveCommand,
        trusted: bool,
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
            warn!(
                "Rejected non-finite move from player {}",
                self.player_name_of(player_id).await
            );
            return;
        }
        new_position.x = wrap_world_x(new_position.x);
        // Dungeon floors (negative) are validated against the entrance
        // registry and the floor's expected world Y before being stored.
        // Deliberately does not carry the name out: this runs for every move
        // packet, and the only consumers are the two rejection logs below,
        // which can afford their own lookup.
        let (current_floor, current_position) = {
            let players = self.players.read().await;
            match players.get(player_id) {
                Some(p) => (p.floor_level, p.position),
                None => {
                    warn!("Attempted to move non-existent player: {}", player_id);
                    return;
                }
            }
        };
        let floor_level = if floor_level < 0 || current_floor < 0 {
            self.validated_dungeon_floor(player_id, current_floor, floor_level, &new_position)
                .await
        } else {
            floor_level
        };

        if trusted {
            let sprinting = sprinting && self.hunger_sprint_allowed(player_id).await;
            self.apply_player_position(
                player_id,
                new_position,
                new_rotation,
                floor_level,
                ServerMessage::PlayerMoved {
                    player_id: *player_id,
                    position: new_position,
                    rotation: new_rotation,
                    floor_level,
                    sprinting,
                },
            )
            .await;
            return;
        }

        let mut queues = self.movement_intents.write().await;
        // Appended legs chain off the queue tail, so the distance guard must
        // measure the new leg, not the whole path from the current position.
        let leg_start = if append {
            queues
                .get(player_id)
                .and_then(|q| q.back())
                .map(|w| w.target)
                .unwrap_or(current_position)
        } else {
            current_position
        };
        let dist_sq = leg_start.dist_xz_sq(&new_position);
        if dist_sq > MAX_MOVE_TARGET_DISTANCE * MAX_MOVE_TARGET_DISTANCE {
            warn!(
                "Rejected move target {:.0}m away from player {}",
                dist_sq.sqrt(),
                self.player_name_of(player_id).await
            );
            return;
        }
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
        let base_step = PLAYER_MOVE_SPEED * MOVE_SPEED_SLACK * dt.max(0.0);
        let mut moved: Vec<(PlayerId, Position, i8, Player, bool)> = Vec::new();
        let mut activities: Vec<(PlayerId, f32, bool)> = Vec::new();
        let mut refused: Vec<RefusedMove> = Vec::new();
        {
            let mut queues = self.movement_intents.write().await;
            if queues.is_empty() {
                return;
            }
            // Hunger scales the speed itself, slack wraps the scaled value — a
            // Weak player's sim must not outrun their slowed client.
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
                let sprint_mult = if sprinting {
                    onlinerpg_shared::hunger::SPRINT_MOVE_MULT
                } else {
                    1.0
                };
                let max_step = base_step * hunger_mult * sprint_mult;
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
        for r in due {
            // Stays at warn: with the slide in place a hit means client and
            // server disagree — a bug signal, not an expected outcome.
            warn!(
                "Blocked move for player {}: ({:.1},{:.1}) -> ({:.1},{:.1}) y={:.1}->{:.1} \
                 floor={} (intent {}) by {} stairwell={} consulted={}",
                r.player_id,
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

    /// Refuse a client-reported floor above the housing limit and snap the
    /// client back. It reads its own floor from local geometry and so keeps
    /// resending the rejected value: without a snap every later move packet is
    /// dropped too and the player is stranded with no signal. Rides the
    /// refused-move throttle, warn included.
    async fn reject_out_of_range_floor(&self, player_id: &PlayerId, floor_level: i8, source: &str) {
        let now = std::time::Instant::now();
        let due = {
            let mut last = self.last_position_correction.write().await;
            correction_due(&mut last, player_id, now)
        };
        if !due {
            return;
        }
        let Some((position, rotation, current_floor)) = self.get_player_position(player_id).await
        else {
            return;
        };
        warn!(
            "Rejected {} with out-of-range floor {} from player {}",
            source, floor_level, player_id
        );
        self.send_direct_message(
            player_id,
            ServerMessage::PositionCorrected {
                position,
                rotation,
                floor_level: current_floor,
            },
        )
        .await;
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
        let (current_floor, position) = {
            let players = self.players.read().await;
            match players.get(player_id) {
                Some(p) => (p.floor_level, p.position),
                None => return,
            }
        };

        let floor_level = if floor_level < 0 || current_floor < 0 {
            self.validated_dungeon_floor(player_id, current_floor, floor_level, &position)
                .await
        } else {
            floor_level
        };
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

    /// A walkable spot at `angle` from `center`. A blocked spot (dungeon
    /// walls run 1m from a corridor's center) retries at half radius and
    /// finally lands on `center` itself — a walkable cell by construction.
    pub(crate) fn open_spot_beside(&self, center: &Position, angle: f32, radius: f32) -> Position {
        let cache = self.passability_read();
        let cell_floor = super::passability::authoritative_floor(&cache, center);
        for radius in [radius, radius * 0.5] {
            let candidate = Position {
                x: center.x + angle.cos() * radius,
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
        } else {
            warn!("Attempted to respawn non-existent player: {}", player_id);
        }
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

    pub async fn set_player_main_hand(&self, player_id: &PlayerId, item_def_id: Option<String>) {
        let position = {
            let mut players = self.players.write().await;
            if let Some(player) = players.get_mut(player_id) {
                if player.main_hand == item_def_id {
                    return;
                }
                player.main_hand = item_def_id.clone();
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
                ServerMessage::PlayerMainHandChanged {
                    player_id: *player_id,
                    item_def_id,
                },
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
        let mut cells = self.player_spatial_cells.write().await;
        cells
            .entry(super::SpatialCell::from_position(position))
            .or_default()
            .insert(*player_id);
    }

    async fn remove_player_spatial_cell(&self, player_id: &PlayerId, position: &Position) {
        let cell = super::SpatialCell::from_position(position);
        let mut cells = self.player_spatial_cells.write().await;
        let should_remove = if let Some(player_ids) = cells.get_mut(&cell) {
            player_ids.remove(player_id);
            player_ids.is_empty()
        } else {
            false
        };

        if should_remove {
            cells.remove(&cell);
        }
    }

    async fn move_player_spatial_cell(
        &self,
        player_id: &PlayerId,
        old_position: &Position,
        new_position: &Position,
    ) {
        let old_cell = super::SpatialCell::from_position(old_position);
        let new_cell = super::SpatialCell::from_position(new_position);
        if old_cell == new_cell {
            return;
        }

        let mut cells = self.player_spatial_cells.write().await;
        let should_remove_old = if let Some(player_ids) = cells.get_mut(&old_cell) {
            player_ids.remove(player_id);
            player_ids.is_empty()
        } else {
            false
        };

        if should_remove_old {
            cells.remove(&old_cell);
        }

        cells.entry(new_cell).or_default().insert(*player_id);
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

        let (monsters_left, monsters_entered) = {
            let monsters = self.monsters.read().await;
            let (left, entered) = aoi_diff(
                monsters.values(),
                |m| (m.position, m.floor_level),
                (old_position, old_floor),
                (&player.position, new_floor),
            );
            (
                left.into_iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
                entered.into_iter().cloned().collect::<Vec<_>>(),
            )
        };

        for monster_id in monsters_left {
            self.send_direct_message(player_id, ServerMessage::MonsterRemoved { monster_id })
                .await;
        }
        for monster in monsters_entered {
            self.send_direct_message(player_id, ServerMessage::MonsterSpawned { monster })
                .await;
        }

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
            self.send_direct_message(player_id, ServerMessage::GroundItemRemoved { instance_id })
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
        let cell_radius = (radius / super::PLAYER_SPATIAL_CELL_SIZE).ceil() as i32;
        let radius_sq = radius * radius;
        let players = self.players.read().await;
        let cells = self.player_spatial_cells.read().await;
        let mut player_ids = HashSet::new();

        // The spatial hash itself stores canonical positions. Near either X
        // edge, query translated copies one circumference away so cells from
        // the opposite edge participate in the same periodic neighborhood.
        for shift_x in [
            -onlinerpg_shared::WORLD_WIDTH_X,
            0.0,
            onlinerpg_shared::WORLD_WIDTH_X,
        ] {
            let shifted = Position {
                x: position.x + shift_x,
                ..*position
            };
            let shifted_center = super::SpatialCell::from_position(&shifted);
            for cell_x in (shifted_center.x - cell_radius)..=(shifted_center.x + cell_radius) {
                for cell_z in (shifted_center.z - cell_radius)..=(shifted_center.z + cell_radius) {
                    let cell = super::SpatialCell {
                        x: cell_x,
                        z: cell_z,
                    };

                    let Some(cell_player_ids) = cells.get(&cell) else {
                        continue;
                    };

                    for player_id in cell_player_ids {
                        let Some(player) = players.get(player_id) else {
                            continue;
                        };

                        if player.floor_level == floor_level
                            && position.dist_xz_sq(&player.position) <= radius_sq
                        {
                            player_ids.insert(*player_id);
                        }
                    }
                }
            }
        }

        player_ids.into_iter().collect()
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
