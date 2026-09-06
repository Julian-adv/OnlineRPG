use super::*;
use onlinerpg_shared::fence::{FenceAxis, FenceEdge};
use onlinerpg_shared::pathfinding::is_movement_blocked;
use onlinerpg_terrain::land::{plot_addr, LandGrade, REGION_PLOTS};

const EDGE: FenceEdge = FenceEdge {
    x: 2,
    z: 1,
    axis: FenceAxis::Z,
};

async fn owner(game: &GameState, auth: &crate::auth::AuthService, name: &str) -> (i64, DirectRx) {
    let account = auth.login_google(name).unwrap();
    let character = create_test_character(auth, &account, name);
    let mut player = make_player(name, 1.5, 1.5);
    player.position.y = 5.05;
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
            bag: vec![
                bag_item(1, "land_deed", 1),
                bag_item(2, "wooden_fence", 100),
            ],
            ..Default::default()
        },
    );
    game.terrain_io
        .write_land_grades(0, 0, &vec![LandGrade::Homestead as u8; REGION_PLOTS])
        .await
        .unwrap();
    let rx = game.register_direct_channel(&pid(name)).await;
    (character.id, rx)
}

async fn claim(game: &GameState, auth: &crate::auth::AuthService, name: &str) {
    game.claim_land(
        &pid(name),
        1,
        super::super::land::plot_key(plot_addr(1.5, 1.5)),
        auth,
    )
    .await;
    assert_eq!(auth.owned_land_plots().unwrap().len(), 1);
}

async fn quantity(game: &GameState, name: &str) -> u32 {
    game.get_player_inventory(&pid(name))
        .await
        .unwrap()
        .bag
        .iter()
        .filter(|i| i.item_def_id == "wooden_fence")
        .map(|i| i.quantity)
        .sum()
}

fn blocked(game: &GameState) -> bool {
    is_movement_blocked(&game.passability_read(), 1.5, 1.5, 2.5, 1.5, 0, Some(5.05))
}

struct FenceTerrain(f32);

#[async_trait::async_trait]
impl onlinerpg_terrain::height::HeightTiles for FenceTerrain {
    async fn read_heightmap(&self, _tx: i32, _tz: i32) -> std::io::Result<Vec<u8>> {
        Ok(uniform_heightmap(self.0))
    }
}

