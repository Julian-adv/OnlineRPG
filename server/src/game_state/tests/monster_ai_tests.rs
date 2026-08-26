//! Server-driven monster brains (doc/SERVER_SIDE_MONSTER_AI.md).
use super::*;

async fn spawn_goblin(game_state: &GameState, owner: &PlayerId, x: f32) -> String {
    game_state
        .spawn_monster(
            "goblin".to_string(),
            Position { x, y: 0.0, z: 0.0 },
            0.0,
            Some(*owner),
            0,
            MonsterLifecycle::Ambient,
            None,
            true,
        )
        .await
        .expect("goblin spawns")
        .id
}

async fn monster_x(game_state: &GameState, id: &str) -> f32 {
    game_state.monsters.read().await[id].position.x
}

async fn health_of(game_state: &GameState, player_id: &PlayerId) -> u32 {
    game_state.players.read().await[player_id].health
}

#[tokio::test]
async fn server_brain_chases_and_attacks_the_player() {
    let game_state = make_flat_world_game_state("server_ai_chase");
    game_state.enable_server_monster_ai();
    let player_id = pid("prey");
    game_state.add_player(make_player("prey", 0.0, 0.0)).await;
    let mut rx = game_state.register_direct_channel(&player_id).await;
    let goblin = spawn_goblin(&game_state, &player_id, 12.0).await;

    let mut attacked = false;
    for _ in 0..60 {
        game_state.tick_monster_ai_by(200.0).await;
        if health_of(&game_state, &player_id).await < 10 {
            attacked = true;
            break;
        }
    }
    let x = monster_x(&game_state, &goblin).await;
    assert!(
        x < 12.0,
        "the brain must have walked the goblin toward the player, x={x}"
    );
    assert!(attacked, "12s of ticks must land at least one goblin hit");
    assert_eq!(game_state.brain_count().await, 1);

    let msgs = drain(&mut rx);
    let moved = msgs
        .iter()
        .any(|m| matches!(m, ServerMessage::MonsterMoved { owner_id: None, .. }));
    assert!(moved, "moves fan out with no owner on the wire: {msgs:?}");
    assert!(
        !msgs
            .iter()
            .any(|m| matches!(m, ServerMessage::MonsterAssigned { .. })),
        "no client is assigned a brain"
    );
    assert!(
        msgs.iter().all(|m| !matches!(
            m,
            ServerMessage::MonsterSpawned { monster } if monster.owner_id.is_some()
        )),
        "spawns reach clients ownerless"
    );
}

#[tokio::test]
async fn client_moves_and_attacks_are_ignored_with_server_brains() {
    let game_state = make_flat_world_game_state("server_ai_ignores_client");
    game_state.enable_server_monster_ai();
    let owner_id = pid("cheater");
    let victim_id = pid("victim");
    game_state
        .add_player(make_player("cheater", 0.0, 0.0))
        .await;
    game_state.add_player(make_player("victim", 1.0, 0.0)).await;
    let goblin = spawn_goblin(&game_state, &owner_id, 3.0).await;

    game_state
        .update_monster_position(
            &owner_id,
            goblin.clone(),
            Position {
                x: 5.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
            MonsterState::Run,
            Position {
                x: 5.0,
                y: 0.0,
                z: 0.0,
            },
        )
        .await;
    assert_eq!(monster_x(&game_state, &goblin).await, 3.0);

    for _ in 0..5 {
        game_state
            .broadcast_monster_attack(&owner_id, &goblin, &victim_id)
            .await;
    }
    assert_eq!(
        game_state.players.read().await[&victim_id].last_combat_at,
        0,
        "the cap owner's client no longer swings its monsters"
    );
}

#[tokio::test]
async fn brain_is_dropped_when_the_monster_dies() {
    let game_state = make_flat_world_game_state("server_ai_death");
    game_state.enable_server_monster_ai();
    let player_id = pid("slayer");
    game_state.add_player(make_player("slayer", 0.0, 0.0)).await;
    let goblin = spawn_goblin(&game_state, &player_id, 2.0).await;
    game_state.tick_monster_ai_by(200.0).await;
    assert_eq!(game_state.brain_count().await, 1);

    game_state.monsters.write().await.mark_dead(&goblin);
    game_state.tick_monster_ai_by(200.0).await;
    assert_eq!(game_state.brain_count().await, 0, "a corpse keeps no brain");
}

#[tokio::test]
async fn a_hit_monster_retaliates() {
    let game_state = make_flat_world_game_state("server_ai_retaliate");
    game_state.enable_server_monster_ai();
    let player_id = pid("poker");
    game_state.add_player(make_player("poker", 0.0, 0.0)).await;
    let goblin = game_state
        .spawn_monster(
            "goblin".to_string(),
            Position {
                x: 2.5,
                y: 0.0,
                z: 0.0,
            },
            0.0,
            Some(player_id),
            0,
            MonsterLifecycle::Ambient,
            None,
            false,
        )
        .await
        .expect("goblin spawns")
        .id;
    game_state.tick_monster_ai_by(200.0).await;

    game_state
        .broadcast_player_attack(&player_id, goblin.clone())
        .await;
    let mut attacked = false;
    for _ in 0..40 {
        game_state.tick_monster_ai_by(200.0).await;
        if health_of(&game_state, &player_id).await < 10 {
            attacked = true;
            break;
        }
    }
    assert!(attacked, "a poked brave goblin must swing back within 8s");
}

/// The registry learns a running monster's position only at each network
/// sync, so between syncs it trails the brain. A swing must be judged on
/// where the brain has the monster, or a charge is refused at contact.
#[tokio::test]
async fn a_swing_is_judged_on_the_brain_not_the_synced_registry() {
    let game_state = make_flat_world_game_state("server_ai_swing_at_charger");
    game_state.enable_server_monster_ai();
    let player_id = pid("swinger");
    game_state
        .add_player(make_player("swinger", 0.0, 0.0))
        .await;
    let mut rx = game_state.register_direct_channel(&player_id).await;
    let goblin = spawn_goblin(&game_state, &player_id, 4.2).await;

    // Melee reach is 2m + 1m tolerance: wait for a tick that leaves the brain
    // inside it while the registry still holds a pose outside it.
    let mut staggered = false;
    for _ in 0..10 {
        game_state.tick_monster_ai_by(200.0).await;
        let brain_x = game_state
            .brain_position_now(&goblin)
            .await
            .expect("the goblin has a brain")
            .x;
        let registry_x = monster_x(&game_state, &goblin).await;
        if brain_x <= 2.9 && registry_x > 3.0 {
            staggered = true;
            break;
        }
    }
    assert!(staggered, "the setup never produced a registry lag to test");
    drain(&mut rx);

    game_state
        .broadcast_player_attack(&player_id, goblin.clone())
        .await;

    let msgs = drain(&mut rx);
    assert!(
        msgs.iter()
            .any(|m| matches!(m, ServerMessage::PlayerAttacked { .. })),
        "the swing must land on the brain's position: {msgs:?}"
    );
    assert!(
        !msgs
            .iter()
            .any(|m| matches!(m, ServerMessage::PlayerAttackRejected { .. })),
        "{msgs:?}"
    );
}
