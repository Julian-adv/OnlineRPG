//! Prompt construction: turn the current `SharedState`, the queue of
//! pending server events, and the active schedule entry into a single
//! string suitable for sending to an LLM. Also resolves which schedule
//! entry should currently be active based on game time.

use onlinerpg_shared::{PlayerId, ServerMessage};

use crate::state::{SharedState, NPC_SIGHT_RADIUS};
use onlinerpg_shared::schedule::ScheduleEntry;

fn within_event_range(state: &SharedState, x: f32, z: f32) -> bool {
    let Some(self_p) = state.self_player.as_ref() else {
        return true;
    };
    crate::geom::PlanarDelta::xz(self_p.position.x, self_p.position.z, x, z).dist
        <= NPC_SIGHT_RADIUS
}

fn player_within_event_range(state: &SharedState, player_id: &PlayerId) -> bool {
    if state.self_player_id.as_ref() == Some(player_id) {
        return true;
    }
    let Some(p) = state.nearby_players.get(player_id) else {
        return true;
    };
    within_event_range(state, p.position.x, p.position.z)
}

fn monster_within_event_range(state: &SharedState, monster_id: &str) -> bool {
    let Some(m) = state.nearby_monsters.get(monster_id) else {
        return true;
    };
    within_event_range(state, m.position.x, m.position.z)
}

/// Build a prompt string from current state and events.
pub(super) fn build_prompt(
    state: &SharedState,
    events: &[ServerMessage],
    agent_events: &[String],
    schedule: &[ScheduleEntry],
    active_schedule_idx: Option<usize>,
) -> String {
    let mut prompt = String::new();

    prompt.push_str("=== CURRENT STATE ===\n");
    prompt.push_str(&state.format_world_state());
    prompt.push('\n');

    // A customer is mid-trade: stay put and keep helping them. Movement is
    // suppressed server-side regardless, but telling the LLM keeps its
    // narration consistent.
    if state.trade_busy {
        prompt.push_str(
            "\nA customer is trading with you right now. Stay where you are and keep \
             helping them — do not walk away or wander off.\n",
        );
    }

    if state.self_fishing {
        prompt.push_str(
            "\nYou are fishing right now. Hooking and fighting the fish is automatic — \
             stay put and wait for the [Fishing] outcome; moving or attacking cancels \
             the session.\n",
        );
    }

    // Resident-trader wishlist, rebuilt every turn from the live bag:
    // owned items drop out, and a fully satisfied wishlist removes the
    // section (and with it the urge to trade) entirely. It is also
    // suppressed for a cooldown after each successful purchase, and
    // whenever no human player is in sight — only players can sell to the
    // NPC, so without one around the desire would only produce futile
    // NPC-to-NPC pestering.
    let satiated = state
        .trade_satiated_until
        .is_some_and(|until| std::time::Instant::now() < until);
    if !satiated && state.has_nearby_human_players() {
        if let Some(p) = state.self_player.as_ref() {
            if let Some(section) =
                crate::shop_info::resident_trade_prompt_for(&p.name, &state.self_bag)
            {
                prompt.push('\n');
                prompt.push_str(&section);
            }
        }
    }

    if let Some(ctx) = format_schedule_context(schedule, active_schedule_idx) {
        prompt.push_str(&ctx);
        prompt.push('\n');
    }

    let has_server_events = events.iter().any(|e| format_event(state, e).is_some());
    if has_server_events || !agent_events.is_empty() {
        prompt.push_str("\n=== EVENTS ===\n");
        for event in events {
            if let Some(line) = format_event(state, event) {
                prompt.push_str(&line);
                prompt.push('\n');
            }
        }
        for line in agent_events {
            prompt.push_str(line);
            prompt.push('\n');
        }
    }

    prompt.push_str("\nWhat do you do?");
    prompt
}

