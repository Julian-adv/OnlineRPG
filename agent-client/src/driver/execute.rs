//! Decode an LLM response and run each action against the game server.
//! Returns the monster_id of the last attack action so `llm_driver` can
//! enter its combat loop. Also persists `memory_update` snippets to the
//! NPC's per-instance memory file when configured.

use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::state::{Carried, SharedState};

use super::action::{action_to_command, parse_agent_response, resolve_move_goal, AgentAction};
use super::combat::{approach_player, chase_monster, ChaseResult};
use super::movement::{execute_move, MoveResult};

/// Parse and execute the agent's response.
/// Returns the monster_id if the last action was an attack (for combat loop).
/// If `memory_file` is set and the response contains `memory_update`, appends to file.
pub(super) async fn handle_response(
    state: &Arc<Mutex<SharedState>>,
    response: &str,
    memory_file: &Option<String>,
    skip_movement: bool,
) -> Option<String> {
    let agent_resp = match parse_agent_response(response) {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to parse agent response: {e}");
            warn!("Raw response: {response}");
            return None;
        }
    };

    // Process memory update if present
    if let (Some(ref update), Some(ref path)) = (&agent_resp.memory_update, memory_file) {
        let update = update.trim();
        if !update.is_empty() {
            use std::io::Write;
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                Ok(mut f) => {
                    if let Err(e) = writeln!(f, "\n{update}") {
                        warn!("Failed to write memory update to {path}: {e}");
                    } else {
                        info!("Memory updated: {path} (+{} bytes)", update.len());
                    }
                }
                Err(e) => {
                    warn!("Failed to open memory file {path}: {e}");
                }
            }
        }
    }

    let mut last_attack_target = None;

    for action in &agent_resp.actions {
        // Skip movement/attack when the NPC must stay put — resting on a
        // scheduled object, or serving a customer with an open trade window.
        if skip_movement
            && matches!(
                action,
                AgentAction::Move { .. } | AgentAction::Attack { .. }
            )
        {
            debug!("Skipping {:?} action — NPC is holding position", action);
            continue;
        }

        // For attack actions, chase the monster and attack
        if let AgentAction::Attack { monster_id } = action {
            info!("Agent attacking monster {monster_id}, chasing...");
            match chase_monster(state, monster_id).await {
                ChaseResult::InRange => {
                    // Face the monster before attacking
                    let mut s = state.lock().await;
                    if let Some(face_cmd) = s.face_monster_command(monster_id) {
                        if let Err(e) = s.send_command(face_cmd).await {
                            error!("Failed to send face-monster move: {e}");
                        }
                    }
                }
                ChaseResult::Lost | ChaseResult::Error => {
                    warn!("Could not reach monster {monster_id}, skipping attack");
                    continue;
                }
            }
            last_attack_target = Some(monster_id.clone());
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
            let Some((target_id, target_is_official_npc)) = s.resolve_nearby_player(player) else {
                warn!("offer_deal: no nearby player named '{player}'");
                s.push_agent_event(format!(
                    "[DealFailed] No player named '{player}' is nearby; the offer was not sent."
                ));
                continue;
            };
            // The server rejects NPC targets anyway; refusing here keeps a
            // false "[DealResult]" exchange out of the LLM's context.
            if target_is_official_npc {
                s.push_agent_event(format!(
                    "[DealFailed] {player} is an NPC — deals can only be offered to player \
                     travelers. Drop the subject."
                ));
                continue;
            }
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
            let Some((target_id, target_is_official_npc)) = s.resolve_nearby_player(player) else {
                warn!("open_trade: no nearby player named '{player}'");
                s.push_agent_event(format!(
                    "[TradeFailed] No player named '{player}' is nearby; no trade window was opened."
                ));
                continue;
            };
            // The server rejects NPC targets anyway; refusing here avoids
            // pairing its TradeError with a false success event below.
            if target_is_official_npc {
                s.push_agent_event(format!(
                    "[TradeFailed] {player} is an NPC — trade windows can only be opened for \
                     player travelers. Drop the subject."
                ));
                continue;
            }
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

        // Handle move actions with pathfinding
        if let AgentAction::Move {
            target,
            x,
            y: _,
            z,
            direction,
            distance,
            depth,
        } = action
        {
            // Name-targeted move: walk up to the character and stop a
            // polite distance short instead of pathing onto their exact
            // position (which would overlap the models).
            if let Some(name) = target.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                let target_id = {
                    let mut s = state.lock().await;
                    match s.resolve_nearby_player(name) {
                        Some((id, _)) => id,
                        None => {
                            warn!("move: no nearby character named '{name}'");
                            s.push_agent_event(format!(
                                "[MoveFailed] No character named '{name}' is nearby to walk to."
                            ));
                            continue;
                        }
                    }
                };
                match approach_player(state, &target_id).await {
                    ChaseResult::InRange => {
                        info!("Agent walked up to {name}");
                        let mut s = state.lock().await;
                        if let Some(face_cmd) = s.face_player_command(&target_id) {
                            if let Err(e) = s.send_command(face_cmd).await {
                                error!("Failed to send face-character move: {e}");
                            }
                        }
                        s.push_agent_event(format!(
                            "[Arrived] You walked up to {name} and now stand right next \
                             to them. No further movement is needed to interact."
                        ));
                    }
                    ChaseResult::Lost => {
                        warn!("move: could not reach '{name}'");
                        let mut s = state.lock().await;
                        s.push_agent_event(format!(
                            "[MoveFailed] You could not reach {name} — they moved away \
                             or out of sight."
                        ));
                    }
                    ChaseResult::Error => {
                        error!("move: error while approaching '{name}'");
                    }
                }
                continue;
            }

            // A depth names a dungeon floor: walk to the entrance first (a
            // cross-floor A* from across the map would blow its node budget),
            // then descend to that floor's stair landing.
            if let Some(depth) = depth {
                // Only an explicit coordinate pair overrides the floor's landing;
                // a direction/distance is meaningless across floors.
                let goal = resolve_move_goal(x, z, &None, &None, None);
                move_to_dungeon_floor(state, *depth, goal).await;
                continue;
            }

            // A bare coordinate carries no floor, so stay on the one we're on
            // rather than dropping to the ground floor.
            let (goal, floor) = {
                let s = state.lock().await;
                let pp = s.self_player.as_ref().map(|p| &p.position);
                (
                    resolve_move_goal(x, z, direction, distance, pp),
                    s.passability_floor(),
                )
            };
            if let Some((gx, gz)) = goal {
                match execute_move(state, gx, gz, floor).await {
                    MoveResult::Arrived => {
                        info!("Agent arrived at ({gx:.1}, {gz:.1})");
                    }
                    MoveResult::Blocked => {
                        warn!("Path blocked to ({gx:.1}, {gz:.1})");
                        let mut s = state.lock().await;
                        s.push_agent_event(format!(
                            "[MoveFailed] 이동 실패: ({gx:.1}, {gz:.1})까지의 경로가 건물에 의해 막혀있습니다. 다른 목표를 선택하세요."
                        ));
                    }
                    MoveResult::Error => {
                        error!("Move error to ({gx:.1}, {gz:.1})");
                    }
                }
            }
            continue;
        }

        {
            let mut s = state.lock().await;
            let player_pos = s.self_player.as_ref().map(|p| &p.position).cloned();
            if let Some(cmd) = action_to_command(action, player_pos.as_ref()) {
                if let Err(e) = s.send_command(cmd).await {
                    error!("Failed to send agent command: {e}");
                }
            }
        }
    }

    last_attack_target
}

