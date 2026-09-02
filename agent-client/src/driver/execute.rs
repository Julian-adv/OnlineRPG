//! Decode an LLM response and run each action against the game server.
//! Returns the monster_id of the last attack action so `llm_driver` can
//! enter its combat loop. Also persists `memory_update` snippets to the
//! NPC's per-instance memory file when configured.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::dungeon::ChestKind;
use crate::state::{Carried, CarriedBagCopies, MoveTarget, MoveTargetError, SharedState};
use onlinerpg_shared::messages::BagLineItem;

use super::action::{
    action_to_command, asks_for_great_chest, parse_turn_tolerant, resolve_move_goal, AgentAction,
    GoalError, PickupRef, Qty,
};
use super::combat::{
    approach_player, chase_monster, chest_arrive_range, follow_player, walk_to_ground_item,
    walk_to_point, ChaseResult,
};
use super::movement::{execute_move, MoveResult};
#[cfg(test)]
use super::outcome::reports_failure;
use super::outcome::{settle_action, ActionOutcome};
use super::walk::LostReason;
use super::MEMORY_LINES;

/// Pause between the crouch broadcast and the actual pickup, approximating
/// the web client's grab moment partway into its pickup animation.
const PICKUP_GRAB_DELAY_MS: u64 = 700;

/// How long a follow rests once it has caught up before checking again.
const FOLLOW_RECHECK: std::time::Duration = std::time::Duration::from_millis(1200);

/// How far the followed character must have travelled during a timed-out
/// chase for that chase to count as "they are still walking" rather than
/// "we are stuck".
const FOLLOW_PROGRESS_M: f32 = 1.0;

/// Where a character stands right now, if we can still see them.
async fn target_pos(
    state: &Arc<Mutex<SharedState>>,
    id: &onlinerpg_shared::PlayerId,
) -> Option<onlinerpg_shared::Position> {
    state
        .lock()
        .await
        .nearby_players
        .get(id)
        .map(|p| p.position)
}

/// Whether the character moved appreciably since `before`.
async fn target_moved(
    state: &Arc<Mutex<SharedState>>,
    id: &onlinerpg_shared::PlayerId,
    before: Option<onlinerpg_shared::Position>,
) -> bool {
    let (Some(was), Some(now)) = (before, target_pos(state, id).await) else {
        return false;
    };
    was.dist_xz_sq(&now) > FOLLOW_PROGRESS_M * FOLLOW_PROGRESS_M
}

/// Feedback for a `move` target string that resolved to nothing. Each case
/// hands back the ids or names that would have worked, so the LLM can fix the
/// target next turn instead of reissuing the same string.
fn move_target_failure(e: &MoveTargetError) -> String {
    match e {
        MoveTargetError::SpeciesNotId {
            species,
            candidates,
        } => {
            let list: Vec<String> = candidates
                .iter()
                .take(5)
                .map(|(id, d)| format!("{id} ({d:.0}m)"))
                .collect();
            format!(
                "[MoveFailed] '{species}' names a kind of monster, not one monster. \
                 Nearby {species}s: {}. Use the id.",
                list.join(", ")
            )
        }
        MoveTargetError::MonsterGone { id } => format!(
            "[MoveFailed] Monster {id} is not in sight — it died, or it wandered off. \
             Pick an id from the current world state."
        ),
        MoveTargetError::Unknown { asked, addressable } => {
            if addressable.is_empty() {
                format!("[MoveFailed] Nothing here is called '{asked}'.")
            } else {
                format!(
                    "[MoveFailed] Nothing here is called '{asked}'. You can move to: {}.",
                    addressable.join(", ")
                )
            }
        }
    }
}

/// What a resolved target is, for the message about a depth that does not
/// belong with it.
fn describe_target(t: &MoveTarget) -> &'static str {
    match t {
        MoveTarget::Character { .. } => "a character",
        MoveTarget::Monster { .. } => "a monster",
        MoveTarget::GroundItem { .. } => "an item on the ground",
        MoveTarget::Prop { .. } => "a prop",
        MoveTarget::Chest { .. } => "a chest",
        MoveTarget::Dungeon { .. } => "a dungeon",
    }
}

type Approach = (onlinerpg_shared::Position, i8, f32);

/// Walk goal beside a sighted prop or chest: its stand cell, floor, and
/// arrive range. The server measures interact range from the thing itself,
/// so the metre between it and the cell we stand on comes out of ours.
fn approach_beside(
    position: &onlinerpg_shared::Position,
    approach: onlinerpg_shared::Position,
    floor: i8,
) -> Approach {
    let gap = crate::geom::PlanarDelta::between(&approach, position).dist;
    (approach, floor, chest_arrive_range(gap))
}

/// The chest a selector picks from the sighted list: the great chest when
/// asked for (falling back to the nearest), else the nearest.
fn pick_chest<'a>(
    sighted: &'a [crate::dungeon::ChestSighting],
    want: Option<&str>,
) -> Option<&'a crate::dungeon::ChestSighting> {
    if asks_for_great_chest(want) {
        sighted
            .iter()
            .find(|c| c.kind == ChestKind::Treasure)
            .or(sighted.first())
    } else {
        sighted.first()
    }
}

async fn prop_approach(state: &Arc<Mutex<SharedState>>, prop_id: u32) -> Option<Approach> {
    let s = state.lock().await;
    let prop = s
        .breakables_in_sight()
        .into_iter()
        .find(|b| b.prop_id == prop_id)?;
    Some(approach_beside(
        &prop.position,
        prop.approach,
        s.self_floor_level,
    ))
}

async fn chest_approach(
    state: &Arc<Mutex<SharedState>>,
    selector: &str,
) -> Option<(Approach, &'static str)> {
    let s = state.lock().await;
    let sighted = s.chests_in_sight();
    let chest = pick_chest(&sighted, Some(selector))?;
    let label = match chest.kind {
        ChestKind::Treasure => "the great chest",
        ChestKind::Prop(_) => "a small chest",
    };
    Some((
        approach_beside(&chest.position, chest.approach, s.self_floor_level),
        label,
    ))
}

/// Walk to a resolved dungeon sighting, reporting either way.
async fn walk_to_sighting(
    state: &Arc<Mutex<SharedState>>,
    approach: Option<Approach>,
    what: &str,
    sprint: Option<bool>,
) {
    let Some((pos, floor, range)) = approach else {
        let mut s = state.lock().await;
        s.push_agent_event(format!(
            "[MoveFailed] {what} is no longer in this room to walk to."
        ));
        return;
    };
    let outcome = match walk_to_point(state, pos, floor, range, sprint).await {
        ChaseResult::InRange => format!("[Arrived] You stand next to {what}."),
        ChaseResult::Lost(loss) => {
            format!("[MoveFailed] You did not reach {what} — {}.", loss.clause())
        }
        ChaseResult::Error => {
            format!("[MoveFailed] Something went wrong walking to {what}.")
        }
    };
    state.lock().await.push_agent_event(outcome);
}

/// Feedback for an attack whose chase never reached the monster, so the agent
/// stops re-issuing it. The reason comes from the chase itself, not a guess.
fn unreachable_note(monster_id: &str, loss: &LostReason) -> String {
    match loss {
        LostReason::TargetGone => {
            format!("[Unreachable] Monster {monster_id} is gone. Pick a different target.")
        }
        LostReason::PlayerDied => {
            format!("[Unreachable] You died before reaching monster {monster_id}.")
        }
        LostReason::TooFar(d) => format!(
            "[Unreachable] Monster {monster_id} is {d:.0}m away, beyond your chase range. \
             Move closer first, or pick a nearer monster."
        ),
        LostReason::Timeout => format!(
            "[Unreachable] The chase for monster {monster_id} ran out of time — the way \
             may be blocked. Move first, or pick a different target."
        ),
        LostReason::NoPath => format!(
            "[Unreachable] No route leads to monster {monster_id} from here — it is \
             behind walls or on another floor. Pick a reachable monster."
        ),
        LostReason::LockedDoor | LostReason::Desynced => format!(
            "[Unreachable] You could not reach monster {monster_id} — {}. Move first, or \
             pick a different target.",
            loss.clause()
        ),
    }
}

/// Resolves a sell/drop qty request against the item's available bag
/// copies (`None` means the default of 1) into concrete
/// `(instance_id, qty)` lines, greedily filled across however many separate
/// copies that takes. `Err` carries a user-facing reason (bad qty value, or
/// not enough units) with no unit word — callers already know whether this
/// is a sell or a drop.
pub(super) fn resolve_qty_lines(
    copies: &[(u64, u32)],
    qty: Option<&Qty>,
) -> Result<Vec<BagLineItem>, String> {
    let available: u32 = copies.iter().map(|(_, q)| q).sum();
    let requested = match qty {
        None => 1,
        Some(q) => q
            .resolve(available)
            .ok_or_else(|| "qty must be a positive number or \"all\"".to_string())?,
    };
    if requested > available {
        return Err(format!("only {available} left, not {requested}"));
    }
    let mut need = requested;
    let mut lines = Vec::new();
    for (instance_id, remaining) in copies {
        if need == 0 {
            break;
        }
        let take = need.min(*remaining);
        lines.push(BagLineItem {
            instance_id: *instance_id,
            qty: take,
        });
        need -= take;
    }
    Ok(lines)
}

