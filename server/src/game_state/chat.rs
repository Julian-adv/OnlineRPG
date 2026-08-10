use super::auth_db;
use crate::auth::{ban_message, unix_now, AuthService, DEFAULT_BAN_REASON};
use crate::types::{ClientKind, Player, PlayerId, ServerMessage};
use crate::world_config::world_config;
use onlinerpg_shared::messages::strip_command;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

/// Upper bound on one character's `/block` list, so the per-recipient chat
/// filter and the DB rows stay bounded at 5,000 concurrent users.
const MAX_BLOCKS: usize = 100;
/// How close an NPC's new fire may be to one already burning.
const NPC_CAMPFIRE_MIN_GAP: f32 = 6.0;

/// `/mute` duration (minutes): default when unstated, and the cap (one day).
const MUTE_DEFAULT_MINUTES: u64 = 10;
const MUTE_MAX_MINUTES: u64 = 1440;
/// A year of minutes: longer than that, use a permanent ban (no minutes).
const BAN_MAX_MINUTES: i64 = 525_600;

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

/// `<name> [minutes]` tail shared by `/ban` and `/mute`; both leave the
/// duration unvalidated so a bad one draws a usage reply.
fn split_name_and_minutes(rest: &str) -> (&str, Option<&str>) {
    match rest.split_once(' ') {
        Some((name, minutes)) => (name, Some(minutes.trim())),
        None => (rest, None),
    }
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

/// `/r <message>` (or `/reply`) whispers back to the last whisper partner.
pub(crate) fn parse_reply_command(message: &str) -> Option<&str> {
    ["/reply", "/r"]
        .iter()
        .find_map(|prefix| strip_command(message, prefix))
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

/// `/friend add <name>` asks, `/friend remove <name>` drops, bare `/friend`
/// lists. `/f` is the short form, like `/w` for `/whisper`.
#[derive(Debug, PartialEq)]
pub(crate) enum FriendCommand<'a> {
    List,
    Add(&'a str),
    Remove(&'a str),
    /// A subcommand that is none of the above, so it draws usage instead of
    /// being read as a name.
    Usage,
}

pub(crate) fn parse_friend_command(message: &str) -> Option<FriendCommand<'_>> {
    let rest = ["/friend", "/f"]
        .iter()
        .find_map(|prefix| strip_command(message, prefix))?;
    if rest.is_empty() {
        return Some(FriendCommand::List);
    }
    let (subcommand, name) = match rest.split_once(' ') {
        Some((subcommand, name)) => (subcommand, name.trim()),
        None => (rest, ""),
    };
    Some(match subcommand {
        "add" => FriendCommand::Add(name),
        "remove" | "delete" => FriendCommand::Remove(name),
        _ => FriendCommand::Usage,
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

/// `/p <message>` speaks on the party channel. Parsed here and not only in the
/// web client's channel UI, or a client without that UI (agent-client `say`, a
/// third-party one — doc/REMOTE_AGENT_CLIENT.md) has its party line broadcast
/// to everyone nearby.
pub(crate) fn parse_party_chat_command(message: &str) -> Option<&str> {
    strip_command(message, "/p")
}

/// `/s <message>` says the rest as ordinary local chat — the web client's
/// "back to say" prefix, honoured here so other clients can type it too.
pub(crate) fn parse_say_command(message: &str) -> Option<&str> {
    strip_command(message, "/s")
}

/// Operator commands (doc/TODO.md 운영자 커맨드). `requires_admin` gates them
/// through this same parser, so the syntax is defined once. Parts return
/// unvalidated, like the whisper parser: a malformed command draws a usage
/// reply instead of leaking into local chat.
#[derive(Debug, PartialEq)]
pub(crate) enum AdminCommand<'a> {
    Kick(&'a str),
    Ban {
        name: &'a str,
        minutes: Option<&'a str>,
    },
    Unban(&'a str),
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
    // Before /ban: otherwise the shorter prefix claims "/unban <name>".
    if let Some(rest) = strip_command(message, "/unban") {
        return Some(AdminCommand::Unban(rest));
    }
    if let Some(rest) = strip_command(message, "/ban") {
        let (name, minutes) = split_name_and_minutes(rest);
        return Some(AdminCommand::Ban { name, minutes });
    }
    if let Some(rest) = strip_command(message, "/mute") {
        let (name, minutes) = split_name_and_minutes(rest);
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
/// applies the same rule in SQL (`AuthService::resolve_character_brief`).
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

/// The one builder of a mid-performance `PlayerMusicStarted`, so the elapsed
/// clock and message shape cannot drift between the AOI-entry and join paths.
pub(super) fn music_started_msg(performer: PlayerId, entry: &(String, Instant)) -> ServerMessage {
    let (track, started) = entry;
    ServerMessage::PlayerMusicStarted {
        player_id: performer,
        track: track.clone(),
        elapsed_secs: started.elapsed().as_secs_f32(),
    }
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

        if let Some(track) = strip_command(&message, "/play_music") {
            self.play_music(player_id, track).await;
            return;
        }

        if message.trim() == "/light_campfire" {
            self.light_npc_campfire(player_id).await;
            return;
        }

        if message.trim() == "/lay_stall" {
            self.lay_stall(player_id).await;
            return;
        }

        if message.trim() == "/pack_stall" {
            self.pack_stall(player_id).await;
            return;
        }

        if let Some(name) = strip_command(&message, "/emote") {
            self.play_emote(player_id, name).await;
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

        if let Some(command) = parse_friend_command(&message) {
            match command {
                FriendCommand::List => {
                    let list = self.describe_friends(player_id).await;
                    self.send_system_message(player_id, list).await;
                }
                FriendCommand::Add(name) => self.request_friend(player_id, name, auth).await,
                FriendCommand::Remove(name) => {
                    self.remove_friend_by_name(player_id, name, auth).await
                }
                FriendCommand::Usage => {
                    self.send_system_message(
                        player_id,
                        "Friend: /friend add <name>, /friend remove <name>, or /friend to list.",
                    )
                    .await
                }
            }
            return;
        }

        if let Some((target_name, whisper)) = parse_whisper_command(&message) {
            self.send_whisper(player_id, target_name, whisper).await;
            return;
        }

        if let Some(reply) = parse_reply_command(&message) {
            self.send_reply(player_id, reply).await;
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

        if let Some(party_message) = parse_party_chat_command(&message) {
            if party_message.is_empty() {
                self.send_system_message(player_id, "Party chat: /p <message>")
                    .await;
            } else {
                self.send_party_chat(player_id, party_message.to_string())
                    .await;
            }
            return;
        }

        if let Some(said) = parse_say_command(&message) {
            if said.is_empty() {
                self.send_system_message(player_id, "Say: /s <message>")
                    .await;
            } else {
                self.speak_locally(player_id, said.to_string()).await;
            }
            return;
        }

        self.speak_locally(player_id, message).await;
    }

    /// `/play_music [title]` — an emote with no object to claim, so object_id
    /// is None and the occupancy check is skipped: several players can play at
    /// once. The pose is stored on the player so late joiners see it, and
    /// StopInteraction clears it — the performer's client sends one when the
    /// track ends. The title is resolved here, nowhere else, so every client
    /// picks tracks the same way — including one with no audio at all
    /// (agent-client): a bare `/play_music` gets a random tune. Nearby clients
    /// play it along and name it in the chat log, the way they announce a
    /// chest being opened — the performer says nothing. Playing takes an
    /// instrument in the inventory (bards start with a worn mandolin); the
    /// same rule binds NPCs, who carry theirs as a keepsake.
    async fn play_music(&self, player_id: &PlayerId, query: &str) {
        if !self.holds_instrument(player_id).await {
            self.send_system_message(player_id, "You need an instrument to play music.")
                .await;
            return;
        }
        let Some(track) = crate::bgm_defs::bgm_defs().resolve(query) else {
            self.send_system_message(player_id, "No such song.").await;
            return;
        };

        self.set_player_interaction(
            player_id,
            Some(onlinerpg_shared::messages::MUSIC_EMOTE.to_string()),
            None,
        )
        .await;
        self.music_performances
            .write()
            .await
            .insert(*player_id, (track.to_string(), Instant::now()));

        let listeners = self
            .player_ids_within(player_id, super::EVENT_DELIVERY_RADIUS)
            .await;
        self.send_direct_message_to_players(
            &listeners,
            ServerMessage::PlayerMusicStarted {
                player_id: *player_id,
                track: track.to_string(),
                elapsed_secs: 0.0,
            },
        )
        .await;
    }

    /// `/emote <name>` — like `/play_music` with nothing attached: no object
    /// to claim, no item requirement. A typo or a bare `/emote` gets the list
    /// back; the client-side flow is documented on `ONE_SHOT_EMOTES`.
    async fn play_emote(&self, player_id: &PlayerId, name: &str) {
        let emotes = onlinerpg_shared::messages::ONE_SHOT_EMOTES;
        if !emotes.contains(&name) {
            self.send_system_message(player_id, format!("Emotes: {}", emotes.join(", ")))
                .await;
            return;
        }
        self.set_player_interaction(player_id, Some(name.to_string()), None)
            .await;
    }

    /// An instrument anywhere in the inventory — bag or hands — is what
    /// turns the strum emote into a performance.
    async fn holds_instrument(&self, player_id: &PlayerId) -> bool {
        let inventories = self.inventories.read().await;
        inventories.get(player_id).is_some_and(|inv| {
            inv.items().any(|item| {
                self.item_defs
                    .get(&item.item_def_id)
                    .is_some_and(|def| def.is_instrument())
            })
        })
    }

    /// `/light_campfire` — an NPC lights a fire in front of itself with no kit
    /// to spend; players still need one. Why NPCs get it free, and why a fire
    /// lit after dark burns to sunrise, is in doc/HUNGER.md.
    async fn light_npc_campfire(&self, player_id: &PlayerId) {
        let is_npc = {
            let players = self.players.read().await;
            players.get(player_id).is_some_and(|p| p.is_official_npc)
        };
        if !is_npc {
            self.send_system_message(player_id, "Use a campfire kit to light a fire.")
                .await;
            return;
        }
        let Some((placement, floor_level)) = self.campfire_placement(player_id).await else {
            return;
        };
        // One fire per pitch: an NPC that lights another every turn would spawn
        // a particle system and a light for every client in range.
        if self
            .nearby_campfire(&placement, floor_level, NPC_CAMPFIRE_MIN_GAP)
            .await
            .is_some()
        {
            self.send_system_message(player_id, "A fire is already burning here.")
                .await;
            return;
        }
        let datetime = self.current_game_datetime();
        let duration_ms = if Self::is_night(&datetime) {
            self.real_ms_until_sunrise()
        } else {
            onlinerpg_shared::hunger::CAMPFIRE_DURATION_MS
        };
        self.spawn_campfire(placement, floor_level, duration_ms)
            .await;
        self.send_system_message(player_id, "You light a campfire.")
            .await;
    }

    /// Fan a spoken line out to everyone in range, minus anyone who blocked the
    /// speaker. Takes the message already past command parsing, which is what
    /// lets `/s` hand it the remainder without re-dispatching: `/s /give x` is
    /// spoken, never run behind `requires_admin`'s back, and `/s /p x` is
    /// spoken, never rerouted to the party.
    async fn speak_locally(&self, player_id: &PlayerId, message: String) {
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
        {
            let mut partners = self.whisper_partners.write().await;
            partners.insert(*player_id, to.clone());
            // A blocked sender must not become the recipient's `/r` target.
            if !suppressed {
                partners.insert(target_id, from.clone());
            }
        }
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

    /// `/r <message>` — whisper the last partner. The target is remembered by
    /// name, so it survives their relog and fails the same way a stale `/w`
    /// would if they left.
    async fn send_reply(&self, player_id: &PlayerId, message: &str) {
        if message.is_empty() {
            self.send_system_message(player_id, "Reply: /r <message>")
                .await;
            return;
        }
        let partner = self.whisper_partners.read().await.get(player_id).cloned();
        match partner {
            Some(target_name) => self.send_whisper(player_id, &target_name, message).await,
            None => {
                self.send_system_message(player_id, "Reply: no one to reply to yet.")
                    .await
            }
        }
    }

    pub(crate) async fn forget_whisper_partner(&self, player_id: &PlayerId) {
        self.whisper_partners.write().await.remove(player_id);
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

    /// Whether `player_id` has `name` blocked. The name must already be the
    /// canonical spelling — the stored list holds nothing else.
    pub(crate) async fn has_blocked(&self, player_id: &PlayerId, name: &str) -> bool {
        let blocked = self.blocked_names.read().await;
        blocked
            .get(player_id)
            .is_some_and(|names| names.contains(name))
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
        // The id comes along because a block also breaks any friendship.
        let (blocked_character_id, canonical) = {
            let auth = auth.clone();
            let query = name.to_string();
            match auth_db(move || auth.resolve_character_brief(&query)).await {
                Ok(Some((id, canonical))) => (id, canonical),
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

        // Blocking and friendship are mutually exclusive: a friend you cannot
        // hear is a state no other path knows how to explain.
        let unfriended = self
            .is_friend_character(player_id, blocked_character_id)
            .await
            && self
                .drop_friendship(player_id, blocked_character_id, auth)
                .await;
        let note = if unfriended {
            " They are no longer your friend."
        } else {
            ""
        };
        format!("Block: {canonical} is blocked; their chat and whispers are hidden.{note} /unblock {canonical} undoes this.")
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

    /// Muted-sender gate shared by chat, whisper and party chat; true when
    /// the message was refused (the sender got a countdown reply).
    pub(super) async fn refuse_if_muted(
        &self,
        player_id: &PlayerId,
        name: &str,
        verb: &str,
    ) -> bool {
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
            AdminCommand::Ban { name, minutes } => {
                self.ban_command(admin_id, name, minutes, auth).await
            }
            AdminCommand::Unban(name) => self.unban_command(name, auth).await,
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

    /// Ban the account behind a character name, so deleting and recreating
    /// characters does not shed it. Resolved against the DB, not the online
    /// roster: banning someone who just logged off has to work.
    async fn ban_command(
        &self,
        admin_id: &PlayerId,
        name: &str,
        raw_minutes: Option<&str>,
        auth: &AuthService,
    ) -> Result<String, String> {
        if name.is_empty() {
            return Err("Ban: /ban <name> [minutes]".to_string());
        }
        let minutes = match raw_minutes {
            None => None,
            Some(raw) => match raw.parse::<i64>() {
                Ok(m) if (1..=BAN_MAX_MINUTES).contains(&m) => Some(m),
                _ => return Err(format!("Ban: minutes must be 1–{BAN_MAX_MINUTES}.")),
            },
        };

        let (canonical, account) = {
            let auth = auth.clone();
            let query = name.to_string();
            match auth_db(move || auth.account_of_character(&query)).await {
                Ok(Some(found)) => found,
                Ok(None) => return Err(format!("Ban: no character named {name}.")),
                Err(err) => {
                    error!("Ban lookup failed: {err}");
                    return Err("Ban: could not read the character.".to_string());
                }
            }
        };

        // Compare accounts, not character names: an operator's own alt is a
        // different name on the same account, and banning it would lock them
        // out with no way back in through the game.
        if self
            .account_of_player(admin_id)
            .await
            .is_some_and(|admin| admin.eq_ignore_ascii_case(&account))
        {
            return Err("Ban: that's your own account.".to_string());
        }

        let until_unix = minutes.map(|m| unix_now() + m * 60);

        // Held across the write and the eviction; the login path takes it
        // around its own ban check. See `finish_auth`.
        let _sessions = self.lock_character_sessions().await;
        {
            let auth = auth.clone();
            let account = account.clone();
            if let Err(err) =
                auth_db(move || auth.ban_account(&account, Some(DEFAULT_BAN_REASON), until_unix))
                    .await
            {
                error!("Ban write failed: {err}");
                return Err("Ban: could not record the ban.".to_string());
            }
        }
        info!(admin = ?admin_id, target = %canonical, ?minutes, "admin ban");

        let kicked_with = ban_message(Some(DEFAULT_BAN_REASON), until_unix);
        self.evict_account_session_locked(&account, &kicked_with, auth)
            .await;

        Ok(match minutes {
            None => format!("Ban: {canonical} is banned. /unban {canonical} undoes this."),
            Some(m) => format!("Ban: {canonical} is banned for {m}m."),
        })
    }

    async fn unban_command(&self, name: &str, auth: &AuthService) -> Result<String, String> {
        if name.is_empty() {
            return Err("Unban: /unban <name>".to_string());
        }
        // A character name is what an operator has to hand, but a ban outlives
        // its characters — so an unmatched name is tried as the account itself,
        // or a banned account whose last character was deleted could only be
        // recovered by editing the database.
        let auth_for_lookup = auth.clone();
        let query = name.to_string();
        let account = match auth_db(move || auth_for_lookup.account_of_character(&query)).await {
            Ok(Some((_, account))) => account,
            Ok(None) => name.to_string(),
            Err(err) => {
                error!("Unban lookup failed: {err}");
                return Err("Unban: could not read the character.".to_string());
            }
        };
        let auth = auth.clone();
        match auth_db(move || auth.unban_account(&account)).await {
            Ok(true) => Ok(format!("Unban: {name} can log in again.")),
            Ok(false) => Err(format!(
                "Unban: {name} is not banned. Name a character on the account, or the account."
            )),
            Err(err) => {
                error!("Unban write failed: {err}");
                Err("Unban: could not clear the ban.".to_string())
            }
        }
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
