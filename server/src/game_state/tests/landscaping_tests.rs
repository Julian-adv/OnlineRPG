use super::*;
use onlinerpg_shared::landscaping::{LandscapingStroke, TOOLBOX_ITEM};
use onlinerpg_terrain::land::{plot_addr, LandGrade, REGION_PLOTS};

async fn gardener(
    game: &GameState,
    auth: &crate::auth::AuthService,
    name: &str,
) -> (i64, DirectRx) {
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
                bag_item(2, TOOLBOX_ITEM, 1),
                bag_item(3, "landscaping_palette_sand", 2),
            ],
            ..Default::default()
        },
    );
    (character.id, game.register_direct_channel(&pid(name)).await)
}

async fn claim(game: &GameState, auth: &crate::auth::AuthService) {
    game.terrain_io
        .write_land_grades(0, 0, &vec![LandGrade::Homestead as u8; REGION_PLOTS])
        .await
        .unwrap();
    game.claim_land(
        &pid("Gardener"),
        1,
        super::super::land::plot_key(plot_addr(1.5, 1.5)),
        auth,
    )
    .await;
    assert_eq!(auth.owned_land_plots().unwrap().len(), 1);
}

fn road() -> LandscapingStroke {
    LandscapingStroke {
        start: [4.0, 4.0],
        end: Some([10.0, 4.0]),
        radius: 2.0,
        strength: 10,
        palette: 5,
    }
}

async fn paint(
    game: &GameState,
    auth: &crate::auth::AuthService,
    rx: &mut DirectRx,
    stroke: LandscapingStroke,
    success: bool,
) {
    drain(rx);
    game.edit_landscape(&pid("Gardener"), stroke, auth, false)
        .await;
    assert!(drain(rx).iter().any(|msg| matches!(msg, ServerMessage::LandscapeEditResult { error } if error.is_none() == success)));
}

#[tokio::test]
async fn landscaping_saves_free_road_without_height_edits_and_syncs_near_and_far_players() {
    let game = make_flat_world_game_state("landscaping_sync");
    let auth = make_test_auth("landscaping_sync");
    let (_, mut rx) = gardener(&game, &auth, "Gardener").await;
    claim(&game, &auth).await;
    let (_, mut near) = gardener(&game, &auth, "Near").await;
    let (_, mut far) = gardener(&game, &auth, "Far").await;
    game.players
        .write()
        .await
        .get_mut(&pid("Far"))
        .unwrap()
        .position
        .x = 5000.0;
    let height = game.terrain_io.read_heightmap(0, 0).await.unwrap();
    let bag = game.get_player_inventory(&pid("Gardener")).await.unwrap();
    paint(&game, &auth, &mut rx, road(), true).await;
    assert_eq!(game.terrain_io.read_heightmap(0, 0).await.unwrap(), height);
    assert_eq!(
        game.get_player_inventory(&pid("Gardener"))
            .await
            .unwrap()
            .bag,
        bag.bag
    );
    assert_eq!(game.splat_sampler.dominant_at(6.0, 4.0).await.unwrap(), 5);
    assert!(drain(&mut near)
        .iter()
        .any(|msg| matches!(msg, ServerMessage::LandscapeChanged { tiles } if tiles.len() == 1)));
    assert!(drain(&mut far).iter().any(
        |msg| matches!(msg, ServerMessage::LandscapeInvalidated { tiles } if tiles == &vec![(0, 0)])
    ));
    let saved = game
        .terrain_io
        .read_landscaping_tile(0, 0)
        .await
        .unwrap()
        .unwrap();
    assert!(saved.cleared.iter().any(|byte| *byte != 0));
    let mut meadow = road();
    meadow.palette = 0;
    paint(&game, &auth, &mut rx, meadow, true).await;
    let restored = game
        .terrain_io
        .read_landscaping_tile(0, 0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.cleared, restored.cleared);
    assert_eq!(game.splat_sampler.dominant_at(6.0, 4.0).await.unwrap(), 0);
}

