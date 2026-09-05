use super::*;
use onlinerpg_terrain::land::{plot_addr, LandGrade, REGION_PLOTS};

async fn land_owner(
    game: &GameState,
    auth: &crate::auth::AuthService,
    account: &str,
    name: &str,
) -> (i64, DirectRx) {
    let character = create_test_character(auth, account, name);
    let mut player = make_player(name, 1.0, 1.0);
    player.level = 10;
    game.add_player(player).await;
    game.register_player_character(
        &pid(name),
        character.id,
        onlinerpg_shared::xp::xp_for_level(10),
        attrs_with_cha(12),
        0,
        None,
    )
    .await;
    game.inventories.write().await.insert(
        pid(name),
        PlayerInventory {
            bag: (1..=20).map(|id| bag_item(id, "land_deed", 1)).collect(),
            ..Default::default()
        },
    );
    game.terrain_io
        .write_land_grades(0, 0, &vec![LandGrade::Homestead as u8; REGION_PLOTS])
        .await
        .unwrap();
    (character.id, game.register_direct_channel(&pid(name)).await)
}

async fn claim_at(
    game: &GameState,
    auth: &crate::auth::AuthService,
    name: &str,
    deed: u64,
    x: f32,
    z: f32,
) {
    game.players
        .write()
        .await
        .get_mut(&pid(name))
        .unwrap()
        .position = Position { x, y: 1.0, z };
    let plot = super::super::land::plot_key(plot_addr(x, z));
    game.claim_land(&pid(name), deed, plot, auth).await;
}

fn rejected(rx: &mut DirectRx, text: &str) {
    assert!(drain(rx).iter().any(
        |message| matches!(message, ServerMessage::LandRejected { reason } if reason.contains(text))
    ));
}

#[tokio::test]
async fn land_expansion_wraps_across_the_world_seam() {
    let game = make_test_game_state("land_seam");
    let auth = make_test_auth("land_seam");
    let account = auth.login_google("land-seam").unwrap();
    let (_, mut rx) = land_owner(&game, &auth, &account, "Settler").await;
    for rx in [-16, 15] {
        game.terrain_io
            .write_land_grades(rx, 0, &vec![LandGrade::Homestead as u8; REGION_PLOTS])
            .await
            .unwrap();
    }
    claim_at(&game, &auth, "Settler", 1, 16351.0, 1.0).await;
    claim_at(&game, &auth, "Settler", 2, -16415.0, 1.0).await;
    assert_eq!(
        drain(&mut rx)
            .iter()
            .filter(|msg| matches!(msg, ServerMessage::LandClaimed { .. }))
            .count(),
        2
    );
    assert_eq!(
        game.get_player_inventory(&pid("Settler"))
            .await
            .unwrap()
            .bag
            .len(),
        18
    );
    let plots = auth.owned_land_plots().unwrap();
    assert_eq!(plots.len(), 2);
    for (x, z) in [(16351.0, 1.0), (-16415.0, 1.0)] {
        let addr = plot_addr(x, z);
        assert!(plots.iter().any(|plot| plot.rx == addr.rx
            && plot.rz == addr.rz
            && plot.index == addr.index
            && plot.owner_name == "Settler"));
    }
}

#[tokio::test]
async fn land_deed_reserved_for_trade_cannot_be_spent() {
    let game = make_test_game_state("land_trade");
    let auth = make_test_auth("land_trade");
    let account = auth.login_google("land-trade").unwrap();
    let (_, mut rx) = land_owner(&game, &auth, &account, "Settler").await;
    land_owner(&game, &auth, &account, "Buyer").await;
    game.request_player_trade(&pid("Settler"), "Buyer").await;
    game.respond_player_trade(&pid("Buyer"), &pid("Settler"), true)
        .await;
    game.set_player_trade_offer(
        &pid("Settler"),
        vec![onlinerpg_shared::messages::PlayerTradeSlot {
            instance_id: 1,
            quantity: 1,
        }],
        0,
    )
    .await;
    assert_eq!(game.trade_reserved_quantity(&pid("Settler"), 1).await, 1);
    claim_at(&game, &auth, "Settler", 1, 1.0, 1.0).await;
    rejected(&mut rx, "reserved for trade");
    assert_eq!(
        game.get_player_inventory(&pid("Settler"))
            .await
            .unwrap()
            .bag
            .len(),
        20
    );
}

