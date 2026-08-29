//! Entry grace: a player still loading the world cannot be damaged and cannot
//! attack, until `WorldReady` or the deadline.

use super::*;
use onlinerpg_shared::entity::WORLD_LOADING_GRACE_MS;

fn loading_player(id: &str, x: f32) -> Player {
    let mut player = make_player(id, x, 0.0);
    player.ready_at = GameState::now_ms() + WORLD_LOADING_GRACE_MS;
    player
}

/// A monster owned by `owner`, standing on the origin.
async fn adjacent_monster(game_state: &GameState, id: &str, owner: &PlayerId) {
    let mut monsters = game_state.monsters.write().await;
    let mut monster = make_monster(id, pos(0.0), 0);
    monster.owner_id = Some(*owner);
    monsters.insert(id.to_string(), monster);
}

/// `last_combat_at` is stamped for any processed swing, hit or miss.
async fn was_attacked(game_state: &GameState, id: &PlayerId) -> bool {
    game_state.players.read().await[id].last_combat_at != 0
}

#[tokio::test]
async fn loading_player_is_shielded_until_world_ready() {
    let game_state = make_test_game_state("loading_no_damage");
    let owner_id = pid("owner");
    let newcomer_id = pid("newcomer");

    game_state.add_player(make_player("owner", 0.0, 0.0)).await;
    game_state.add_player(loading_player("newcomer", 0.0)).await;
    // Each swing burns its monster's cooldown, rejected or not.
    for id in ["loading_monster", "ready_monster"] {
        adjacent_monster(&game_state, id, &owner_id).await;
    }

    game_state
        .broadcast_monster_attack(&owner_id, "loading_monster", &newcomer_id)
        .await;
    assert!(!was_attacked(&game_state, &newcomer_id).await);
    assert_eq!(game_state.players.read().await[&newcomer_id].health, 10);

    game_state.mark_world_ready(&newcomer_id).await;
    game_state
        .broadcast_monster_attack(&owner_id, "ready_monster", &newcomer_id)
        .await;
    assert!(was_attacked(&game_state, &newcomer_id).await);
}

#[tokio::test]
async fn loading_grace_expires_without_world_ready() {
    let game_state = make_test_game_state("loading_grace_expiry");
    let owner_id = pid("owner");
    let auto_ready_id = pid("auto_ready_player");

    game_state.add_player(make_player("owner", 0.0, 0.0)).await;
    let mut auto_ready_player = make_player("auto_ready_player", 0.0, 0.0);
    auto_ready_player.ready_at = GameState::now_ms() - 1_000;
    game_state.add_player(auto_ready_player).await;

    adjacent_monster(&game_state, "patient_monster", &owner_id).await;
    game_state
        .broadcast_monster_attack(&owner_id, "patient_monster", &auto_ready_id)
        .await;
    assert!(was_attacked(&game_state, &auto_ready_id).await);
}

#[tokio::test]
async fn loading_player_cannot_attack() {
    let game_state = make_test_game_state("loading_cannot_attack");
    let attacker_id = pid("attacker");
    game_state.add_player(loading_player("attacker", 0.0)).await;
    let mut attacker_rx = game_state.register_direct_channel(&attacker_id).await;
    game_state.monsters.write().await.insert(
        "victim_monster".to_string(),
        make_monster("victim_monster", pos(1.0), 0),
    );

    game_state
        .broadcast_player_attack(&attacker_id, "victim_monster".to_string())
        .await;

    expect_attack_rejected(
        &mut attacker_rx,
        "victim_monster",
        AttackRejectReason::NotInGame,
    );
    assert_eq!(
        game_state.monsters.read().await["victim_monster"].health,
        10
    );
}
