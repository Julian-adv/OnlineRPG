use super::*;

/// A party invite the agent hasn't answered yet.
pub struct PendingPartyInvite {
    pub inviter_id: PlayerId,
    pub inviter_name: String,
    pub expires_at: std::time::Instant,
}

/// A summoning-scroll consent request the agent hasn't answered yet.
pub struct PendingPartySummon {
    pub caster_id: PlayerId,
    pub caster_name: String,
    pub expires_at: std::time::Instant,
}

/// A friend request the agent hasn't answered yet.
pub struct PendingFriendRequest {
    pub requester_id: PlayerId,
    pub requester_name: String,
    pub expires_at: std::time::Instant,
}

/// An NPC-pushed trade window the agent hasn't answered yet — the web
/// client's offer toast, which also lapses unanswered.
pub struct PushedTrade {
    pub merchant_id: PlayerId,
    pub merchant_name: String,
    pub expires_at: std::time::Instant,
}

impl PushedTrade {
    /// Whether the offer is still answerable.
    pub fn is_live(&self) -> bool {
        self.expires_at > std::time::Instant::now()
    }
}

impl SharedState {
    /// Returns true if any non-NPC (human) player is in `nearby_players`.
    pub fn has_nearby_human_players(&self) -> bool {
        self.nearby_human_players().next().is_some()
    }

    /// Human players on our floor, within sight, excluding ourselves.
    fn nearby_human_players(&self) -> impl Iterator<Item = (&PlayerId, &Player)> {
        let self_pos = self.self_player.as_ref().map(|p| p.position);
        let self_id = self.self_player_id.as_ref();
        let radius_sq = NPC_SIGHT_RADIUS * NPC_SIGHT_RADIUS;
        self.players_on_my_floor().filter(move |(id, p)| {
            self_id != Some(*id)
                && !p.is_official_npc
                && self_pos.is_some_and(|sp| p.position.dist_xz_sq(&sp) <= radius_sq)
        })
    }

    /// Emit an agent event for any player on our floor that just entered
    /// NEARBY_PLAYER_RADIUS for the first time.
    pub(super) fn check_nearby_player_proximity(&mut self) {
        let self_pos = match self.self_player.as_ref() {
            Some(p) => &p.position,
            None => return,
        };
        let self_id = match self.self_player_id.as_ref() {
            Some(id) => id,
            None => return,
        };

        let arrived: Vec<(PlayerId, String)> = self
            .players_on_my_floor()
            .filter(|(pid, _)| *pid != self_id && !self.seen_nearby_players.contains(pid))
            .filter_map(|(pid, player)| {
                let dist = crate::geom::PlanarDelta::between(&player.position, self_pos).dist;
                (dist <= NEARBY_PLAYER_RADIUS).then(|| {
                    (
                        *pid,
                        format!(
                            "[PlayerNearby] {} Lv.{} appeared {:.1}m away at ({:.1}, {:.1}, {:.1})",
                            player.name,
                            player.level,
                            dist,
                            player.position.x,
                            player.position.y,
                            player.position.z
                        ),
                    )
                })
            })
            .collect();

        for (pid, event) in arrived {
            self.seen_nearby_players.insert(pid);
            self.push_ambient_event_quiet(event);
            // A person arriving, not our own bookkeeping — urgent lane.
            self.wake(EventUrgency::Urgent);
        }
    }

    /// A name the agent can say out loud, never a raw id: someone who acted
    /// and then walked out of sight is just "Someone".
    pub(super) fn visible_name(&self, player_id: &PlayerId) -> String {
        self.nearby_players
            .get(player_id)
            .map_or_else(|| "Someone".to_string(), |p| p.name.clone())
    }

    /// Display name for a player id, falling back to the raw id for someone
    /// out of sight. The one statement of that contract — prompt rendering
    /// and synthetic events both go through here.
    pub fn player_display_name(&self, player_id: &PlayerId) -> String {
        if self.self_player_id.as_ref() == Some(player_id) {
            if let Some(p) = &self.self_player {
                return p.name.clone();
            }
        }
        if let Some(p) = self.nearby_players.get(player_id) {
            return p.name.clone();
        }
        player_id.to_string()
    }

    /// Remember a conversation line for the RECENT CONVERSATION prompt
    /// section, stamped with the game clock so the LLM can judge staleness.
    pub fn push_chat_history(&mut self, line: &str) {
        let stamped = match (self.game_hour, self.game_minute) {
            (Some(h), Some(m)) => format!("[{h:02}:{m:02}] {line}"),
            _ => line.to_string(),
        };
        push_capped(&mut self.chat_history, stamped, MAX_CHAT_HISTORY);
    }

    /// Conversation lines already handled, oldest first.
    pub fn chat_history(&self) -> &VecDeque<String> {
        &self.chat_history
    }

    /// Apply one favor delta from the LLM. Only a nearby human player
    /// counts — unknown names and NPCs are dropped — and both the step
    /// (±1) and the running total are clamped. Returns whether anything
    /// changed, so the caller knows to persist.
    pub fn apply_favor(&mut self, name: &str, delta: i32) -> bool {
        let delta = delta.clamp(-1, 1);
        if delta == 0 {
            return false;
        }
        let Some((id, is_npc)) = self.resolve_nearby_player(name) else {
            return false;
        };
        if is_npc {
            return false;
        }
        let canonical = self.player_display_name(&id);
        let entry = self.favor.entry(canonical).or_insert(0);
        let next = (*entry + delta).clamp(FAVOR_MIN, FAVOR_MAX);
        let changed = next != *entry;
        *entry = next;
        changed
    }