/// Parse and execute the agent's response.
/// Returns the monster_id if the last action was an attack (for combat loop).
/// If `memory_file` is set and the response contains `memory_update`, appends to file.
/// Pick a pending entry by (case-insensitive) name, or the oldest when bare.
fn pick_pending<T>(items: &[T], name: Option<&str>, name_of: impl Fn(&T) -> &str) -> Option<usize> {
    match name {
        Some(name) => items
            .iter()
            .position(|item| name_of(item).eq_ignore_ascii_case(name)),
        None => (!items.is_empty()).then_some(0),
    }
}

/// Append a memory update, dropping lines already in the file (and repeats
/// within the update itself), then rewrite the file keeping only the newest
/// `MEMORY_LINES` lines.
pub(super) fn append_memory(path: &str, update: &str) {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<&str> = existing
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let before = lines.len();
    for line in update.lines().map(str::trim) {
        if !line.is_empty() && !lines.contains(&line) {
            lines.push(line);
        }
    }
    if lines.len() == before {
        debug!("Memory update skipped, all lines duplicate: {path}");
        return;
    }
    let added = lines.len() - before;
    let start = lines.len().saturating_sub(MEMORY_LINES);
    match std::fs::write(path, lines[start..].join("\n") + "\n") {
        Ok(()) => info!(
            "Memory updated: {path} (+{added} lines, {} kept)",
            lines.len() - start
        ),
        Err(e) => warn!("Failed to write memory file {path}: {e}"),
    }
}

