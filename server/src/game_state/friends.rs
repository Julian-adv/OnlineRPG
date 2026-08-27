use super::auth_db;
use super::consent::{answer_consent, PendingConsent};
use crate::auth::AuthService;
use crate::types::{PlayerId, ServerMessage};
use onlinerpg_shared::character::CharacterClass;
use onlinerpg_shared::messages::{FriendEntry, OnlineFriend, FRIEND_REQUEST_TTL};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{error, info, warn};

/// Upper bound on one character's friend list, so the per-session snapshot and
/// the poll's name lookups stay bounded at 5,000 concurrent users.
pub(crate) const MAX_FRIENDS: usize = 100;

/// Outstanding requests one player may have pending at once (spam brake),
/// matching the party invites' cap.
const FRIEND_PENDING_REQUEST_CAP: usize = 5;

/// One friend in a session's snapshot.
pub(crate) struct FriendSnapshot {
    character_id: i64,
    name: String,
    /// Lowercased `name`, shared: a poll clones a refcount, not a String, for
    /// up to 100 names on every one of 5,000 players.
    lower_name: Arc<str>,
    /// Level as of this session's login. Only offline friends are shown with
    /// it — an online friend's live level rides `FriendsOnline`.
    level: u32,
    class: CharacterClass,
}

/// All friend state behind one lock. The lists are per-session snapshots of
/// DB rows (the DB stays the source of truth); the requests are in-memory
/// only, and both sides must be online to have one.
#[derive(Default)]
pub(crate) struct Friends {
    lists: HashMap<PlayerId, Vec<FriendSnapshot>>,
    requests: HashMap<(PlayerId, PlayerId), PendingConsent>,
}

impl Friends {
    fn list(&self, player_id: &PlayerId) -> &[FriendSnapshot] {
        self.lists.get(player_id).map_or(&[], Vec::as_slice)
    }

    fn insert(&mut self, player_id: &PlayerId, friend: FriendSnapshot) {
        let list = self.lists.entry(*player_id).or_default();
        if !list.iter().any(|f| f.character_id == friend.character_id) {
            list.push(friend);
        }
    }

    fn remove(&mut self, player_id: &PlayerId, character_id: i64) {
        if let Some(list) = self.lists.get_mut(player_id) {
            list.retain(|f| f.character_id != character_id);
        }
    }

    /// Drop expired entries and count what `requester` still has out.
    fn sweep_and_count(&mut self, requester: &PlayerId) -> usize {
        let now = Instant::now();
        let mut pending = 0;
        self.requests.retain(|(from, _), request| {
            let keep = request.expires_at > now;
            if keep && from == requester {
                pending += 1;
            }
            keep
        });
        pending
    }
}

/// The identity a friendship is recorded against, gathered before any write.
struct FriendParty {
    player_id: PlayerId,
    character_id: i64,
    name: String,
    level: u32,
    class: CharacterClass,
}

impl FriendParty {
    fn snapshot(&self) -> FriendSnapshot {
        FriendSnapshot {
            character_id: self.character_id,
            name: self.name.clone(),
            lower_name: self.name.to_ascii_lowercase().into(),
            level: self.level,
            class: self.class.clone(),
        }
    }
}

impl super::GameState {
    /// Install a character's persisted friends for this session. Empty lists
    /// are not stored, so the poll's fast path stays effective while nobody
    /// online has friends.
    pub async fn set_player_friends(&self, player_id: &PlayerId, friends: Vec<FriendEntry>) {
        if friends.is_empty() {
            return;
        }
        let list = friends
            .into_iter()
            .map(|f| FriendSnapshot {
                character_id: f.character_id,
                lower_name: f.name.to_ascii_lowercase().into(),
                name: f.name,
                level: f.level,
                class: f.class,
            })
            .collect();
        self.friends.write().await.lists.insert(*player_id, list);
    }

    /// Logout hook: drop the snapshot and every request involving the player,
    /// in either direction.
    pub(crate) async fn remove_player_friends(&self, player_id: &PlayerId) {
        let mut friends = self.friends.write().await;
        friends.lists.remove(player_id);
        friends
            .requests
            .retain(|(from, to), _| from != player_id && to != player_id);
    }