/// Format a server event as a human-readable line for LLM prompts.
/// Returns `None` for events that should not be forwarded to the LLM.
pub(crate) fn format_event(state: &SharedState, msg: &ServerMessage) -> Option<String> {
    match msg {
        ServerMessage::JoinSuccess { player, .. } => Some(format!(
            "[Join] You joined as {} at ({:.1}, {:.1}, {:.1})",
            player.name, player.position.x, player.position.y, player.position.z
        )),
        ServerMessage::GameState {
            players, monsters, ..
        } => {
            let mut lines = vec![format!(
                "[World] {} player(s), {} monster(s)",
                players.len(),
                monsters.len()
            )];
            for p in players.iter() {
                lines.push(format!(
                    "  Player: {} Lv.{} HP {}/{}",
                    p.name, p.level, p.health, p.max_health
                ));
            }
            for m in monsters.values() {
                lines.push(format!(
                    "  Monster: {} [{}] HP {}/{}",
                    m.monster_type, m.id, m.health, m.max_health
                ));
            }
            Some(lines.join("\n"))
        }
        ServerMessage::GameTimeSync { datetime, is_night } => Some(format!(
            "[Time] Y{} M{} D{} {:02}:{:02} ({})",
            datetime.year,
            datetime.month,
            datetime.day,
            datetime.hour,
            datetime.minute,
            if *is_night { "night" } else { "day" }
        )),
        ServerMessage::ChatMessage {
            player_id, message, ..
        } => {
            if !player_within_event_range(state, player_id) {
                return None;
            }
            let is_npc = state
                .nearby_players
                .get(player_id)
                .is_some_and(|p| p.is_official_npc);
            let tag = if is_npc { "[NpcChat]" } else { "[Chat]" };
            Some(format!(
                "{tag} {}: {message}",
                player_name(state, player_id)
            ))
        }
        ServerMessage::WhisperMessage { from, message, .. } => {
            // Skip the echo of our own outgoing whisper.
            let self_name = state.self_player.as_ref().map(|p| p.name.as_str());
            if Some(from.as_str()) == self_name {
                return None;
            }
            Some(format!("[Whisper] {from}: {message}"))
        }
        ServerMessage::PartyChatMessage { from, message } => {
            // Skip the echo of our own outgoing party line.
            let self_name = state.self_player.as_ref().map(|p| p.name.as_str());
            if Some(from.as_str()) == self_name {
                return None;
            }
            Some(format!("[Party] {from}: {message}"))
        }
        ServerMessage::SystemMessage { message } => Some(format!("[System] {message}")),
        ServerMessage::InteractionRejected { reason } => {
            Some(format!("[InteractionRejected] {reason}"))
        }
        ServerMessage::DungeonChestOpened {
            player_id,
            item_def_ids,
            gold,
            ..
        } => {
            let who = if state.self_player_id.as_ref() == Some(player_id) {
                "You".to_string()
            } else {
                player_name(state, player_id)
            };
            Some(format!(
                "[Chest] {who} opened the treasure chest: {} + {gold} gold.",
                item_def_ids.join(", ")
            ))
        }
        ServerMessage::DungeonPropBroken { depth, prop_id, .. } => Some(format!(
            "[Prop] Prop {prop_id} on floor {depth} was smashed — its cell is now walkable."
        )),
        ServerMessage::DungeonPropOpened { depth, prop_id, .. } => Some(format!(
            "[Prop] Chest prop {prop_id} on floor {depth} swung open."
        )),
        ServerMessage::BuybackUpdated {
            merchant_player_id,
            buyback,
        } => {
            let who = player_name(state, merchant_player_id);
            if buyback.is_empty() {
                return Some(format!(
                    "[Buyback] {who}'s buyback list is now empty (your last repurchase or \
                     sale cleared it)."
                ));
            }
            // Enchant is the only thing telling two entries of the same item
            // apart — the payout ignores it, so the prices match.
            let list: Vec<String> = buyback
                .iter()
                .take(6)
                .map(|e| {
                    let price = crate::shop_info::format_price(e.price);
                    match e.enchant {
                        0 => format!("{} @{price}", e.item_def_id),
                        n => format!("{} +{n} @{price}", e.item_def_id),
                    }
                })
                .collect();
            Some(format!(
                "[Buyback] {who} will sell back: {} — {{\"type\": \"buyback\", \"item\": ..., \
                 \"merchant\": \"{who}\"}}.",
                list.join(", ")
            ))
        }
        ServerMessage::PlayerMusicStarted {
            player_id, track, ..
        } => {
            if !player_within_event_range(state, player_id) {
                return None;
            }
            let who = if state.self_player_id.as_ref() == Some(player_id) {
                "You".to_string()
            } else {
                player_name(state, player_id)
            };
            // Everyone in earshot hears the same tune; it plays until the
            // performer moves or the track runs out.
            Some(format!("[PlayMusic] {who} started playing \"{track}\"."))
        }
        ServerMessage::PlayerJoined { player } => {
            if !within_event_range(state, player.position.x, player.position.z) {
                return None;
            }
            Some(format!("[PlayerJoined] {}", player.name))
        }
        ServerMessage::PlayerAppeared { player } => {
            if !within_event_range(state, player.position.x, player.position.z) {
                return None;
            }
            Some(format!("[PlayerAppeared] {}", player.name))
        }
        ServerMessage::PlayerLeft { player_id } => {
            if !player_within_event_range(state, player_id) {
                return None;
            }
            Some(format!("[PlayerLeft] {}", player_name(state, player_id)))
        }
        ServerMessage::PlayerDisappeared { player_id } => Some(format!(
            "[PlayerDisappeared] {}",
            player_name(state, player_id)
        )),
        ServerMessage::PlayerTeleported {
            player_id,
            position,
            floor_level,
            ..
        } => {
            if state.self_player_id.as_ref() != Some(player_id) {
                return None;
            }
            let floor = match *floor_level {
                0 => "the surface".to_string(),
                f if f < 0 => format!("dungeon floor {}", f.unsigned_abs()),
                f => format!("floor {f}"),
            };
            Some(format!(
                "[Teleported] You were teleported to ({:.0}, {:.0}) on {floor}. Your old \
                 walk targets no longer apply.",
                position.x, position.z
            ))
        }
        ServerMessage::PlayerMoved {
            player_id,
            position,
            ..
        } => {
            if !within_event_range(state, position.x, position.z) {
                return None;
            }
            Some(format!(
                "[Move] {} -> ({:.1}, {:.1}, {:.1})",
                player_name(state, player_id),
                position.x,
                position.y,
                position.z
            ))
        }
        ServerMessage::MonsterSpawned { monster } => {
            if !within_event_range(state, monster.position.x, monster.position.z) {
                return None;
            }
            Some(format!(
                "[MonsterSpawned] {} ({})",
                monster.id, monster.monster_type
            ))
        }
        ServerMessage::MonsterDead { monster_id, .. } => {
            if !monster_within_event_range(state, monster_id) {
                return None;
            }
            Some(format!("[MonsterDead] {monster_id}"))
        }
        ServerMessage::PlayerAttacked {
            player_id,
            monster_id,
            hit,
            damage,
            ..
        } => {
            let is_self = state.self_player_id.as_ref() == Some(player_id);
            if !is_self && !monster_within_event_range(state, monster_id) {
                return None;
            }
            Some(format!(
                "[Attack] {} -> {monster_id}: hit={hit} dmg={damage}",
                player_name(state, player_id)
            ))
        }
        ServerMessage::PlayerAttackRejected { monster_id, reason } => {
            Some(format!("[AttackRejected] {monster_id}: {reason}"))
        }
        ServerMessage::MonsterAttackedPlayer {
            monster_id,
            player_id,
            hit,
            damage,
            current_health,
            ..
        } => {
            let is_self = state.self_player_id.as_ref() == Some(player_id);
            if !is_self && !monster_within_event_range(state, monster_id) {
                return None;
            }
            Some(format!(
                "[MonsterAttack] {monster_id} -> {}: hit={hit} dmg={damage} hp={current_health}",
                player_name(state, player_id)
            ))
        }
        ServerMessage::PlayerDead { player_id } => {
            if !player_within_event_range(state, player_id) {
                return None;
            }
            Some(format!("[PlayerDead] {}", player_name(state, player_id)))
        }
        ServerMessage::PlayerRespawned { player } => {
            let is_self = state.self_player_id.as_ref() == Some(&player.id);
            if !is_self && !within_event_range(state, player.position.x, player.position.z) {
                return None;
            }
            Some(format!(
                "[Respawn] {} HP {}/{}",
                player.name, player.health, player.max_health
            ))
        }
        ServerMessage::XpGained {
            xp_amount,
            total_xp,
            new_level,
            leveled_up,
            ..
        } => {
            let mut s = format!("[XP] +{xp_amount} (total: {total_xp}, level: {new_level})");
            if *leveled_up {
                s.push_str(" LEVEL UP!");
            }
            Some(s)
        }
        ServerMessage::CharacterError { message } => Some(format!("[CharacterError] {message}")),
        ServerMessage::CharacterCreated { character } => Some(format!(
            "[CharacterCreated] id={} {} Lv.{} {:?}",
            character.id, character.name, character.level, character.class
        )),
        ServerMessage::CharacterStatsRolled { attributes, max_hp } => Some(format!(
            "[StatsRolled] STR:{} DEX:{} CON:{} INT:{} WIS:{} CHA:{} HP:{}",
            attributes.r#str,
            attributes.dex,
            attributes.con,
            attributes.int,
            attributes.wis,
            attributes.cha,
            max_hp
        )),
        ServerMessage::MonsterMoved {
            monster_id,
            position,
            state: monster_state,
            ..
        } => {
            if !within_event_range(state, position.x, position.z) {
                return None;
            }
            Some(format!(
                "[MonsterMoved] {monster_id} -> ({:.1}, {:.1}, {:.1}) state={monster_state}",
                position.x, position.y, position.z
            ))
        }
        ServerMessage::Kicked { reason, .. } => Some(format!("[Kicked] {reason}")),
        ServerMessage::DealResult {
            target_player_name,
            item_def_id,
            kind,
            accepted,
            applied_modifier_pct,
            message,
            ..
        } => Some(format!(
            "[DealResult] your offer on {item_def_id} ({kind:?}) for {target_player_name}: {} \
             at {applied_modifier_pct}% — {message}",
            if *accepted { "GRANTED" } else { "REJECTED" }
        )),
        ServerMessage::TradeNotice {
            player_name,
            item_def_id,
            kind,
            price,
            npc_gold,
        } => Some(format!(
            "[Trade] {player_name} {} for {} — your gold is now {}",
            match kind {
                onlinerpg_shared::messages::DealKind::Buy =>
                    format!("bought {item_def_id} from you"),
                onlinerpg_shared::messages::DealKind::Sell => format!("sold you {item_def_id}"),
            },
            crate::shop_info::format_price(*price),
            crate::shop_info::format_price(*npc_gold),
        )),
        ServerMessage::TradeError { message } => Some(format!("[TradeError] {message}")),
        ServerMessage::PartyInviteReceived { inviter_name, .. } => Some(format!(
            "[PartyInvite] {inviter_name} invited you to their party. Accept with \
             {{\"type\":\"party_accept\"}} or decline with party_decline."
        )),
        ServerMessage::PartyInviteResult { message, .. } => Some(format!("[Party] {message}")),
        ServerMessage::PartySummonReceived { caster_name, .. } => Some(format!(
            "[PartySummon] {caster_name} calls you to their side (summoning scroll). Accept \
             with {{\"type\":\"summon_accept\"}} or decline with summon_decline."
        )),
        ServerMessage::PartyState { members, .. } => Some(if members.is_empty() {
            "[Party] You are no longer in a party.".to_string()
        } else {
            format!(
                "[Party] Members: {}",
                members
                    .iter()
                    .map(|m| m.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }),
        // Fishing: only the endings reach the LLM (reflexes answered the
        // rest). Word the outcome so the model knows what it can do next.
        ServerMessage::FishingEnded { player_id, outcome } => {
            if state.self_player_id.as_ref() != Some(player_id) {
                return None;
            }
            use onlinerpg_shared::fishing::FishingOutcome;
            Some(match outcome {
                FishingOutcome::Caught {
                    item_def_id,
                    size_cm,
                    trophy,
                } => caught_line(item_def_id, *size_cm, *trophy),
                FishingOutcome::Escaped => {
                    "[Fishing] The fish got away. You can cast again with the fish action."
                        .to_string()
                }
                FishingOutcome::Aborted => "[Fishing] You reeled in your line.".to_string(),
            })
        }
        ServerMessage::FishingError { message } => Some(format!("[FishingError] {message}")),
        // Hunger transitions arrive direct (owner-only), a few per game day.
        ServerMessage::HungerUpdate {
            satiation,
            state,
            poisoned_ms,
            ..
        } => {
            use onlinerpg_shared::hunger::HungerState;
            let mut line = match state {
                HungerState::Normal => format!(
                    "[Hunger] You are adequately fed ({satiation}/1000) and can sprint."
                ),
                HungerState::Hungry => format!(
                    "[Hunger] You are getting hungry ({satiation}/1000): sprinting is unavailable and natural healing is slower."
                ),
                HungerState::Weak => format!(
                    "[Hunger] You are weak from hunger ({satiation}/1000): slower movement and attacks, less carry weight, and no natural healing. Eat something."
                ),
            };
            if *poisoned_ms > 0 {
                line.push_str(&format!(
                    " Food poisoning for {} more minutes: heavy penalties — cooked food next time.",
                    poisoned_ms.div_ceil(60_000)
                ));
            }
            Some(line)
        }
        ServerMessage::GrillEnded {
            grilled_item_def_id,
        } => Some(match grilled_item_def_id {
            Some(id) => format!("[Grill] Your fish is done: {id} is in your bag."),
            None => "[Grill] Your grilling was interrupted.".to_string(),
        }),
        // Skip unknown/unhandled event types
        _ => None,
    }
}

/// The `[Fishing]` line for a landed catch, with category-aware next steps.
/// Keep the phrasing in sync with the browser client's `catchMessage`
/// (client/src/lib/network/fishingMessages.ts).
fn caught_line(item_def_id: &str, size_cm: u16, trophy: bool) -> String {
    match crate::item_defs::get(item_def_id).and_then(|d| d.category.as_deref()) {
        Some("coin_catch") => format!(
            "[Fishing] You hauled up a {item_def_id}. It is in your bag — use it to open it and collect the coins, or fish again."
        ),
        Some("fish") => format!(
            "[Fishing] You caught a {item_def_id} ({size_cm} cm){}. It is in your bag — you can eat it, sell it, or fish again.",
            if trophy { " — a TROPHY catch!" } else { "" }
        ),
        _ => format!(
            "[Fishing] You fished up a {item_def_id}. It is in your bag — junk like this can be sold if a merchant pays for it, or dropped."
        ),
    }
}

/// Resolve a player_id to a display name using SharedState.
/// Falls back to the raw ID if the player is not found.
pub(super) fn player_name(state: &SharedState, player_id: &PlayerId) -> String {
    state.player_display_name(player_id)
}

/// Format current schedule context for inclusion in LLM prompts.
fn format_schedule_context(
    schedule: &[ScheduleEntry],
    active_idx: Option<usize>,
) -> Option<String> {
    let entry = &schedule[active_idx?];
    let mut line = format!(
        "Schedule: go to {} at ({:.1}, {:.1}, {:.1})",
        entry.display_label(),
        entry.pos[0],
        entry.pos[1],
        entry.pos[2]
    );
    if let Some(ref action) = entry.action {
        line.push_str(&format!(" — using {action} (DO NOT move, you are resting)"));
    }
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::{caught_line, format_event};
    use crate::state::tests::{test_player, test_state};
    use crate::state::NPC_SIGHT_RADIUS;
    use onlinerpg_shared::{PlayerId, ServerMessage};

    /// A tune someone strikes up nearby is worth a prompt line — the title is
    /// what makes it something an NPC can talk about. Out of earshot it is
    /// not our business.
    #[test]
    fn music_reaches_the_llm_only_within_earshot() {
        let (mut state, _rx) = test_state();
        let me = test_player(0.0, 0.0);
        state.self_player_id = Some(me.id);
        state.self_player = Some(me);

        let mut bard = test_player(5.0, 0.0);
        bard.id = PlayerId::from(2);
        bard.name = "Rica".to_string();
        state.nearby_players.insert(bard.id, bard);

        let music = |player_id| ServerMessage::PlayerMusicStarted {
            player_id,
            track: "Twilight Fields".to_string(),
            elapsed_secs: 0.0,
        };

        let line = format_event(&state, &music(PlayerId::from(2))).expect("bard is in earshot");
        assert!(line.contains("Rica"), "{line}");
        assert!(line.contains("Twilight Fields"), "{line}");

        state
            .nearby_players
            .get_mut(&PlayerId::from(2))
            .unwrap()
            .position
            .x = NPC_SIGHT_RADIUS + 10.0;
        assert_eq!(format_event(&state, &music(PlayerId::from(2))), None);
    }

    // The wording contract with the LLM: each catch category tells the model
    // what it can actually do next (against the real embedded item defs).

    #[test]
    fn a_fish_is_edible_and_sellable() {
        let line = caught_line("raw_trout", 34, false);
        assert!(line.contains("caught a raw_trout (34 cm)"), "{line}");
        assert!(line.contains("eat it, sell it"), "{line}");
        assert!(!line.contains("TROPHY"), "{line}");
    }

    #[test]
    fn a_trophy_fish_celebrates() {
        let line = caught_line("golden_sturgeon", 120, true);
        assert!(line.contains("TROPHY"), "{line}");
        assert!(line.contains("eat it, sell it"), "{line}");
    }

    #[test]
    fn junk_is_not_presented_as_edible() {
        let line = caught_line("old_boot", 40, false);
        assert!(line.contains("fished up a old_boot"), "{line}");
        assert!(
            !line.contains("eat"),
            "junk must not be offered as food: {line}"
        );
    }

    #[test]
    fn a_coin_catch_points_at_the_bag_and_the_use_action() {
        let line = caught_line("sunken_coin_pouch", 12, false);
        assert!(
            line.contains("bag"),
            "the pouch lands in the bag sealed: {line}"
        );
        assert!(
            line.contains("use it to open it"),
            "the model must learn the `use` action opens it: {line}"
        );
    }
}
