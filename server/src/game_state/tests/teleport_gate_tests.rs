use super::*;
use onlinerpg_shared::teleport::{teleport_gate, teleport_gate_fare};

async fn setup_gate_player(
    game_state: &GameState,
    name: &str,
    gate_id: &str,
    gold: i64,
) -> DirectRx {
    let gate = teleport_gate(gate_id).unwrap();
    game_state
        .add_player(make_player(name, gate.x, gate.z))
        .await;
    game_state
        .register_player_character(&pid(name), 1, 0, attrs_with_cha(10), gold, None)
        .await;
    game_state.register_direct_channel(&pid(name)).await
}

#[tokio::test]
async fn gate_menu_quotes_every_other_town_by_distance() {
    let game_state = make_test_game_state("gate_quotes");
    let mut rx = setup_gate_player(&game_state, "traveler", "aldermark", 100_000).await;

    game_state
        .open_teleport_gate(&pid("traveler"), "aldermark")
        .await;

    let message = drain(&mut rx)
        .into_iter()
        .find(|message| matches!(message, ServerMessage::TeleportGateState { .. }))
        .unwrap();
    let ServerMessage::TeleportGateState {
        destinations,
        misfire_chance_bps,
        ..
    } = message
    else {
        unreachable!()
    };
    assert_eq!(destinations.len(), 7);
    assert_eq!(misfire_chance_bps, 50);
    assert!(destinations
        .windows(2)
        .all(|pair| pair[0].fare <= pair[1].fare));
}

#[tokio::test]
async fn paid_gate_travel_deducts_the_quote_and_uses_server_terrain_height() {
    let game_state = make_test_game_state("gate_paid_travel");
    let mut rx = setup_gate_player(&game_state, "traveler", "aldermark", 100_000).await;
    let source = teleport_gate("aldermark").unwrap();
    let destination = teleport_gate("garasden").unwrap();
    let fare = teleport_gate_fare(source, destination);

    let outcome = game_state
        .use_teleport_gate_for_test(&pid("traveler"), "aldermark", "garasden", 50, 0, 0)
        .await
        .unwrap();

    assert_eq!(outcome, ("Garasden".to_string(), false, fare));
    assert_eq!(
        game_state.get_player_gold(&pid("traveler")).await,
        100_000 - fare
    );
    let player = &game_state.get_all_players().await[&pid("traveler")];
    let (arrival_x, arrival_z) = destination.arrival_xz();
    assert!((player.position.x - arrival_x).abs() < 0.001);
    assert!((player.position.z - arrival_z).abs() < 0.001);
    assert_eq!(player.position.y, 5.0);
    assert!(drain(&mut rx).iter().any(
        |message| matches!(message, ServerMessage::GoldUpdate { gold } if *gold == 100_000 - fare)
    ));
}

#[tokio::test]
async fn insufficient_funds_do_not_move_or_charge_the_player() {
    let game_state = make_test_game_state("gate_insufficient");
    setup_gate_player(&game_state, "traveler", "aldermark", 1).await;
    let before = game_state.get_all_players().await[&pid("traveler")].position;

    let result = game_state
        .use_teleport_gate_for_test(&pid("traveler"), "aldermark", "garasden", 9_999, 0, 0)
        .await;

    assert_eq!(result.unwrap_err(), "Not enough gold for that journey");
    assert_eq!(game_state.get_player_gold(&pid("traveler")).await, 1);
    assert_eq!(
        game_state.get_all_players().await[&pid("traveler")].position,
        before
    );
}

#[tokio::test]
async fn server_rejects_gate_use_after_the_player_moves_out_of_range() {
    let game_state = make_test_game_state("gate_out_of_range");
    setup_gate_player(&game_state, "traveler", "aldermark", 100_000).await;
    let before = {
        let mut players = game_state.players.write().await;
        let player = players.get_mut(&pid("traveler")).unwrap();
        player.position.x += 20.0;
        player.position
    };

    let result = game_state
        .use_teleport_gate_for_test(&pid("traveler"), "aldermark", "garasden", 9_999, 0, 0)
        .await;

    assert_eq!(result.unwrap_err(), "Move closer to the town gate");
    assert_eq!(game_state.get_player_gold(&pid("traveler")).await, 100_000);
    assert_eq!(
        game_state.get_all_players().await[&pid("traveler")].position,
        before
    );
}

#[tokio::test]
async fn rare_misfire_redirects_to_a_random_surface_point_with_the_original_fare() {
    let game_state = make_test_game_state("gate_surface_misfire");
    setup_gate_player(&game_state, "traveler", "aldermark", 100_000).await;
    let quoted = teleport_gate_fare(
        teleport_gate("aldermark").unwrap(),
        teleport_gate("garasden").unwrap(),
    );

    let (arrival, misfired, charged) = game_state
        .use_teleport_gate_for_test(&pid("traveler"), "aldermark", "garasden", 0, 99, 42)
        .await
        .unwrap();

    assert!(misfired);
    assert!(arrival.starts_with("open water") || arrival.starts_with("remote wilderness"));
    assert_eq!(charged, quoted);
    let player = &game_state.get_all_players().await[&pid("traveler")];
    assert_eq!(player.floor_level, 0);
    assert!(
        (onlinerpg_shared::WORLD_MIN_X..onlinerpg_shared::WORLD_MAX_X).contains(&player.position.x)
    );
    assert!(
        (onlinerpg_shared::WORLD_MIN_Z..onlinerpg_shared::WORLD_MAX_Z).contains(&player.position.z)
    );
    assert_ne!(
        player.position,
        teleport_gate("garasden").unwrap().position(5.0)
    );
}

#[tokio::test]
async fn rare_misfire_can_drop_the_player_inside_a_real_dungeon() {
    let game_state = make_test_game_state("gate_dungeon_misfire");
    setup_gate_player(&game_state, "traveler", "aldermark", 100_000).await;
    let quoted = teleport_gate_fare(
        teleport_gate("aldermark").unwrap(),
        teleport_gate("garasden").unwrap(),
    );

    let (arrival, misfired, charged) = game_state
        .use_teleport_gate_for_test(&pid("traveler"), "aldermark", "garasden", 0, 0, 42)
        .await
        .unwrap();

    let player = &game_state.get_all_players().await[&pid("traveler")];
    assert!(misfired);
    assert!(arrival.contains("dungeon (depth"));
    assert!(player.floor_level < 0);
    assert!(game_state
        .dungeon_defs
        .entrance_at(player.position.x, player.position.z)
        .is_some());
    assert_eq!(charged, quoted);
}