pub(super) async fn handle_response(
    state: &Arc<Mutex<SharedState>>,
    response: &str,
    memory_file: &Option<String>,
    favor_file: &Option<String>,
    skip_movement: bool,
) -> Option<(String, Option<bool>)> {
    let agent_resp = match parse_turn_tolerant(response) {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to parse agent response: {e}");
            warn!("Raw response: {response}");
            // The response itself stays out of the prompt: a model fed its
            // own malformed output continues in that shape.
            let mut s = state.lock().await;
            s.push_agent_event(format!(
                "[BadResponse] Your last reply was not the required JSON object ({e}), so \
                 nothing ran. Reply with only the JSON object — no prose, no code fences."
            ));
            return None;
        }
    };

    // Report any actions that failed to parse, naming the offending field, so
    // the agent fixes that action instead of thinking the whole class of action
    // is broken. The valid actions below still run.
    if !agent_resp.errors.is_empty() {
        let mut s = state.lock().await;
        for err in &agent_resp.errors {
            s.push_agent_event(format!("[BadAction] {err}."));
        }
    }

    if let (Some(ref update), Some(ref path)) = (&agent_resp.memory_update, memory_file) {
        append_memory(path, update);
    }

    // Favor deltas: fold into the per-player map (state clamps and drops
    // NPCs/strangers) and persist the whole map when anything moved. The
    // write happens after the lock is released — disk must not stall the
    // event ingest sharing this mutex.
    let deltas = agent_resp.favor_deltas();
    if !deltas.is_empty() {
        let mut s = state.lock().await;
        let mut changed = false;
        for (name, delta) in &deltas {
            changed |= s.apply_favor(name, *delta);
        }
        if changed {
            if let Some(path) = favor_file {
                let json = serde_json::to_string_pretty(&s.favor).expect("favor map serializes");
                drop(s);
                if let Err(e) = std::fs::write(path, json) {
                    warn!("Failed to write favor file {path}: {e}");
                } else {
                    info!("Favor updated: {path}");
                }
            }
        }
    }

    let mut last_attack_target = None;
    // Units of each bag instance already given away this turn: the bag
    // snapshot only refreshes on InventoryUpdated, so without this a second
    // sell would resend an instance the server has already emptied.
    let mut spent_units: HashMap<u64, u32> = HashMap::new();

    // Holding position means holding it: a follow started before the trade
    // window opened would keep walking us away from the customer.
    if skip_movement {
        let mut s = state.lock().await;
        if let Some(name) = s.cancel_follow() {
            info!("Follow of {name} cancelled — NPC is holding position");
        }
    }

    let mut pending: Option<(&AgentAction, crate::state::ActionProgress)> = None;
    let mut lost_position: Option<&str> = None;
    let mut skipped: Vec<&str> = Vec::new();

    for action in &agent_resp.actions {
        if let Some((prev, mark)) = pending.take() {
            if settle_action(state, prev, mark).await == ActionOutcome::Failed
                && prev.takes_over_movement()
            {
                warn!("Position lost by a failed {}", prev.label());
                lost_position = lost_position.or(Some(prev.label()));
            }
        }

        if lost_position.is_some() && action.needs_position() {
            skipped.push(action.label());
            continue;
        }

        // Skip movement/attack when the NPC must stay put — resting on a
        // scheduled object, or serving a customer with an open trade window.
        if skip_movement && action.blocked_while_holding_position() {
            debug!("Skipping {:?} action — NPC is holding position", action);
            let mut s = state.lock().await;
            s.push_agent_event(format!(
                "[ActionFailed] You are holding position right now, so your {} did not \
                 run. Finish what you are doing first.",
                action.label()
            ));
            continue;
        }

        pending = Some((action, state.lock().await.action_progress()));

        // A running follow yields, or the two tasks fight over the same body.
        if action.takes_over_movement() {
            let mut s = state.lock().await;
            if let Some(name) = s.cancel_follow() {
                info!("Follow of {name} cancelled by a new movement action");
            }
        }

        // Keep following a character until something else takes over.
        if let AgentAction::Follow { target, sprint } = action {
            let name = target.trim();
            let target_id = {
                let mut s = state.lock().await;
                match s.resolve_nearby_player(name) {
                    Some((id, _)) => id,
                    None => {
                        warn!("follow: no nearby character named '{name}'");
                        s.push_agent_event(format!(
                            "[MoveFailed] No character named '{name}' is nearby to follow."
                        ));
                        continue;
                    }
                }
            };
            let name = name.to_string();
            let task_state = Arc::clone(state);
            let task_name = name.clone();
            let sprint = *sprint;
            let handle = tokio::spawn(async move {
                loop {
                    let before = target_pos(&task_state, &target_id).await;
                    let ended = match follow_player(&task_state, &target_id, sprint).await {
                        ChaseResult::InRange => {
                            tokio::time::sleep(FOLLOW_RECHECK).await;
                            None
                        }
                        // A chase that runs out of time while the target keeps
                        // walking is following working, not failing — only a
                        // deadline against a target that never moved is stuck.
                        ChaseResult::Lost(LostReason::Timeout | LostReason::Desynced)
                            if target_moved(&task_state, &target_id, before).await =>
                        {
                            None
                        }
                        ChaseResult::Lost(loss) => Some(format!(
                            "[FollowEnded] You are no longer following {task_name} — {}.",
                            loss.clause()
                        )),
                        ChaseResult::Error => Some(format!(
                            "[FollowEnded] Something went wrong while following {task_name}."
                        )),
                    };
                    if let Some(note) = ended {
                        // Ambient: this task outlives the turn, and its note
                        // must not settle whatever action is running by then.
                        task_state.lock().await.push_ambient_event(note);
                        break;
                    }
                }
            });
            let mut s = state.lock().await;
            s.follow_task = Some((name.clone(), handle));
            s.push_agent_event(format!(
                "[Following] You are now following {name}. Any other action that \
                 takes your body over — a move, attack, pickup, chest, trade or \
                 fishing — stops the follow."
            ));
            continue;
        }

        // For attack actions, chase the monster and attack
        if let AgentAction::Attack { monster_id, sprint } = action {
            info!("Agent attacking monster {monster_id}, chasing...");
            match chase_monster(state, monster_id, *sprint).await {
                ChaseResult::InRange => {
                    // Face the monster before attacking
                    let mut s = state.lock().await;
                    if let Some(face_cmd) = s.face_monster_command(monster_id) {
                        if let Err(e) = s.send_command(face_cmd).await {
                            error!("Failed to send face-monster move: {e}");
                        }
                    }
                }
                ChaseResult::Lost(loss) => {
                    warn!("Could not reach monster {monster_id} ({loss:?}), skipping attack");
                    let mut s = state.lock().await;
                    s.push_agent_event(unreachable_note(monster_id, &loss));
                    continue;
                }
                ChaseResult::Error => {
                    warn!("Error while chasing monster {monster_id}, skipping attack");
                    let mut s = state.lock().await;
                    s.push_agent_event(format!(
                        "[Unreachable] Could not reach monster {monster_id}: something went \
                         wrong on the way."
                    ));
                    continue;
                }
            }
            last_attack_target = Some((monster_id.clone(), *sprint));
        }

        // Haggling: resolve the target player's name to an id and send the
        // offer. The server clamps the modifier and enforces budgets.
        if let AgentAction::OfferDeal {
            player,
            item,
            kind,
            modifier_pct,
            reason,
        } = action
        {
            let mut s = state.lock().await;
            let Some(target_id) = resolve_trade_push_target(&mut s, player, "DealFailed") else {
                continue;
            };
            let kind = match kind.as_deref() {
                Some("sell") => onlinerpg_shared::messages::DealKind::Sell,
                _ => onlinerpg_shared::messages::DealKind::Buy,
            };
            let cmd = onlinerpg_shared::ClientMessage::OfferDeal {
                target_player_id: target_id,
                item_def_id: item.clone(),
                kind,
                modifier_pct: *modifier_pct,
                reason: reason.clone().unwrap_or_default(),
            };
            if let Err(e) = s.send_command(cmd).await {
                error!("Failed to send offer_deal: {e}");
            }
            continue;
        }

        // Trade-window push: resolve the target player's name to an id and
        // ask the server to open our shop on their client. The server
        // validates range and trading capability; failures come back as a
        // TradeError event.
        if let AgentAction::OpenTrade { player } = action {
            let mut s = state.lock().await;
            let Some(target_id) = resolve_trade_push_target(&mut s, player, "TradeFailed") else {
                continue;
            };
            let cmd = onlinerpg_shared::ClientMessage::OpenTrade {
                target_player_id: target_id,
            };
            if let Err(e) = s.send_command(cmd).await {
                error!("Failed to send open_trade: {e}");
            } else {
                s.push_agent_event(format!(
                    "[OpenTrade] You asked the server to open your trade window for {player} — \
                     it only appears on their screen if they are within a few meters and accept."
                ));
            }
            continue;
        }

        // Party answers need the stored invite: the inviter may be outside
        // the AOI, where name resolution finds nobody.
        if let AgentAction::PartyAccept { player } | AgentAction::PartyDecline { player } = action {
            let accept = matches!(action, AgentAction::PartyAccept { .. });
            let mut s = state.lock().await;
            s.prune_expired_party_invites();
            let idx = pick_pending(&s.pending_party_invites, player.as_deref(), |i| {
                &i.inviter_name
            });
            let Some(idx) = idx else {
                s.push_agent_event("[PartyFailed] No pending party invite to answer.".to_string());
                continue;
            };
            let invite = s.pending_party_invites.remove(idx);
            let cmd = onlinerpg_shared::ClientMessage::PartyRespond {
                inviter_id: invite.inviter_id,
                accept,
            };
            if let Err(e) = s.send_command(cmd).await {
                error!("Failed to send party response: {e}");
            } else if !accept {
                s.push_agent_event(format!(
                    "[Party] You declined {}'s invite.",
                    invite.inviter_name
                ));
            }
            continue;
        }

        // Summon answers likewise need the stored request. An accept keeps
        // the entry — retryable after a mid-combat refusal; the teleport (or
        // the TTL) clears it.
        if let AgentAction::SummonAccept { player } | AgentAction::SummonDecline { player } = action
        {
            let accept = matches!(action, AgentAction::SummonAccept { .. });
            let mut s = state.lock().await;
            s.prune_expired_party_summons();
            let idx = pick_pending(&s.pending_party_summons, player.as_deref(), |i| {
                &i.caster_name
            });
            let Some(idx) = idx else {
                s.push_agent_event("[SummonFailed] No pending summons to answer.".to_string());
                continue;
            };
            let caster_id = s.pending_party_summons[idx].caster_id;
            let caster_name = s.pending_party_summons[idx].caster_name.clone();
            if !accept {
                s.pending_party_summons.remove(idx);
            }
            let cmd = onlinerpg_shared::ClientMessage::PartySummonRespond { caster_id, accept };
            if let Err(e) = s.send_command(cmd).await {
                error!("Failed to send summon response: {e}");
            } else if !accept {
                s.push_agent_event(format!("[Party] You declined {caster_name}'s summons."));
            }
            continue;
        }

        // Kick and promote need the party roster: resolve the named member
        // and check we actually lead before bothering the server.
        if let AgentAction::PartyKick { player } | AgentAction::PartyPromote { player } = action {
            let kick = matches!(action, AgentAction::PartyKick { .. });
            let verb = if kick { "remove" } else { "promote" };
            let mut s = state.lock().await;
            if s.party_members.is_empty() {
                s.push_agent_event(format!(
                    "[PartyFailed] You are not in a party — nobody to {verb}."
                ));
                continue;
            }
            if s.party_leader != s.self_player_id {
                s.push_agent_event(format!(
                    "[PartyFailed] Only the party leader can {verb} members."
                ));
                continue;
            }
            let member = s
                .party_members
                .iter()
                .find(|m| m.name.eq_ignore_ascii_case(player.trim()));
            let Some(member) = member else {
                let roster = s
                    .party_members
                    .iter()
                    .map(|m| m.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                s.push_agent_event(format!(
                    "[PartyFailed] '{player}' is not in your party. Members: {roster}."
                ));
                continue;
            };
            let (target_id, target_name) = (member.id, member.name.clone());
            if Some(target_id) == s.self_player_id {
                s.push_agent_event(format!(
                    "[PartyFailed] That names yourself — party_leave quits, and you \
                     already lead. Name another member to {verb}."
                ));
                continue;
            }
            let cmd = if kick {
                onlinerpg_shared::ClientMessage::PartyKick { target_id }
            } else {
                onlinerpg_shared::ClientMessage::PartyPromote { target_id }
            };
            if let Err(e) = s.send_command(cmd).await {
                error!("Failed to send party {verb}: {e}");
            } else if kick {
                s.push_agent_event(format!("[Party] You removed {target_name} from the party."));
            } else {
                s.push_agent_event(format!("[Party] {target_name} now leads the party."));
            }
            continue;
        }

        // Friend answers need the stored request: the requester may be
        // outside the AOI, like a party inviter.
        if let AgentAction::FriendAccept { player } | AgentAction::FriendDecline { player } = action
        {
            let accept = matches!(action, AgentAction::FriendAccept { .. });
            let mut s = state.lock().await;
            s.prune_expired_friend_requests();
            let idx = pick_pending(&s.pending_friend_requests, player.as_deref(), |r| {
                &r.requester_name
            });
            let Some(idx) = idx else {
                s.push_agent_event(
                    "[FriendFailed] No pending friend request to answer.".to_string(),
                );
                continue;
            };
            let request = s.pending_friend_requests.remove(idx);
            let cmd = onlinerpg_shared::ClientMessage::FriendRespond {
                requester_id: request.requester_id,
                accept,
            };
            // Only the decline is ours to report; an accept can still be
            // refused, so its verdict comes from the server (party_accept
            // precedent).
            if let Err(e) = s.send_command(cmd).await {
                error!("Failed to send friend response: {e}");
            } else if !accept {
                s.push_agent_event(format!(
                    "[Friend] You declined {}'s friend request.",
                    request.requester_name
                ));
            }
            continue;
        }

        // Dropping a friend is name-based — the friend may be offline — so
        // check the roster rather than nearby players.
        if let AgentAction::FriendRemove { player } = action {
            let mut s = state.lock().await;
            let name = s
                .friends
                .iter()
                .find(|f| f.name.eq_ignore_ascii_case(player.trim()))
                .map(|f| f.name.clone());
            let Some(name) = name else {
                s.push_agent_event(format!(
                    "[FriendFailed] '{player}' is not on your friend list."
                ));
                continue;
            };
            let cmd = onlinerpg_shared::ClientMessage::FriendRemove { name: name.clone() };
            if let Err(e) = s.send_command(cmd).await {
                error!("Failed to send friend removal: {e}");
            } else {
                s.push_agent_event(format!("[Friend] You dropped {name} as a friend."));
            }
            continue;
        }

        // Tip a performer's hat: resolve the named hat (or the nearest one
        // that is not ours) and let the server judge wallet and distance —
        // its system messages report the outcome either way.
        if let AgentAction::TipHat { hat, amount } = action {
            let mut s = state.lock().await;
            if *amount <= 0 {
                s.push_agent_event(
                    "[TipFailed] \"amount\" must be a positive number of copper.".to_string(),
                );
                continue;
            }
            let hat_id = match hat {
                Some(id) if s.tip_hats.contains_key(id) => Some(*id),
                Some(id) => {
                    s.push_agent_event(format!(
                        "[TipFailed] No tip hat with id {id} is around — the world state \
                         lists the hats you can see."
                    ));
                    continue;
                }
                None => {
                    let me = s.self_player_id;
                    let my_pos = s.self_player.as_ref().map(|p| p.position);
                    s.tip_hats
                        .values()
                        .filter(|h| Some(h.owner) != me)
                        .min_by(|a, b| match my_pos {
                            Some(p) => a
                                .position
                                .dist_xz_sq(&p)
                                .total_cmp(&b.position.dist_xz_sq(&p)),
                            None => std::cmp::Ordering::Equal,
                        })
                        .map(|h| h.id)
                }
            };
            let Some(hat_id) = hat_id else {
                s.push_agent_event("[TipFailed] No tip hat is laid out near you.".to_string());
                continue;
            };
            let cmd = onlinerpg_shared::ClientMessage::TipHat {
                hat_id,
                amount: *amount,
            };
            if let Err(e) = s.send_command(cmd).await {
                error!("Failed to send tip: {e}");
            }
            continue;
        }

        // An order taken: the driver walks the kitchen round trip and sets
        // the plate down, so only the guest and the dish are checked here.
        if let AgentAction::Serve { player, dish } = action {
            let mut s = state.lock().await;
            let Some((guest_id, _)) = s.resolve_nearby_player(player) else {
                s.push_agent_event(format!(
                    "[ServeFailed] No one named '{player}' is nearby; nothing was served."
                ));
                continue;
            };
            let seat = s
                .nearby_players
                .get(&guest_id)
                .filter(|p| p.object_type.as_deref() == Some(crate::state::SIT_OBJECT_TYPE))
                .and_then(|p| p.object_id.map(|id| (id, p.name.clone())));
            let Some((chair_object_id, guest_name)) = seat else {
                s.push_agent_event(format!(
                    "[ServeFailed] {player} is not sitting at a table — ask them to take a \
                     seat first, then serve."
                ));
                continue;
            };
            let Some(dish_id) = crate::item_defs::resolve_dish(dish) else {
                s.push_agent_event(format!(
                    "[ServeFailed] '{dish}' is not on the menu: {}.",
                    crate::item_defs::dish_ids().join(", ")
                ));
                continue;
            };
            s.pending_serve = Some(crate::state::ServeRequest {
                guest_id,
                guest_name: guest_name.clone(),
                chair_object_id,
                dish: dish_id.to_string(),
            });
            s.push_agent_event_quiet(format!(
                "[Serve] You go to fetch the {dish_id} and bring it to {guest_name}'s table \
                 — no move action needed."
            ));
            continue;
        }

        // Wave off a pushed trade window. The offer is stored when ShopState
        // arrives; the server relays the decline to the merchant's agent.
        if matches!(action, AgentAction::DeclineTrade) {
            let mut s = state.lock().await;
            let Some(offer) = s.pushed_trade.take().filter(|t| t.is_live()) else {
                s.push_agent_event(
                    "[TradeFailed] No trade window is open on your screen to decline.".to_string(),
                );
                continue;
            };
            let cmd = onlinerpg_shared::ClientMessage::DeclineTrade {
                merchant_player_id: offer.merchant_id,
            };
            if let Err(e) = s.send_command(cmd).await {
                error!("Failed to send trade decline: {e}");
            } else {
                s.push_agent_event(format!(
                    "[Trade] You waved off {}'s trade window.",
                    offer.merchant_name
                ));
            }
            continue;
        }

        // Use an item: worn gear comes off, anything else is equipped or
        // consumed out of the bag. Lighting a torch is equipping one.
        if let AgentAction::Use { item } = action {
            let mut s = state.lock().await;
            let Some((def_id, placed)) = s.find_carried(item) else {
                warn!("use: nothing carried matching '{item}'");
                s.push_agent_event(format!(
                    "[UseFailed] You are not carrying anything called '{item}'."
                ));
                continue;
            };
            // No TorchToggle: the server derives the light from the off-hand
            // slot, and a refused equip would light us anyway.
            let cmd = match placed {
                Carried::Worn(slot) => onlinerpg_shared::ClientMessage::UnequipItem { slot },
                Carried::InBag(instance_id) => {
                    let def = crate::item_defs::get(&def_id);
                    if def.is_some_and(|d| d.equip_slot.is_some()) {
                        onlinerpg_shared::ClientMessage::EquipItem { instance_id }
                    } else if def.is_some_and(|d| d.is_consumable()) {
                        onlinerpg_shared::ClientMessage::UseItem { instance_id }
                    } else {
                        s.push_agent_event(format!("[UseFailed] {def_id} cannot be worn or used."));
                        continue;
                    }
                }
            };
            if let Err(e) = s.send_command(cmd).await {
                error!("Failed to send use action: {e}");
            }
            continue;
        }

        // Sell one or more units of a bag item: resolve the merchant, walk up
        // to them, and send the sale as one all-or-nothing batch. The server
        // owns pricing, proximity and wallet checks and answers with
        // GoldUpdate/InventoryUpdated.
        if let AgentAction::Sell {
            item,
            merchant,
            qty,
            sprint,
        } = action
        {
            let Some((merchant_id, merchant_name)) =
                reach_merchant(state, "SellFailed", merchant.as_deref(), *sprint).await
            else {
                continue;
            };
            let mut s = state.lock().await;
            let Some(copies) = s.find_carried_bag_copies(item, &spent_units) else {
                s.push_agent_event(format!(
                    "[SellFailed] Nothing called '{item}' is left in your bag."
                ));
                continue;
            };
            let CarriedBagCopies::InBag { def_id, copies } = copies else {
                let CarriedBagCopies::WornOnly { def_id } = copies else {
                    unreachable!("InBag already matched above")
                };
                s.push_agent_event(format!(
                    "[SellFailed] {def_id} is equipped — worn gear is not for sale."
                ));
                continue;
            };
            let lines = match resolve_qty_lines(&copies, qty.as_ref()) {
                Ok(lines) => lines,
                Err(reason) => {
                    s.push_agent_event(format!("[SellFailed] {def_id}: {reason}."));
                    continue;
                }
            };
            let total: u32 = lines.iter().map(|l| l.qty).sum();
            let cmd = onlinerpg_shared::ClientMessage::SellItems {
                merchant_player_id: merchant_id,
                items: lines.clone(),
            };
            if let Err(e) = s.send_command(cmd).await {
                error!("Failed to send sell: {e}");
            } else {
                for line in &lines {
                    *spent_units.entry(line.instance_id).or_default() += line.qty;
                }
                s.clear_pushed_trade(&merchant_id);
                info!(
                    "Agent selling {total}x {def_id} [{} instance(s)] to {merchant_name}",
                    lines.len()
                );
            }
            continue;
        }

        // Drop one or more units of a bag item where we stand, as one
        // all-or-nothing batch. Stricter than the web client: worn gear must
        // be taken off first.
        if let AgentAction::Drop { item, qty } = action {
            let mut s = state.lock().await;
            let Some(copies) = s.find_carried_bag_copies(item, &spent_units) else {
                s.push_agent_event(format!(
                    "[DropFailed] Nothing called '{item}' is left in your bag."
                ));
                continue;
            };
            let CarriedBagCopies::InBag { def_id, copies } = copies else {
                let CarriedBagCopies::WornOnly { def_id } = copies else {
                    unreachable!("InBag already matched above")
                };
                s.push_agent_event(format!(
                    "[DropFailed] {def_id} is equipped — worn gear cannot be dropped."
                ));
                continue;
            };
            let lines = match resolve_qty_lines(&copies, qty.as_ref()) {
                Ok(lines) => lines,
                Err(reason) => {
                    s.push_agent_event(format!("[DropFailed] {def_id}: {reason}."));
                    continue;
                }
            };
            let total: u32 = lines.iter().map(|l| l.qty).sum();
            let cmd = onlinerpg_shared::ClientMessage::DropItems {
                items: lines.clone(),
            };
            if let Err(e) = s.send_command(cmd).await {
                error!("Failed to send drop: {e}");
            } else {
                for line in &lines {
                    *spent_units.entry(line.instance_id).or_default() += line.qty;
                }
                info!(
                    "Agent dropped {total}x {def_id} [{} instance(s)]",
                    lines.len()
                );
            }
            continue;
        }

        // Buy a catalog item: resolve the merchant, walk up to them, and
        // send the purchase. The server owns catalog, pricing and gold
        // checks and answers with GoldUpdate/InventoryUpdated or TradeError.
        if let AgentAction::Buy {
            item,
            merchant,
            sprint,
        } = action
        {
            let Some((merchant_id, merchant_name)) =
                reach_merchant(state, "BuyFailed", merchant.as_deref(), *sprint).await
            else {
                continue;
            };
            let mut s = state.lock().await;
            // Match the LLM's spelling against this shop's shelf, so "sword"
            // cannot land on one they do not stock. A resident sells out of a
            // bag we cannot see, so there every item is the best we have.
            let shelf = crate::shop_info::merchant_shop(&merchant_name)
                .map_or_else(crate::item_defs::all_ids, |(catalog, _)| catalog);
            let def_id = crate::item_defs::resolve_named(&shelf, item)
                .unwrap_or_else(|| item.trim())
                .to_string();
            let cmd = onlinerpg_shared::ClientMessage::BuyItem {
                merchant_player_id: merchant_id,
                item_def_id: def_id,
            };
            if let Err(e) = s.send_command(cmd).await {
                error!("Failed to send buy: {e}");
            } else {
                s.clear_pushed_trade(&merchant_id);
                info!("Agent buying {item} from {merchant_name}");
            }
            continue;
        }

        // Buy back a unit sold to this merchant this session, at the exact
        // payout recorded server-side. Entry list arrives via BuybackUpdated.
        if let AgentAction::Buyback {
            item,
            merchant,
            sprint,
        } = action
        {
            let Some((merchant_id, merchant_name)) =
                reach_merchant(state, "BuybackFailed", merchant.as_deref(), *sprint).await
            else {
                continue;
            };
            let mut s = state.lock().await;
            let entries = s
                .merchant_buyback
                .get(&merchant_id)
                .cloned()
                .unwrap_or_default();
            if entries.is_empty() {
                s.push_agent_event(format!(
                    "[BuybackFailed] Nothing of yours is waiting with {merchant_name} — the \
                     [Buyback] event after a sale lists what they still hold."
                ));
                continue;
            }
            // Match against what they actually hold, not every item in the
            // game — a name that resolves elsewhere is a miss here.
            let held: Vec<&str> = entries.iter().map(|e| e.item_def_id.as_str()).collect();
            let want = crate::item_defs::resolve_named(&held, item)
                .unwrap_or_else(|| item.trim())
                .to_string();
            // Same def, different enchants: take the best one back — the payout
            // was the same, so nothing else tells them apart.
            let Some(entry) = entries
                .iter()
                .filter(|e| e.item_def_id == want)
                .max_by_key(|e| e.enchant)
            else {
                s.push_agent_event(format!(
                    "[BuybackFailed] {merchant_name}'s buyback list has no '{item}' — it holds: {}.",
                    held.join(", ")
                ));
                continue;
            };
            let cmd = onlinerpg_shared::ClientMessage::BuybackItem {
                merchant_player_id: merchant_id,
                entry_id: entry.entry_id,
            };
            if let Err(e) = s.send_command(cmd).await {
                error!("Failed to send buyback: {e}");
            } else {
                s.clear_pushed_trade(&merchant_id);
                info!(
                    "Agent buying back {want} [entry {}] from {merchant_name}",
                    entry.entry_id
                );
            }
            continue;
        }

        // Smash a breakable prop in our own room: cross to the cell beside it
        // and ask the server, which owns the floor, proximity and kind checks.
        // Only a prop we share a room with counts, the way `open_chest` only
        // reaches the chests in sight. Chest props go through that action.
        if let AgentAction::BreakProp { prop_id, sprint } = action {
            let sighted = {
                let s = state.lock().await;
                s.dungeon_here()
                    .zip(
                        s.breakables_in_sight()
                            .into_iter()
                            .find(|b| b.prop_id == *prop_id),
                    )
                    .map(|(d, prop)| (d.id.clone(), s.self_floor_level, prop))
            };
            let Some((entrance_id, floor_level, prop)) = sighted else {
                let mut s = state.lock().await;
                let depth = s.self_floor_level.unsigned_abs();
                let smashed = s
                    .dungeon_here()
                    .is_some_and(|d| s.is_prop_broken(&d.id, depth, *prop_id));
                s.push_agent_event(if smashed {
                    format!("[PropFailed] Prop {prop_id} is already smashed.")
                } else {
                    format!(
                        "[PropFailed] Prop {prop_id} is not a barrel or crate standing in this \
                         room — smash one of the props the world state lists."
                    )
                });
                continue;
            };
            let (pos, floor, range) = approach_beside(&prop.position, prop.approach, floor_level);
            match walk_to_point(state, pos, floor, range, *sprint).await {
                ChaseResult::InRange => {
                    let mut s = state.lock().await;
                    let cmd = onlinerpg_shared::ClientMessage::BreakDungeonProp {
                        entrance_id,
                        depth: floor_level.unsigned_abs(),
                        prop_id: *prop_id,
                    };
                    if let Err(e) = s.send_command(cmd).await {
                        error!("Failed to send break_prop: {e}");
                    } else {
                        info!("Agent requested break on prop {prop_id}");
                    }
                }
                ChaseResult::Lost(loss) => {
                    let mut s = state.lock().await;
                    s.push_agent_event(format!(
                        "[PropFailed] You could not get to prop {prop_id} — {}.",
                        loss.clause()
                    ));
                }
                ChaseResult::Error => {
                    let mut s = state.lock().await;
                    s.push_agent_event(format!(
                        "[PropFailed] You could not get to prop {prop_id}."
                    ));
                }
            }
            continue;
        }

        // Open a chest in our own room: cross to it and ask the server. Only a
        // chest we share a room with counts, so this walks what a web player's
        // click walks and never routes to one the agent has not found.
        if let AgentAction::OpenChest {
            chest: want,
            sprint,
        } = action
        {
            let target = {
                let s = state.lock().await;
                let sighted = s.chests_in_sight();
                pick_chest(&sighted, want.as_deref())
                    .copied()
                    .zip(s.dungeon_here())
                    .map(|(c, d)| (c, d.id.clone(), s.self_floor_level))
            };
            let Some((chest, entrance_id, chest_floor)) = target else {
                let mut s = state.lock().await;
                s.push_agent_event(
                    "[ChestFailed] You see no chest in this room — go and find one.".to_string(),
                );
                continue;
            };
            let (pos, floor, range) = approach_beside(&chest.position, chest.approach, chest_floor);
            match walk_to_point(state, pos, floor, range, *sprint).await {
                ChaseResult::InRange => {
                    let depth = chest_floor.unsigned_abs();
                    let mut s = state.lock().await;
                    s.chest_open_sent(&entrance_id, depth, chest.kind);
                    let cmd = match chest.kind {
                        ChestKind::Treasure => {
                            onlinerpg_shared::ClientMessage::OpenDungeonChest { entrance_id }
                        }
                        ChestKind::Prop(prop_id) => {
                            onlinerpg_shared::ClientMessage::OpenDungeonProp {
                                entrance_id,
                                depth,
                                prop_id,
                            }
                        }
                    };
                    if let Err(e) = s.send_command(cmd).await {
                        error!("Failed to send open_chest: {e}");
                    } else {
                        info!("Agent requested chest open ({:?})", chest.kind);
                    }
                }
                ChaseResult::Lost(loss) => {
                    let mut s = state.lock().await;
                    s.push_agent_event(format!(
                        "[ChestFailed] You did not reach the chest — {}.",
                        loss.clause()
                    ));
                }
                ChaseResult::Error => {
                    let mut s = state.lock().await;
                    s.push_agent_event(
                        "[ChestFailed] You did not reach the chest — something went wrong \
                         on the way."
                            .to_string(),
                    );
                }
            }
            continue;
        }

        // Pick up a ground item: resolve the reference, walk into pickup
        // range, and send the pickup. The server owns the range, floor and
        // weight checks and answers with InventoryUpdated or a SystemMessage.
        if let AgentAction::Pickup { item, sprint } = action {
            let resolved = {
                let s = state.lock().await;
                resolve_ground_item(&s, item)
            };
            let Some((instance_id, def_id)) = resolved else {
                warn!("pickup: nothing in sight matches '{item}'");
                let mut s = state.lock().await;
                s.push_agent_event(format!(
                    "[PickupFailed] No item matching '{item}' is on the ground nearby."
                ));
                continue;
            };
            match walk_to_ground_item(state, instance_id, *sprint).await {
                ChaseResult::InRange => {
                    let mut s = state.lock().await;
                    // The crouch nearby players see from a web client.
                    if let Err(e) = s
                        .send_command(onlinerpg_shared::ClientMessage::PickupStarted)
                        .await
                    {
                        error!("Failed to send pickup animation: {e}");
                    }
                    drop(s);
                    tokio::time::sleep(std::time::Duration::from_millis(PICKUP_GRAB_DELAY_MS))
                        .await;
                    let mut s = state.lock().await;
                    if let Err(e) = s
                        .send_command(onlinerpg_shared::ClientMessage::PickupItem { instance_id })
                        .await
                    {
                        error!("Failed to send pickup: {e}");
                    } else {
                        info!("Agent picked up {def_id} [id {instance_id}]");
                    }
                }
                ChaseResult::Lost(loss) => {
                    warn!("pickup: could not reach {def_id} [id {instance_id}] ({loss:?})");
                    let why = match loss {
                        LostReason::TargetGone => {
                            "it was taken or despawned before you got there".to_string()
                        }
                        other => other.clause(),
                    };
                    let mut s = state.lock().await;
                    s.push_agent_event(format!(
                        "[PickupFailed] You could not reach the {def_id} — {why}."
                    ));
                }
                ChaseResult::Error => {
                    error!("pickup: error while walking to {def_id} [id {instance_id}]");
                    let mut s = state.lock().await;
                    s.push_agent_event(format!(
                        "[PickupFailed] Something went wrong on the way to the {def_id}."
                    ));
                }
            }
            continue;
        }

        // Handle move actions with pathfinding
        if let AgentAction::Move {
            target,
            x,
            y: _,
            z,
            direction,
            distance,
            depth,
            sprint,
        } = action
        {
            let sprint = *sprint;
            // A name resolves across every namespace the world state prints,
            // and that decides which mover runs.
            let named = target.as_deref().map(str::trim).filter(|s| !s.is_empty());
            let resolved = match named {
                Some(name) => {
                    let mut s = state.lock().await;
                    match s.resolve_move_target(name) {
                        Ok(t) => Some(t),
                        Err(e) => {
                            warn!("move: could not resolve target '{name}' ({e:?})");
                            s.push_agent_event(move_target_failure(&e));
                            continue;
                        }
                    }
                }
                None => None,
            };

            // A depth names a dungeon floor: walk to the entrance, then
            // descend to that floor's landing (a cross-floor A* from across
            // the map would blow its node budget). Only explicit coordinates
            // override the landing.
            match (&resolved, depth) {
                (Some(MoveTarget::Dungeon { id, .. }), _) => {
                    let goal = resolve_move_goal(x, z, &None, &None, None).ok();
                    move_to_dungeon_floor(state, *depth, goal, Some(id), sprint).await;
                    continue;
                }
                (None, Some(_)) => {
                    let goal = resolve_move_goal(x, z, &None, &None, None).ok();
                    move_to_dungeon_floor(state, *depth, goal, None, sprint).await;
                    continue;
                }
                (Some(other), Some(d)) => {
                    let mut s = state.lock().await;
                    s.push_agent_event(format!(
                        "[MoveFailed] \"depth\": {d} only goes with a dungeon name, but \
                         '{}' is {}. Move to it without a depth, or name a dungeon.",
                        named.unwrap_or_default(),
                        describe_target(other)
                    ));
                    continue;
                }
                (_, None) => {}
            }

            match resolved {
                // Stop a polite distance short instead of pathing onto their
                // exact position, which would overlap the models.
                Some(MoveTarget::Character { id, name }) => {
                    match approach_player(state, &id, sprint).await {
                        ChaseResult::InRange => {
                            info!("Agent walked up to {name}");
                            let mut s = state.lock().await;
                            if let Some(face_cmd) = s.face_player_command(&id) {
                                if let Err(e) = s.send_command(face_cmd).await {
                                    error!("Failed to send face-character move: {e}");
                                }
                            }
                            s.push_agent_event(format!(
                                "[Arrived] You walked up to character {} ({name}) and now stand \
                                 right next to them. No further movement is needed to interact.",
                                id.get()
                            ));
                        }
                        ChaseResult::Lost(loss) => {
                            warn!("move: could not reach '{name}' ({loss:?})");
                            let why = match loss {
                                LostReason::TargetGone => "they moved out of sight".to_string(),
                                LostReason::TooFar(d) => {
                                    format!("they are {d:.0}m away, too far to follow")
                                }
                                other => other.clause(),
                            };
                            let mut s = state.lock().await;
                            s.push_agent_event(format!(
                                "[MoveFailed] You could not reach {name} — {why}."
                            ));
                        }
                        ChaseResult::Error => {
                            error!("move: error while approaching '{name}'");
                            let mut s = state.lock().await;
                            s.push_agent_event(format!(
                                "[MoveFailed] Something went wrong walking to {name}."
                            ));
                        }
                    }
                }
                // Walking to a monster is the same chase an attack runs, minus
                // the swing: it closes the gap so the next attack lands.
                Some(MoveTarget::Monster { id }) => match chase_monster(state, &id, sprint).await {
                    ChaseResult::InRange => {
                        info!("Agent walked up to monster {id}");
                        let mut s = state.lock().await;
                        if let Some(face_cmd) = s.face_monster_command(&id) {
                            if let Err(e) = s.send_command(face_cmd).await {
                                error!("Failed to send face-monster move: {e}");
                            }
                        }
                        s.push_agent_event(format!(
                            "[Arrived] You stand within striking distance of {id}. \
                             Attack it with {{\"type\": \"attack\", \"target\": \"{id}\"}}."
                        ));
                    }
                    ChaseResult::Lost(loss) => {
                        warn!("move: could not reach monster {id} ({loss:?})");
                        let mut s = state.lock().await;
                        s.push_agent_event(unreachable_note(&id, &loss));
                    }
                    ChaseResult::Error => {
                        error!("move: error while chasing monster {id}");
                        let mut s = state.lock().await;
                        s.push_agent_event(format!(
                            "[MoveFailed] Something went wrong walking to {id}."
                        ));
                    }
                },
                Some(MoveTarget::GroundItem { instance_id, name }) => {
                    match walk_to_ground_item(state, instance_id, sprint).await {
                        ChaseResult::InRange => {
                            info!("Agent walked to ground item {name} [{instance_id}]");
                            let mut s = state.lock().await;
                            s.push_agent_event(format!(
                                "[Arrived] You stand over ground item {instance_id} ({name}). \
                                 Take it with {{\"type\": \"pickup\", \"target\": \
                                 {instance_id}}}."
                            ));
                        }
                        ChaseResult::Lost(loss) => {
                            let mut s = state.lock().await;
                            s.push_agent_event(format!(
                                "[MoveFailed] You did not reach {name} [id {instance_id}] — {}.",
                                loss.clause()
                            ));
                        }
                        ChaseResult::Error => {
                            let mut s = state.lock().await;
                            s.push_agent_event(format!(
                                "[MoveFailed] Something went wrong walking to {name}."
                            ));
                        }
                    }
                }
                Some(MoveTarget::Prop { prop_id }) => {
                    walk_to_sighting(
                        state,
                        prop_approach(state, prop_id).await,
                        &format!("prop {prop_id}"),
                        sprint,
                    )
                    .await;
                }
                Some(MoveTarget::Chest { selector }) => {
                    match chest_approach(state, &selector).await {
                        Some((approach, label)) => {
                            walk_to_sighting(state, Some(approach), label, sprint).await;
                        }
                        None => {
                            walk_to_sighting(state, None, "the selected chest", sprint).await;
                        }
                    }
                }
                Some(MoveTarget::Dungeon { .. }) => unreachable!("handled above"),
                None => {
                    // A bare coordinate carries no floor, so stay on the one
                    // we're on rather than dropping to the ground floor.
                    let (goal, floor) = {
                        let s = state.lock().await;
                        let pp = s.self_player.as_ref().map(|p| &p.position);
                        (
                            resolve_move_goal(x, z, direction, distance, pp),
                            s.passability_floor(),
                        )
                    };
                    match goal {
                        Ok((gx, gz)) => match execute_move(state, gx, gz, floor, sprint).await {
                            MoveResult::Arrived => {
                                info!("Agent arrived at ({gx:.1}, {gz:.1})");
                                let mut s = state.lock().await;
                                s.push_agent_event(format!(
                                    "[Arrived] You reached ({gx:.1}, {gz:.1}). Look around and \
                                     decide your next move."
                                ));
                            }
                            MoveResult::Blocked => {
                                warn!("Path blocked to ({gx:.1}, {gz:.1})");
                                let mut s = state.lock().await;
                                s.push_agent_event(format!(
                                    "[MoveFailed] No route to ({gx:.1}, {gz:.1}) — a wall or a \
                                     shut door stands in the way. Try a different goal."
                                ));
                            }
                            MoveResult::Died => warn!("Died on the way to ({gx:.1}, {gz:.1})"),
                            MoveResult::Error => {
                                error!("Move error to ({gx:.1}, {gz:.1})");
                                let mut s = state.lock().await;
                                s.push_agent_event(format!(
                                    "[MoveFailed] Something went wrong on the way to \
                                     ({gx:.1}, {gz:.1})."
                                ));
                            }
                        },
                        Err(GoalError::BadDirection(dir)) => {
                            warn!("move: '{dir}' is not a direction");
                            let mut s = state.lock().await;
                            s.push_agent_event(format!(
                                "[MoveFailed] '{dir}' is not a direction, so you stayed put. \
                                 Use north, south, east, west, northeast, northwest, \
                                 southeast or southwest."
                            ));
                        }
                        Err(GoalError::NoGoal) => {
                            warn!("move: no usable goal (x={x:?} z={z:?} dir={direction:?} dist={distance:?})");
                            let mut s = state.lock().await;
                            s.push_agent_event(
                                "[MoveFailed] That move had no usable goal — give a target, \
                                 both x and z, or a direction with a distance."
                                    .to_string(),
                            );
                        }
                    }
                }
            }
            continue;
        }

        {
            let mut s = state.lock().await;
            if let AgentAction::Say { message } = action {
                if s.refuses_play_command(message) {
                    continue;
                }
            }
            if let AgentAction::Recite { verses } = action {
                match s.begin_recital(verses) {
                    Ok(()) => s.recited_this_turn = true,
                    Err(reason) => s.push_agent_event(format!("[ActionFailed] recite: {reason}")),
                }
                continue;
            }
            // The server answers a friendless player with nothing at all, so
            // asking would leave the LLM waiting on an event that never comes.
            if matches!(action, AgentAction::FriendsOnline) && s.friends.is_empty() {
                s.push_agent_event(
                    "[FriendsOnline] Your friend list is empty — nobody to check on.".to_string(),
                );
                continue;
            }
            let player_pos = s.self_player.as_ref().map(|p| &p.position).cloned();
            if let Some(cmd) = action_to_command(action, player_pos.as_ref()) {
                if let Err(e) = s.send_command(cmd).await {
                    error!("Failed to send agent command: {e}");
                }
            }
        }
    }

    if let Some((prev, mark)) = pending.take() {
        settle_action(state, prev, mark).await;
    }

    if let Some(failed) = lost_position.filter(|_| !skipped.is_empty()) {
        let mut s = state.lock().await;
        s.push_agent_event(format!(
            "[Aborted] Your {failed} failed, so the rest of the turn that needed you to be \
             somewhere did not run: {}. Reissue what still applies.",
            skipped.join(", ")
        ));
    }

    last_attack_target
}

