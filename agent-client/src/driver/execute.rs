//! Decode an LLM response and run each action against the game server.
//! Returns the monster_id of the last attack action so `llm_driver` can
//! enter its combat loop. Also persists `memory_update` snippets to the
//! NPC's per-instance memory file when configured.

use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::dungeon::ChestKind;
use crate::state::{Carried, SharedState};

use super::action::{
    action_to_command, asks_for_great_chest, parse_agent_response, resolve_move_goal, AgentAction,
    PickupRef,
};
use super::combat::{
    approach_player, chase_monster, chest_arrive_range, walk_to_ground_item, walk_to_point,
    ChaseResult,
};
use super::movement::{execute_move, MoveResult};

/// Pause between the crouch broadcast and the actual pickup, approximating
/// the web client's grab moment partway into its pickup animation.
const PICKUP_GRAB_DELAY_MS: u64 = 700;

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
                AgentAction::Move { .. }
                    | AgentAction::Attack { .. }
                    | AgentAction::Pickup { .. }
                    | AgentAction::OpenChest { .. }
                    | AgentAction::Sell { .. }
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

        // Open a chest in our own room: cross to it and ask the server. Only a
        // chest we share a room with counts, so this walks what a web player's
        // click walks and never routes to one the agent has not found.
        if let AgentAction::OpenChest { chest: want } = action {
            let target = {
                let s = state.lock().await;
                let sighted = s.chests_in_sight();
                let pick = if asks_for_great_chest(want.as_deref()) {
                    sighted
                        .iter()
                        .find(|c| c.kind == ChestKind::Treasure)
                        .or(sighted.first())
                } else {
                    sighted.first()
                };
                pick.copied()
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
            // The server measures its range from the chest, so the metre
            // between a prop and the cell we can stand on comes out of ours.
            let gap = crate::geom::PlanarDelta::between(&chest.approach, &chest.position).dist;
            match walk_to_point(state, chest.approach, chest_floor, chest_arrive_range(gap)).await {
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
                ChaseResult::Lost | ChaseResult::Error => {
                    let mut s = state.lock().await;
                    s.push_agent_event(
                        "[ChestFailed] You did not reach the chest — the way is blocked, \
                         it is too far to walk in one go, or you died on the way."
                            .to_string(),
                    );
                }
            }
            continue;
        }

        // Drop a carried item (bag or worn) onto the ground. Pure inventory
        // manipulation like Use — no range check, no walking, so it isn't
        // gated by holding_position/trade_busy either.
        if let AgentAction::Drop { item } = action {
            let mut s = state.lock().await;
            let Some((def_id, placed)) = s.find_carried(item) else {
                warn!("drop: nothing carried matching '{item}'");
                s.push_agent_event(format!(
                    "[DropFailed] You are not carrying anything called '{item}'."
                ));
                continue;
            };
            let instance_id = match placed {
                Carried::Worn(slot) => s.self_equipped[&slot].instance_id,
                Carried::InBag(instance_id) => instance_id,
            };
            let cmd = onlinerpg_shared::ClientMessage::DropItem { instance_id };
            if let Err(e) = s.send_command(cmd).await {
                error!("Failed to send drop action: {e}");
            } else {
                // Never re-surface this instance as loot for ourselves — the
                // server echoes the drop back as an ordinary GroundItemSpawned,
                // which would otherwise put it right back in "Item on ground"
                // next turn and loop the drop/pickup pair forever.
                s.mark_self_dropped(instance_id);
                info!("Agent dropped {def_id} [id {instance_id}]");
            }
            continue;
        }

        // Pick up a ground item: resolve the reference, walk into pickup
        // range, and send the pickup. The server owns the range, floor and
        // weight checks and answers with InventoryUpdated or a SystemMessage.
        if let AgentAction::Pickup { item } = action {
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
            match walk_to_ground_item(state, instance_id).await {
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
                ChaseResult::Lost => {
                    warn!("pickup: could not reach {def_id} [id {instance_id}]");
                    let mut s = state.lock().await;
                    s.push_agent_event(format!(
                        "[PickupFailed] You could not reach the {def_id} — it was taken \
                         or despawned before you got there."
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

        // Sell every copy of a bag item to a nearby merchant: resolve the
        // merchant's name and the item's canonical id, walk over if needed,
        // then sell each matching instance. Server-side rejections (too far,
        // not a trader, no base price) surface automatically as [TradeError].
        if let AgentAction::Sell { player, item } = action {
            let resolved = {
                let mut s = state.lock().await;
                let Some((target_id, _)) = s.resolve_nearby_player(player) else {
                    warn!("sell: no nearby merchant named '{player}'");
                    s.push_agent_event(format!(
                        "[SellFailed] No merchant named '{player}' is nearby."
                    ));
                    continue;
                };
                let ids: Vec<&str> = s.self_bag.iter().map(|i| i.item_def_id.as_str()).collect();
                let Some(def_id) = crate::item_defs::resolve_named(&ids, item) else {
                    warn!("sell: nothing carried matching '{item}'");
                    s.push_agent_event(format!(
                        "[SellFailed] You are not carrying anything called '{item}'."
                    ));
                    continue;
                };
                let def_id = def_id.to_string();
                let instance_ids: Vec<u64> = s
                    .self_bag
                    .iter()
                    .filter(|i| i.item_def_id == def_id)
                    .map(|i| i.instance_id)
                    .collect();
                (target_id, def_id, instance_ids)
            };
            let (target_id, def_id, instance_ids) = resolved;
            info!(
                "Agent selling {} x {def_id} to {player}, approaching...",
                instance_ids.len()
            );
            match approach_player(state, &target_id).await {
                ChaseResult::InRange => {
                    let mut s = state.lock().await;
                    for instance_id in instance_ids {
                        let cmd = onlinerpg_shared::ClientMessage::SellItem {
                            merchant_player_id: target_id,
                            instance_id,
                        };
                        if let Err(e) = s.send_command(cmd).await {
                            error!("Failed to send sell command: {e}");
                            break;
                        }
                    }
                }
                ChaseResult::Lost => {
                    warn!("Could not reach merchant '{player}', skipping sell");
                    let mut s = state.lock().await;
                    s.push_agent_event(format!(
                        "[SellFailed] Could not reach {player} — they moved away or out of sight."
                    ));
                }
                ChaseResult::Error => {
                    error!("sell: error while approaching '{player}'");
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
                            "[MoveFailed] No route to ({gx:.1}, {gz:.1}) — a wall or a shut \
                             door stands in the way. Try a different goal."
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

/// Resolve a pickup reference to an item the agent can actually walk to:
/// by instance id, or by name to the nearest match. Only what the agent can
/// see counts: an id it read several turns ago may have fallen out of sight
/// since, and a pickup it cannot perceive is one it never learns about.
fn resolve_ground_item(s: &SharedState, r: &PickupRef) -> Option<(u64, String)> {
    let in_sight = s.ground_items_in_sight();
    let (_, item) = match r.as_id() {
        Some(id) => in_sight.iter().find(|(_, i)| i.instance_id == id)?,
        None => {
            let ids: Vec<&str> = in_sight
                .iter()
                .map(|(_, i)| i.item_def_id.as_str())
                .collect();
            // Nearest first, so the resolver's first match is the closest.
            let def_id = crate::item_defs::resolve_named(&ids, &r.to_string())?;
            in_sight.iter().find(|(_, i)| i.item_def_id == def_id)?
        }
    };
    Some((item.instance_id, item.item_def_id.clone()))
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
            s.remember_ground_item(item, std::time::Instant::now());
        }
        s
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
}
