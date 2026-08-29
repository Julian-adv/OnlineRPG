//! The entry grace: a client still compiling its scene cannot be damaged, so
//! logging back in beside an aggroed monster is survivable.

use super::*;
use onlinerpg_shared::entity::WORLD_LOADING_GRACE_MS;

/// A player who just entered: the full grace still ahead, so not yet hittable.
fn loading_player(id: &str, x: f32) -> Player {
    let mut player = make_player(id, x, 0.0);
    player.ready_at = GameState::now_ms() + WORLD_LOADING_GRACE_MS;
    player
}

/// Give `owner` a monster standing on top of `target`, so only the grace can
/// stop the attack.
async fn adjacent_monster(game_state: &GameState, id: &str, owner: &PlayerId) {
    let mut monsters = game_state.monsters.write().await;
    let mut monster = make_monster(id, pos(0.0), 0);
    monster.owner_id = Some(*owner);
    monsters.insert(id.to_string(), monster);
}

/// Two players in the same spot, told apart only by `ready_at`: the ready one
/// (`make_player`, ready_at 0) is hit, the loading one is not, and after
/// `WorldReady` the same attack lands on them too. The ready player doubles as
/// the control — if the attack setup were broken, their half would fail.
#[tokio::test]
async fn only_the_loading_player_is_shielded() {
    let game_state = make_test_game_state("loading_no_damage");
    let owner_id = pid("owner");
    let newcomer_id = pid("newcomer");
    let normal_player_id = pid("normal_player");

    game_state.add_player(make_player("owner", 0.0, 0.0)).await;
    game_state.add_player(loading_player("newcomer", 0.0)).await;
    game_state
        .add_player(make_player("normal_player", 0.0, 0.0))
        .await;

    // Every swing burns its monster's cooldown, rejected or not, so each of
    // the three attacks gets its own monster.
    for id in ["loading_monster", "normal_player_monster", "ready_monster"] {
        adjacent_monster(&game_state, id, &owner_id).await;
    }

    game_state
        .broadcast_monster_attack(&owner_id, "loading_monster", &newcomer_id)
        .await;
    game_state
        .broadcast_monster_attack(&owner_id, "normal_player_monster", &normal_player_id)
        .await;

    // `last_combat_at` is stamped for any attack that lands, hit or miss, so it
    // records that the swing was processed without depending on a damage roll.
    assert_eq!(
        game_state.players.read().await[&newcomer_id].last_combat_at,
        0,
        "a monster must not reach a player who is still loading the world"
    );
    assert_eq!(
        game_state.players.read().await[&newcomer_id].health,
        10,
        "a loading player must take no damage"
    );
    assert_ne!(
        game_state.players.read().await[&normal_player_id].last_combat_at,
        0,
        "the identical attack must be processed against the ready player"
    );

    game_state.mark_world_ready(&newcomer_id).await;
    game_state
        .broadcast_monster_attack(&owner_id, "ready_monster", &newcomer_id)
        .await;

    assert_ne!(
        game_state.players.read().await[&newcomer_id].last_combat_at,
        0,
        "the same attack must land once the client reports the world is ready"
    );
}

/// The grace is a deadline, not a switch a client can hold down: a client that
/// never sends `WorldReady` becomes damageable on its own.
#[tokio::test]
async fn loading_grace_expires_without_world_ready() {
    let game_state = make_test_game_state("loading_grace_expiry");
    let owner_id = pid("owner");
    let auto_ready_id = pid("auto_ready_player");

    game_state.add_player(make_player("owner", 0.0, 0.0)).await;
    // Never sent WorldReady, but the entry deadline passed a second ago.
    let mut auto_ready_player = make_player("auto_ready_player", 0.0, 0.0);
    auto_ready_player.ready_at = GameState::now_ms() - 1_000;
    game_state.add_player(auto_ready_player).await;

    adjacent_monster(&game_state, "patient_monster", &owner_id).await;
    game_state
        .broadcast_monster_attack(&owner_id, "patient_monster", &auto_ready_id)
        .await;

    assert_ne!(
        game_state.players.read().await[&auto_ready_id].last_combat_at,
        0,
        "the grace must expire on its own for a client that never reports ready"
    );
}

/// Untouchable means untouching: a client that withholds `WorldReady` must not
/// get free swings out of the grace.
#[tokio::test]
async fn loading_player_cannot_attack() {
    let game_state = make_test_game_state("loading_cannot_attack");
    let attacker_id = pid("attacker");
    game_state.add_player(loading_player("attacker", 0.0)).await;
    let mut attacker_rx = game_state.register_direct_channel(&attacker_id).await;

    {
        let mut monsters = game_state.monsters.write().await;
        monsters.insert(
            "victim_monster".to_string(),
            make_monster("victim_monster", pos(1.0), 0),
        );
    }

    game_state
        .broadcast_player_attack(&attacker_id, "victim_monster".to_string())
        .await;

    match attacker_rx.try_recv() {
        Ok(ServerMessage::PlayerAttackRejected { monster_id, reason }) => {
            assert_eq!(monster_id, "victim_monster");
            assert_eq!(reason, AttackRejectReason::NotInGame);
        }
        other => panic!("Expected a rejection ack while loading, got {other:?}"),
    }
    assert_eq!(
        game_state.monsters.read().await["victim_monster"].health,
        10,
        "a loading player's swing must not damage the monster"
    );
}
