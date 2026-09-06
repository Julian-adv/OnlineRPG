use super::*;
use onlinerpg_shared::messages::BagLineItem;
use onlinerpg_terrain::land::{plot_addr, LandGrade, REGION_PLOTS};

async fn storage_owner(game: &GameState, auth: &crate::auth::AuthService, name: &str) -> i64 {
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
                bag_item(2, "storage_chest", 1),
                bag_item(3, "apple", 3),
                bag_item(4, "worn_torch", 1),
            ],
            ..Default::default()
        },
    );
    game.terrain_io
        .write_land_grades(0, 0, &vec![LandGrade::Homestead as u8; REGION_PLOTS])
        .await
        .unwrap();
    game.claim_land(
        &pid(name),
        1,
        super::super::land::plot_key(plot_addr(1.5, 1.5)),
        auth,
    )
    .await;
    character.id
}

fn item_quantity(inventory: &PlayerInventory, item_def_id: &str) -> u32 {
    inventory
        .bag
        .iter()
        .filter(|item| item.item_def_id == item_def_id)
        .map(|item| item.quantity)
        .sum()
}

#[tokio::test]
async fn estate_storage_can_be_placed_anywhere_on_owned_estate() {
    let game = make_flat_world_game_state("estate_storage_remote_placement");
    let (auth, _) = make_test_auth_with_path("estate_storage_remote_placement");
    storage_owner(&game, &auth, "RemoteKeeper").await;

    game.place_estate_chest(
        &pid("RemoteKeeper"),
        2,
        Position {
            x: 30.5,
            y: 5.0,
            z: 30.5,
        },
        0.0,
        0,
        &auth,
    )
    .await;

    let chests = auth.load_estate_chests().unwrap();
    assert_eq!(chests.len(), 1);
    assert_eq!(chests[0].position.x, 30.5);
    assert_eq!(chests[0].position.z, 30.5);
}

#[tokio::test]
async fn estate_can_hold_multiple_storage_chests() {
    let game = make_flat_world_game_state("estate_storage_multiple");
    let (auth, _) = make_test_auth_with_path("estate_storage_multiple");
    storage_owner(&game, &auth, "MultiKeeper").await;

    game.place_estate_chest(
        &pid("MultiKeeper"),
        2,
        Position {
            x: 2.5,
            y: 5.0,
            z: 2.5,
        },
        0.0,
        0,
        &auth,
    )
    .await;
    game.inventories
        .write()
        .await
        .get_mut(&pid("MultiKeeper"))
        .unwrap()
        .bag
        .push(bag_item(5, "storage_chest", 1));
    game.place_estate_chest(
        &pid("MultiKeeper"),
        5,
        Position {
            x: 10.5,
            y: 5.0,
            z: 10.5,
        },
        90.0,
        0,
        &auth,
    )
    .await;

    assert_eq!(auth.load_estate_chests().unwrap().len(), 2);

    game.inventories
        .write()
        .await
        .get_mut(&pid("MultiKeeper"))
        .unwrap()
        .bag
        .push(bag_item(6, "storage_chest", 1));
    game.place_estate_chest(
        &pid("MultiKeeper"),
        6,
        Position {
            x: 10.5,
            y: 5.0,
            z: 10.5,
        },
        90.0,
        0,
        &auth,
    )
    .await;

    assert_eq!(auth.load_estate_chests().unwrap().len(), 2);
    assert_eq!(
        item_quantity(
            &game
                .get_player_inventory(&pid("MultiKeeper"))
                .await
                .unwrap(),
            "storage_chest"
        ),
        1
    );
}

#[tokio::test]
async fn estate_storage_round_trip_is_persistent_and_recovery_requires_empty() {
    let game = make_flat_world_game_state("estate_storage_round_trip");
    let (auth, path) = make_test_auth_with_path("estate_storage_round_trip");
    let character_id = storage_owner(&game, &auth, "Keeper").await;

    game.place_estate_chest(
        &pid("Keeper"),
        2,
        Position {
            x: 2.5,
            y: 99.0,
            z: 2.5,
        },
        90.0,
        0,
        &auth,
    )
    .await;

    let chest = auth.load_estate_chests().unwrap().remove(0);
    assert!((chest.position.y - 5.0).abs() < 0.01);
    assert_eq!(chest.owner_id, character_id);
    assert_eq!(chest.item_def_id, "storage_chest");
    assert_eq!(
        item_quantity(
            &game.get_player_inventory(&pid("Keeper")).await.unwrap(),
            "storage_chest"
        ),
        0
    );

    game.transfer_estate_items(
        &pid("Keeper"),
        chest.id,
        vec![BagLineItem {
            instance_id: 3,
            qty: 2,
        }],
        vec![],
        0,
        &auth,
    )
    .await;
    let state = auth
        .estate_chest_state(chest.id, character_id)
        .unwrap()
        .unwrap();
    assert_eq!(state.revision, 1);
    assert_eq!(state.item_def_id, "storage_chest");
    assert_eq!(state.items[0].item_def_id, "apple");
    assert_eq!(state.items[0].quantity, 2);
    let stored_id = state.items[0].instance_id;

    game.transfer_estate_items(
        &pid("Keeper"),
        chest.id,
        vec![BagLineItem {
            instance_id: 4,
            qty: 1,
        }],
        vec![],
        1,
        &auth,
    )
    .await;
    let unchanged = auth
        .estate_chest_state(chest.id, character_id)
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.revision, 1);
    assert_eq!(unchanged.items.len(), 1);

    let db = rusqlite::Connection::open(path).unwrap();
    db.execute("UPDATE land_estates SET missed=1", []).unwrap();
    game.transfer_estate_items(
        &pid("Keeper"),
        chest.id,
        vec![BagLineItem {
            instance_id: 3,
            qty: 1,
        }],
        vec![],
        1,
        &auth,
    )
    .await;
    let overdue = auth
        .estate_chest_state(chest.id, character_id)
        .unwrap()
        .unwrap();
    assert!(!overdue.can_deposit);
    assert_eq!(overdue.revision, 1);
    assert_eq!(overdue.items[0].quantity, 2);

    game.recover_estate_chest(&pid("Keeper"), chest.id, &auth)
        .await;
    assert_eq!(auth.load_estate_chests().unwrap().len(), 1);

    game.transfer_estate_items(
        &pid("Keeper"),
        chest.id,
        vec![],
        vec![BagLineItem {
            instance_id: stored_id,
            qty: 2,
        }],
        1,
        &auth,
    )
    .await;
    assert!(auth
        .estate_chest_state(chest.id, character_id)
        .unwrap()
        .unwrap()
        .items
        .is_empty());
    assert_eq!(
        item_quantity(
            &game.get_player_inventory(&pid("Keeper")).await.unwrap(),
            "apple"
        ),
        3
    );

    db.execute("DELETE FROM land_estates", []).unwrap();
    let scavenger_account = auth.login_google("Scavenger").unwrap();
    let scavenger = create_test_character(&auth, &scavenger_account, "Scavenger");
    let abandoned = auth
        .estate_chest_state(chest.id, scavenger.id)
        .unwrap()
        .unwrap();
    assert!(!abandoned.can_deposit);
    assert!(abandoned.items.is_empty());

    game.recover_estate_chest(&pid("Keeper"), chest.id, &auth)
        .await;
    assert!(auth.load_estate_chests().unwrap().is_empty());
    assert_eq!(
        item_quantity(
            &game.get_player_inventory(&pid("Keeper")).await.unwrap(),
            "storage_chest"
        ),
        1
    );
}