#[tokio::test]
async fn landscaping_rechecks_toolbox_life_location_palette_and_tax_permissions() {
    let game = make_flat_world_game_state("landscaping_permissions");
    let (auth, path) = make_test_auth_with_path("landscaping_permissions");
    let (owner, mut rx) = gardener(&game, &auth, "Gardener").await;
    claim(&game, &auth).await;
    let toolbox = game
        .inventories
        .write()
        .await
        .get_mut(&pid("Gardener"))
        .unwrap()
        .bag
        .remove(0);
    assert_eq!(toolbox.item_def_id, TOOLBOX_ITEM);
    paint(&game, &auth, &mut rx, road(), false).await;
    game.inventories
        .write()
        .await
        .get_mut(&pid("Gardener"))
        .unwrap()
        .bag
        .push(toolbox);
    let mut locked = road();
    locked.palette = 1;
    paint(&game, &auth, &mut rx, locked, false).await;
    for (x, health, floor) in [(1000.0, 100, 0), (1.5, 0, 0), (1.5, 100, 1)] {
        {
            let mut players = game.players.write().await;
            let player = players.get_mut(&pid("Gardener")).unwrap();
            player.position.x = x;
            player.health = health;
            player.floor_level = floor;
        }
        paint(&game, &auth, &mut rx, road(), false).await;
    }
    {
        let mut players = game.players.write().await;
        let player = players.get_mut(&pid("Gardener")).unwrap();
        player.floor_level = 0;
    }
    let mut outside = road();
    outside.start = [500.0, 500.0];
    outside.end = None;
    paint(&game, &auth, &mut rx, outside, false).await;
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute(
        "UPDATE land_estates SET missed=1 WHERE owner_id=?1",
        [owner],
    )
    .unwrap();
    paint(&game, &auth, &mut rx, road(), false).await;
    assert!(game
        .terrain_io
        .read_landscaping_tile(0, 0)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn landscaping_toolbox_is_reusable_and_palette_consumption_is_atomic_and_character_bound() {
    let game = make_flat_world_game_state("landscaping_palette");
    let (auth, path) = make_test_auth_with_path("landscaping_palette");
    let (owner, mut rx) = gardener(&game, &auth, "Gardener").await;
    let (other, _) = gardener(&game, &auth, "Other").await;
    claim(&game, &auth).await;
    drain(&mut rx);
    assert!(
        game.try_use_landscaping_item(&pid("Gardener"), 2, &auth, false)
            .await
    );
    assert!(drain(&mut rx).iter().any(|msg| matches!(msg, ServerMessage::LandscapingMode { palette, has_toolbox: true, .. } if palette == &vec![0, 5])));
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_palette BEFORE INSERT ON character_landscaping_palettes BEGIN SELECT RAISE(ABORT, 'test'); END;").unwrap();
    game.try_use_landscaping_item(&pid("Gardener"), 3, &auth, false)
        .await;
    assert_eq!(auth.landscaping_palette(owner).unwrap(), vec![0, 5]);
    assert_eq!(
        game.get_player_inventory(&pid("Gardener"))
            .await
            .unwrap()
            .bag
            .iter()
            .find(|item| item.item_def_id == "landscaping_palette_sand")
            .unwrap()
            .quantity,
        2
    );
    db.execute_batch("DROP TRIGGER reject_palette").unwrap();
    for _ in 0..2 {
        game.try_use_landscaping_item(&pid("Gardener"), 3, &auth, false)
            .await;
    }
    let reloaded = crate::auth::AuthService::new(path).unwrap();
    assert_eq!(reloaded.landscaping_palette(owner).unwrap(), vec![0, 1, 5]);
    assert_eq!(reloaded.landscaping_palette(other).unwrap(), vec![0, 5]);
    game.load_player_inventory(&pid("Gardener"), owner, &reloaded)
        .await;
    let bag = game
        .get_player_inventory(&pid("Gardener"))
        .await
        .unwrap()
        .bag;
    assert_eq!(
        bag.iter()
            .find(|item| item.item_def_id == "landscaping_palette_sand")
            .unwrap()
            .quantity,
        1
    );
    assert_eq!(
        bag.iter()
            .find(|item| item.item_def_id == TOOLBOX_ITEM)
            .unwrap()
            .quantity,
        1
    );
    let mut sand = road();
    sand.palette = 1;
    paint(&game, &auth, &mut rx, sand, true).await;
}

#[tokio::test]
async fn landscaping_rejects_visitors_and_items_reserved_in_player_trade() {
    let game = make_flat_world_game_state("landscaping_trade");
    let auth = make_test_auth("landscaping_trade");
    let (_, mut rx) = gardener(&game, &auth, "Gardener").await;
    claim(&game, &auth).await;
    let (_, mut visitor_rx) = gardener(&game, &auth, "Visitor").await;
    game.edit_landscape(&pid("Visitor"), road(), &auth, false)
        .await;
    assert!(drain(&mut visitor_rx)
        .iter()
        .any(|msg| matches!(msg, ServerMessage::LandscapeEditResult { error: Some(_) })));
    game.request_player_trade(&pid("Gardener"), "Visitor").await;
    game.respond_player_trade(&pid("Visitor"), &pid("Gardener"), true)
        .await;
    paint(&game, &auth, &mut rx, road(), false).await;
    game.try_use_landscaping_item(&pid("Gardener"), 3, &auth, false)
        .await;
    assert_eq!(
        game.get_player_inventory(&pid("Gardener"))
            .await
            .unwrap()
            .bag
            .iter()
            .find(|item| item.item_def_id == "landscaping_palette_sand")
            .unwrap()
            .quantity,
        2
    );
    assert!(game
        .terrain_io
        .read_landscaping_tile(0, 0)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn landscaping_admin_can_open_toolbox_and_clear_trees_on_unowned_ground() {
    let game = make_flat_world_game_state("landscaping_admin");
    let auth = make_test_auth("landscaping_admin");
    let (_, mut rx) = gardener(&game, &auth, "Gardener").await;
    let id = pid("Gardener");
    assert!(auth.owned_land_plots().unwrap().is_empty());
    game.try_use_landscaping_item(&id, 2, &auth, false).await;
    assert!(!drain(&mut rx)
        .iter()
        .any(|msg| matches!(msg, ServerMessage::LandscapingMode { .. })));
    game.try_use_landscaping_item(&id, 2, &auth, true).await;
    assert!(drain(&mut rx).iter().any(|msg| matches!(msg, ServerMessage::LandscapingMode { plots, has_toolbox: true, .. } if plots.is_empty())));

    let mut trees = Vec::new();
    trees.extend_from_slice(&onlinerpg_shared::tree_format::TREE_V1_MAGIC.to_le_bytes());
    trees.extend_from_slice(&2u32.to_le_bytes());
    trees.extend_from_slice(&0u32.to_le_bytes());
    for (x, z) in [(38.5f32, 36.5f32), (52.5, 52.5)] {
        for value in [x, z] {
            trees.extend_from_slice(&((value / 64.0 * 65535.0) as u16).to_le_bytes());
        }
        trees.extend_from_slice(&[0, 128]);
    }
    game.terrain_io.write_trees(0, 0, &trees).await.unwrap();
    paint(&game, &auth, &mut rx, road(), false).await;
    game.edit_landscape(&id, road(), &auth, true).await;
    assert!(drain(&mut rx)
        .iter()
        .any(|msg| matches!(msg, ServerMessage::LandscapeEditResult { error: None })));
    assert_eq!(game.splat_sampler.dominant_at(6.0, 4.0).await.unwrap(), 5);
    let filtered = game.terrain_io.read_trees(0, 0).await.unwrap().unwrap();
    assert_eq!(u32::from_le_bytes(filtered[4..8].try_into().unwrap()), 1);
    assert_eq!(
        filtered.len(),
        trees.len() - onlinerpg_shared::tree_format::TREE_V1_BYTES_PER_INSTANCE
    );
    paint(&game, &auth, &mut rx, road(), false).await;

    for (health, floor, palette) in [(0, 0, 5), (100, 1, 5), (100, 0, 1)] {
        {
            let mut players = game.players.write().await;
            let player = players.get_mut(&id).unwrap();
            player.health = health;
            player.floor_level = floor;
        }
        game.edit_landscape(&id, LandscapingStroke { palette, ..road() }, &auth, true)
            .await;
        assert!(drain(&mut rx)
            .iter()
            .any(|msg| matches!(msg, ServerMessage::LandscapeEditResult { error: Some(_) })));
    }
    game.inventories
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .bag
        .clear();
    game.edit_landscape(&id, road(), &auth, true).await;
    assert!(drain(&mut rx)
        .iter()
        .any(|msg| matches!(msg, ServerMessage::LandscapeEditResult { error: Some(_) })));
}

#[tokio::test]
async fn landscaping_meadow_does_not_clear_existing_trees_on_grassless_ground() {
    let game = make_flat_world_game_state("landscaping_meadow");
    let auth = make_test_auth("landscaping_meadow");
    let (_, mut rx) = gardener(&game, &auth, "Gardener").await;
    claim(&game, &auth).await;
    let mut meadow = road();
    meadow.palette = 0;
    paint(&game, &auth, &mut rx, meadow, true).await;
    assert!(game
        .terrain_io
        .read_landscaping_tile(0, 0)
        .await
        .unwrap()
        .is_none());
}