/// Resolve the player a trade push (offer_deal / open_trade) names,
/// applying the shared gates: they must be nearby, human (the server
/// rejects NPC targets anyway; refusing here keeps a false exchange out of
/// the LLM's context), and outside the decline cooldown. Pushes the
/// failure event under `tag`; `None` means the action is done.
fn resolve_trade_push_target(
    s: &mut SharedState,
    player: &str,
    tag: &str,
) -> Option<onlinerpg_shared::PlayerId> {
    let Some((target_id, target_is_official_npc)) = s.resolve_nearby_player(player) else {
        warn!("{tag}: no nearby player named '{player}'");
        s.push_agent_event(format!(
            "[{tag}] No player named '{player}' is nearby; nothing was sent."
        ));
        return None;
    };
    if target_is_official_npc {
        s.push_agent_event(format!(
            "[{tag}] {player} is an NPC — trade offers and windows are for player \
             travelers only. Drop the subject."
        ));
        return None;
    }
    if s.trade_offer_blocked(&target_id) {
        s.push_agent_event(format!(
            "[{tag}] {player} waved off your trade window not long ago — no trade \
             pushes at them for now; just talk."
        ));
        return None;
    }
    Some(target_id)
}

/// Resolve the trader an LLM action named and walk up to them, answering with
/// their id and registry name. Trading is NPC-only server-side ("That
/// character is not a trader"), so a fellow traveler is refused here instead
/// of after a round trip. `tag` labels the failure event; `None` means the
/// event is already pushed and the action is done.
async fn reach_merchant(
    state: &Arc<Mutex<SharedState>>,
    tag: &str,
    merchant: Option<&str>,
    sprint: Option<bool>,
) -> Option<(onlinerpg_shared::PlayerId, String)> {
    let resolved = {
        let mut s = state.lock().await;
        let id = match merchant {
            // Named merchant: resolve by name.
            Some(name) => match s.resolve_nearby_player(name) {
                Some((id, true)) => Some(id),
                Some((_, false)) => {
                    s.push_agent_event(format!(
                        "[{tag}] {name} is a fellow traveler, not a shopkeeper — trading is \
                         with NPC merchants only."
                    ));
                    None
                }
                None => {
                    s.push_agent_event(format!("[{tag}] No one named '{name}' is nearby."));
                    None
                }
            },
            // No merchant given: aim at the nearest NPC merchant in range.
            None => {
                let nearest = s.nearest_merchant();
                if nearest.is_none() {
                    s.push_agent_event(format!(
                        "[{tag}] No merchant is nearby — walk to one first."
                    ));
                }
                nearest
            }
        };
        id.map(|id| (id, super::prompt::player_name(&s, &id)))
    };
    let (id, name) = resolved?;
    match approach_player(state, &id, sprint).await {
        ChaseResult::InRange => Some((id, name)),
        ChaseResult::Lost(loss) => {
            let mut s = state.lock().await;
            s.push_agent_event(format!(
                "[{tag}] You could not reach {name} — {}.",
                loss.clause()
            ));
            None
        }
        ChaseResult::Error => {
            let mut s = state.lock().await;
            s.push_agent_event(format!("[{tag}] You could not reach {name}."));
            None
        }
    }
}

