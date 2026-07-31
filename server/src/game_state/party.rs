use crate::types::{PlayerId, ServerMessage};
use onlinerpg_shared::messages::{
    PartyMember, PartyMemberPosition, PARTY_INVITE_TTL, PARTY_SUMMON_TTL,
};
use onlinerpg_shared::world::Position;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tracing::info;

pub(crate) const PARTY_MAX_MEMBERS: usize = 8;

/// Mirrors auth's character-name cap; anything longer cannot be a real name,
/// and rejecting it early keeps oversized input out of the echoed failure.
const MAX_TARGET_NAME_CHARS: usize = 32;

/// Outstanding invites one player may have pending at once (spam brake).
const PARTY_PENDING_INVITE_CAP: usize = 5;

/// Arrival ring radius (m) around a summon's caster; golden-angle spacing
/// keeps simultaneous arrivals apart.
const SUMMON_RING_RADIUS: f32 = 1.6;

pub(crate) struct Party {
    pub leader: PlayerId,
    /// Join order; the leader leaving promotes the earliest remaining member.
    pub members: Vec<PlayerId>,
}

/// All party state behind one lock, so the membership index can never drift
/// from the parties themselves. In-memory only: parties live within one
/// server run, and a disconnect is a leave.
#[derive(Default)]
pub(crate) struct Parties {
    next_id: u64,
    parties: HashMap<u64, Party>,
    member_of: HashMap<PlayerId, u64>,
    /// (inviter, invitee) → pending invite. Swept lazily on the invite paths
    /// and purged with the player, so it stays tiny without its own tick.
    invites: HashMap<(PlayerId, PlayerId), PendingInvite>,
    /// (caster, member) → pending summon, same lifecycle as `invites`. No
    /// pending cap: the consumed scroll is the spam brake.
    summons: HashMap<(PlayerId, PlayerId), PendingSummon>,
}

pub(crate) struct PendingInvite {
    expires_at: Instant,
    /// A declined invite stays here (answered) until it expires: removing it
    /// would hand the spam brake back to the inviter on every decline.
    answered: bool,
}

pub(crate) struct PendingSummon {
    expires_at: Instant,
}

impl Parties {
    fn party_of(&self, player_id: &PlayerId) -> Option<&Party> {
        self.member_of
            .get(player_id)
            .and_then(|id| self.parties.get(id))
    }
}

/// What became of a member's removal, computed under the lock and delivered
/// after it.
enum Removal {
    NotInParty,
    Remaining {
        leader: PlayerId,
        members: Vec<PlayerId>,
    },
    Disbanded {
        last: PlayerId,
    },
}