    /// Whether this player waved off our trade window recently, so
    /// open_trade/offer_deal at them are structurally suppressed until the
    /// cooldown runs out.
    pub fn trade_offer_blocked(&self, player_id: &PlayerId) -> bool {
        self.trade_declined_until
            .get(player_id)
            .is_some_and(|until| std::time::Instant::now() < *until)
    }

    /// Nearby human players whose favor has crossed the trade threshold —
    /// the audience the whole Personal Trading section (wishlist pitches
    /// and keepsake offers alike) is written for. A regular inside the
    /// decline cooldown drops out: they just said not now, so not even
    /// the section should court them.
    pub fn trade_worthy_players(&self) -> Vec<String> {
        self.nearby_human_players()
            .filter(|(id, p)| {
                !self.trade_offer_blocked(id)
                    && self.favor.get(&p.name).copied().unwrap_or(0) >= TRADE_FAVOR_THRESHOLD
            })
            .map(|(_, p)| p.name.clone())
            .collect()
    }

    /// Resolve a player name (or raw id) among nearby players, as used by
    /// player-targeting LLM actions. Returns `(player_id, is_official_npc)`.
    /// `name_or_id` stays `&str` because it comes straight from LLM output and
    /// may be either form; the resolved handle is what gets typed.
    /// Only same-floor characters resolve: the world state the LLM saw lists
    /// no one else, and a cross-floor chase would burn its A* budget to reach
    /// a name it never should have been offered.
    pub fn resolve_nearby_player(&self, name_or_id: &str) -> Option<(PlayerId, bool)> {
        self.players_on_my_floor()
            .find(|(id, p)| {
                p.name.eq_ignore_ascii_case(name_or_id)
                    || name_or_id.parse::<u64>().is_ok_and(|n| id.get() == n)
            })
            .map(|(id, p)| (*id, p.is_official_npc))
    }

    /// The nearest NPC merchant on our floor, for trade actions that omit a
    /// merchant name. Usually there is exactly one in range, so guessing is
    /// safe and spares the LLM from naming it.
    pub fn nearest_merchant(&self) -> Option<PlayerId> {
        let self_pos = self.self_player.as_ref().map(|p| p.position)?;
        self.players_on_my_floor()
            .filter(|(_, p)| {
                p.is_official_npc && crate::shop_info::shop_line_for(&p.name).is_some()
            })
            // Only merchants the agent can actually see — the server
            // broadcasts players well beyond that, and "nearest" must not
            // start a long blind walk to one outside the CURRENT STATE list.
            .filter(|(_, p)| {
                p.position.dist_xz_sq(&self_pos) <= NPC_SIGHT_RADIUS * NPC_SIGHT_RADIUS
            })
            .min_by(|(_, a), (_, b)| {
                let da = a.position.dist_xz_sq(&self_pos);
                let db = b.position.dist_xz_sq(&self_pos);
                da.total_cmp(&db)
            })
            .map(|(id, _)| *id)
    }

    /// Build a text summary of current world state for the LLM prompt.
    /// Drop invites past the server-side TTL; call before mutating or
    /// answering the queue.
    pub fn prune_expired_party_invites(&mut self) {
        let now = std::time::Instant::now();
        self.pending_party_invites.retain(|i| i.expires_at > now);
    }

    /// Invites still answerable right now (`format_world_state` is `&self`,
    /// so expired entries are skipped rather than pruned).
    pub fn live_party_invites(&self) -> impl Iterator<Item = &PendingPartyInvite> {
        let now = std::time::Instant::now();
        self.pending_party_invites
            .iter()
            .filter(move |i| i.expires_at > now)
    }

    pub fn prune_expired_party_summons(&mut self) {
        let now = std::time::Instant::now();
        self.pending_party_summons.retain(|s| s.expires_at > now);
    }

    pub fn prune_expired_friend_requests(&mut self) {
        let now = std::time::Instant::now();
        self.pending_friend_requests.retain(|r| r.expires_at > now);
    }

    /// Friend requests still answerable right now, `live_party_invites`'s twin.
    pub fn live_friend_requests(&self) -> impl Iterator<Item = &PendingFriendRequest> {
        let now = std::time::Instant::now();
        self.pending_friend_requests
            .iter()
            .filter(move |r| r.expires_at > now)
    }

    /// Trading with a merchant answers its pushed window, the web client's
    /// "Open" path — no decline goes out, the sale itself tells the NPC.
    pub fn clear_pushed_trade(&mut self, merchant_id: &PlayerId) {
        self.pushed_trade.take_if(|t| t.merchant_id == *merchant_id);
    }

    /// Summons still answerable right now, `live_party_invites`'s twin.
    pub fn live_party_summons(&self) -> impl Iterator<Item = &PendingPartySummon> {
        let now = std::time::Instant::now();
        self.pending_party_summons
            .iter()
            .filter(move |s| s.expires_at > now)
    }
}