/// Walk to a dungeon floor. `depth` counts downward from the surface (1 is the
/// first floor below ground, 0 leaves the dungeon); the sign the LLM writes is
/// ignored. Runs in two legs — surface approach, then descent — because a
/// cross-floor A* started from across the map exhausts its node budget long
/// before it reaches the stairs.
async fn move_to_dungeon_floor(
    state: &Arc<Mutex<SharedState>>,
    depth: Option<i32>,
    goal_xz: Option<(f32, f32)>,
    named: Option<&str>,
    sprint: Option<bool>,
) {
    // No depth means "walk to that entrance and stop there", which only a
    // named dungeon can ask for.
    let depth = depth.map(|d| d.unsigned_abs().min(u8::MAX as u32) as u8);

    let (dungeon, outside, floor_level) = {
        let mut s = state.lock().await;
        let Some(position) = s.self_player.as_ref().map(|p| p.position) else {
            return;
        };
        if depth == Some(0) && s.self_floor_level >= 0 {
            s.push_agent_event(
                "[Arrived] You are already above ground — there is no dungeon to leave."
                    .to_string(),
            );
            return;
        }
        let dungeon = {
            let world = s.world_cache.read().unwrap();
            match named {
                Some(id) => world.dungeon_by_id(id),
                None => world
                    .dungeon_at(position.x, position.z)
                    .or_else(|| world.nearest_dungeon(position.x, position.z)),
            }
        };
        let Some(dungeon) = dungeon else {
            s.push_agent_event("[MoveFailed] There is no dungeon here to go into.".to_string());
            return;
        };
        if depth.is_some_and(|d| d > dungeon.max_depth()) {
            s.push_agent_event(format!(
                "[MoveFailed] {} only goes down to floor {}.",
                dungeon.name,
                dungeon.max_depth()
            ));
            return;
        }
        s.request_dungeon_doors_here();
        let outside = !dungeon.footprint_contains(position.x, position.z);
        (dungeon, outside, s.self_floor_level)
    };

    // A bare "go to X" from underground asks to be at X, and we already are;
    // walking the surface grid from three floors down would report the
    // opposite of what happened.
    if depth.is_none() && floor_level < 0 {
        let mut s = state.lock().await;
        s.push_agent_event(if outside {
            format!(
                "[MoveFailed] You are on floor {} of another dungeon. Come back up with \
                 {{\"type\": \"move\", \"depth\": 0}} before heading for {}.",
                floor_level.abs(),
                dungeon.name
            )
        } else {
            format!(
                "[Arrived] You are already inside {}, on floor {}. Name a \"depth\" to change \
                 floor, or 0 to leave.",
                dungeon.name,
                floor_level.abs()
            )
        });
        return;
    }

    // Leg 1: get onto the dungeon's own grid on the surface.
    if outside || depth.is_none_or(|d| d == 0) {
        let entrance = dungeon.entrance;
        match execute_move(state, entrance.x, entrance.z, 0, sprint).await {
            MoveResult::Blocked => {
                warn!("Could not reach the {} entrance", dungeon.name);
                let mut s = state.lock().await;
                s.push_agent_event(format!(
                    "[MoveFailed] You could not reach the entrance of {}.",
                    dungeon.name
                ));
                return;
            }
            MoveResult::Died => return,
            MoveResult::Arrived | MoveResult::Error => {}
        }
        state.lock().await.request_dungeon_doors_here();
        match depth {
            Some(0) => {
                info!("Agent left {}", dungeon.name);
                let mut s = state.lock().await;
                s.push_agent_event(format!("[Arrived] You are back outside {}.", dungeon.name));
                return;
            }
            None => {
                info!("Agent reached the {} entrance", dungeon.name);
                let mut s = state.lock().await;
                s.push_agent_event(format!(
                    "[Arrived] You stand at the entrance of {} ({} floors deep). Go in with \
                     {{\"type\": \"move\", \"target\": \"{}\", \"depth\": 1}}.",
                    dungeon.name,
                    dungeon.max_depth(),
                    dungeon.name
                ));
                return;
            }
            Some(_) => {}
        }
    }
    let depth = depth.unwrap_or(1);

    // Leg 2: descend. The shared A* walks the stair shafts on its own; the
    // mover opens whatever doors stand in the way.
    let Some(landing) = dungeon.arrival_position(depth) else {
        return;
    };
    let (gx, gz) = goal_xz.unwrap_or((landing.x, landing.z));
    let floor = dungeon.passability_floor(depth);
    match execute_move(state, gx, gz, floor, sprint).await {
        MoveResult::Arrived => {
            info!("Agent reached {} floor {depth}", dungeon.name);
            let mut s = state.lock().await;
            s.push_agent_event(format!(
                "[Arrived] You are on floor {depth} of {}.",
                dungeon.name
            ));
        }
        MoveResult::Blocked => {
            warn!("Descent to {} floor {depth} blocked", dungeon.name);
            let mut s = state.lock().await;
            s.push_agent_event(format!(
                "[MoveFailed] You could not get to floor {depth} of {} — the way is \
                 sealed. Try a floor closer to where you are.",
                dungeon.name
            ));
        }
        MoveResult::Died => {}
        MoveResult::Error => {
            error!("Descent to {} floor {depth} errored", dungeon.name);
            let mut s = state.lock().await;
            s.push_agent_event(format!(
                "[MoveFailed] Something went wrong on the way to floor {depth} of {}.",
                dungeon.name
            ));
        }
    }
}