impl super::GameState {
    /// Invite `target_name` to the sender's party. Like whisper the target is
    /// resolved by name among online players, wherever they are.
    pub async fn invite_to_party(&self, inviter_id: &PlayerId, target_name: &str) {
        if target_name.chars().count() > MAX_TARGET_NAME_CHARS {
            self.party_invite_failed(inviter_id, "", "that name is too long.".to_string())
                .await;
            return;
        }
        let target_id = self.player_id_by_name(target_name).await;
        let (inviter, target) = {
            let players = self.players.read().await;
            let inviter = players
                .get(inviter_id)
                .map(|p| (p.name.clone(), p.is_official_npc));
            let target = target_id
                .and_then(|id| players.get(&id))
                .map(|p| (p.id, p.name.clone(), p.is_official_npc))
                .ok_or_else(|| format!("no one called {target_name} is online."));
            (inviter, target)
        };
        let Some((inviter_name, inviter_is_npc)) = inviter else {
            return;
        };
        // Both directions: official NPCs neither receive nor send invites.
        if inviter_is_npc {
            self.party_invite_failed(
                inviter_id,
                target_name,
                "parties are for player travelers.".to_string(),
            )
            .await;
            return;
        }
        let (target_id, target_name, target_is_npc) = match target {
            Ok(target) => target,
            Err(reason) => {
                self.party_invite_failed(inviter_id, target_name, reason)
                    .await;
                return;
            }
        };
        if target_id == *inviter_id {
            self.party_invite_failed(inviter_id, &target_name, "that's you.".to_string())
                .await;
            return;
        }
        // Official NPCs stay out of parties, like deals and trade windows.
        if target_is_npc {
            self.party_invite_failed(
                inviter_id,
                &target_name,
                format!("{target_name} is an NPC — parties are for player travelers."),
            )
            .await;
            return;
        }

        // Read, not act on, before the verdict: a block must change ONLY the
        // final delivery, never the computed outcome, or the differences
        // would let a blocked sender detect the block.
        let suppressed = {
            let blocked = self.blocked_names.read().await;
            blocked
                .get(&target_id)
                .is_some_and(|names| names.contains(&inviter_name))
        };

        enum Outcome {
            Deliver,
            /// Same invite already pending: ack again, but don't re-deliver
            /// (each re-send would re-pop the target's toast) or refresh the
            /// TTL.
            AckOnly,
            Fail(String),
        }
        // `players` stays locked through the mutation: a disconnect blocks on
        // it and its party sweep runs strictly afterwards, so no invite can
        // be recorded for a player mid-removal.
        let outcome = {
            let players = self.players.read().await;
            let mut parties = self.parties.write().await;
            let now = Instant::now();
            let mut pending = 0;
            parties.invites.retain(|(from, _), invite| {
                let keep = invite.expires_at > now;
                if keep && from == inviter_id {
                    pending += 1;
                }
                keep
            });
            if !players.contains_key(inviter_id) {
                return;
            }
            if !players.contains_key(&target_id) {
                Outcome::Fail(format!("no one called {target_name} is online."))
            } else {
                let already_pending = parties.invites.contains_key(&(*inviter_id, target_id));
                let same_party = parties.member_of.contains_key(inviter_id)
                    && parties.member_of.get(inviter_id) == parties.member_of.get(&target_id);
                let failure = match parties.party_of(inviter_id) {
                    Some(party) if party.leader != *inviter_id => {
                        Some("only the party leader can invite.".to_string())
                    }
                    Some(party) if party.members.len() >= PARTY_MAX_MEMBERS => {
                        Some(format!("the party is full ({PARTY_MAX_MEMBERS} members)."))
                    }
                    // Being in SOME party is deliberately not checked here:
                    // any answer keyed on it would let anyone poll who is
                    // grouped with whom. The invite is delivered and the
                    // accept path sorts it out.
                    _ if same_party => Some(format!("{target_name} is already in your party.")),
                    _ => None,
                };
                let failure = failure.or_else(|| {
                    (!already_pending && pending >= PARTY_PENDING_INVITE_CAP)
                        .then(|| "you have too many pending invites.".to_string())
                });
                match failure {
                    Some(reason) => Outcome::Fail(reason),
                    None if already_pending => Outcome::AckOnly,
                    None => {
                        parties.invites.insert(
                            (*inviter_id, target_id),
                            PendingInvite {
                                expires_at: now + PARTY_INVITE_TTL,
                                answered: false,
                            },
                        );
                        Outcome::Deliver
                    }
                }
            }
        };
        match outcome {
            Outcome::Fail(reason) => {
                self.party_invite_failed(inviter_id, &target_name, reason)
                    .await;
            }
            Outcome::AckOnly => {
                self.send_system_message(inviter_id, format!("Party: invited {target_name}."))
                    .await;
            }
            Outcome::Deliver => {
                if !suppressed {
                    self.send_direct_message(
                        &target_id,
                        ServerMessage::PartyInviteReceived {
                            inviter_id: *inviter_id,
                            inviter_name,
                        },
                    )
                    .await;
                }
                self.send_system_message(inviter_id, format!("Party: invited {target_name}."))
                    .await;
            }
        }
    }

    async fn party_invite_failed(&self, inviter_id: &PlayerId, target_name: &str, reason: String) {
        self.send_direct_message(
            inviter_id,
            ServerMessage::PartyInviteResult {
                target_name: target_name.to_string(),
                accepted: false,
                message: format!("Party: {reason}"),
            },
        )
        .await;
    }

