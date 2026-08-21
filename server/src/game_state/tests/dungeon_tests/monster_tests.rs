use super::*;
use crate::dungeon_defs::DungeonEntranceDef;
use onlinerpg_shared::dungeon::GRID;

const DEPTH: u8 = 1;
const FLOOR: i8 = -(DEPTH as i8);

/// A spot on floor `DEPTH`, inside the entrance's footprint so the floor
/// change resolves the dungeon from it.
fn inside(entrance: &DungeonEntranceDef) -> Position {
    Position {
        x: entrance.x,
        y: entrance.y - 4.0,
        z: entrance.z,
    }
}

/// A player standing on floor `DEPTH`, registered with the floor runtime so
/// the exit path counts it as an occupant. Entry populates the floor's spawn
/// slots, so the first occupant added is who the monsters belong to.
async fn add_occupant(
    game_state: &GameState,
    name: &str,
    entrance: &DungeonEntranceDef,
) -> PlayerId {
    let at = inside(entrance);
    let mut player = make_player(name, at.x, at.z);
    player.position.y = at.y;
    player.floor_level = FLOOR;
    game_state.add_player(player).await;
    let id = pid(name);
    game_state
        .handle_player_floor_change(&id, 0, FLOOR, &at, &at)
        .await;
    id
}

/// Walks `player_id` off the floor the way a stair climb does, without the
/// position fan-out — these tests are about `leave_dungeon_floor` itself.
async fn leave_floor(game_state: &GameState, player_id: &PlayerId, entrance: &DungeonEntranceDef) {
    game_state
        .handle_player_floor_change(player_id, FLOOR, 0, &inside(entrance), &entrance.position())
        .await;
}

/// Kills `player_id` and revives it, the exit route the client used to patch
/// around. Stairs, /escape and teleports share its `finish_position_update`.
async fn die_and_respawn(game_state: &GameState, player_id: &PlayerId) {
    game_state
        .players
        .write()
        .await
        .get_mut(player_id)
        .expect("the player is on the floor")
        .health = 0;
    game_state.respawn_player(player_id).await;
}

