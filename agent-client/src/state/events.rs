use super::*;

/// How urgently an event needs LLM attention. Ordered most urgent first, so
/// `min` picks the one that decides a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventUrgency {
    /// Must be processed immediately (combat damage to self, death, direct chat, kicked)
    Urgent,
    /// Can wait and be batched with next prompt (world state changes, xp, spawns)
    Routine,
    /// Don't send to LLM at all (high-frequency movement, time sync)
    Noise,
}

use onlinerpg_shared::fishing::{auto_stance, FishingAction, HOOK_REACTION_MS, STANCE_REACTION_MS};
use std::ops::RangeInclusive;
use std::time::Duration;

impl SharedState {
    /// Classify how urgent a server event is for LLM processing.
    pub fn classify_event(&self, msg: &ServerMessage) -> EventUrgency {
        let self_id = self.self_player_id.as_ref();
        match msg {
            // Urgent: we are being attacked or we died
            ServerMessage::MonsterAttackedPlayer { player_id, .. } => {
                if self_id == Some(player_id) {
                    EventUrgency::Urgent
                } else {
                    EventUrgency::Routine
                }
            }
            ServerMessage::PlayerDead { player_id } => {
                if self_id == Some(player_id) {
                    EventUrgency::Urgent
                } else {
                    EventUrgency::Routine
                }
            }
            // Urgent: a human chats (not ourselves). NPC→NPC chat is only
            // Routine: urgent wakeups on both sides turn any shared topic
            // into an endless conversation loop (and an LLM-cost leak), so
            // NPC replies wait for the next batched prompt instead.
            ServerMessage::ChatMessage { player_id, .. } => {
                if self_id == Some(player_id) {
                    EventUrgency::Noise
                } else if self
                    .nearby_players
                    .get(player_id)
                    .is_some_and(|p| p.is_official_npc)
                {
                    EventUrgency::Routine
                } else {
                    EventUrgency::Urgent
                }
            }
            // Urgent: a whisper is always addressed to us; the echo of our
            // own outgoing whisper is the Noise case.
            ServerMessage::WhisperMessage { from, .. } => {
                let self_name = self.self_player.as_ref().map(|p| p.name.as_str());
                if Some(from.as_str()) == self_name {
                    EventUrgency::Noise
                } else {
                    EventUrgency::Urgent
                }
            }
            // Party chat is addressed to our group, so it wakes us like a
            // whisper; the own-echo Noise rule is the same.
            ServerMessage::PartyChatMessage { from, .. } => {
                let self_name = self.self_player.as_ref().map(|p| p.name.as_str());
                if Some(from.as_str()) == self_name {
                    EventUrgency::Noise
                } else {
                    EventUrgency::Urgent
                }
            }
            // Routine: feedback on our own command (/who output, whisper
            // errors) — worth seeing, not worth an immediate wakeup.
            ServerMessage::SystemMessage { .. } => EventUrgency::Routine,
            // Urgent: an invite to answer while it is live, or the verdict
            // on our own invite.
            ServerMessage::PartyInviteReceived { .. }
            | ServerMessage::PartyInviteResult { .. }
            | ServerMessage::PartySummonReceived { .. } => EventUrgency::Urgent,
            // Urgent: a friend request to answer while it is live, and the
            // answer to our own friends_online ask.
            ServerMessage::FriendRequestReceived { .. } | ServerMessage::FriendsOnline { .. } => {
                EventUrgency::Urgent
            }
            // Urgent: someone opened their trade window on us — a person is
            // standing there waiting for an answer.
            ServerMessage::ShopState { .. } => EventUrgency::Urgent,
            ServerMessage::PartyState { .. } => EventUrgency::Routine,
            // Urgent: kicked
            ServerMessage::Kicked { .. } => EventUrgency::Urgent,

            // Urgent: verdict on our haggling offer — the NPC should follow
            // up in the ongoing conversation (e.g. correct a clamped price).
            ServerMessage::DealResult { .. } => EventUrgency::Urgent,

            // Urgent: a player traded with us, or our trade request failed —
            // both deserve an in-character reaction.
            ServerMessage::TradeNotice { .. } | ServerMessage::TradeError { .. } => {
                EventUrgency::Urgent
            }

            // State-only: tracked on SharedState, shown in the world state.
            ServerMessage::GoldUpdate { .. }
            | ServerMessage::GoldGained { .. }
            | ServerMessage::InventoryState { .. }
            | ServerMessage::InventoryUpdated { .. }
            | ServerMessage::GroundItemSpawned { .. }
            | ServerMessage::GroundItemAppeared { .. }
            | ServerMessage::GroundItemRemoved { .. }
            | ServerMessage::GroundItemQuantityChanged { .. }
            | ServerMessage::TradeBusy { .. } => EventUrgency::Noise,

            // Urgent: another player attacks a monster (so we can join in)
            ServerMessage::PlayerAttacked { player_id, .. } => {
                if self_id != Some(player_id) {
                    EventUrgency::Urgent
                } else {
                    EventUrgency::Routine
                }
            }
            ServerMessage::MonsterProvoked { .. } => EventUrgency::Routine,

            // Routine: world state changes
            ServerMessage::JoinSuccess { .. }
            | ServerMessage::GameState { .. }
            | ServerMessage::PlayerJoined { .. }
            | ServerMessage::PlayerLeft { .. }
            | ServerMessage::PlayerAppeared { .. }
            | ServerMessage::PlayerDisappeared { .. }
            | ServerMessage::MonsterSpawned { .. }
            | ServerMessage::MonsterAssigned { .. }
            | ServerMessage::SpawnMonsterRequest { .. }
            | ServerMessage::MonsterDead { .. }
            | ServerMessage::MonsterRemoved { .. }
            | ServerMessage::XpGained { .. }
            | ServerMessage::PlayerHealthUpdate { .. }
            | ServerMessage::PlayerTorchToggled { .. }
            | ServerMessage::PlayerMainHandChanged { .. }
            | ServerMessage::PlayerBackChanged { .. } => EventUrgency::Routine,

            // Being relocated invalidates our walk targets and floor
            // assumptions; someone else being relocated does not.
            ServerMessage::PlayerTeleported { player_id, .. } => {
                if self_id == Some(player_id) {
                    EventUrgency::Urgent
                } else {
                    EventUrgency::Noise
                }
            }
            ServerMessage::PlayerRespawned { player } => {
                if self_id == Some(&player.id) {
                    EventUrgency::Urgent
                } else {
                    EventUrgency::Routine
                }
            }

            // Fishing: only our own outcome is worth an LLM look — recast, eat
            // the catch, or give up. In-flight events are reflex-handled, and
            // another player's ending renders no prompt line (driver/prompt.rs),
            // so both are noise.
            ServerMessage::FishingEnded { player_id, .. } => {
                if self_id == Some(player_id) {
                    EventUrgency::Urgent
                } else {
                    EventUrgency::Noise
                }
            }
            ServerMessage::FishingError { .. } => EventUrgency::Urgent,
            ServerMessage::FishingCasted { .. }
            | ServerMessage::FishingBite { .. }
            | ServerMessage::FishingFight { .. } => EventUrgency::Noise,

            // Noise: high-frequency, irrelevant, or housing updates
            ServerMessage::PlayerMoved { .. }
            | ServerMessage::MonsterMoved { .. }
            | ServerMessage::PartyPositions { .. }
            | ServerMessage::GameTimeSync { .. }
            | ServerMessage::HouseSpawned { .. }
            | ServerMessage::HousesInArea { .. }
            | ServerMessage::HouseUpdated { .. }
            | ServerMessage::HouseRemoved { .. }
            | ServerMessage::DoorToggled { .. } => EventUrgency::Noise,

            // A refused interaction should reach the LLM at poll priority, not
            // sink to the idle queue behind everything else.
            ServerMessage::InteractionRejected { .. }
            | ServerMessage::PlayerAttackRejected { .. } => EventUrgency::Routine,

            // Campfire churn and the grill-cast start are world-state, not
            // events; the outcome (`GrillEnded`) rides the Routine catch-all.
            ServerMessage::CampfireSpawned { .. }
            | ServerMessage::CampfireAppeared { .. }
            | ServerMessage::CampfireRemoved { .. }
            | ServerMessage::StallPlaced { .. }
            | ServerMessage::StallAppeared { .. }
            | ServerMessage::StallRemoved { .. }
            | ServerMessage::GrillStarted
            // NPCs are refused player-to-player trades server-side, so these
            // should never arrive; classified rather than left to the default.
            | ServerMessage::PlayerTradeRequested { .. }
            | ServerMessage::PlayerTradeRequestResult { .. }
            | ServerMessage::PlayerTradeUpdate { .. }
            | ServerMessage::PlayerTradeEnded { .. }
            | ServerMessage::PlayerTradeError { .. } => EventUrgency::Noise,

            // Auth/character events: routine (handled before game entry)
            _ => EventUrgency::Routine,
        }
    }