    pub async fn respond_to_party_invite(
        &self,
        invitee_id: &PlayerId,
        inviter_id: &PlayerId,
        accept: bool,
    ) {
        let valid = {
            let mut parties = self.parties.write().await;
            let key = (*inviter_id, *invitee_id);
            let now = Instant::now();
            let usable = parties
                .invites
                .get(&key)
                .is_some_and(|invite| !invite.answered && invite.expires_at > now);
            if usable {
                if accept {
                    parties.invites.remove(&key);
                } else if let Some(invite) = parties.invites.get_mut(&key) {
                    // Keep the declined entry until it expires: the spam
                    // brake must not reset on the victim's own click.
                    invite.answered = true;
                }
            }
            usable
        };
        if !valid {
            self.send_system_message(invitee_id, "Party: that invite has expired.")
                .await;
            return;
        }
        let invitee_name = self.player_name_of(invitee_id).await;
        if !accept {
            self.send_direct_message(
                inviter_id,
                ServerMessage::PartyInviteResult {
                    target_name: invitee_name.clone(),
                    accepted: false,
                    message: format!("Party: {invitee_name} declined."),
                },
            )
            .await;
            return;
        }

        // `players` stays locked through the mutation (see invite_to_party):
        // if either side disconnects mid-accept, their removal sweep runs
        // after this insert and cleans it up, instead of leaving a ghost
        // member no removal will ever find.
        let players = self.players.read().await;
        if !players.contains_key(invitee_id) {
            return;
        }
        if !players.contains_key(inviter_id) {
            drop(players);
            self.send_system_message(invitee_id, "Party: that invite is no longer valid.")
                .await;
            return;
        }
        let result = {
            let mut parties = self.parties.write().await;
            if parties.member_of.contains_key(invitee_id) {
                Err((
                    "you are already in a party.".to_string(),
                    format!("{invitee_name} can't accept a party invite right now."),
                ))
            } else if let Some(party_id) = parties.member_of.get(inviter_id).copied() {
                let party = parties.parties.get_mut(&party_id).expect("indexed party");
                if party.leader != *inviter_id {
                    Err((
                        "that invite is no longer valid.".to_string(),
                        format!("the invite to {invitee_name} is no longer valid."),
                    ))
                } else if party.members.len() >= PARTY_MAX_MEMBERS {
                    Err((
                        "that party is full.".to_string(),
                        "the party is full.".to_string(),
                    ))
                } else {
                    party.members.push(*invitee_id);
                    let roster = (party.leader, party.members.clone());
                    parties.member_of.insert(*invitee_id, party_id);
                    Ok(roster)
                }
            } else {
                // First accept creates the party.
                let party_id = parties.next_id;
                parties.next_id += 1;
                parties.parties.insert(
                    party_id,
                    Party {
                        leader: *inviter_id,
                        members: vec![*inviter_id, *invitee_id],
                    },
                );
                parties.member_of.insert(*inviter_id, party_id);
                parties.member_of.insert(*invitee_id, party_id);
                Ok((*inviter_id, vec![*inviter_id, *invitee_id]))
            }
        };
        // Before the sends: broadcast_party_state re-reads `players`, and a
        // second same-task read can deadlock behind a queued writer.
        drop(players);
        match result {
            Err((invitee_msg, inviter_msg)) => {
                self.send_system_message(invitee_id, format!("Party: {invitee_msg}"))
                    .await;
                self.send_direct_message(
                    inviter_id,
                    ServerMessage::PartyInviteResult {
                        target_name: invitee_name,
                        accepted: false,
                        message: format!("Party: {inviter_msg}"),
                    },
                )
                .await;
            }
            Ok((leader, members)) => {
                info!(
                    invitee = %invitee_name,
                    size = members.len(),
                    "party join"
                );
                self.send_direct_message(
                    inviter_id,
                    ServerMessage::PartyInviteResult {
                        target_name: invitee_name.clone(),
                        accepted: true,
                        message: format!("Party: {invitee_name} joined."),
                    },
                )
                .await;
                self.broadcast_party_state(leader, &members).await;
            }
        }
    }

    /// Other party members who are still online.
    pub(crate) async fn other_party_members(&self, player_id: &PlayerId) -> Vec<PlayerId> {
        let ids = {
            let parties = self.parties.read().await;
            parties
                .party_of(player_id)
                .map(|party| party.members.clone())
                .unwrap_or_default()
        };
        let players = self.players.read().await;
        ids.into_iter()
            .filter(|id| id != player_id && players.contains_key(id))
            .collect()
    }

    /// The summonable set: online members without a live pending summons
    /// from this caster. Re-reading while a call is out must neither refresh
    /// nor re-pop it — the invites' ack-only rule — so those members are
    /// excluded rather than overwritten.
    pub(crate) async fn summonable_party_members(&self, caster_id: &PlayerId) -> Vec<PlayerId> {
        let members = self.other_party_members(caster_id).await;
        let mut parties = self.parties.write().await;
        let now = Instant::now();
        parties.summons.retain(|_, summon| summon.expires_at > now);
        members
            .into_iter()
            .filter(|member| !parties.summons.contains_key(&(*caster_id, *member)))
            .collect()
    }