/// The monsters the floor's spawn slots are holding, which floor entry filled.
async fn floor_monster_ids(game_state: &GameState, entrance: &DungeonEntranceDef) -> Vec<String> {
    let ids: Vec<String> = {
        let dungeons = game_state.dungeons.read().await;
        dungeons
            .get(&entrance.id)
            .and_then(|rt| rt.floors.get(&DEPTH))
            .map(|fr| {
                fr.slots
                    .iter()
                    .filter_map(|s| s.alive_monster_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    };
    assert!(
        !ids.is_empty(),
        "floor {DEPTH} should hold spawn slots to test with"
    );
    ids
}

fn removed_ids(msgs: &[ServerMessage]) -> Vec<String> {
    msgs.iter()
        .filter_map(|m| match m {
            ServerMessage::MonsterRemoved { monster_id } => Some(monster_id.clone()),
            _ => None,
        })
        .collect()
}

/// Asserts `rx` was told about exactly `ids`, in any order.
fn assert_removed_exactly(msgs: &[ServerMessage], ids: &[String], context: &str) {
    let mut removed = removed_ids(msgs);
    removed.sort();
    let mut expected = ids.to_vec();
    expected.sort();
    assert_eq!(removed, expected, "{context}");
}

/// The slot spawn path must tag its monsters `DungeonSlot`: that tag is what
/// exempts them from the ownership sweep's despawn and the abandonment
/// despawn — the floor owns their removal.
#[tokio::test]
async fn floor_slots_spawn_dungeon_slot_lifecycle_monsters() {
    let game_state = make_test_game_state("dungeon_slot_lifecycle");
    let entrance = first_dungeon(&game_state);
    add_occupant(&game_state, "delver", &entrance).await;
    let ids = floor_monster_ids(&game_state, &entrance).await;

    let monsters = game_state.monsters.read().await;
    for id in &ids {
        assert_eq!(
            monsters.get(id).map(|m| m.lifecycle),
            Some(MonsterLifecycle::DungeonSlot),
            "slot-spawned monster {id} must carry the DungeonSlot lifecycle"
        );
    }
}

/// The leaver's client is what simulates its monsters, and it keeps rendering
/// them until told otherwise. Handing them to whoever stayed behind without
/// telling the leaver leaves it holding monsters it no longer owns.
#[tokio::test]
async fn leaving_a_still_occupied_floor_tells_the_leaver_its_monsters_are_gone() {
    let game_state = make_test_game_state("dungeon_leave_reassign");
    let entrance = first_dungeon(&game_state);
    let leaver = add_occupant(&game_state, "leaver", &entrance).await;
    let stayer = add_occupant(&game_state, "stayer", &entrance).await;
    let ids = floor_monster_ids(&game_state, &entrance).await;

    let mut leaver_rx = game_state.register_direct_channel(&leaver).await;
    let mut stayer_rx = game_state.register_direct_channel(&stayer).await;
    drain(&mut leaver_rx);
    drain(&mut stayer_rx);

    leave_floor(&game_state, &leaver, &entrance).await;

    for id in &ids {
        assert_eq!(
            owner_of(&game_state, id).await,
            Some(stayer),
            "monster {id} should have gone to the player still on the floor"
        );
    }
    assert_removed_exactly(
        &drain(&mut leaver_rx),
        &ids,
        "the leaver needs MonsterRemoved for every monster it handed over",
    );
    let assigned: Vec<String> = drain(&mut stayer_rx)
        .iter()
        .filter_map(|m| match m {
            ServerMessage::MonsterAssigned { monster } => Some(monster.id.clone()),
            _ => None,
        })
        .collect();
    for id in &ids {
        assert!(
            assigned.contains(id),
            "the new owner needs MonsterAssigned for {id} to start running its AI"
        );
    }
}

/// The last player out despawns the floor. The removal broadcast is filtered
/// to the monster's floor, which the now-surfaced leaver is no longer on, so
/// without a direct message it keeps ghosts whose ids no longer exist.
#[tokio::test]
async fn emptying_a_floor_tells_the_leaver_its_monsters_are_gone() {
    let game_state = make_test_game_state("dungeon_leave_despawn");
    let entrance = first_dungeon(&game_state);
    let leaver = add_occupant(&game_state, "solo", &entrance).await;
    let ids = floor_monster_ids(&game_state, &entrance).await;

    let mut leaver_rx = game_state.register_direct_channel(&leaver).await;
    drain(&mut leaver_rx);

    leave_floor(&game_state, &leaver, &entrance).await;

    for id in &ids {
        assert!(
            game_state.monsters.read().await.get(id).is_none(),
            "monster {id} should be gone from the registry"
        );
    }
    assert_removed_exactly(
        &drain(&mut leaver_rx),
        &ids,
        "the leaver needs MonsterRemoved for every despawned monster",
    );
    assert!(
        game_state.dungeon_monsters.read().await.is_empty(),
        "the slot index must not outlive the monsters it points at"
    );
}

/// A dungeon floor is wider than the event radius, so the AOI rule
/// `tick_monster_ownership` uses would find no candidate and despawn monsters
/// out from under a player still fighting on the far side of the floor.
/// Occupancy, not proximity, is what a floor hands off by.
#[tokio::test]
async fn a_floor_mate_out_of_range_still_inherits_rather_than_losing_the_monsters() {
    let game_state = make_test_game_state("dungeon_leave_far_side");
    let entrance = first_dungeon(&game_state);
    let leaver = add_occupant(&game_state, "leaver", &entrance).await;
    let far = add_occupant(&game_state, "farside", &entrance).await;
    let ids = floor_monster_ids(&game_state, &entrance).await;

    // Push the remaining occupant past the event radius from every monster,
    // where the AOI rule would see nobody at all.
    let positions: Vec<Position> = {
        let monsters = game_state.monsters.read().await;
        ids.iter()
            .filter_map(|id| monsters.get(id).map(|m| m.position))
            .collect()
    };
    let away = Position {
        x: entrance.x + GRID as f32,
        z: entrance.z + GRID as f32,
        ..inside(&entrance)
    };
    for p in &positions {
        let d = ((away.x - p.x).powi(2) + (away.z - p.z).powi(2)).sqrt();
        assert!(
            d > EVENT_DELIVERY_RADIUS,
            "the far-side player must be outside every monster's AOI, got {d}m"
        );
    }
    game_state.teleport_player(&far, away, 0.0, FLOOR).await;

    leave_floor(&game_state, &leaver, &entrance).await;

    for id in &ids {
        assert_eq!(
            owner_of(&game_state, id).await,
            Some(far),
            "monster {id} should go to the floor's remaining occupant, not despawn"
        );
    }
}

/// What the client is left holding after a real exit, not just what
/// `leave_dungeon_floor` sends. The exit only touches the leaver's own
/// monsters; the ones it merely watched are cleared by the floor-aware AOI
/// diff in `fanout_player_position_update`, which has to reach back to the
/// floor from the world spawn to find them. Both halves have to land for the
/// client to be able to drop its own by-floor purge.
#[tokio::test]
async fn dying_beside_a_party_member_clears_watched_monsters_as_well_as_owned_ones() {
    let game_state = make_test_game_state("dungeon_leave_party_respawn");
    let entrance = first_dungeon(&game_state);
    let leaver = add_occupant(&game_state, "leaver", &entrance).await;
    let stayer = add_occupant(&game_state, "stayer", &entrance).await;
    let ids = floor_monster_ids(&game_state, &entrance).await;
    assert!(
        ids.len() >= 2,
        "need a monster each to test the not-owned half"
    );

    // Give the stayer one of them, so the leaver is holding a monster whose
    // ownership the exit never touches.
    let not_owned = ids[0].clone();
    game_state
        .monsters
        .write()
        .await
        .reassign_owner(&not_owned, stayer, 0);
    assert_eq!(
        owner_of(&game_state, &not_owned).await,
        Some(stayer),
        "the not-owned half is only tested if the monster really moved"
    );

    let mut leaver_rx = game_state.register_direct_channel(&leaver).await;
    drain(&mut leaver_rx);

    die_and_respawn(&game_state, &leaver).await;

    let removed = removed_ids(&drain(&mut leaver_rx));
    for id in &ids {
        assert!(
            removed.contains(id),
            "respawning off a shared floor must clear {id}; got {removed:?}"
        );
    }
}

/// Dying alone in a dungeon is the case the client's by-floor purge was
/// written for, and the one it hooks: `finish_position_update` runs the floor
/// change before the position fan-out, so the despawned monsters are already
/// out of the registry when the AOI diff looks for them. Only the direct
/// message to the old owner reaches the leaver — and respawn lands at the
/// world spawn, far enough that no radius-filtered broadcast would.
#[tokio::test]
async fn dying_alone_on_a_floor_clears_the_monsters_the_exit_despawned() {
    let game_state = make_test_game_state("dungeon_leave_solo_respawn");
    let entrance = first_dungeon(&game_state);
    let leaver = add_occupant(&game_state, "solo", &entrance).await;
    let ids = floor_monster_ids(&game_state, &entrance).await;

    let mut leaver_rx = game_state.register_direct_channel(&leaver).await;
    drain(&mut leaver_rx);

    die_and_respawn(&game_state, &leaver).await;

    let removed = removed_ids(&drain(&mut leaver_rx));
    for id in &ids {
        assert!(
            removed.contains(id),
            "respawning out of a dungeon must clear {id}; got {removed:?}"
        );
    }
}

/// Emptying a floor used to zero every slot's respawn clock, including the
/// boss's — so a party could step onto the stairs and back to refight the
/// guardian every couple of minutes, each kill a guaranteed weapon drop.
/// A slain boss now holds its slot until the dungeon resets.
#[tokio::test]
async fn a_slain_boss_does_not_return_when_the_floor_empties() {
    let game_state = make_test_game_state("boss_slot_holds");
    let entrance = first_dungeon(&game_state);
    let player_id = add_occupant(&game_state, "Delver", &entrance).await;

    // A boss slot on this floor, freshly slain.
    {
        let mut dungeons = game_state.dungeons.write().await;
        let floor = dungeons
            .get_mut(&entrance.id)
            .and_then(|rt| rt.floors.get_mut(&DEPTH))
            .expect("entered floor");
        floor.slots.push(super::dungeon::SpawnSlot {
            alive_monster_id: None,
            respawn_at_ms: super::dungeon::BOSS_RESPAWN_NEVER,
            is_boss: true,
        });
        floor.boss_defeated = true;
        floor.chest_claimants.insert(7);
    }

    leave_floor(&game_state, &player_id, &entrance).await;

    let dungeons = game_state.dungeons.read().await;
    let floor = dungeons
        .get(&entrance.id)
        .and_then(|rt| rt.floors.get(&DEPTH))
        .expect("floor runtime outlives its occupants");
    let boss = floor.slots.iter().find(|s| s.is_boss).expect("boss slot");
    assert_eq!(
        boss.respawn_at_ms,
        super::dungeon::BOSS_RESPAWN_NEVER,
        "an empty floor must not free the guardian's slot"
    );
    assert!(floor.boss_defeated, "the guardian stays down");
    assert!(
        floor.chest_claimants.contains(&7),
        "and the claim it earned stays with it"
    );
}