    /// The whole roster, offline friends included. Goes out with the login
    /// batch and again after every change.
    pub async fn friend_list_message(&self, player_id: &PlayerId) -> ServerMessage {
        let friends: Vec<FriendEntry> = {
            let state = self.friends.read().await;
            state
                .list(player_id)
                .iter()
                .map(|f| FriendEntry {
                    character_id: f.character_id,
                    name: f.name.clone(),
                    level: f.level,
                    class: f.class.clone(),
                })
                .collect()
        };
        ServerMessage::FriendList { friends }
    }

    pub async fn send_friend_list(&self, player_id: &PlayerId) {
        let message = self.friend_list_message(player_id).await;
        self.send_direct_message(player_id, message).await;
    }

    /// Answer a presence poll. Resolution goes name → online id → level, so
    /// the cost is one hash lookup per friend rather than a scan of the
    /// roster; a friendless player is answered with nothing at all.
    pub async fn send_friends_online(&self, player_id: &PlayerId) {
        let snapshot: Vec<(i64, Arc<str>)> = {
            let state = self.friends.read().await;
            state
                .list(player_id)
                .iter()
                .map(|f| (f.character_id, f.lower_name.clone()))
                .collect()
        };
        if snapshot.is_empty() {
            return;
        }
        // Names are globally unique, so the id under a friend's name is that
        // friend, never a namesake.
        let online: Vec<(i64, PlayerId)> = {
            let names = self.player_ids_by_name.read().await;
            snapshot
                .iter()
                .filter_map(|(character_id, lower)| {
                    names.get(lower.as_ref()).map(|id| (*character_id, *id))
                })
                .collect()
        };
        let friends: Vec<OnlineFriend> = {
            let players = self.players.read().await;
            online
                .into_iter()
                .filter_map(|(character_id, id)| {
                    players.get(&id).map(|p| OnlineFriend {
                        character_id,
                        level: p.level,
                    })
                })
                .collect()
        };
        self.send_direct_message(player_id, ServerMessage::FriendsOnline { friends })
            .await;
    }

    pub async fn describe_friends(&self, player_id: &PlayerId) -> String {
        let entries: Vec<(String, Arc<str>)> = {
            let state = self.friends.read().await;
            state
                .list(player_id)
                .iter()
                .map(|f| (f.name.clone(), f.lower_name.clone()))
                .collect()
        };
        if entries.is_empty() {
            return "Friends: no one yet. /friend add <name> asks someone.".to_string();
        }
        // Online first, then by name — the panel's order, in text.
        let mut listed: Vec<(bool, String)> = {
            let names = self.player_ids_by_name.read().await;
            entries
                .into_iter()
                .map(|(name, lower)| (!names.contains_key(lower.as_ref()), name))
                .collect()
        };
        listed.sort_unstable();
        let parts: Vec<String> = listed
            .into_iter()
            .map(|(offline, name)| {
                if offline {
                    name
                } else {
                    format!("{name} (online)")
                }
            })
            .collect();
        format!("Friends: {}", parts.join(", "))
    }