#[tokio::test]
async fn land_preview_spends_nothing_and_confirm_persists_once() {
    let game = make_test_game_state("land_persist");
    let (auth, path) = make_test_auth_with_path("land_persist");
    let account = auth.login_google("land-persist").unwrap();
    let (character, mut rx) = land_owner(&game, &auth, &account, "Settler").await;
    assert!(game.try_preview_land_claim(&pid("Settler"), 1, &auth).await);
    assert!(drain(&mut rx).iter().any(|message| matches!(
        message,
        ServerMessage::LandClaimPrompt {
            instance_id: 1,
            tile_x: 0,
            tile_z: 0,
            quadrant: 3,
            reason: None,
        }
    )));
    assert_eq!(
        game.get_player_inventory(&pid("Settler"))
            .await
            .unwrap()
            .bag
            .len(),
        20
    );

    claim_at(&game, &auth, "Settler", 1, 1.0, 1.0).await;
    assert!(drain(&mut rx)
        .iter()
        .any(|message| matches!(message, ServerMessage::LandClaimed { .. })));
    assert_eq!(
        game.get_player_inventory(&pid("Settler"))
            .await
            .unwrap()
            .bag
            .len(),
        19
    );
    let reopened = crate::auth::AuthService::new(path.clone()).unwrap();
    assert_eq!(reopened.load_inventory(character).unwrap().len(), 19);
    let conn = rusqlite::Connection::open(path).unwrap();
    let owner: i64 = conn.query_row("SELECT owner_id FROM land_estates JOIN land_plots ON estate_id=land_estates.id WHERE tile_x=0 AND tile_z=0 AND quadrant=3", [], |row| row.get(0)).unwrap();
    assert_eq!(owner, character);
    claim_at(&game, &auth, "Settler", 2, 1.0, 1.0).await;
    rejected(&mut rx, "already belongs");
    claim_at(&game, &auth, "Settler", 1, 33.0, 1.0).await;
    rejected(&mut rx, "not found");
    assert_eq!(
        game.get_player_inventory(&pid("Settler"))
            .await
            .unwrap()
            .bag
            .len(),
        19
    );
}

#[tokio::test]
async fn land_rechecks_level_location_alive_floor_and_document() {
    let game = make_test_game_state("land_conditions");
    let auth = make_test_auth("land_conditions");
    let account = auth.login_google("land-conditions").unwrap();
    let (_, mut rx) = land_owner(&game, &auth, &account, "Settler").await;
    game.players
        .write()
        .await
        .get_mut(&pid("Settler"))
        .unwrap()
        .level = 9;
    claim_at(&game, &auth, "Settler", 1, 1.0, 1.0).await;
    rejected(&mut rx, "level 10");
    game.players
        .write()
        .await
        .get_mut(&pid("Settler"))
        .unwrap()
        .level = 10;
    game.players
        .write()
        .await
        .get_mut(&pid("Settler"))
        .unwrap()
        .health = 0;
    claim_at(&game, &auth, "Settler", 1, 1.0, 1.0).await;
    rejected(&mut rx, "alive");
    game.players
        .write()
        .await
        .get_mut(&pid("Settler"))
        .unwrap()
        .health = 1;
    game.players
        .write()
        .await
        .get_mut(&pid("Settler"))
        .unwrap()
        .floor_level = -1;
    claim_at(&game, &auth, "Settler", 1, 1.0, 1.0).await;
    rejected(&mut rx, "outdoor");
    game.players
        .write()
        .await
        .get_mut(&pid("Settler"))
        .unwrap()
        .floor_level = 0;
    game.claim_land(&pid("Settler"), 1, (0, 0, 2), &auth).await;
    rejected(&mut rx, "left the selected");
    game.claim_land(&pid("Settler"), 1, (0, 0, 255), &auth)
        .await;
    rejected(&mut rx, "left the selected");
    game.inventories
        .write()
        .await
        .get_mut(&pid("Settler"))
        .unwrap()
        .bag[0]
        .item_def_id = "torch".into();
    claim_at(&game, &auth, "Settler", 1, 1.0, 1.0).await;
    rejected(&mut rx, "not found");
    assert_eq!(
        game.get_player_inventory(&pid("Settler"))
            .await
            .unwrap()
            .bag
            .len(),
        20
    );
}

