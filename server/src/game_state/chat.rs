use super::auth_db;
use crate::auth::AuthService;
use crate::types::{ClientKind, Player, PlayerId, ServerMessage};
use crate::world_config::world_config;
use tracing::{error, info, warn};

/// Upper bound on one character's `/block` list, so the per-recipient chat
/// filter and the DB rows stay bounded at 5,000 concurrent users.
const MAX_BLOCKS: usize = 100;

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
            Self::now_ms().saturating_sub(player.last_combat_at) < super::OUT_OF_COMBAT_MS
        };
        if in_combat {
            self.send_system_message(player_id, "Escape: not while in combat.")
                .await;
            return;
        }

        // Queued waypoints target the place being escaped from; snapping to one
        // after the teleport would drag the player straight back.
        self.movement_intents.write().await.remove(player_id);

        let spawn = &world_config().spawn_position;
        self.teleport_player(player_id, spawn.position(), spawn.rotation, 0)
            .await;
        info!("Player {} escaped to spawn", player_id);
        self.send_system_message(player_id, "Escape: returned to the starting point.")
            .await;
    }
}