    /// Ask `target_name` to be friends. Both sides must be online, like a
    /// party invite and for the same reason: the request lives in memory only.
    pub async fn request_friend(
        &self,
        requester_id: &PlayerId,
        target_name: &str,
        auth: &AuthService,
    ) {
        if target_name.chars().count() > super::party::MAX_TARGET_NAME_CHARS {
            self.send_system_message(requester_id, "Friend: that name is too long.")
                .await;
            return;
        }
        if target_name.is_empty() {
            self.send_system_message(requester_id, "Friend: /friend add <name>")
                .await;
            return;
        }
        let target_id = self.player_id_by_name(target_name).await;
        let (requester, target) = {
            let players = self.players.read().await;
            let requester = players
                .get(requester_id)
                .map(|p| (p.name.clone(), p.level, p.class.clone(), p.is_official_npc));
            let target = target_id.and_then(|id| {
                players.get(&id).map(|p| {
                    (
                        id,
                        p.name.clone(),
                        p.level,
                        p.class.clone(),
                        p.is_official_npc,
                    )
                })
            });
            (requester, target)
        };
        let Some((requester_name, requester_level, requester_class, requester_is_npc)) = requester
        else {
            return;
        };
        if requester_is_npc {
            self.send_system_message(requester_id, "Friend: friends are for player travelers.")
                .await;
            return;
        }
        let Some((target_id, target_name, target_level, target_class, target_is_npc)) = target
        else {
            self.send_system_message(
                requester_id,
                format!("Friend: no one called {target_name} is online."),
            )
            .await;
            return;
        };
        if target_id == *requester_id {
            self.send_system_message(requester_id, "Friend: that's you.")
                .await;
            return;
        }
        if target_is_npc {
            self.send_system_message(
                requester_id,
                format!("Friend: {target_name} is an NPC — friends are for player travelers."),
            )
            .await;
            return;
        }
        // Blocking is mutually exclusive with friendship, and this direction
        // is the requester's own doing, so naming it leaks nothing.
        if self.has_blocked(requester_id, &target_name).await {
            self.send_system_message(
                requester_id,
                format!("Friend: you have blocked {target_name}; /unblock {target_name} first."),
            )
            .await;
            return;
        }
        let Some(requester_party) = self
            .friend_party(
                requester_id,
                requester_name.clone(),
                requester_level,
                requester_class,
            )
            .await
        else {
            warn!("/friend from player without a character: {requester_id}");
            self.send_system_message(requester_id, "Friend: server error, try again.")
                .await;
            return;
        };
        let Some(target_party) = self
            .friend_party(&target_id, target_name.clone(), target_level, target_class)
            .await
        else {
            self.send_system_message(requester_id, "Friend: server error, try again.")
                .await;
            return;
        };

        // Read, not act on, before the verdict: a block must change ONLY the
        // final delivery, never the computed outcome, or the differences would
        // let a blocked requester detect the block (the party-invite rule).
        let suppressed = self.has_blocked(&target_id, &requester_name).await;

        enum Outcome {
            Deliver,
            /// Already pending: ack again without re-popping the target's
            /// toast or refreshing the TTL.
            AckOnly,
            /// The target had asked first — two willing sides need no toast.
            Reciprocated,
            Fail(String),
        }
        let outcome = {
            let mut state = self.friends.write().await;
            let pending = state.sweep_and_count(requester_id);
            // Live entries only — the sweep just dropped the expired ones.
            // `Some(true)` is a decline still inside its window.
            let outstanding = state
                .requests
                .get(&(*requester_id, target_id))
                .map(|r| r.answered);
            if state
                .list(requester_id)
                .iter()
                .any(|f| f.character_id == target_party.character_id)
            {
                Outcome::Fail(format!("{target_name} is already your friend."))
            } else if state.list(requester_id).len() >= MAX_FRIENDS {
                Outcome::Fail(format!(
                    "your friend list is full ({MAX_FRIENDS}); /friend remove someone first."
                ))
            } else if state
                .requests
                .get(&(target_id, *requester_id))
                .is_some_and(PendingConsent::awaiting)
            {
                // Their list can have filled since they asked, and the
                // reciprocal path skips the accept path's own cap check.
                if state.list(&target_id).len() >= MAX_FRIENDS {
                    Outcome::Fail(format!("{target_name}'s friend list is full."))
                } else {
                    state.requests.remove(&(target_id, *requester_id));
                    Outcome::Reciprocated
                }
            } else if outstanding == Some(true) {
                // The declined entry is kept as the spam brake, so no toast
                // can be re-popped inside the window — but saying "asked
                // them" would be a lie, and the requester already knows they
                // were declined.
                Outcome::Fail(format!(
                    "{target_name} declined; you can ask again in a couple of minutes."
                ))
            } else if outstanding == Some(false) {
                Outcome::AckOnly
            } else if pending >= FRIEND_PENDING_REQUEST_CAP {
                Outcome::Fail("you have too many pending requests.".to_string())
            } else {
                state.requests.insert(
                    (*requester_id, target_id),
                    PendingConsent::new(FRIEND_REQUEST_TTL),
                );
                Outcome::Deliver
            }
        };
        match outcome {
            Outcome::Fail(reason) => {
                self.send_system_message(requester_id, format!("Friend: {reason}"))
                    .await;
            }
            Outcome::AckOnly => {
                self.send_system_message(
                    requester_id,
                    format!("Friend: asked {target_name} to be friends."),
                )
                .await;
            }
            Outcome::Reciprocated => {
                self.establish_friendship(&requester_party, &target_party, auth)
                    .await;
            }
            Outcome::Deliver => {
                if !suppressed {
                    self.send_direct_message(
                        &target_id,
                        ServerMessage::FriendRequestReceived {
                            requester_id: *requester_id,
                            requester_name,
                        },
                    )
                    .await;
                }
                self.send_system_message(
                    requester_id,
                    format!("Friend: asked {target_name} to be friends."),
                )
                .await;
            }
        }
    }