/// Walk to a dungeon floor. `depth` counts downward from the surface (1 is the
/// first floor below ground, 0 leaves the dungeon); the sign the LLM writes is
/// ignored. Runs in two legs — surface approach, then descent — because a
/// cross-floor A* started from across the map exhausts its node budget long
/// before it reaches the stairs.
async fn move_to_dungeon_floor(
    state: &Arc<Mutex<SharedState>>,
    depth: i32,
    goal_xz: Option<(f32, f32)>,
) {
    let depth = depth.unsigned_abs().min(u8::MAX as u32) as u8;

    let (dungeon, outside) = {
        let mut s = state.lock().await;
        let Some(position) = s.self_player.as_ref().map(|p| p.position) else {
            return;
        };
        if depth == 0 && s.self_floor_level >= 0 {
            debug!("move depth 0: already above ground");
            return;
        }
        let dungeon = {
            let world = s.world_cache.read().unwrap();
            world
                .dungeon_at(position.x, position.z)
                .or_else(|| world.nearest_dungeon(position.x, position.z))
        };
        let Some(dungeon) = dungeon else {
            s.push_agent_event("[MoveFailed] There is no dungeon here to go into.".to_string());
            return;
        };
        if depth > dungeon.max_depth() {
            s.push_agent_event(format!(
                "[MoveFailed] {} only goes down to floor {}.",
                dungeon.name,
                dungeon.max_depth()
            ));
            return;
        }
        s.request_dungeon_doors_here();
        let outside = !dungeon.footprint_contains(position.x, position.z);
        (dungeon, outside)
    };

    // Leg 1: get onto the dungeon's own grid on the surface.
    if outside || depth == 0 {
        let entrance = dungeon.entrance;
        if matches!(
            execute_move(state, entrance.x, entrance.z, 0).await,
            MoveResult::Blocked
        ) {
            warn!("Could not reach the {} entrance", dungeon.name);
            let mut s = state.lock().await;
            s.push_agent_event(format!(
                "[MoveFailed] You could not reach the entrance of {}.",
                dungeon.name
            ));
            return;
        }
        state.lock().await.request_dungeon_doors_here();
        if depth == 0 {
            info!("Agent left {}", dungeon.name);
            return;
        }
    }

    // Leg 2: descend. The shared A* walks the stair shafts on its own; the
    // mover opens whatever doors stand in the way.
    let Some(landing) = dungeon.arrival_position(depth) else {
        return;
    };
    let (gx, gz) = goal_xz.unwrap_or((landing.x, landing.z));
    let floor = dungeon.passability_floor(depth);
    match execute_move(state, gx, gz, floor).await {
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
        MoveResult::Error => error!("Descent to {} floor {depth} errored", dungeon.name),
    }
}