    fn handle_managed_monster_hit(
        &mut self,
        monster_id: &str,
        player_id: &PlayerId,
        hit: bool,
        damage: u32,
    ) {
        if !self.monster_ai.manages(monster_id) {
            return;
        }

        let world = self.world_cache.read().unwrap();
        let commands = self.monster_ai.handle_monster_hit(
            monster_id,
            player_id,
            hit,
            damage,
            world.passability_cache(),
        );
        drop(world);
        self.pending_commands.extend(commands);
    }

    /// Answer a fishing beat the way a person would: after a reaction delay,
    /// and only one answer in flight. A beat that lands mid-reaction is
    /// missed; the next beat corrects it. Returns whether it was taken.
    /// Sends on `cmd_tx` rather than `send_command` — a spawned task cannot
    /// hold `&mut self`, and `FishingRespond` needs no send-time rewriting.
    fn react_fishing(&mut self, action: FishingAction, delay_ms: RangeInclusive<u64>) -> bool {
        if self
            .fishing_reaction
            .as_ref()
            .is_some_and(|h| !h.is_finished())
        {
            return false;
        }
        let delay = Duration::from_millis(rand::thread_rng().gen_range(delay_ms));
        let tx = self.cmd_tx.clone();
        self.fishing_reaction = Some(tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = tx.send(ClientMessage::FishingRespond { action }).await;
        }));
        true
    }

    /// Start or end our own session: any in-flight answer is dropped, since a
    /// stale one landing in the next cast scares the fish off.
    fn set_self_fishing(&mut self, fishing: bool) {
        self.self_fishing = fishing;
        self.fishing_stance = None;
        if let Some(h) = self.fishing_reaction.take() {
            h.abort();
        }
    }

    /// Push an event and update tracked state. Returns the urgency of the event.
    pub fn push_event(&mut self, msg: ServerMessage) -> EventUrgency {
        // Feed the spectator panel before mutating, while names still resolve
        if let Some(watch) = self.watch.clone() {
            if let Some(kind) = crate::watch::feed_kind(&msg) {
                let line = crate::watch::feed_fallback(&msg)
                    .or_else(|| crate::driver::format_event(self, &msg));
                if let Some(line) = line {
                    watch.push(kind, line);
                }
            }
        }

        // Update tracked state from certain messages
        match &msg {
            ServerMessage::JoinSuccess { player, .. } => {
                self.in_game = true;
                self.self_player_id = Some(player.id);
                self.self_player = Some(player.clone());
                self.self_fishing = false;
                // A character saved underground rejoins there (the server
                // rehydrates it), so adopt the floor instead of assuming 0.
                self.adopt_floor_level(player.floor_level);
                self.request_dungeon_doors_here();
            }
            ServerMessage::PositionCorrected {
                position,
                rotation,
                floor_level,
            } => {
                self.relocate_self(*position, *rotation, *floor_level);
            }
            ServerMessage::PlayerTeleported {
                player_id,
                position,
                rotation,
                floor_level,
            } => {
                if self.self_player_id.as_ref() == Some(player_id) {
                    self.relocate_self(*position, *rotation, *floor_level);
                    // Any teleport settles the pending summons.
                    self.pending_party_summons.clear();
                }
                self.apply_player_pose(player_id, *position, *rotation, *floor_level);
            }
            ServerMessage::PlayerRespawned { player } => {
                if self.self_player_id.as_ref() == Some(&player.id) {
                    self.self_player = Some(player.clone());
                    self.relocate_self(player.position, player.rotation, player.floor_level);
                }
                if let Some(p) = self.nearby_players.get_mut(&player.id) {
                    *p = player.clone();
                }
                self.latest_player_moves.remove(&player.id);
            }
            ServerMessage::DungeonDoorsState {
                ref entrance_id,
                ref doors,
            } => {
                self.world_cache
                    .write()
                    .unwrap()
                    .set_dungeon_doors(entrance_id, doors);
            }
            ServerMessage::DungeonDoorToggled {
                ref entrance_id,
                depth,
                door_id,
                is_open,
            } => {
                self.world_cache.write().unwrap().set_dungeon_door(
                    entrance_id,
                    *depth,
                    *door_id,
                    *is_open,
                );
            }
            ServerMessage::DungeonPropsState {
                ref entrance_id,
                depth,
                ref broken,
                ref opened,
            } => {
                let mut cache = self.world_cache.write().unwrap();
                cache.set_dungeon_broken_props(entrance_id, *depth, broken.clone());
                cache.set_dungeon_opened_props(entrance_id, *depth, opened.clone());
            }
            ServerMessage::DungeonPropOpened {
                ref entrance_id,
                depth,
                prop_id,
            } => {
                self.world_cache.write().unwrap().add_dungeon_opened_prop(
                    entrance_id,
                    *depth,
                    *prop_id,
                );
                self.pending_chest_open = None;
            }
            // Our own open landed: the chest owes us nothing until nightfall.
            ServerMessage::DungeonChestOpened {
                ref entrance_id,
                player_id,
                ..
            } if self.self_player_id.as_ref() == Some(player_id) => {
                self.treasure_chests_spent.insert(entrance_id.clone());
                self.pending_chest_open = None;
            }
            // A rejection means the interaction we recorded never happened:
            // a pending chest open, or a schedule pose adopted on send
            // (occupied bed) that must revert to standing.
            ServerMessage::InteractionRejected { ref reason } => {
                if let Some((entrance_id, depth, kind)) = self.pending_chest_open.take() {
                    match kind {
                        crate::dungeon::ChestKind::Prop(prop_id) => {
                            self.world_cache
                                .write()
                                .unwrap()
                                .remove_dungeon_opened_prop(&entrance_id, depth, prop_id);
                        }
                        // "The chest is empty (it refills at nightfall)" — the
                        // other refusals (boss alive, too far) are ours to fix.
                        crate::dungeon::ChestKind::Treasure if reason.contains("empty") => {
                            self.treasure_chests_spent.insert(entrance_id);
                        }
                        crate::dungeon::ChestKind::Treasure => {}
                    }
                } else if self.held_pose().is_some() {
                    self.set_self_pose(None);
                }
            }
            ServerMessage::DungeonPropBroken {
                ref entrance_id,
                depth,
                prop_id,
                ..
            } => {
                self.world_cache.write().unwrap().add_dungeon_broken_prop(
                    entrance_id,
                    *depth,
                    *prop_id,
                );
            }
            ServerMessage::BuybackUpdated {
                merchant_player_id,
                ref buyback,
            } => {
                self.merchant_buyback
                    .insert(*merchant_player_id, buyback.clone());
            }
            ServerMessage::ShopState {
                merchant_player_id,
                ref merchant_name,
                ref buyback,
                ..
            } => {
                self.merchant_buyback
                    .insert(*merchant_player_id, buyback.clone());
                // The agent never sends OpenShop, so a ShopState is always a
                // trade window pushed at us by an NPC's OpenTrade — the offer
                // toast a web player would see. A re-send from the same
                // merchant (a deal changed mid-trade) is not a new offer, but
                // one arriving after the last offer lapsed is.
                let repeat = self
                    .pushed_trade
                    .as_ref()
                    .is_some_and(|t| t.merchant_id == *merchant_player_id && t.is_live());
                self.pushed_trade = Some(PushedTrade {
                    merchant_id: *merchant_player_id,
                    merchant_name: merchant_name.clone(),
                    expires_at: std::time::Instant::now() + TRADE_OFFER_TTL,
                });
                if !repeat {
                    self.push_agent_event(format!(
                        "[TradeOffer] {merchant_name} opened their trade window on you — buy or \
                         sell with them, or wave it off with decline_trade."
                    ));
                }
            }
            ServerMessage::GameState {
                players,
                monsters,
                ground_items,
                campfires,
                stalls,
                tip_hats,
            } => {
                self.nearby_players = players.iter().map(|p| (p.id, p.clone())).collect();
                self.nearby_monsters = monsters.clone();
                self.ground_items.clear();
                for item in ground_items {
                    self.remember_ground_item(item.clone());
                }
                self.campfires.clear();
                for campfire in campfires {
                    self.campfires.insert(campfire.id, campfire.clone());
                }
                self.stalls.clear();
                for stall in stalls {
                    self.stalls.insert(stall.id, stall.clone());
                }
                self.tip_hats.clear();
                for hat in tip_hats {
                    self.tip_hats.insert(hat.id, hat.clone());
                }
                // Update self_player from game state
                if let Some(self_id) = self.self_player_id {
                    if let Some(p) = self.nearby_players.get(&self_id).cloned() {
                        self.self_player = Some(p);
                    }
                }
            }
            ServerMessage::PlayerHealthUpdate {
                player_id,
                health,
                max_health,
            } if self.self_player_id.as_ref() == Some(player_id) => {
                if let Some(p) = self.self_player.as_mut() {
                    p.health = *health;
                    p.max_health = *max_health;
                }
            }
            // Only ever sent direct to the player who earned (or lost) the XP,
            // so this never describes anyone in `nearby_players`.
            ServerMessage::XpGained {
                player_id,
                new_level,
                max_hp,
                current_hp,
                ..
            } if self.self_player_id.as_ref() == Some(player_id) => {
                if let Some(ref mut p) = self.self_player {
                    p.level = *new_level;
                    p.health = *current_hp;
                    p.max_health = *max_hp;
                }
            }
            ServerMessage::PlayerJoined { player } | ServerMessage::PlayerAppeared { player } => {
                self.nearby_players.insert(player.id, player.clone());
            }
            ServerMessage::PlayerLeft { player_id }
            | ServerMessage::PlayerDisappeared { player_id } => {
                self.nearby_players.remove(player_id);
                self.seen_nearby_players.remove(player_id);
                // Out of earshot: the tune is gone, and [PlayerLeft] already
                // says why — no second line about it.
                self.music_performers.remove(player_id);
            }
            ServerMessage::PlayerMusicStarted {
                player_id, track, ..
            } => {
                self.music_performers.insert(*player_id, track.clone());
                if self.self_player_id.as_ref() == Some(player_id) {
                    self.bad_song_title_refused = false;
                    self.tips_noticed = 0;
                    push_capped(&mut self.recent_songs, track.clone(), MAX_RECENT_SONGS);
                    self.self_performance = self.self_player.as_ref().map(|me| SelfPerformance {
                        ends_at: std::time::Instant::now() + crate::bgm_defs::duration(track),
                        from: me.position,
                    });
                }
            }
            ServerMessage::TradeDeclined { player_id, .. } => {
                // Prune on insert so the map cannot grow one dead entry per
                // decliner over a long session.
                let now = std::time::Instant::now();
                self.trade_declined_until.retain(|_, until| now < *until);
                self.trade_declined_until
                    .insert(*player_id, now + TRADE_DECLINE_COOLDOWN);
            }
            ServerMessage::PlayerInteractionChanged {
                player_id,
                object_type,
            } => {
                if self.self_player_id.as_ref() == Some(player_id) {
                    self.set_self_pose(object_type.clone());
                }
                if object_type.as_deref() != Some(MUSIC_EMOTE) {
                    self.finish_music(player_id);
                }
            }
            ServerMessage::MonsterSpawned { monster } => {
                self.nearby_monsters
                    .insert(monster.id.clone(), monster.clone());
            }
            ServerMessage::SpawnMonsterRequest { monster_type } => {
                if let Some(pos) = self.find_valid_spawn_position() {
                    let mut rng = rand::thread_rng();
                    let rotation = rng.gen_range(0.0..std::f32::consts::TAU);
                    self.pending_commands
                        .push(ClientMessage::RequestSpawnMonster {
                            monster_type: monster_type.clone(),
                            position: pos,
                            rotation,
                        });
                }
            }
            ServerMessage::NoSpawnZones { zones } => {
                self.no_spawn_zones = zones.clone();
            }
            ServerMessage::MonsterAssigned { monster } => {
                self.nearby_monsters
                    .insert(monster.id.clone(), monster.clone());
                self.monster_ai.add_monster(monster);
            }
            ServerMessage::MonsterDead { monster_id, .. } => {
                self.nearby_monsters.remove(monster_id);
                self.monster_ai.handle_monster_dead(monster_id);
            }
            ServerMessage::MonsterRemoved { monster_id } => {
                self.forget_monster(monster_id);
            }
            // The server just said this monster does not exist: its
            // MonsterDead/MonsterRemoved never reached us. Silently drop the
            // ghost — the [AttackRejected] event already tells the agent the
            // swing failed, and the next CURRENT STATE no longer lists it.
            ServerMessage::PlayerAttackRejected {
                monster_id,
                reason: onlinerpg_shared::AttackRejectReason::InvalidTarget,
            } => {
                self.forget_monster(monster_id);
            }

            ServerMessage::GroundItemSpawned { item } => {
                self.note_tip(item);
                self.remember_ground_item(item.clone());
            }
            // Not a fresh drop, just an item coming into view — never a tip.
            ServerMessage::GroundItemAppeared { item } => {
                self.remember_ground_item(item.clone());
            }
            ServerMessage::GroundItemRemoved {
                instance_id,
                picked_up_by,
            } => {
                let removed = self.ground_items.remove(instance_id);
                self.pending_tips.retain(|(id, _)| id != instance_id);
                // Only player-dropped items are worth a line — see note_pickup.
                if let Some(item) = removed.filter(|item| item.dropped_by.is_some()) {
                    if let Some(picker) = picked_up_by.filter(|id| self.self_player_id != Some(*id))
                    {
                        self.note_pickup(&item, &picker);
                    }
                }
            }
            ServerMessage::GroundItemQuantityChanged {
                instance_id,
                quantity,
                ..
            } => {
                if let Some(item) = self.ground_items.get_mut(instance_id) {
                    item.quantity = *quantity;
                }
            }
            ServerMessage::CharacterCreated { ref character } => {
                self.characters.push(character.clone());
            }
            ServerMessage::GoldUpdate { gold } => {
                self.self_gold = Some(*gold);
            }
            ServerMessage::HungerUpdate {
                satiation, state, ..
            } => {
                self.self_hunger = Some((*satiation, *state));
            }
            ServerMessage::DebuffUpdate { ref debuffs } => {
                self.self_debuffs = debuffs.iter().map(|d| d.id.clone()).collect();
            }
            ServerMessage::CampfireSpawned { ref campfire }
            | ServerMessage::CampfireAppeared { ref campfire } => {
                self.campfires.insert(campfire.id, campfire.clone());
            }
            ServerMessage::CampfireRemoved { campfire_id } => {
                self.campfires.remove(campfire_id);
            }
            ServerMessage::StallPlaced { ref stall }
            | ServerMessage::StallAppeared { ref stall } => {
                self.stalls.insert(stall.id, stall.clone());
            }
            ServerMessage::StallRemoved { stall_id } => {
                self.stalls.remove(stall_id);
            }
            ServerMessage::TradeBusy { busy } => {
                self.trade_busy = *busy;
            }
            ServerMessage::PartyInviteReceived {
                inviter_id,
                ref inviter_name,
            } => {
                self.prune_expired_party_invites();
                let queue = &mut self.pending_party_invites;
                if queue.len() < MAX_PENDING_PARTY_INVITES
                    && !queue.iter().any(|i| i.inviter_id == *inviter_id)
                {
                    queue.push(PendingPartyInvite {
                        inviter_id: *inviter_id,
                        inviter_name: inviter_name.clone(),
                        expires_at: std::time::Instant::now() + PARTY_INVITE_TTL,
                    });
                }
            }
            ServerMessage::PartySummonReceived {
                caster_id,
                ref caster_name,
            } => {
                self.prune_expired_party_summons();
                // Replace any same-caster entry (always stale: the ack-only
                // cast never re-sends for a live one). No cap — distinct
                // casters bound the queue at the party size.
                let queue = &mut self.pending_party_summons;
                queue.retain(|s| s.caster_id != *caster_id);
                queue.push(PendingPartySummon {
                    caster_id: *caster_id,
                    caster_name: caster_name.clone(),
                    expires_at: std::time::Instant::now() + PARTY_SUMMON_TTL,
                });
            }
            ServerMessage::FriendRequestReceived {
                requester_id,
                ref requester_name,
            } => {
                self.prune_expired_friend_requests();
                let queue = &mut self.pending_friend_requests;
                if queue.len() < MAX_PENDING_FRIEND_REQUESTS
                    && !queue.iter().any(|r| r.requester_id == *requester_id)
                {
                    queue.push(PendingFriendRequest {
                        requester_id: *requester_id,
                        requester_name: requester_name.clone(),
                        expires_at: std::time::Instant::now()
                            + onlinerpg_shared::messages::FRIEND_REQUEST_TTL,
                    });
                }
            }
            ServerMessage::FriendList { ref friends } => {
                // Answering a request settles it; the roster names the verdict.
                self.pending_friend_requests
                    .retain(|r| !friends.iter().any(|f| f.name == r.requester_name));
                self.friends = friends.clone();
            }
            ServerMessage::FriendsOnline { ref friends } => {
                // The answer to our own friends_online ask. Ids map to names
                // through the roster; an id off the roster shows as is.
                if friends.is_empty() {
                    self.push_agent_event(
                        "[FriendsOnline] None of your friends are online right now.".to_string(),
                    );
                } else {
                    let names: Vec<String> = friends
                        .iter()
                        .map(|f| {
                            let name = self
                                .friends
                                .iter()
                                .find(|e| e.character_id == f.character_id)
                                .map(|e| e.name.as_str())
                                .unwrap_or("(unknown)");
                            format!("{name} (Lv.{})", f.level)
                        })
                        .collect();
                    self.push_agent_event(format!(
                        "[FriendsOnline] Online now: {}.",
                        names.join(", ")
                    ));
                }
            }
            ServerMessage::TipHatPlaced { ref tip_hat }
            | ServerMessage::TipHatAppeared { ref tip_hat } => {
                self.tip_hats.insert(tip_hat.id, tip_hat.clone());
            }
            ServerMessage::TipHatRemoved { tip_hat_id } => {
                self.tip_hats.remove(tip_hat_id);
            }
            ServerMessage::PartyState {
                leader_id,
                ref members,
            } => {
                self.party_leader = (!members.is_empty()).then_some(*leader_id);
                self.party_members = members.clone();
                // Joining a party settles whichever invite led to it.
                if !members.is_empty() {
                    self.pending_party_invites.clear();
                }
                // A summons only lives while its caster shares the roster.
                self.pending_party_summons
                    .retain(|s| members.iter().any(|m| m.id == s.caster_id));
            }
            ServerMessage::InventoryState { ref inventory }
            | ServerMessage::InventoryUpdated { ref inventory } => {
                self.self_bag = inventory.bag.clone();
                self.self_equipped = inventory.equipped.clone();
                // The join snapshot only — mid-session hands are the agent's.
                if matches!(msg, ServerMessage::InventoryState { .. }) {
                    self.take_up_instrument();
                }
            }
            // A player sold to us = we bought a wishlist item (the server
            // only lets residents buy their wishlist): shopping mood
            // satisfied for a while.
            ServerMessage::TradeNotice {
                kind: onlinerpg_shared::messages::DealKind::Sell,
                ..
            } => {
                self.trade_satiated_until =
                    Some(std::time::Instant::now() + WISHLIST_TRADE_COOLDOWN);
            }
            ServerMessage::PlayerMoved {
                player_id,
                position,
                ..
            } => {
                // Update tracked position for self and nearby players
                if self.self_player_id.as_ref() == Some(player_id) {
                    if let Some(ref mut p) = self.self_player {
                        p.position = *position;
                    }
                }
                if let Some(p) = self.nearby_players.get_mut(player_id) {
                    p.position = *position;
                }
            }
            ServerMessage::MonsterMoved {
                monster_id,
                position,
                rotation,
                state,
                ..
            } => {
                self.apply_monster_pose(monster_id, *position, *rotation, *state);
                self.monster_ai
                    .apply_authoritative_position(monster_id, *position);
            }
            ServerMessage::HouseSpawned { ref house } => {
                self.world_cache.write().unwrap().add_house(house.clone());
            }
            ServerMessage::HousesInArea { ref houses } => {
                let mut world = self.world_cache.write().unwrap();
                for house in houses {
                    world.add_house(house.clone());
                }
            }
            ServerMessage::HouseUpdated { ref house } => {
                self.world_cache.write().unwrap().add_house(house.clone());
            }
            ServerMessage::HouseRemoved { ref house_id } => {
                self.world_cache.write().unwrap().remove_house(house_id);
            }
            ServerMessage::DoorToggled {
                ref house_id,
                room_index,
                ref wall_dir,
                segment_index,
                is_open,
            } => {
                self.world_cache.write().unwrap().update_door(
                    house_id,
                    *room_index,
                    *wall_dir,
                    *segment_index as usize,
                    *is_open,
                );
            }
            // Notify monster AI when a managed monster is attacked
            ServerMessage::PlayerAttacked {
                player_id,
                monster_id,
                hit,
                damage,
                ..
            } => {
                self.handle_managed_monster_hit(monster_id, player_id, *hit, *damage);
            }
            ServerMessage::MonsterProvoked {
                player_id,
                monster_id,
            } => {
                self.handle_managed_monster_hit(monster_id, player_id, false, 0);
            }
            // Fishing reflexes: answer bites/beats mechanically; the LLM only
            // decides whether to fish. Answers carry a human reaction delay
            // so the agent has no edge over a player at the same rod.
            ServerMessage::FishingCasted { player_id, .. }
                if self.self_player_id.as_ref() == Some(player_id) =>
            {
                self.set_self_fishing(true);
            }
            ServerMessage::FishingEnded { player_id, .. }
                if self.self_player_id.as_ref() == Some(player_id) =>
            {
                self.set_self_fishing(false);
            }
            ServerMessage::FishingBite { player_id }
                if self.self_player_id.as_ref() == Some(player_id) =>
            {
                self.react_fishing(FishingAction::Hook, HOOK_REACTION_MS);
            }
            ServerMessage::FishingFight {
                player_id,
                fish_state,
                tension_pct,
                ..
            } if self.self_player_id.as_ref() == Some(player_id) => {
                // Same policy a practiced human plays from the gauge; answered
                // only on change — a stance holds until replaced.
                let stance = auto_stance(*fish_state, *tension_pct);
                if self.fishing_stance != Some(stance)
                    && self.react_fishing(stance, STANCE_REACTION_MS)
                {
                    self.fishing_stance = Some(stance);
                }
            }
            _ => {}
        }

        // Check if any player just entered the nearby radius
        match &msg {
            ServerMessage::GameState { .. }
            | ServerMessage::PlayerJoined { .. }
            | ServerMessage::PlayerAppeared { .. }
            | ServerMessage::PlayerMoved { .. } => {
                self.check_nearby_player_proximity();
            }
            _ => {}
        }

        // Check if any POI just entered sight. Only our own relocations
        // matter on the player side — walking (echoed as PlayerMoved),
        // teleports, server corrections; other players never affect what
        // we can see.
        match &msg {
            ServerMessage::GameState { .. }
            | ServerMessage::MonsterSpawned { .. }
            | ServerMessage::MonsterAssigned { .. }
            | ServerMessage::MonsterMoved { .. }
            | ServerMessage::GroundItemSpawned { .. }
            | ServerMessage::GroundItemAppeared { .. }
            | ServerMessage::PositionCorrected { .. } => {
                self.check_sightings();
            }
            ServerMessage::PlayerMoved { player_id, .. }
            | ServerMessage::PlayerTeleported { player_id, .. }
                if self.self_player_id.as_ref() == Some(player_id) =>
            {
                self.check_sightings();
            }
            _ => {}
        }

        let urgency = self.classify_event(&msg);

        // Deduplicate high-frequency movement events: keep only latest per entity
        match &msg {
            ServerMessage::MonsterMoved {
                monster_id,
                position,
                ..
            } => {
                // Only forward to LLM if monster is within sight radius
                let dominated_by_distance = self.self_player.as_ref().is_some_and(|sp| {
                    position.dist_xz_sq(&sp.position) > NPC_SIGHT_RADIUS * NPC_SIGHT_RADIUS
                });
                if !dominated_by_distance {
                    self.latest_monster_moves.insert(monster_id.clone(), msg);
                }
                return urgency;
            }
            ServerMessage::PlayerMoved { player_id, .. } => {
                self.latest_player_moves.insert(*player_id, msg);
                return urgency;
            }
            // A pure state flag; it changes movement gating but is not an LLM
            // event in its own right.
            ServerMessage::TradeBusy { .. } => return urgency,
            ServerMessage::PartyPositions { .. } => return urgency,
            // In-flight fishing beats: the reflex layer above already
            // answered them; the LLM only needs the FishingEnded outcome.
            ServerMessage::FishingCasted { .. }
            | ServerMessage::FishingBite { .. }
            | ServerMessage::FishingFight { .. } => return urgency,
            // Another player's ending renders no prompt line, so buffering it
            // would turn an otherwise-skipped poll into a blank LLM call.
            ServerMessage::FishingEnded { player_id, .. }
                if self.self_player_id.as_ref() != Some(player_id) =>
            {
                return urgency;
            }
            // Ground items churn in and out of the AOI as everyone moves;
            // the world state lists what is nearby each turn instead.
            ServerMessage::GroundItemSpawned { .. }
            | ServerMessage::GroundItemAppeared { .. }
            | ServerMessage::GroundItemRemoved { .. }
            | ServerMessage::GroundItemQuantityChanged { .. } => return urgency,
            // Campfires likewise live in the world state, and the grill start
            // is answered by GrillEnded a few seconds later.
            ServerMessage::CampfireSpawned { .. }
            | ServerMessage::CampfireAppeared { .. }
            | ServerMessage::CampfireRemoved { .. }
            | ServerMessage::GrillStarted => return urgency,
            ServerMessage::GameTimeSync { datetime, is_night } => {
                let prev_night = self.is_night;
                let prev_hour = self.game_hour;
                let hour = datetime.hour as u32;
                let minute = datetime.minute as u32;
                let night = *is_night;
                self.is_night = Some(night);
                self.game_hour = Some(hour);
                self.game_minute = Some(minute);
                self.latest_time = Some(msg);
                // Detect day/night transition or hour change → wake driver
                if (prev_night.is_some() && prev_night != self.is_night)
                    || (prev_hour.is_some() && prev_hour != self.game_hour)
                {
                    self.push_ambient_event(format!(
                        "[TimeChange] It is now {hour:02}:{minute:02} ({}).",
                        if night { "night" } else { "day" }
                    ));
                }
                return urgency;
            }
            _ => {}
        }

        self.events.push(msg);

        // Cap buffer size: drop oldest events
        if self.events.len() > MAX_EVENTS {
            let overflow = self.events.len() - MAX_EVENTS;
            self.events.drain(..overflow);
        }

        // Notify Claude driver if urgent
        if urgency == EventUrgency::Urgent {
            self.wake(EventUrgency::Urgent);
        }

        urgency
    }

    pub fn drain_events(&mut self) -> Vec<ServerMessage> {
        let mut events = std::mem::take(&mut self.events);

        // Append latest snapshots
        if let Some(time) = self.latest_time.take() {
            events.push(time);
        }
        events.extend(self.latest_monster_moves.drain().map(|(_, v)| v));
        events.extend(self.latest_player_moves.drain().map(|(_, v)| v));

        events
    }

    /// Drain synthetic agent-side events (e.g. player proximity alerts).
    pub fn drain_agent_events(&mut self) -> Vec<String> {
        std::mem::take(&mut self.agent_events)
    }

    /// Agent events pushed since a mark taken from [`Self::action_progress`].
    pub fn agent_events_from(&self, from: usize) -> &[String] {
        self.agent_events.get(from..).unwrap_or(&[])
    }

    /// Push a synthetic agent event visible to the LLM. Synthetic events are
    /// feedback on the agent's own actions (arrival, a failed move, a kill),
    /// so they wake the LLM driver instead of waiting out the idle interval.
    /// They wake it at `Routine` though: an agent's own arrival note must
    /// never outrank a human talking to some other NPC in the LLM queue.
    /// Counts as the running action's result — anything no action caused
    /// belongs in [`Self::push_ambient_event`] instead.
    pub fn push_agent_event(&mut self, event: String) {
        self.push_agent_event_inner(event, true, false);
    }

    /// Same, but without waking the driver: the event rides along with
    /// whatever prompt happens next (scenery noted in passing, not danger).
    pub fn push_agent_event_quiet(&mut self, event: String) {
        self.push_agent_event_inner(event, false, false);
    }

    /// An event no action of the agent caused — a clock tick, a sighting, a
    /// tip. Kept out of `action_events_pushed`, or `settle_action` would read
    /// it as the concurrent action's result and rob a dud of its [NoResult].
    pub fn push_ambient_event(&mut self, event: String) {
        self.push_agent_event_inner(event, true, true);
    }

    /// Ambient and quiet: rides along with the next prompt.
    pub fn push_ambient_event_quiet(&mut self, event: String) {
        self.push_agent_event_inner(event, false, true);
    }

    fn push_agent_event_inner(&mut self, event: String, wake: bool, ambient: bool) {
        if let Some(watch) = &self.watch {
            watch.push("agent", event.clone());
        }
        self.agent_events.push(event);
        if !ambient {
            self.action_events_pushed += 1;
        }
        if wake {
            self.wake(EventUrgency::Routine);
        }
    }

    /// Wake the LLM driver, remembering how urgent the reason was. The driver
    /// takes the urgency at wake-up to pick its rate-limit floor and the
    /// prompt's scheduler priority.
    pub(super) fn wake(&mut self, urgency: EventUrgency) {
        self.wake_urgency = self.wake_urgency.min(urgency);
        self.urgent_notify.notify_one();
    }

    /// Take the urgency accumulated since the last wake-up, resetting it.
    pub fn take_wake_urgency(&mut self) -> EventUrgency {
        std::mem::replace(&mut self.wake_urgency, EventUrgency::Noise)
    }
}