    pub async fn respond_to_friend_request(
        &self,
        target_id: &PlayerId,
        requester_id: &PlayerId,
        accept: bool,
        auth: &AuthService,
    ) {
        let usable = {
            let mut state = self.friends.write().await;
            answer_consent(&mut state.requests, (*requester_id, *target_id), accept)
        };
        if !usable {
            self.send_system_message(target_id, "Friend: that request has expired.")
                .await;
            return;
        }
        let (requester, target) = {
            let players = self.players.read().await;
            let requester = players
                .get(requester_id)
                .map(|p| (p.name.clone(), p.level, p.class.clone()));
            let target = players
                .get(target_id)
                .map(|p| (p.name.clone(), p.level, p.class.clone()));
            (requester, target)
        };
        let Some((target_name, target_level, target_class)) = target else {
            return;
        };
        let Some((requester_name, requester_level, requester_class)) = requester else {
            self.send_system_message(target_id, "Friend: that request is no longer valid.")
                .await;
            return;
        };
        if !accept {
            self.send_system_message(
                requester_id,
                format!("Friend: {target_name} declined your request."),
            )
            .await;
            return;
        }
        let Some(requester_party) = self
            .friend_party(
                requester_id,
                requester_name,
                requester_level,
                requester_class,
            )
            .await
        else {
            self.send_system_message(target_id, "Friend: that request is no longer valid.")
                .await;
            return;
        };
        let Some(target_party) = self
            .friend_party(target_id, target_name, target_level, target_class)
            .await
        else {
            self.send_system_message(target_id, "Friend: server error, try again.")
                .await;
            return;
        };
        let refusal = {
            let state = self.friends.read().await;
            if state
                .list(target_id)
                .iter()
                .any(|f| f.character_id == requester_party.character_id)
            {
                Some(format!("{} is already your friend.", requester_party.name))
            } else if state.list(target_id).len() >= MAX_FRIENDS {
                Some(format!("your friend list is full ({MAX_FRIENDS})."))
            } else if state.list(requester_id).len() >= MAX_FRIENDS {
                Some(format!("{}'s friend list is full.", requester_party.name))
            } else {
                None
            }
        };
        if let Some(reason) = refusal {
            self.send_system_message(target_id, format!("Friend: {reason}"))
                .await;
            return;
        }
        self.establish_friendship(&requester_party, &target_party, auth)
            .await;
    }

    /// Persist the friendship, update both live snapshots, and tell both
    /// sides. Ordered DB-first: a failed write must leave nothing behind.
    async fn establish_friendship(&self, a: &FriendParty, b: &FriendParty, auth: &AuthService) {
        {
            let auth = auth.clone();
            let (a_char, b_char) = (a.character_id, b.character_id);
            if let Err(err) = auth_db(move || auth.add_friend(a_char, b_char)).await {
                error!("Failed to save friendship: {err}");
                for party in [a, b] {
                    self.send_system_message(&party.player_id, "Friend: server error, try again.")
                        .await;
                }
                return;
            }
        }
        {
            let mut state = self.friends.write().await;
            state.insert(&a.player_id, b.snapshot());
            state.insert(&b.player_id, a.snapshot());
            // Nothing left to answer in either direction.
            state.requests.retain(|(from, to), _| {
                !((*from == a.player_id && *to == b.player_id)
                    || (*from == b.player_id && *to == a.player_id))
            });
        }
        info!(a = %a.name, b = %b.name, "friendship formed");
        for (party, other) in [(a, b), (b, a)] {
            self.send_friend_list(&party.player_id).await;
            self.send_system_message(
                &party.player_id,
                format!("Friend: you and {} are now friends.", other.name),
            )
            .await;
        }
    }

