use super::auth_db;
use crate::auth::AuthService;
use crate::types::{ClientKind, Player, PlayerId, ServerMessage};
use crate::world_config::world_config;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

/// Upper bound on one character's `/block` list, so the per-recipient chat
/// filter and the DB rows stay bounded at 5,000 concurrent users.
const MAX_BLOCKS: usize = 100;

/// `/mute` duration (minutes): default when unstated, and the cap (one day).
const MUTE_DEFAULT_MINUTES: u64 = 10;
const MUTE_MAX_MINUTES: u64 = 1440;

/// `/who` breakdown. Splits by client program rather than by "human vs bot":
/// the server cannot tell whether a person or an LLM is driving a web client,
/// and does not try to (`doc/REMOTE_AGENT_CLIENT.md`). Official NPCs are
/// counted separately because that one the server does know for certain.
#[derive(Default)]
struct OnlineCounts {
    web: u32,
    cli: u32,
    other: u32,
    official_npc: u32,
}

impl OnlineCounts {
    fn tally<'a>(players: impl Iterator<Item = &'a Player>) -> Self {
        let mut counts = Self::default();
        for player in players {
            if player.is_official_npc {
                counts.official_npc += 1;
                continue;
            }
            match player.client_kind {
                ClientKind::Web => counts.web += 1,
                ClientKind::Cli => counts.cli += 1,
                ClientKind::Other | ClientKind::Unknown => counts.other += 1,
            }
        }
        counts
    }

    fn describe(&self) -> String {
        let total = self.web + self.cli + self.other + self.official_npc;
        let mut parts = vec![format!("{} web", self.web), format!("{} cli", self.cli)];
        if self.other > 0 {
            parts.push(format!("{} other", self.other));
        }
        parts.push(format!("{} npc", self.official_npc));
        format!("Online: {total} ({})", parts.join(", "))
    }
}

/// `message` is `prefix` as a whole slash-command word; returns the trimmed
/// remainder.
fn strip_command<'a>(message: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = message.trim().strip_prefix(prefix)?;
    (rest.is_empty() || rest.starts_with(' ')).then(|| rest.trim())
}

/// `/w <name> <message>` (or `/whisper`). Returns the parts unvalidated so a
/// malformed whisper draws a usage reply instead of leaking into local chat.
pub(crate) fn parse_whisper_command(message: &str) -> Option<(&str, &str)> {
    let rest = ["/whisper", "/w"]
        .iter()
        .find_map(|prefix| strip_command(message, prefix))?;
    Some(match rest.split_once(' ') {
        Some((name, message)) => (name, message.trim()),
        None => (rest, ""),
    })
}

/// `/block <name>` mutes a character, `/unblock <name>` undoes it, bare
/// `/block` lists.
#[derive(Debug, PartialEq)]
pub(crate) enum BlockCommand<'a> {
    List,
    Block(&'a str),
    Unblock(&'a str),
}

pub(crate) fn parse_block_command(message: &str) -> Option<BlockCommand<'_>> {
    if let Some(rest) = strip_command(message, "/unblock") {
        return Some(BlockCommand::Unblock(rest));
    }
    let rest = strip_command(message, "/block")?;
    Some(if rest.is_empty() {
        BlockCommand::List
    } else {
        BlockCommand::Block(rest)
    })
}

/// `/notice <message>` sets the server banner, bare `/notice` clears it.
/// `requires_admin` gates the command through this same parser, so the syntax
/// is defined once.
pub(crate) fn parse_notice_command(message: &str) -> Option<Option<&str>> {
    let rest = strip_command(message, "/notice")?;
    Some(Some(rest).filter(|rest| !rest.is_empty()))
}

/// `/party <name>` invites; bare `/party` reports the roster.
pub(crate) fn parse_party_command(message: &str) -> Option<Option<&str>> {
    let rest = strip_command(message, "/party")?;
    Some((!rest.is_empty()).then_some(rest))
}