#[tokio::test]
async fn fence_migration_preserves_ownership_and_reloads_current_terrain_height() {
    let game = make_flat_world_game_state("fence_height_migration");
    let (auth, path) = make_test_auth_with_path("fence_height_migration");
    let (owner_id, _) = owner(&game, &auth, "Builder").await;
    claim(&game, &auth, "Builder").await;
    game.edit_fence(&pid("Builder"), EDGE, true, &auth).await;
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("ALTER TABLE land_fences ADD COLUMN y REAL NOT NULL DEFAULT -123;
        ALTER TABLE land_fences ADD COLUMN owner_id INTEGER REFERENCES characters(id) ON DELETE CASCADE;")
        .unwrap();
    db.execute("UPDATE land_fences SET owner_id=?1", [owner_id])
        .unwrap();
    let estate: i64 = db
        .query_row("SELECT estate_id FROM land_fences", [], |row| row.get(0))
        .unwrap();
    let migrated = crate::auth::AuthService::new(path.clone()).unwrap();
    let columns: i64 = db
        .query_row(
            "SELECT count(*) FROM pragma_table_info('land_fences') WHERE name IN ('y','owner_id')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(columns, 0);
    let row = migrated.load_fences().unwrap().remove(0);
    assert_eq!(row.edge, EDGE);
    assert_eq!(row.owner_id, owner_id);
    assert_eq!(
        db.query_row("SELECT estate_id FROM land_fences", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        estate
    );
    crate::auth::AuthService::new(path).unwrap();

    let restarted = make_game_state_with("fence_height_restart", FenceTerrain(12.0), SeaOnlyWater);
    restarted.load_fences(&migrated).await.unwrap();
    assert!(!is_movement_blocked(
        &restarted.passability_read(),
        1.5,
        1.5,
        2.5,
        1.5,
        0,
        Some(13.0)
    ));
    assert!(is_movement_blocked(
        &restarted.passability_read(),
        1.5,
        1.5,
        2.5,
        1.5,
        0,
        Some(12.05)
    ));
    let fences = restarted.fences.read().await;
    let visible = fences.nearby(&Position {
        x: 1.5,
        y: 12.0,
        z: 1.5,
    });
    assert!((visible[0].y - 12.0).abs() < 0.01);
    drop(fences);
    game.edit_fence(&pid("Builder"), EDGE, false, &migrated)
        .await;
    assert_eq!(quantity(&game, "Builder").await, 100);
    assert!(migrated.load_fences().unwrap().is_empty());
}

#[tokio::test]
async fn fence_heights_and_collision_follow_saved_terrain_across_tile_edges() {
    let game = make_flat_world_game_state("fence_height_update");
    let auth = make_test_auth("fence_height_update");
    let (_, mut rx) = owner(&game, &auth, "Builder").await;
    claim(&game, &auth, "Builder").await;
    let border = FenceEdge {
        x: 31,
        z: 1,
        axis: FenceAxis::X,
    };
    for edge in [EDGE, border] {
        game.edit_fence(&pid("Builder"), edge, true, &auth).await;
    }
    drain(&mut rx);
    game.save_terrain_heightmap(0, 0, &uniform_heightmap(9.0))
        .await
        .unwrap();
    assert!(!is_movement_blocked(
        &game.passability_read(),
        1.5,
        1.5,
        2.5,
        1.5,
        0,
        Some(10.0)
    ));
    assert!(is_movement_blocked(
        &game.passability_read(),
        1.5,
        1.5,
        2.5,
        1.5,
        0,
        Some(9.05)
    ));
    assert!(drain(&mut rx).iter().any(|m| matches!(m,
        ServerMessage::FenceVisibility { added, .. }
        if added.len() == 1 && added[0].edge == EDGE && (added[0].y - 9.0).abs() < 0.01
    )));
    assert!(is_movement_blocked(
        &game.passability_read(),
        31.5,
        0.5,
        31.5,
        1.5,
        0,
        Some(5.05)
    ));
    game.save_terrain_heightmap(1, 0, &uniform_heightmap(10.0))
        .await
        .unwrap();
    assert!(!is_movement_blocked(
        &game.passability_read(),
        31.5,
        0.5,
        31.5,
        1.5,
        0,
        Some(10.0)
    ));
    assert!(is_movement_blocked(
        &game.passability_read(),
        31.5,
        0.5,
        31.5,
        1.5,
        0,
        Some(9.05)
    ));
    assert!(drain(&mut rx).iter().any(|m| matches!(m,
        ServerMessage::FenceVisibility { added, .. }
        if added.len() == 1 && added[0].edge == border && (added[0].y - 9.0).abs() < 0.01
    )));
    assert_eq!(quantity(&game, "Builder").await, 98);
    assert_eq!(auth.load_fences().unwrap().len(), 2);
    assert!(game.save_terrain_heightmap(0, 0, &[0]).await.is_err());
    assert!((game.height_sampler.sample_height(2.0, 1.0).await.unwrap() - 9.0).abs() < 0.01);
    game.save_terrain_heightmap(0, 0, &uniform_heightmap(3.0))
        .await
        .unwrap();
    assert!(!blocked(&game));
    assert!(is_movement_blocked(
        &game.passability_read(),
        1.5,
        1.5,
        2.5,
        1.5,
        0,
        Some(3.05)
    ));
}

#[tokio::test]
async fn fence_place_recover_and_restart_preserve_inventory_and_edges() {
    let game = make_flat_world_game_state("fence_roundtrip");
    let auth = make_test_auth("fence_roundtrip");
    let (character_id, mut rx) = owner(&game, &auth, "Builder").await;
    claim(&game, &auth, "Builder").await;
    drain(&mut rx);
    assert!(game.try_start_fence_mode(&pid("Builder"), 2, &auth).await);
    assert!(drain(&mut rx)
        .iter()
        .any(|m| matches!(m, ServerMessage::LandscapingMode { plots, .. } if !plots.is_empty())));
    game.edit_fence(&pid("Builder"), EDGE, true, &auth).await;
    assert_eq!(quantity(&game, "Builder").await, 99);
    assert!(blocked(&game));
    assert!(!is_movement_blocked(
        &game.passability_read(),
        1.5,
        1.5,
        1.5,
        2.5,
        0,
        Some(5.05)
    ));
    assert_eq!(auth.load_fences().unwrap().len(), 1);
    assert!(drain(&mut rx)
        .iter()
        .any(|m| matches!(m, ServerMessage::FenceVisibility { added, .. } if added.len() == 1)));
    let restarted = make_flat_world_game_state("fence_restarted");
    restarted.load_fences(&auth).await.unwrap();
    assert!(blocked(&restarted));
    game.load_player_inventory(&pid("Builder"), character_id, &auth)
        .await;
    assert_eq!(quantity(&game, "Builder").await, 99);
    game.edit_fence(&pid("Builder"), EDGE, false, &auth).await;
    assert_eq!(quantity(&game, "Builder").await, 100);
    assert!(!blocked(&game));
    assert!(auth.load_fences().unwrap().is_empty());
    game.load_player_inventory(&pid("Builder"), character_id, &auth)
        .await;
    assert_eq!(quantity(&game, "Builder").await, 100);
}

#[tokio::test]
async fn fences_cannot_be_stolen_and_overdue_land_can_only_recover() {
    let game = make_flat_world_game_state("fence_owner");
    let (auth, path) = make_test_auth_with_path("fence_owner");
    let (character_id, _) = owner(&game, &auth, "Builder").await;
    claim(&game, &auth, "Builder").await;
    owner(&game, &auth, "Visitor").await;
    game.edit_fence(&pid("Builder"), EDGE, true, &auth).await;
    game.edit_fence(&pid("Visitor"), EDGE, false, &auth).await;
    assert_eq!(quantity(&game, "Visitor").await, 100);
    assert!(blocked(&game));
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute(
        "UPDATE land_estates SET missed=1 WHERE owner_id=?1",
        [character_id],
    )
    .unwrap();
    game.edit_fence(&pid("Builder"), FenceEdge { z: 2, ..EDGE }, true, &auth)
        .await;
    assert_eq!(quantity(&game, "Builder").await, 99);
    game.edit_fence(&pid("Builder"), EDGE, false, &auth).await;
    assert_eq!(quantity(&game, "Builder").await, 100);
    assert!(!blocked(&game));
}

#[tokio::test]
async fn fence_recovery_follows_the_estate_owner_instead_of_the_installer() {
    let game = make_flat_world_game_state("fence_estate_owner");
    let (auth, path) = make_test_auth_with_path("fence_estate_owner");
    let (installer, mut builder_rx) = owner(&game, &auth, "Builder").await;
    claim(&game, &auth, "Builder").await;
    let (new_owner, mut visitor_rx) = owner(&game, &auth, "Visitor").await;
    game.edit_fence(&pid("Builder"), EDGE, true, &auth).await;
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    db.execute(
        "UPDATE land_estates SET owner_id=?1 WHERE owner_id=?2",
        rusqlite::params![new_owner, installer],
    )
    .unwrap();
    assert_eq!(auth.load_fences().unwrap()[0].owner_id, new_owner);

    game.edit_fence(&pid("Builder"), EDGE, false, &auth).await;
    assert_eq!(quantity(&game, "Builder").await, 99);
    assert!(blocked(&game));
    drain(&mut builder_rx);
    drain(&mut visitor_rx);
    game.start_fence_mode(&pid("Visitor"), &auth).await;
    let messages = drain(&mut visitor_rx);
    assert!(messages.iter().any(|m| matches!(m, ServerMessage::LandscapingMode { owner_id, plots, .. } if *owner_id == new_owner && !plots.is_empty())));
    for messages in [&messages, &drain(&mut builder_rx)] {
        assert!(messages
            .iter()
            .any(|m| matches!(m, ServerMessage::FenceVisibility { added, .. }
            if added.iter().any(|f| f.edge == EDGE && f.owner_id == new_owner))));
    }
    assert!(blocked(&game));
    db.execute("DELETE FROM characters WHERE id=?1", [installer])
        .unwrap();
    assert_eq!(auth.load_fences().unwrap().len(), 1);
    game.edit_fence(&pid("Visitor"), EDGE, false, &auth).await;
    assert_eq!(quantity(&game, "Visitor").await, 101);
    assert!(auth.load_fences().unwrap().is_empty());
    assert!(!blocked(&game));
}

#[tokio::test]
async fn last_fence_can_be_recovered_with_an_empty_bag() {
    let game = make_flat_world_game_state("fence_last");
    let auth = make_test_auth("fence_last");
    let (_, mut rx) = owner(&game, &auth, "Builder").await;
    claim(&game, &auth, "Builder").await;
    game.inventories
        .write()
        .await
        .get_mut(&pid("Builder"))
        .unwrap()
        .bag[0]
        .quantity = 1;
    game.edit_fence(&pid("Builder"), EDGE, true, &auth).await;
    assert_eq!(quantity(&game, "Builder").await, 0);
    drain(&mut rx);
    game.start_fence_mode(&pid("Builder"), &auth).await;
    assert!(drain(&mut rx)
        .iter()
        .any(|m| matches!(m, ServerMessage::LandscapingMode { .. })));
    game.edit_fence(&pid("Builder"), EDGE, false, &auth).await;
    assert_eq!(quantity(&game, "Builder").await, 1);
}

#[tokio::test]
async fn failed_inventory_save_restores_removed_fence() {
    let game = make_flat_world_game_state("fence_inventory_rollback");
    let (auth, path) = make_test_auth_with_path("fence_inventory_rollback");
    owner(&game, &auth, "Builder").await;
    claim(&game, &auth, "Builder").await;
    game.edit_fence(&pid("Builder"), EDGE, true, &auth).await;
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_fence_inventory BEFORE DELETE ON character_items BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
    game.edit_fence(&pid("Builder"), EDGE, false, &auth).await;
    assert_eq!(quantity(&game, "Builder").await, 99);
    assert!(blocked(&game));
    assert_eq!(auth.load_fences().unwrap().len(), 1);
}

#[tokio::test]
async fn fence_visibility_follows_join_movement_and_world_wrap() {
    let game = make_flat_world_game_state("fence_visibility");
    let auth = make_test_auth("fence_visibility");
    owner(&game, &auth, "Builder").await;
    claim(&game, &auth, "Builder").await;
    game.edit_fence(&pid("Builder"), EDGE, true, &auth).await;
    let mut viewer = make_player("Viewer", 1.5, 2.5);
    viewer.position.y = 5.05;
    let messages = game.add_player(viewer).await;
    assert!(messages
        .iter()
        .any(|m| matches!(m, ServerMessage::FenceVisibility { added, .. } if added.len() == 1)));
    let mut rx = game.register_direct_channel(&pid("Viewer")).await;
    game.teleport_player(
        &pid("Viewer"),
        Position {
            x: 500.0,
            y: 5.05,
            z: 1.5,
        },
        0.0,
        0,
    )
    .await;
    assert!(drain(&mut rx).iter().any(
        |m| matches!(m, ServerMessage::FenceVisibility { removed, .. } if removed == &vec![EDGE])
    ));
    game.teleport_player(
        &pid("Viewer"),
        Position {
            x: 1.5,
            y: 5.05,
            z: 1.5,
        },
        0.0,
        0,
    )
    .await;
    assert!(drain(&mut rx)
        .iter()
        .any(|m| matches!(m, ServerMessage::FenceVisibility { added, .. } if added.len() == 1)));
    let near = Position {
        x: 1.5 + onlinerpg_shared::WORLD_WIDTH_X,
        y: 5.05,
        z: 1.5,
    };
    assert_eq!(game.fences.read().await.nearby(&near).len(), 1);
}

#[tokio::test]
async fn duplicate_and_concurrent_fence_requests_never_duplicate_items() {
    let game = make_flat_world_game_state("fence_race");
    let auth = make_test_auth("fence_race");
    owner(&game, &auth, "Builder").await;
    claim(&game, &auth, "Builder").await;
    let id = pid("Builder");
    tokio::join!(
        game.edit_fence(&id, EDGE, true, &auth),
        game.edit_fence(&id, EDGE, true, &auth)
    );
    assert_eq!(quantity(&game, "Builder").await, 99);
    assert_eq!(auth.load_fences().unwrap().len(), 1);
    tokio::join!(
        game.edit_fence(&id, EDGE, false, &auth),
        game.edit_fence(&id, EDGE, false, &auth)
    );
    assert_eq!(quantity(&game, "Builder").await, 100);
    assert!(auth.load_fences().unwrap().is_empty());
}

#[tokio::test]
async fn fences_require_owned_land_life_and_available_inventory() {
    let game = make_flat_world_game_state("fence_reject");
    let auth = make_test_auth("fence_reject");
    let (_, mut rx) = owner(&game, &auth, "Builder").await;
    game.edit_fence(&pid("Builder"), EDGE, true, &auth).await;
    assert_eq!(quantity(&game, "Builder").await, 100);
    assert!(auth.load_fences().unwrap().is_empty());
    claim(&game, &auth, "Builder").await;
    for edge in [
        FenceEdge { x: 33, ..EDGE },
        FenceEdge {
            x: i32::MAX,
            ..EDGE
        },
    ] {
        game.edit_fence(&pid("Builder"), edge, true, &auth).await;
    }
    game.players
        .write()
        .await
        .get_mut(&pid("Builder"))
        .unwrap()
        .health = 0;
    game.edit_fence(&pid("Builder"), EDGE, true, &auth).await;
    game.players
        .write()
        .await
        .get_mut(&pid("Builder"))
        .unwrap()
        .health = 10;
    game.inventories
        .write()
        .await
        .get_mut(&pid("Builder"))
        .unwrap()
        .bag
        .clear();
    game.edit_fence(&pid("Builder"), EDGE, true, &auth).await;
    assert!(auth.load_fences().unwrap().is_empty());
    assert!(
        drain(&mut rx)
            .iter()
            .filter(|m| matches!(m, ServerMessage::FenceEditResult { error: Some(_) }))
            .count()
            >= 5
    );
}

#[tokio::test]
async fn fence_editing_reaches_the_whole_estate_and_notifies_the_owner() {
    let game = make_flat_world_game_state("fence_estate_reach");
    let auth = make_test_auth("fence_estate_reach");
    let (_, mut rx) = owner(&game, &auth, "Builder").await;
    claim(&game, &auth, "Builder").await;
    {
        let mut players = game.players.write().await;
        let player = players.get_mut(&pid("Builder")).unwrap();
        player.position.x = super::super::EVENT_DELIVERY_RADIUS + 100.0;
        player.position.y = 10.0;
    }
    drain(&mut rx);
    let edge = FenceEdge { x: 31, ..EDGE };
    game.edit_fence(&pid("Builder"), edge, true, &auth).await;
    assert_eq!(quantity(&game, "Builder").await, 99);
    assert_eq!(auth.load_fences().unwrap()[0].edge, edge);
    assert!(drain(&mut rx).iter().any(|m| matches!(
        m, ServerMessage::FenceVisibility { added, .. } if added.iter().any(|f| f.edge == edge)
    )));
    assert!(is_movement_blocked(
        &game.passability_read(),
        30.5,
        1.5,
        31.5,
        1.5,
        0,
        Some(5.05)
    ));
    game.edit_fence(&pid("Builder"), edge, false, &auth).await;
    assert_eq!(quantity(&game, "Builder").await, 100);
    assert!(auth.load_fences().unwrap().is_empty());
    assert!(drain(&mut rx).iter().any(|m| matches!(
        m, ServerMessage::FenceVisibility { removed, .. } if removed.contains(&edge)
    )));
    assert!(!is_movement_blocked(
        &game.passability_read(),
        30.5,
        1.5,
        31.5,
        1.5,
        0,
        Some(5.05)
    ));
}

#[tokio::test]
async fn failed_fence_save_rolls_back_inventory_and_collision() {
    let game = make_flat_world_game_state("fence_rollback");
    let (auth, path) = make_test_auth_with_path("fence_rollback");
    let (character_id, _) = owner(&game, &auth, "Builder").await;
    claim(&game, &auth, "Builder").await;
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_fence BEFORE INSERT ON land_fences BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
    game.edit_fence(&pid("Builder"), EDGE, true, &auth).await;
    assert_eq!(quantity(&game, "Builder").await, 100);
    assert!(!blocked(&game));
    assert!(auth.load_fences().unwrap().is_empty());
    game.load_player_inventory(&pid("Builder"), character_id, &auth)
        .await;
    assert_eq!(quantity(&game, "Builder").await, 100);
}