#[tokio::test]
async fn estate_storage_accepts_fifty_kg_and_rejects_more() {
    let game = make_flat_world_game_state("estate_storage_weight_limit");
    let (auth, _) = make_test_auth_with_path("estate_storage_weight_limit");
    let character_id = storage_owner(&game, &auth, "WeightKeeper").await;
    {
        let mut inventories = game.inventories.write().await;
        let inventory = inventories.get_mut(&pid("WeightKeeper")).unwrap();
        inventory.bag.push(bag_item(5, "stone_hearth", 1));
        inventory.bag.push(bag_item(6, "campfire_kit", 51));
    }

    game.place_estate_chest(
        &pid("WeightKeeper"),
        2,
        Position {
            x: 2.5,
            y: 5.0,
            z: 2.5,
        },
        0.0,
        0,
        &auth,
    )
    .await;
    let chest = auth.load_estate_chests().unwrap().remove(0);

    game.transfer_estate_items(
        &pid("WeightKeeper"),
        chest.id,
        vec![
            BagLineItem {
                instance_id: 5,
                qty: 1,
            },
            BagLineItem {
                instance_id: 6,
                qty: 50,
            },
        ],
        vec![],
        0,
        &auth,
    )
    .await;
    let full = auth
        .estate_chest_state(chest.id, character_id)
        .unwrap()
        .unwrap();
    assert_eq!(full.max_weight, 500.0);
    assert_eq!(full.revision, 1);
    assert_eq!(full.items.len(), 2);

    game.transfer_estate_items(
        &pid("WeightKeeper"),
        chest.id,
        vec![BagLineItem {
            instance_id: 3,
            qty: 1,
        }],
        vec![],
        1,
        &auth,
    )
    .await;
    let unchanged = auth
        .estate_chest_state(chest.id, character_id)
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.revision, 1);
    assert_eq!(unchanged.items.len(), 2);
    assert_eq!(
        item_quantity(
            &game
                .get_player_inventory(&pid("WeightKeeper"))
                .await
                .unwrap(),
            "apple"
        ),
        3
    );
}

#[tokio::test]
async fn overweight_recovery_turns_the_empty_chest_into_a_ground_item() {
    let game = make_flat_world_game_state("estate_storage_overweight_recovery");
    let (auth, _) = make_test_auth_with_path("estate_storage_overweight_recovery");
    storage_owner(&game, &auth, "HeavyKeeper").await;

    game.place_estate_chest(
        &pid("HeavyKeeper"),
        2,
        Position {
            x: 2.5,
            y: 5.0,
            z: 2.5,
        },
        0.0,
        0,
        &auth,
    )
    .await;
    let chest = auth.load_estate_chests().unwrap().remove(0);
    game.inventories
        .write()
        .await
        .get_mut(&pid("HeavyKeeper"))
        .unwrap()
        .bag
        .push(bag_item(5, "campfire_kit", 100));

    game.recover_estate_chest(&pid("HeavyKeeper"), chest.id, &auth)
        .await;

    assert!(auth.load_estate_chests().unwrap().is_empty());
    assert_eq!(
        item_quantity(
            &game
                .get_player_inventory(&pid("HeavyKeeper"))
                .await
                .unwrap(),
            "storage_chest"
        ),
        0
    );
    let ground_items = game.ground_items.read().await;
    let dropped = ground_items.values().next().unwrap();
    assert_eq!(dropped.item.item_def_id, "storage_chest");
    assert_eq!(dropped.item.position, chest.position);
    assert_eq!(dropped.item.floor_level, chest.floor_level);
    assert_eq!(dropped.item.dropped_by, Some(pid("HeavyKeeper")));
}