    /// `/friend remove <name>` and the panel's Remove button. Drops both
    /// directions; the other side is not told (the `/block` rule).
    pub async fn remove_friend_by_name(
        &self,
        player_id: &PlayerId,
        name: &str,
        auth: &AuthService,
    ) {
        if name.is_empty() {
            self.send_system_message(player_id, "Friend: /friend remove <name>")
                .await;
            return;
        }
        // Match the stored list itself, so a friend who has since been
        // renamed out of existence can still be dropped.
        let found = {
            let state = self.friends.read().await;
            state
                .list(player_id)
                .iter()
                .find(|f| f.name.eq_ignore_ascii_case(name))
                .map(|f| (f.character_id, f.name.clone()))
        };
        let Some((character_id, canonical)) = found else {
            self.send_system_message(player_id, format!("Friend: {name} is not your friend."))
                .await;
            return;
        };
        if !self.drop_friendship(player_id, character_id, auth).await {
            self.send_system_message(player_id, "Friend: server error, try again.")
                .await;
            return;
        }
        self.send_system_message(
            player_id,
            format!("Friend: {canonical} is no longer your friend."),
        )
        .await;
    }

    /// Break a friendship by the friend's character id, wherever the request
    /// came from (`/friend remove`, or `/block` making the two mutually
    /// exclusive). Updates the friend's live snapshot too when they are
    /// online, so neither side waits for a relog. Returns false only when the
    /// DB write failed.
    pub(crate) async fn drop_friendship(
        &self,
        player_id: &PlayerId,
        friend_character_id: i64,
        auth: &AuthService,
    ) -> bool {
        let Some(character_id) = self.character_id_of(player_id).await else {
            warn!("friend removal from player without a character: {player_id}");
            return false;
        };
        {
            let auth = auth.clone();
            if let Err(err) =
                auth_db(move || auth.remove_friend(character_id, friend_character_id)).await
            {
                error!("Failed to remove friendship: {err}");
                return false;
            }
        }
        let friend_name = {
            let mut state = self.friends.write().await;
            let name = state
                .list(player_id)
                .iter()
                .find(|f| f.character_id == friend_character_id)
                .map(|f| f.lower_name.clone());
            state.remove(player_id, friend_character_id);
            name
        };
        self.send_friend_list(player_id).await;

        // The other side keeps a snapshot of its own; leaving it stale would
        // show a phantom friend until their next login.
        let Some(lower_name) = friend_name else {
            return true;
        };
        let Some(friend_player_id) = self.player_id_by_name(&lower_name).await else {
            return true;
        };
        let still_theirs =
            self.character_id_of(&friend_player_id).await == Some(friend_character_id);
        if !still_theirs {
            return true;
        }
        self.friends
            .write()
            .await
            .remove(&friend_player_id, character_id);
        self.send_friend_list(&friend_player_id).await;
        true
    }

    /// The friend, if any, whose character id matches — the `/block` hook's
    /// question ("is the person I am blocking on my list?").
    pub(crate) async fn is_friend_character(
        &self,
        player_id: &PlayerId,
        character_id: i64,
    ) -> bool {
        self.friends
            .read()
            .await
            .list(player_id)
            .iter()
            .any(|f| f.character_id == character_id)
    }

    /// Everything a friendship row needs about one live player.
    async fn friend_party(
        &self,
        player_id: &PlayerId,
        name: String,
        level: u32,
        class: CharacterClass,
    ) -> Option<FriendParty> {
        Some(FriendParty {
            player_id: *player_id,
            character_id: self.character_id_of(player_id).await?,
            name,
            level,
            class,
        })
    }
}