#[tokio::test]
async fn land_uses_current_edited_grades_and_fails_closed_on_bad_files() {
    let game = make_test_game_state("land_grades");
    let auth = make_test_auth("land_grades");
    let account = auth.login_google("land-grades").unwrap();
    let (_, mut rx) = land_owner(&game, &auth, &account, "Settler").await;
    for (grade, reason) in [
        (LandGrade::Crown, "Crown"),
        (LandGrade::Reserved, "reserved"),
    ] {
        game.terrain_io
            .write_land_grades(0, 0, &vec![grade as u8; REGION_PLOTS])
            .await
            .unwrap();
        claim_at(&game, &auth, "Settler", 1, 1.0, 1.0).await;
        rejected(&mut rx, reason);
    }
    let path = onlinerpg_terrain::coords::land_grade_path(game.terrain_io.base_dir(), 0, 0);
    std::fs::write(path, [1]).unwrap();
    claim_at(&game, &auth, "Settler", 1, 1.0, 1.0).await;
    rejected(&mut rx, "temporarily unavailable");
    assert_eq!(
        game.get_player_inventory(&pid("Settler"))
            .await
            .unwrap()
            .bag
            .len(),
        20
    );
}

#[tokio::test]
async fn land_expansion_requires_edges_and_enforces_account_and_size_limits() {
    let game = make_test_game_state("land_limits");
    let auth = make_test_auth("land_limits");
    let account = auth.login_google("land-limits").unwrap();
    let (_, mut rx) = land_owner(&game, &auth, &account, "Settler").await;
    let (_, mut alt_rx) = land_owner(&game, &auth, &account, "Sibling").await;
    claim_at(&game, &auth, "Settler", 1, 1.0, 1.0).await;
    claim_at(&game, &auth, "Sibling", 1, 33.0, 1.0).await;
    rejected(&mut alt_rx, "Another character");
    claim_at(&game, &auth, "Settler", 2, 33.0, 33.0).await;
    rejected(&mut rx, "shares an edge");
    for i in 1..8 {
        claim_at(&game, &auth, "Settler", i + 1, i as f32 * 32.0 + 1.0, 1.0).await;
    }
    claim_at(&game, &auth, "Settler", 9, 257.0, 1.0).await;
    rejected(&mut rx, "8 by 8");
    for i in 0..8 {
        claim_at(&game, &auth, "Settler", i + 9, i as f32 * 32.0 + 1.0, 33.0).await;
    }
    claim_at(&game, &auth, "Settler", 17, 1.0, 65.0).await;
    rejected(&mut rx, "16 plots");
    assert_eq!(
        game.get_player_inventory(&pid("Settler"))
            .await
            .unwrap()
            .bag
            .len(),
        4
    );
}

#[tokio::test]
async fn land_competing_claims_have_one_winner() {
    let game = make_test_game_state("land_race");
    let auth = make_test_auth("land_race");
    let a = auth.login_google("land-race-a").unwrap();
    let b = auth.login_google("land-race-b").unwrap();
    let (_, mut arx) = land_owner(&game, &auth, &a, "First").await;
    let (_, mut brx) = land_owner(&game, &auth, &b, "Second").await;
    let first = pid("First");
    let second = pid("Second");
    tokio::join!(
        game.claim_land(&first, 1, (0, 0, 3), &auth),
        game.claim_land(&second, 1, (0, 0, 3), &auth)
    );
    let messages: Vec<_> = drain(&mut arx).into_iter().chain(drain(&mut brx)).collect();
    assert_eq!(
        messages
            .iter()
            .filter(|msg| matches!(msg, ServerMessage::LandClaimed { .. }))
            .count(),
        1
    );
    assert_eq!(
        messages
            .iter()
            .filter(|msg| matches!(msg, ServerMessage::LandRejected { .. }))
            .count(),
        1
    );
    assert_eq!(
        game.get_player_inventory(&pid("First"))
            .await
            .unwrap()
            .bag
            .len()
            + game
                .get_player_inventory(&pid("Second"))
                .await
                .unwrap()
                .bag
                .len(),
        39
    );
}