/// Resolve a pickup reference to an item the agent can actually walk to:
/// by instance id, or by name to the nearest match. Only what the agent can
/// see counts: an id it read several turns ago may have fallen out of sight
/// since, and a pickup it cannot perceive is one it never learns about.
fn resolve_ground_item(s: &SharedState, r: &PickupRef) -> Option<(u64, String)> {
    match r.as_id() {
        Some(id) => {
            let in_sight = s.ground_items_in_sight();
            let (_, item) = in_sight.iter().find(|(_, i)| i.instance_id == id)?;
            Some((item.instance_id, item.item_def_id.clone()))
        }
        None => s.ground_item_named(&r.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::tests::{ground_item, test_player, test_state};
    use crate::state::NPC_SIGHT_RADIUS;
    use onlinerpg_shared::inventory::GroundItem;

    /// Resolution only reads state, so the dropped command receiver is fine.
    fn state_with(items: Vec<GroundItem>) -> SharedState {
        let (mut s, _rx) = test_state();
        s.self_player = Some(test_player(0.0, 0.0));
        for item in items {
            s.remember_ground_item(item);
        }
        s
    }

    /// Keep the returned `RunDir` alive — dropping it deletes the file.
    fn temp_memory_file() -> (crate::driver::RunDir, String) {
        let dir = crate::driver::RunDir::create().unwrap();
        let path = dir.path().join("memory.txt").to_str().unwrap().to_string();
        (dir, path)
    }

    #[test]
    fn append_memory_skips_lines_already_in_the_file() {
        let (_dir, path) = temp_memory_file();
        append_memory(&path, "jake1 speaks Korean.\nRica is a trader.");
        append_memory(&path, "jake1 speaks Korean.\njake1 sold me a torch.");
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            content.lines().collect::<Vec<_>>(),
            vec![
                "jake1 speaks Korean.",
                "Rica is a trader.",
                "jake1 sold me a torch."
            ]
        );
    }

    #[test]
    fn append_memory_dedups_within_one_update_and_skips_all_duplicate_updates() {
        let (_dir, path) = temp_memory_file();
        append_memory(&path, "a\na\nb");
        let after_first = std::fs::read_to_string(&path).unwrap();
        append_memory(&path, "  a  \nb");
        let after_second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after_first, after_second);
        assert_eq!(after_second.lines().collect::<Vec<_>>(), vec!["a", "b"]);
    }

    #[test]
    fn append_memory_caps_the_file_at_the_newest_lines() {
        let (_dir, path) = temp_memory_file();
        let bulk: Vec<String> = (0..MEMORY_LINES).map(|i| format!("note {i}")).collect();
        append_memory(&path, &bulk.join("\n"));
        append_memory(&path, "the newest note");
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), MEMORY_LINES);
        assert_eq!(lines.first(), Some(&"note 1"));
        assert_eq!(lines.last(), Some(&"the newest note"));
    }

    #[test]
    fn resolve_qty_lines_defaults_to_one_unit_from_the_first_copy() {
        let lines = resolve_qty_lines(&[(1, 3), (2, 2)], None).unwrap();
        assert_eq!(
            lines,
            vec![BagLineItem {
                instance_id: 1,
                qty: 1
            }]
        );
    }

    #[test]
    fn resolve_qty_lines_spans_several_copies_when_one_is_not_enough() {
        // Two separately-picked-up non-stackable copies (qty 1 each) plus a
        // fragmented stack — asking for 3 should drain the first two copies
        // fully before touching the third.
        let lines = resolve_qty_lines(&[(1, 1), (2, 1), (3, 5)], Some(&Qty::Count(3))).unwrap();
        assert_eq!(
            lines,
            vec![
                BagLineItem {
                    instance_id: 1,
                    qty: 1
                },
                BagLineItem {
                    instance_id: 2,
                    qty: 1
                },
                BagLineItem {
                    instance_id: 3,
                    qty: 1
                },
            ]
        );
    }

    #[test]
    fn resolve_qty_lines_all_takes_every_copy_in_full() {
        let lines =
            resolve_qty_lines(&[(1, 3), (2, 2)], Some(&Qty::Named("all".to_string()))).unwrap();
        assert_eq!(
            lines,
            vec![
                BagLineItem {
                    instance_id: 1,
                    qty: 3
                },
                BagLineItem {
                    instance_id: 2,
                    qty: 2
                },
            ]
        );
    }

    #[test]
    fn resolve_qty_lines_rejects_more_than_is_available() {
        let err = resolve_qty_lines(&[(1, 3), (2, 2)], Some(&Qty::Count(10))).unwrap_err();
        assert_eq!(err, "only 5 left, not 10");
    }

    #[test]
    fn resolve_qty_lines_rejects_a_bad_qty_value() {
        let err = resolve_qty_lines(&[(1, 3)], Some(&Qty::Count(0))).unwrap_err();
        assert!(err.contains("positive number"), "{err}");

        let err = resolve_qty_lines(&[(1, 3)], Some(&Qty::Named("half".to_string()))).unwrap_err();
        assert!(err.contains("positive number"), "{err}");
    }

    #[test]
    fn resolves_by_instance_id_however_the_llm_spells_it() {
        let s = state_with(vec![ground_item(9215, "goblin_sword", 2.0, 0.0, 0)]);

        for r in [PickupRef::Id(9215), PickupRef::Name("9215".to_string())] {
            assert_eq!(
                resolve_ground_item(&s, &r),
                Some((9215, "goblin_sword".to_string())),
                "for {r}"
            );
        }
    }

    /// Names resolve the way `use` resolves them — a partial id, or the
    /// display name from items.json — and a loose match takes the nearest.
    #[test]
    fn resolves_a_name_the_same_way_the_use_action_does() {
        let s = state_with(vec![
            ground_item(1, "iron_sword", 6.0, 0.0, 0),
            ground_item(2, "small_sword", 3.0, 0.0, 0),
            ground_item(3, "wooden_shield", 1.0, 0.0, 0),
        ]);

        for asked in ["Sword", "sword"] {
            assert_eq!(
                resolve_ground_item(&s, &PickupRef::Name(asked.to_string())),
                Some((2, "small_sword".to_string())),
                "for {asked}"
            );
        }
        // An exact display name wins over the nearer loose match.
        assert_eq!(
            resolve_ground_item(&s, &PickupRef::Name("Iron Sword".to_string())),
            Some((1, "iron_sword".to_string()))
        );
    }

    /// A too-far monster, a walled-off one, and a vanished one produce
    /// different guidance: walk up to the first, retarget off the other two
    /// instead of re-attacking.
    #[test]
    fn an_unreachable_attack_tells_the_agent_why() {
        let far = unreachable_note("m19_21", &LostReason::TooFar(22.8));
        assert!(far.contains("m19_21"));
        assert!(far.contains("23m"));
        assert!(far.to_lowercase().contains("closer"));

        let walled = unreachable_note("m7_3", &LostReason::NoPath);
        assert!(walled.contains("m7_3"));
        assert!(walled.to_lowercase().contains("no route"));
        assert!(!walled.to_lowercase().contains("closer"));

        let gone = unreachable_note("m5_2", &LostReason::TargetGone);
        assert!(gone.contains("m5_2"));
        assert!(gone.to_lowercase().contains("gone"));
    }

    /// The item map reaches further than both the world state and the chase,
    /// so a stale id the agent read turns ago is refused instead of walked
    /// at — and refused the same way a nonexistent one is.
    #[test]
    fn refuses_items_outside_perception() {
        let s = state_with(vec![
            ground_item(1, "iron_sword", NPC_SIGHT_RADIUS + 5.0, 0.0, 0),
            ground_item(2, "healing_potion", 3.0, 0.0, -1),
        ]);

        for r in [
            PickupRef::Id(1),
            PickupRef::Id(2),
            PickupRef::Id(42),
            PickupRef::Name("sword".to_string()),
            PickupRef::Name("potion".to_string()),
        ] {
            assert!(resolve_ground_item(&s, &r).is_none(), "for {r}");
        }
    }

    #[test]
    fn failure_is_read_off_the_event_tag() {
        for failure in [
            "[MoveFailed] No route to (1.0, 2.0).",
            "[ChestFailed] You see no chest.",
            "[PropFailed] Prop 3 is already smashed.",
            "[ActionFailed] 'dance' is not an action type.",
            "[NoResult] Your use did nothing.",
            "[Unreachable] Monster m2_1 is gone.",
        ] {
            assert!(reports_failure(failure), "{failure}");
        }
        for fine in [
            "[Arrived] You reached (1.0, 2.0).",
            "[Chat] Karl: hello",
            "[Following] You are now following Karl.",
            "no tag at all",
        ] {
            assert!(!reports_failure(fine), "{fine}");
        }
    }

    #[tokio::test]
    async fn an_action_that_left_no_trace_gets_a_no_result() {
        let (s, _rx) = test_state();
        let state = Arc::new(Mutex::new(s));
        let mark = state.lock().await.action_progress();

        let outcome = settle_action(
            &state,
            &AgentAction::Use {
                item: "torch".to_string(),
            },
            mark,
        )
        .await;

        assert_eq!(outcome, ActionOutcome::Failed);
        let events = state.lock().await.drain_agent_events();
        assert!(
            events.iter().any(|e| e.starts_with("[NoResult]")),
            "{events:?}"
        );
    }

    #[tokio::test]
    async fn self_reporting_actions_are_left_alone() {
        let (s, _rx) = test_state();
        let state = Arc::new(Mutex::new(s));
        let mark = state.lock().await.action_progress();

        for action in [
            AgentAction::Say {
                message: "hello".to_string(),
            },
            AgentAction::Wait,
        ] {
            assert_eq!(
                settle_action(&state, &action, mark).await,
                ActionOutcome::Ran
            );
        }
        assert!(state.lock().await.drain_agent_events().is_empty());
    }

    #[tokio::test]
    async fn a_dispatched_command_counts_as_a_result() {
        let (s, _rx) = test_state();
        let state = Arc::new(Mutex::new(s));
        let mark = state.lock().await.action_progress();
        state
            .lock()
            .await
            .send_command(onlinerpg_shared::ClientMessage::PartyLeave)
            .await
            .unwrap();

        let outcome = settle_action(&state, &AgentAction::PartyLeave, mark).await;

        assert_eq!(outcome, ActionOutcome::Ran);
        assert!(state.lock().await.drain_agent_events().is_empty());
    }

    #[tokio::test]
    async fn an_ambient_event_is_not_an_actions_result() {
        let (s, _rx) = test_state();
        let state = Arc::new(Mutex::new(s));
        let mark = state.lock().await.action_progress();
        state
            .lock()
            .await
            .push_ambient_event("[TimeChange] It is now 21:00 (night).".to_string());

        let outcome = settle_action(
            &state,
            &AgentAction::Use {
                item: "torch".to_string(),
            },
            mark,
        )
        .await;

        assert_eq!(outcome, ActionOutcome::Failed);
        let events = state.lock().await.drain_agent_events();
        assert!(
            events.iter().any(|e| e.starts_with("[NoResult]")),
            "{events:?}"
        );
    }

    #[tokio::test]
    async fn background_traffic_is_not_an_actions_result() {
        let (s, _rx) = test_state();
        let state = Arc::new(Mutex::new(s));
        let mark = state.lock().await.action_progress();
        state
            .lock()
            .await
            .send_background_command(onlinerpg_shared::ClientMessage::Heartbeat)
            .await
            .unwrap();

        let outcome = settle_action(
            &state,
            &AgentAction::Use {
                item: "torch".to_string(),
            },
            mark,
        )
        .await;

        assert_eq!(outcome, ActionOutcome::Failed);
        let events = state.lock().await.drain_agent_events();
        assert!(
            events.iter().any(|e| e.starts_with("[NoResult]")),
            "{events:?}"
        );
    }

    #[tokio::test]
    async fn friends_online_answers_an_empty_roster_without_asking() {
        let (s, mut rx) = test_state();
        let state = Arc::new(Mutex::new(s));
        handle_response(
            &state,
            r#"{"actions": [{"type": "friends_online"}]}"#,
            &None,
            &None,
            true,
        )
        .await;

        assert!(rx.try_recv().is_err(), "nothing goes to the server");
        let events = state.lock().await.drain_agent_events();
        assert!(
            events.iter().any(|e| e.starts_with("[FriendsOnline]")),
            "{events:?}"
        );
    }

    /// With someone on the roster the ask goes out as it always did.
    #[tokio::test]
    async fn friends_online_asks_the_server_when_the_roster_has_someone() {
        let (mut s, mut rx) = test_state();
        s.friends = vec![onlinerpg_shared::messages::FriendEntry {
            character_id: 7,
            name: "jake1".to_string(),
            level: 3,
            class: onlinerpg_shared::character::CharacterClass::Knight,
        }];
        let state = Arc::new(Mutex::new(s));
        handle_response(
            &state,
            r#"{"actions": [{"type": "friends_online"}]}"#,
            &None,
            &None,
            true,
        )
        .await;

        assert!(matches!(
            rx.try_recv(),
            Ok(onlinerpg_shared::ClientMessage::RequestFriendsOnline)
        ));
    }
}