/// Operator commands (doc/TODO.md 운영자 커맨드). `requires_admin` gates them
/// through this same parser, so the syntax is defined once. Parts return
/// unvalidated, like the whisper parser: a malformed command draws a usage
/// reply instead of leaking into local chat.
#[derive(Debug, PartialEq)]
pub(crate) enum AdminCommand<'a> {
    Kick(&'a str),
    Mute {
        name: &'a str,
        minutes: Option<&'a str>,
    },
    Unmute(&'a str),
    Summon(&'a str),
    Goto(&'a str),
}

pub(crate) fn parse_admin_command(message: &str) -> Option<AdminCommand<'_>> {
    if let Some(rest) = strip_command(message, "/kick") {
        return Some(AdminCommand::Kick(rest));
    }
    if let Some(rest) = strip_command(message, "/unmute") {
        return Some(AdminCommand::Unmute(rest));
    }
    if let Some(rest) = strip_command(message, "/mute") {
        let (name, minutes) = match rest.split_once(' ') {
            Some((name, minutes)) => (name, Some(minutes.trim())),
            None => (rest, None),
        };
        return Some(AdminCommand::Mute { name, minutes });
    }
    if let Some(rest) = strip_command(message, "/summon") {
        return Some(AdminCommand::Summon(rest));
    }
    strip_command(message, "/goto").map(AdminCommand::Goto)
}

/// Case-insensitive lookup of a player-typed name. Names are unique ignoring
/// ASCII case (the characters table enforces it), so the first match is the
/// match. Used by `/unblock` against the stored block list; online-player
/// lookups go through the `player_ids_by_name` index instead, and `/block`
/// applies the same rule in SQL (`AuthService::resolve_character_name`).
fn match_name<T, I>(mut candidates: I, name_of: impl Fn(&T) -> &str, query: &str) -> Option<T>
where
    I: Iterator<Item = T>,
{
    candidates.find(|c| name_of(c).eq_ignore_ascii_case(query))
}

/// Drop expired mutes; callers already hold the write lock.
fn prune_expired_mutes(muted: &mut HashMap<String, (String, Instant)>) {
    let now = Instant::now();
    muted.retain(|_, (_, expiry)| *expiry > now);
}

impl super::GameState {
    pub async fn send_chat_message(
        &self,
        player_id: &PlayerId,
        message: String,
        auth: &AuthService,
    ) {
        if let Some(notice) = parse_notice_command(&message) {
            info!(
                player = ?player_id,
                cleared = notice.is_none(),
                len = notice.map_or(0, str::len),
                "server notice updated"
            );
            self.set_server_notice(notice.map(str::to_string)).await;
            return;
        }

        if let Some(command) = parse_admin_command(&message) {
            self.handle_admin_command(player_id, command, auth).await;
            return;
        }

        if message.trim() == "/escape" {
            self.escape_to_spawn(player_id).await;
            return;
        }

        if message.trim() == "/who" {
            let counts = {
                let players = self.players.read().await;
                OnlineCounts::tally(players.values())
            };
            self.send_system_message(player_id, counts.describe()).await;
            return;
        }

        // Handle /give command
        if let Some(item_id) = message.strip_prefix("/give ") {
            let item_id = item_id.trim();
            if self.give_item(player_id, item_id).await {
                info!(player = ?player_id, item = item_id, "item granted via /give");
                self.send_system_message(player_id, format!("Gave item: {}", item_id))
                    .await;
            } else {
                self.send_system_message(player_id, format!("Unknown item: {}", item_id))
                    .await;
            }
            return;
        }

        if let Some(command) = parse_block_command(&message) {
            self.handle_block_command(player_id, command, auth).await;
            return;
        }

        if let Some((target_name, whisper)) = parse_whisper_command(&message) {
            self.send_whisper(player_id, target_name, whisper).await;
            return;
        }

        if let Some(target) = parse_party_command(&message) {
            match target {
                Some(name) => self.invite_to_party(player_id, name).await,
                None => {
                    let status = self.describe_party(player_id).await;
                    self.send_system_message(player_id, status).await;
                }
            }
            return;
        }

        let player_name = {
            let players = self.players.read().await;
            players.get(player_id).map(|player| player.name.clone())
        };

        if let Some(player_name) = player_name {
            if self.refuse_if_muted(player_id, &player_name, "chat").await {
                return;
            }
            // Chat content stays out of logs on purpose (privacy, F-012).
            info!(from = %player_name, len = message.len(), "chat message");
            let mut recipients = self
                .player_ids_within(player_id, super::EVENT_DELIVERY_RADIUS)
                .await;
            {
                let blocked = self.blocked_names.read().await;
                if !blocked.is_empty() {
                    recipients.retain(|id| {
                        !blocked
                            .get(id)
                            .is_some_and(|names| names.contains(&player_name))
                    });
                }
            }
            self.send_direct_message_to_players(
                &recipients,
                ServerMessage::ChatMessage {
                    player_id: *player_id,
                    message,
                },
            )
            .await;
        } else {
            warn!("Chat message from non-existent player: {}", player_id);
        }
    }

    /// Deliver a whisper to the named player, wherever they are, and echo it
    /// back so the sender's client can render the outgoing line. Errors go
    /// only to the sender.
    async fn send_whisper(&self, player_id: &PlayerId, target_name: &str, message: &str) {
        if target_name.is_empty() || message.is_empty() {
            self.send_system_message(player_id, "Whisper: /w <name> <message>")
                .await;
            return;
        }

        let target_id = self.player_id_by_name(target_name).await;
        let (sender_name, target) = {
            let players = self.players.read().await;
            let sender_name = players.get(player_id).map(|player| player.name.clone());
            let target = target_id
                .and_then(|id| players.get(&id))
                .map(|player| (player.id, player.name.clone()))
                .ok_or_else(|| format!("Whisper: no one called {target_name} is here."));
            (sender_name, target)
        };

        let Some(from) = sender_name else {
            warn!("Whisper from non-existent player: {}", player_id);
            return;
        };
        if self.refuse_if_muted(player_id, &from, "whisper").await {
            return;
        }
        let (target_id, to) = match target {
            Ok(target) => target,
            Err(message) => {
                self.send_system_message(player_id, message).await;
                return;
            }
        };
        if target_id == *player_id {
            self.send_system_message(player_id, "Whisper: that's you.")
                .await;
            return;
        }

        // Content and recipient both stay out of logs (privacy, F-012).
        info!(from = %from, len = message.len(), "whisper");
        let suppressed = {
            let blocked = self.blocked_names.read().await;
            blocked
                .get(&target_id)
                .is_some_and(|names| names.contains(&from))
        };
        let whisper = ServerMessage::WhisperMessage {
            from,
            to,
            message: message.to_string(),
        };
        // A blocked sender still gets the normal echo, so the block is
        // invisible to them.
        if !suppressed {
            self.send_direct_message(&target_id, whisper.clone()).await;
        }
        self.send_direct_message(player_id, whisper).await;
    }

    /// Install a character's persisted `/block` list for this session. Empty
    /// lists are not stored, so the chat filter's `blocked.is_empty()` fast
    /// path stays effective while nobody online has blocks.
    pub async fn set_player_blocks(&self, player_id: &PlayerId, names: Vec<String>) {
        if names.is_empty() {
            return;
        }
        let mut blocked = self.blocked_names.write().await;
        blocked.insert(*player_id, names.into_iter().collect());
    }

    pub(crate) async fn remove_player_blocks(&self, player_id: &PlayerId) {
        self.blocked_names.write().await.remove(player_id);
    }

    async fn handle_block_command(
        &self,
        player_id: &PlayerId,
        command: BlockCommand<'_>,
        auth: &AuthService,
    ) {
        let reply = match command {
            BlockCommand::List => self.describe_blocks(player_id).await,
            BlockCommand::Block(name) => self.block_character(player_id, name, auth).await,
            BlockCommand::Unblock(name) => self.unblock_character(player_id, name, auth).await,
        };
        self.send_system_message(player_id, reply).await;
    }

    async fn describe_blocks(&self, player_id: &PlayerId) -> String {
        let blocked = self.blocked_names.read().await;
        let Some(names) = blocked.get(player_id).filter(|names| !names.is_empty()) else {
            return "Block: no one is blocked. /block <name> mutes a player.".to_string();
        };
        let mut names: Vec<_> = names.iter().cloned().collect();
        names.sort();
        format!("Blocked: {}", names.join(", "))
    }

    async fn block_character(
        &self,
        player_id: &PlayerId,
        name: &str,
        auth: &AuthService,
    ) -> String {
        // Resolve against the DB, not online players: blocking someone who
        // just logged off must work, and every online character is in the DB.
        let canonical = {
            let auth = auth.clone();
            let query = name.to_string();
            match auth_db(move || auth.resolve_character_name(&query)).await {
                Ok(Some(canonical)) => canonical,
                Ok(None) => return format!("Block: no character named {name}."),
                Err(err) => {
                    error!("Block lookup failed: {err}");
                    return "Block: server error, try again.".to_string();
                }
            }
        };
        if self.player_name_of(player_id).await == canonical {
            return "Block: that's you.".to_string();
        }
        let Some(character_id) = self.character_id_of(player_id).await else {
            warn!("/block from player without a character: {player_id}");
            return "Block: server error, try again.".to_string();
        };

        // Checks precede the DB write so a failure needs no rollback; a
        // player's commands are serialized on their connection.
        {
            let blocked = self.blocked_names.read().await;
            if let Some(names) = blocked.get(player_id) {
                if names.contains(&canonical) {
                    return format!("Block: {canonical} is already blocked.");
                }
                if names.len() >= MAX_BLOCKS {
                    return format!("Block: list is full ({MAX_BLOCKS}); /unblock someone first.");
                }
            }
        }
        {
            let auth = auth.clone();
            let blocked_name = canonical.clone();
            if let Err(err) = auth_db(move || auth.add_block(character_id, &blocked_name)).await {
                error!("Failed to save block: {err}");
                return "Block: server error, try again.".to_string();
            }
        }
        self.blocked_names
            .write()
            .await
            .entry(*player_id)
            .or_default()
            .insert(canonical.clone());
        format!("Block: {canonical} is blocked; their chat and whispers are hidden. /unblock {canonical} undoes this.")
    }

    async fn unblock_character(
        &self,
        player_id: &PlayerId,
        name: &str,
        auth: &AuthService,
    ) -> String {
        if name.is_empty() {
            return "Unblock: /unblock <name>".to_string();
        }
        // Match against the stored list itself, so a blocked-then-deleted
        // character can still be unblocked.
        let stored = {
            let blocked = self.blocked_names.read().await;
            let Some(names) = blocked.get(player_id) else {
                return format!("Unblock: {name} is not blocked.");
            };
            match match_name(names.iter(), |stored| stored.as_str(), name) {
                Some(stored) => stored.clone(),
                None => return format!("Unblock: {name} is not blocked."),
            }
        };
        let Some(character_id) = self.character_id_of(player_id).await else {
            warn!("/unblock from player without a character: {player_id}");
            return "Unblock: server error, try again.".to_string();
        };
        {
            let auth = auth.clone();
            let blocked_name = stored.clone();
            if let Err(err) = auth_db(move || auth.remove_block(character_id, &blocked_name)).await
            {
                error!("Failed to remove block: {err}");
                return "Unblock: server error, try again.".to_string();
            }
        }
        let mut blocked = self.blocked_names.write().await;
        if let Some(names) = blocked.get_mut(player_id) {
            names.remove(&stored);
            if names.is_empty() {
                blocked.remove(player_id);
            }
        }
        format!("Unblock: {stored} is no longer blocked.")
    }

    /// Remaining mute minutes for a character name (rounded up, so a mute
    /// never reads "0m" while still active), pruning the entry on expiry.
    async fn muted_minutes_left(&self, name: &str) -> Option<u64> {
        let key = {
            let muted = self.muted_until.read().await;
            if muted.is_empty() {
                return None;
            }
            let key = name.to_ascii_lowercase();
            let (_, until) = muted.get(&key)?;
            if let Some(left) = until.checked_duration_since(Instant::now()) {
                return Some(left.as_secs().div_ceil(60).max(1));
            }
            key
        };
        self.muted_until.write().await.remove(&key);
        None
    }

    /// Muted-sender gate shared by chat and whisper; true when the message
    /// was refused (the sender got a countdown reply).
    async fn refuse_if_muted(&self, player_id: &PlayerId, name: &str, verb: &str) -> bool {
        let Some(minutes) = self.muted_minutes_left(name).await else {
            return false;
        };
        self.send_system_message(
            player_id,
            format!("Muted: you cannot {verb} for another {minutes}m."),
        )
        .await;
        true
    }

    async fn handle_admin_command(
        &self,
        admin_id: &PlayerId,
        command: AdminCommand<'_>,
        auth: &AuthService,
    ) {
        let reply = match command {
            AdminCommand::Kick(name) => self.kick_command(admin_id, name, auth).await,
            AdminCommand::Mute { name, minutes } => {
                self.mute_command(admin_id, name, minutes).await
            }
            AdminCommand::Unmute(name) => self.unmute_command(name).await,
            AdminCommand::Summon(name) => self.summon_command(admin_id, name).await,
            AdminCommand::Goto(name) => self.goto_command(admin_id, name).await,
        }
        .unwrap_or_else(std::convert::identity);
        self.send_system_message(admin_id, reply).await;
    }

    /// Shared preamble for admin commands aimed at an online player: usage on
    /// a bare command, name resolution, self-target refusal.
    async fn admin_target(
        &self,
        admin_id: &PlayerId,
        name: &str,
        label: &str,
        usage: &str,
    ) -> Result<PlayerId, String> {
        if name.is_empty() {
            return Err(format!("{label}: {usage}"));
        }
        let Some(target_id) = self.player_id_by_name(name).await else {
            return Err(format!("{label}: no one called {name} is here."));
        };
        if target_id == *admin_id {
            return Err(format!("{label}: that's you."));
        }
        Ok(target_id)
    }

    async fn kick_command(
        &self,
        admin_id: &PlayerId,
        name: &str,
        auth: &AuthService,
    ) -> Result<String, String> {
        let target_id = self
            .admin_target(admin_id, name, "Kick", "/kick <name>")
            .await?;
        let canonical = self.player_name_of(&target_id).await;
        info!(admin = ?admin_id, target = %canonical, "admin kick");
        self.kick_player(&target_id, "Removed by an operator", auth)
            .await;
        Ok(format!("Kick: {canonical} was disconnected."))
    }

    async fn mute_command(
        &self,
        admin_id: &PlayerId,
        name: &str,
        raw_minutes: Option<&str>,
    ) -> Result<String, String> {
        // Online-only: the roster carries the canonical spelling, and a mute
        // on someone absent has nothing to bite on anyway.
        let target_id = self
            .admin_target(admin_id, name, "Mute", "/mute <name> [minutes]")
            .await?;
        let minutes = match raw_minutes {
            None => MUTE_DEFAULT_MINUTES,
            Some(raw) => match raw.parse() {
                Ok(m) if (1..=MUTE_MAX_MINUTES).contains(&m) => m,
                _ => return Err(format!("Mute: minutes must be 1–{MUTE_MAX_MINUTES}.")),
            },
        };
        let canonical = self.player_name_of(&target_id).await;
        let until = Instant::now() + Duration::from_secs(minutes * 60);
        {
            let mut muted = self.muted_until.write().await;
            prune_expired_mutes(&mut muted);
            muted.insert(canonical.to_ascii_lowercase(), (canonical.clone(), until));
        }
        info!(admin = ?admin_id, target = %canonical, minutes, "admin mute");
        self.send_system_message(&target_id, format!("You are muted for {minutes}m."))
            .await;
        Ok(format!(
            "Mute: {canonical} cannot chat for {minutes}m. /unmute {canonical} undoes this."
        ))
    }

    async fn unmute_command(&self, name: &str) -> Result<String, String> {
        if name.is_empty() {
            return Err("Unmute: /unmute <name>".to_string());
        }
        let mut muted = self.muted_until.write().await;
        prune_expired_mutes(&mut muted);
        Ok(match muted.remove(&name.to_ascii_lowercase()) {
            Some((canonical, _)) => format!("Unmute: {canonical} can chat again."),
            None => format!("Unmute: {name} is not muted."),
        })
    }

    async fn summon_command(&self, admin_id: &PlayerId, name: &str) -> Result<String, String> {
        // No combat or death gate on purpose: moderation must work mid-fight.
        let target_id = self
            .admin_target(admin_id, name, "Summon", "/summon <name>")
            .await?;
        let Some((center, rotation, floor, admin_name)) = self.player_pose(admin_id).await else {
            warn!("/summon from non-existent player: {admin_id}");
            return Err("Summon: server error, try again.".to_string());
        };
        let canonical = self.player_name_of(&target_id).await;
        let arrival = self.arrival_beside(&target_id, &center);
        self.teleport_player(&target_id, arrival, rotation, floor)
            .await;
        info!(admin = ?admin_id, target = %canonical, "admin summon");
        self.send_system_message(
            &target_id,
            format!("An operator summoned you to {admin_name}'s side."),
        )
        .await;
        Ok(format!("Summon: {canonical} is at your side."))
    }

    async fn goto_command(&self, admin_id: &PlayerId, name: &str) -> Result<String, String> {
        let target_id = self
            .admin_target(admin_id, name, "Goto", "/goto <name>")
            .await?;
        let Some((center, rotation, floor, canonical)) = self.player_pose(&target_id).await else {
            return Err(format!("Goto: no one called {name} is here."));
        };
        let arrival = self.arrival_beside(admin_id, &center);
        self.teleport_player(admin_id, arrival, rotation, floor)
            .await;
        info!(admin = ?admin_id, target = %canonical, "admin goto");
        Ok(format!("Goto: you are at {canonical}'s side."))
    }

    /// Last resort for a player wedged somewhere movement can't undo: return
    /// them to the world spawn on the surface.
    ///
    /// Open to everyone by design — the players who need it are precisely the
    /// ones who cannot reach an admin. The combat lockout is what keeps it from
    /// doubling as a free disengage.
    async fn escape_to_spawn(&self, player_id: &PlayerId) {
        let in_combat = {
            let players = self.players.read().await;
            let Some(player) = players.get(player_id) else {
                warn!("/escape from non-existent player: {}", player_id);
                return;
            };
            Self::in_combat(player)
        };
        if in_combat {
            self.send_system_message(player_id, "Escape: not while in combat.")
                .await;
            return;
        }

        let spawn = &world_config().spawn_position;
        self.teleport_player(player_id, spawn.position(), spawn.rotation, 0)
            .await;
        info!("Player {} escaped to spawn", player_id);
        self.send_system_message(player_id, "Escape: returned to the starting point.")
            .await;
    }
}