    /// Fan a consumed summoning scroll out as consent requests. Like invites,
    /// a block changes only the delivery: a suppressed member still gets an
    /// entry, they just never see the toast.
    pub(crate) async fn cast_party_summon(&self, caster_id: &PlayerId, members: Vec<PlayerId>) {
        let caster_name = self.player_name_of(caster_id).await;
        let suppressed: HashSet<PlayerId> = {
            let blocked = self.blocked_names.read().await;
            members
                .iter()
                .filter(|id| {
                    blocked
                        .get(id)
                        .is_some_and(|names| names.contains(&caster_name))
                })
                .copied()
                .collect()
        };
        {
            let mut parties = self.parties.write().await;
            let now = Instant::now();
            parties.summons.retain(|_, summon| summon.expires_at > now);
            for member in &members {
                parties.summons.insert(
                    (*caster_id, *member),
                    PendingSummon {
                        expires_at: now + PARTY_SUMMON_TTL,
                    },
                );
            }
        }
        for member in members.iter().filter(|m| !suppressed.contains(m)) {
            self.send_direct_message(
                member,
                ServerMessage::PartySummonReceived {
                    caster_id: *caster_id,
                    caster_name: caster_name.clone(),
                },
            )
            .await;
        }
        self.send_system_message(
            caster_id,
            format!("Summon: calling {} party member(s).", members.len()),
        )
        .await;
    }