#[tokio::test]
async fn land_db_failure_rolls_back_ownership_and_keeps_deed() {
    let game = make_test_game_state("land_rollback");
    let (auth, path) = make_test_auth_with_path("land_rollback");
    let account = auth.login_google("land-rollback").unwrap();
    let (character, mut rx) = land_owner(&game, &auth, &account, "Settler").await;
    let initial = auth.load_inventory(character).unwrap();
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_land_inventory BEFORE INSERT ON character_items BEGIN SELECT RAISE(FAIL, 'test failure'); END;").unwrap();
    claim_at(&game, &auth, "Settler", 1, 1.0, 1.0).await;
    rejected(&mut rx, "could not be saved");
    assert_eq!(
        game.get_player_inventory(&pid("Settler"))
            .await
            .unwrap()
            .bag
            .len(),
        20
    );
    assert_eq!(auth.load_inventory(character).unwrap(), initial);
    for table in ["land_estates", "land_plots"] {
        assert_eq!(
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

async fn preview_rejected(
    game: &GameState,
    auth: &crate::auth::AuthService,
    rx: &mut DirectRx,
    reason: &str,
) {
    assert!(game.try_preview_land_claim(&pid("Settler"), 2, auth).await);
    let messages = drain(rx);
    assert!(messages.iter().any(|message| matches!(message,
        ServerMessage::LandClaimPrompt { instance_id: 2, reason: Some(text), .. } if text.contains(reason)
    )), "expected preview rejection: {reason}, got {messages:?}");
    assert!(!messages
        .iter()
        .any(|message| matches!(message, ServerMessage::LandClaimed { .. })));
}

#[tokio::test]
async fn land_preview_checks_level_grades_and_ownership_without_spending() {
    let game = make_test_game_state("land_preview_rejected");
    let auth = make_test_auth("land_preview_rejected");
    let account = auth.login_google("land-preview-rejected").unwrap();
    let (_, mut rx) = land_owner(&game, &auth, &account, "Settler").await;
    game.players
        .write()
        .await
        .get_mut(&pid("Settler"))
        .unwrap()
        .level = 9;
    preview_rejected(&game, &auth, &mut rx, "level 10").await;
    game.players
        .write()
        .await
        .get_mut(&pid("Settler"))
        .unwrap()
        .level = 10;
    for (grade, reason) in [
        (LandGrade::Crown, "Crown"),
        (LandGrade::Reserved, "reserved"),
    ] {
        game.terrain_io
            .write_land_grades(0, 0, &vec![grade as u8; REGION_PLOTS])
            .await
            .unwrap();
        preview_rejected(&game, &auth, &mut rx, reason).await;
    }
    assert_eq!(
        game.get_player_inventory(&pid("Settler"))
            .await
            .unwrap()
            .bag
            .len(),
        20
    );
    game.terrain_io
        .write_land_grades(0, 0, &vec![LandGrade::Homestead as u8; REGION_PLOTS])
        .await
        .unwrap();
    claim_at(&game, &auth, "Settler", 1, 1.0, 1.0).await;
    drain(&mut rx);
    preview_rejected(&game, &auth, &mut rx, "already belongs").await;
    game.players
        .write()
        .await
        .get_mut(&pid("Settler"))
        .unwrap()
        .position
        .x = 65.0;
    preview_rejected(&game, &auth, &mut rx, "shares an edge").await;
    game.players
        .write()
        .await
        .get_mut(&pid("Settler"))
        .unwrap()
        .position
        .x = 33.0;
    assert!(game.try_preview_land_claim(&pid("Settler"), 2, &auth).await);
    assert!(drain(&mut rx)
        .iter()
        .any(|message| matches!(message, ServerMessage::LandClaimPrompt { reason: None, .. })));
    assert_eq!(
        game.get_player_inventory(&pid("Settler"))
            .await
            .unwrap()
            .bag
            .len(),
        19
    );
}
