// ---- Tip hats (the `tip_hat` item) -----------------------------------------

use super::*;
use onlinerpg_shared::tip_hat::TIP_HAT_LEASH_M;

/// A registered player with an empty bag and `gold` in the wallet.
async fn make_tipper(game_state: &GameState, name: &str, x: f32, z: f32, gold: i64) -> PlayerId {
    let id = pid(name);
    game_state.add_player(make_player(name, x, z)).await;
    game_state
        .inventories
        .write()
        .await
        .insert(id, PlayerInventory::default());
    game_state
        .register_player_character(&id, 1, 0, attrs_with_cha(10), gold, None)
        .await;
    id
}

#[tokio::test]
async fn using_the_item_sets_a_hat_down_ahead_and_using_it_again_picks_it_up() {
    let game_state = make_test_game_state("tip_hat_place");
    let bard_id = make_tipper(&game_state, "bard", 100.0, 50.0, 0).await;
    game_state
        .inventories
        .write()
        .await
        .get_mut(&bard_id)
        .unwrap()
        .bag
        .push(bag_item(1, "tip_hat", 1));

    game_state.use_item(&bard_id, 1).await;

    {
        let hats = game_state.tip_hats.read().await;
        let hat = hats.get(&bard_id).expect("hat placed");
        assert_eq!(hat.owner_name, "bard");
        // Rotation 0 faces +z, so the hat lands two cells ahead.
        assert_eq!(hat.position.z, 52.0);
        assert_eq!(hat.position.x, 100.0);
    }
    assert_eq!(
        game_state.inventories.read().await[&bard_id].bag.len(),
        1,
        "the hat item is not consumed"
    );

    game_state.use_item(&bard_id, 1).await;
    assert!(
        game_state.tip_hats.read().await.is_empty(),
        "using it again picks the hat back up"
    );
}

#[tokio::test]
async fn straying_past_the_leash_or_logging_out_packs_the_hat_up() {
    let game_state = make_test_game_state("tip_hat_leash");
    let bard_id = make_tipper(&game_state, "bard", 100.0, 50.0, 0).await;
    game_state.toggle_tip_hat(&bard_id).await;

    let near = Position {
        x: 100.0,
        y: 0.0,
        z: 52.0 + TIP_HAT_LEASH_M - 1.0,
    };
    game_state.teleport_player(&bard_id, near, 0.0, 0).await;
    assert_eq!(game_state.tip_hats.read().await.len(), 1);

    // Same spot but underground: a floor change strands the hat too.
    game_state.teleport_player(&bard_id, near, 0.0, -1).await;
    assert!(game_state.tip_hats.read().await.is_empty());

    game_state.teleport_player(&bard_id, near, 0.0, 0).await;
    game_state.toggle_tip_hat(&bard_id).await;
    let far = Position {
        x: 100.0,
        y: 0.0,
        z: near.z + TIP_HAT_LEASH_M + 3.0,
    };
    game_state.teleport_player(&bard_id, far, 0.0, 0).await;
    assert!(game_state.tip_hats.read().await.is_empty());

    game_state.toggle_tip_hat(&bard_id).await;
    game_state.remove_player(&bard_id).await;
    assert!(
        game_state.tip_hats.read().await.is_empty(),
        "logout packs the hat up"
    );
}

#[tokio::test]
async fn tipping_moves_copper_from_the_tipper_to_the_owner() {
    let game_state = make_test_game_state("tip_hat_pay");
    let bard_id = make_tipper(&game_state, "bard", 100.0, 50.0, 0).await;
    let fan_id = make_tipper(&game_state, "fan", 100.0, 51.0, 500).await;
    game_state.toggle_tip_hat(&bard_id).await;
    let hat_id = game_state.tip_hats.read().await[&bard_id].id;

    game_state.tip_hat_tip(&fan_id, hat_id, 120).await;

    assert_eq!(game_state.get_player_gold(&fan_id).await, 380);
    assert_eq!(game_state.get_player_gold(&bard_id).await, 120);

    game_state.tip_hat_tip(&fan_id, hat_id, 10_000).await;
    assert_eq!(
        game_state.get_player_gold(&fan_id).await,
        380,
        "a tip beyond the wallet is refused"
    );

    game_state.tip_hat_tip(&bard_id, hat_id, 10).await;
    assert_eq!(
        game_state.get_player_gold(&bard_id).await,
        120,
        "tipping your own hat pays nothing"
    );
}

#[tokio::test]
async fn tipping_from_out_of_range_is_refused() {
    let game_state = make_test_game_state("tip_hat_range");
    let bard_id = make_tipper(&game_state, "bard", 100.0, 50.0, 0).await;
    let fan_id = make_tipper(&game_state, "fan", 100.0, 70.0, 500).await;
    game_state.toggle_tip_hat(&bard_id).await;
    let hat_id = game_state.tip_hats.read().await[&bard_id].id;

    game_state.tip_hat_tip(&fan_id, hat_id, 100).await;

    assert_eq!(game_state.get_player_gold(&fan_id).await, 500);
    assert_eq!(game_state.get_player_gold(&bard_id).await, 0);
}