    pub async fn respond_to_party_summon(
        &self,
        member_id: &PlayerId,
        caster_id: &PlayerId,
        accept: bool,
    ) {
        let key = (*caster_id, *member_id);
        let usable = {
            let mut parties = self.parties.write().await;
            let now = Instant::now();
            parties.summons.retain(|_, summon| summon.expires_at > now);
            parties.summons.contains_key(&key)
        };
        if !usable {
            self.send_system_message(member_id, "Summon: that summons has expired.")
                .await;
            return;
        }
        let member_name = self.player_name_of(member_id).await;
        if !accept {
            self.parties.write().await.summons.remove(&key);
            self.send_system_message(caster_id, format!("Summon: {member_name} declined."))
                .await;
            return;
        }
        // Same clock as /escape: accepting must not double as a free
        // disengage. The entry survives the refusal for a retry in the window.
        let refusal = {
            let players = self.players.read().await;
            let Some(member) = players.get(member_id) else {
                return;
            };
            if member.health == 0 {
                Some("not while defeated.")
            } else if Self::now_ms().saturating_sub(member.last_combat_at) < super::OUT_OF_COMBAT_MS
            {
                Some("not while in combat.")
            } else {
                None
            }
        };
        if let Some(reason) = refusal {
            self.send_system_message(member_id, format!("Summon: {reason}"))
                .await;
            return;
        }
        // The caster must still be online and share the member's party.
        let destination = {
            let players = self.players.read().await;
            let parties = self.parties.read().await;
            let same_party = parties.member_of.contains_key(caster_id)
                && parties.member_of.get(caster_id) == parties.member_of.get(member_id);
            players.get(caster_id).filter(|_| same_party).map(|caster| {
                (
                    caster.position,
                    caster.rotation,
                    caster.floor_level,
                    caster.name.clone(),
                    caster.health,
                    caster.last_combat_at,
                )
            })
        };
        let Some((center, rotation, floor, caster_name, caster_health, caster_combat_at)) =
            destination
        else {
            self.parties.write().await.summons.remove(&key);
            self.send_system_message(member_id, "Summon: that summons has faded.")
                .await;
            return;
        };
        // The read-time caster gate, re-checked at delivery: the 30s window
        // must not hand out fights the 10s clock just refused. Same retry
        // semantics as the member-side gate — the entry stays.
        let caster_refusal = if caster_health == 0 {
            Some(format!("Summon: {caster_name} has fallen."))
        } else if Self::now_ms().saturating_sub(caster_combat_at) < super::OUT_OF_COMBAT_MS {
            Some(format!("Summon: {caster_name} is in combat."))
        } else {
            None
        };
        if let Some(message) = caster_refusal {
            self.send_system_message(member_id, message).await;
            return;
        }
        self.parties.write().await.summons.remove(&key);
        // Golden-angle ring: a per-member arrival spot without a placement
        // scan, so simultaneous accepts don't stack. A blocked spot (dungeon
        // walls run 1m from a corridor's center) retries at half radius and
        // finally lands on the caster's own — walkable — cell.
        let angle = (member_id.get() % 360) as f32 * 2.399_963;
        let arrival = {
            let cache = self.passability_read();
            let cell_floor = super::passability::authoritative_floor(&cache, &center);
            let mut arrival = center;
            for radius in [SUMMON_RING_RADIUS, SUMMON_RING_RADIUS * 0.5] {
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
                    arrival = candidate;
                    break;
                }
            }
            arrival
        };
        info!(member = %member_name, caster = %caster_name, "party summon accepted");
        self.teleport_player(member_id, arrival, rotation, floor)
            .await;
        self.send_system_message(
            member_id,
            format!("Summon: you answer {caster_name}'s call."),
        )
        .await;
        self.send_system_message(caster_id, format!("Summon: {member_name} is at your side."))
            .await;
    }

    pub async fn leave_party(&self, player_id: &PlayerId) {
        if self.remove_party_member(player_id).await {
            self.send_system_message(player_id, "Party: you left the party.")
                .await;
        } else {
            self.send_system_message(player_id, "Party: you are not in a party.")
                .await;
        }
    }

    /// Disconnect hook: drop the player's party membership and every invite
    /// involving them, in either direction.
    pub(crate) async fn clear_party_for_player(&self, player_id: &PlayerId) {
        {
            let mut parties = self.parties.write().await;
            parties
                .invites
                .retain(|(from, to), _| from != player_id && to != player_id);
            parties
                .summons
                .retain(|(from, to), _| from != player_id && to != player_id);
        }
        self.remove_party_member(player_id).await;
    }

    /// Remove a player from its party — promoting the earliest remaining
    /// member if it led, disbanding when one member would remain. Pending
    /// invites are untouched (leaving a party shouldn't void one you
    /// received); returns false when the player was in no party.
    async fn remove_party_member(&self, player_id: &PlayerId) -> bool {
        let removal = {
            let mut parties = self.parties.write().await;
            match parties.member_of.remove(player_id) {
                None => Removal::NotInParty,
                Some(party_id) => {
                    let party = parties.parties.get_mut(&party_id).expect("indexed party");
                    party.members.retain(|m| m != player_id);
                    if party.members.len() < 2 {
                        let last = party.members.first().copied();
                        parties.parties.remove(&party_id);
                        match last {
                            Some(last) => {
                                parties.member_of.remove(&last);
                                Removal::Disbanded { last }
                            }
                            None => Removal::NotInParty,
                        }
                    } else {
                        if party.leader == *player_id {
                            party.leader = party.members[0];
                        }
                        Removal::Remaining {
                            leader: party.leader,
                            members: party.members.clone(),
                        }
                    }
                }
            }
        };
        match removal {
            Removal::NotInParty => false,
            Removal::Remaining { leader, members } => {
                self.send_party_cleared(player_id).await;
                self.broadcast_party_state(leader, &members).await;
                true
            }
            Removal::Disbanded { last } => {
                self.send_party_cleared(player_id).await;
                self.send_party_cleared(&last).await;
                self.send_system_message(&last, "Party: disbanded.").await;
                true
            }
        }
    }

    pub async fn describe_party(&self, player_id: &PlayerId) -> String {
        let (leader, ids) = {
            let parties = self.parties.read().await;
            match parties.party_of(player_id) {
                Some(party) => (party.leader, party.members.clone()),
                None => return "Party: you are not in a party. /party <name> invites.".to_string(),
            }
        };
        let players = self.players.read().await;
        let names: Vec<String> = ids
            .iter()
            .map(|id| {
                let name = players.get(id).map_or("?", |p| p.name.as_str());
                if *id == leader {
                    format!("{name} (leader)")
                } else {
                    name.to_string()
                }
            })
            .collect();
        format!("Party: {}", names.join(", "))
    }

    /// Answer a positions poll: where the sender's other members are.
    /// Rate-limited per connection before this is called.
    pub async fn send_party_positions(&self, player_id: &PlayerId) {
        let member_ids = {
            let parties = self.parties.read().await;
            parties
                .party_of(player_id)
                .map(|party| party.members.clone())
                .unwrap_or_default()
        };
        let members: Vec<PartyMemberPosition> = {
            let players = self.players.read().await;
            member_ids
                .iter()
                .filter(|id| *id != player_id)
                .filter_map(|id| {
                    players.get(id).map(|p| PartyMemberPosition {
                        id: *id,
                        x: p.position.x,
                        z: p.position.z,
                        floor_level: p.floor_level,
                    })
                })
                .collect()
        };
        self.send_direct_message(player_id, ServerMessage::PartyPositions { members })
            .await;
    }

    async fn broadcast_party_state(&self, leader_id: PlayerId, member_ids: &[PlayerId]) {
        let members: Vec<PartyMember> = {
            let players = self.players.read().await;
            member_ids
                .iter()
                .filter_map(|id| {
                    players.get(id).map(|p| PartyMember {
                        id: *id,
                        name: p.name.clone(),
                    })
                })
                .collect()
        };
        let msg = ServerMessage::PartyState { leader_id, members };
        for id in member_ids {
            self.send_direct_message(id, msg.clone()).await;
        }
    }

    async fn send_party_cleared(&self, player_id: &PlayerId) {
        self.send_direct_message(
            player_id,
            ServerMessage::PartyState {
                leader_id: PlayerId::from(0),
                members: Vec::new(),
            },
        )
        .await;
    }
}
