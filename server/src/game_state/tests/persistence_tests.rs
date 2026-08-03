use super::*;

#[tokio::test]
async fn dirty_save_is_retried_after_failure() {
    let (auth, db_path) = make_test_auth_with_path("dirty_save_retry");
    let account = auth.login_npc("npc_dirty_save_retry").unwrap();
    let attributes = attrs_with_cha(12);
    let record = create_test_character(&auth, &account, "Retryknight");
    let initial_inventory = auth.load_inventory(record.id).unwrap();

    let game_state = make_test_game_state("dirty_save_retry");
    let player_id = pid("dirty_save_retry");
    let mut player = make_player("dirty_save_retry", 42.0, 0.0);
    player.name = record.name.clone();
    game_state.add_player(player).await;
    game_state
        .register_player_character(
            &player_id,
            record.id,
            record.xp,
            attributes,
            record.gold,
            None,
        )
        .await;
    game_state.inventories.write().await.insert(
        player_id,
        PlayerInventory {
            bag: vec![bag_item(1, "torch", 1)],
            equipped: Default::default(),
        },
    );
    game_state.mark_dirty(&player_id).await;
    game_state.mark_inventory_dirty(&player_id).await;

    let failure_connection = rusqlite::Connection::open(&db_path).unwrap();
    failure_connection
        .execute_batch(
            "CREATE TRIGGER fail_character_update
             BEFORE UPDATE ON characters
             BEGIN
                 SELECT RAISE(FAIL, 'test save failure');
             END;",
        )
        .unwrap();

    game_state.flush_dirty_saves(&auth).await;

    assert!(game_state.dirty_players.read().await.contains(&player_id));
    assert!(game_state
        .dirty_inventories
        .read()
        .await
        .contains(&player_id));
    assert_ne!(
        auth.get_character_for_account(&account, record.id)
            .unwrap()
            .last_x,
        42.0
    );
    assert_eq!(auth.load_inventory(record.id).unwrap(), initial_inventory);

    failure_connection
        .execute("DROP TRIGGER fail_character_update", [])
        .unwrap();
    game_state.flush_dirty_saves(&auth).await;

    assert!(!game_state.dirty_players.read().await.contains(&player_id));
    assert!(!game_state
        .dirty_inventories
        .read()
        .await
        .contains(&player_id));
    assert_eq!(
        auth.get_character_for_account(&account, record.id)
            .unwrap()
            .last_x,
        42.0
    );
    assert_eq!(
        auth.load_inventory(record.id).unwrap(),
        vec![crate::auth::ItemRow {
            item_def_id: "torch".to_string(),
            quantity: 1,
            equip_slot: None,
            enchant: 0,
        }]
    );
}

// F-015: session replacement (kick) must flush the departing session's
// inventory to the DB before a replacement login can load it. Otherwise the
// old async disconnect-save races the new session's load, letting a dropped
// item survive in the DB snapshot and be picked up again (duplication).
#[tokio::test]
async fn kick_flushes_dropped_inventory_before_replacement_load() {
    let auth = make_test_auth("f015_kick_flush");
    let account = auth.login_npc("npc_f015_account").unwrap();
    let attributes = attrs_with_cha(12);
    let record = create_test_character(&auth, &account, "Dupeknight");
    let char_id = record.id;

    // DB snapshot before the drop: one sword in the bag.
    auth.save_batch(
        &[],
        &[(
            char_id,
            vec![crate::auth::ItemRow {
                item_def_id: "worn_iron_sword".to_string(),
                quantity: 1,
                equip_slot: None,
                enchant: 0,
            }],
        )],
        &[],
        &[],
        None,
    )
    .unwrap();

    let game_state = make_test_game_state("f015_kick_flush");

    // Session A enters with its account, player, and inventory registered.
    let a = pid("session_a");
    let (kick_tx, mut kick_rx) = tokio::sync::mpsc::unbounded_channel();
    let session_a = game_state
        .register_account_session(&account, kick_tx, &auth)
        .await;
    let mut player = make_player("session_a", 0.0, 0.0);
    player.name = record.name.clone();
    game_state.add_player(player).await;
    game_state
        .register_player_character(&a, char_id, record.xp, attributes, record.gold, None)
        .await;
    assert!(
        game_state
            .attach_player_to_account_session(&account, session_a, a)
            .await
    );
    game_state.load_player_inventory(&a, char_id, &auth).await;

    // A drops the sword (in-memory only; not yet persisted).
    let instance_id = game_state.get_player_inventory(&a).await.unwrap().bag[0].instance_id;
    game_state.drop_item(&a, instance_id).await;
    assert!(game_state
        .get_player_inventory(&a)
        .await
        .unwrap()
        .bag
        .is_empty());
    // The window F-015 raced: the DB still holds the pre-drop snapshot.
    assert_eq!(auth.load_inventory(char_id).unwrap().len(), 1);

    // A replacement login kicks A at the account level.
    let (replacement_tx, _replacement_rx) = tokio::sync::mpsc::unbounded_channel();
    game_state
        .register_account_session(&account, replacement_tx, &auth)
        .await;
    assert!(matches!(
        kick_rx.try_recv(),
        Ok(ServerMessage::Kicked { player_id, .. }) if player_id == a
    ));

    // The kick flushed A's post-drop inventory and detached it, so a
    // replacement load reads zero swords (no dupe) instead of a stale one.
    assert_eq!(auth.load_inventory(char_id).unwrap().len(), 0);
    assert!(game_state.get_player_inventory(&a).await.is_none());
}

fn make_door_test_house() -> onlinerpg_shared::housing::HouseData {
    use crate::housing::test_fixtures::{house_at, room_at};
    let mut house = house_at(10.0, 10.0, vec![room_at(0, 0)]);
    house.rooms[0].wall_north[0].variant = onlinerpg_shared::housing::WallVariant::WithDoor;
    house
}

#[tokio::test]
async fn open_door_state_is_stamped_onto_served_house_data() {
    let game_state = make_test_game_state("door_state_stamp");
    let house = make_door_test_house();
    game_state.housing_io.write_house(&house).await.unwrap();

    // Door world position: origin + (segment 0 center, north edge) = (10.5, 10)
    let toggler = pid("toggler");
    game_state
        .add_player(make_player("toggler", 10.5, 10.5))
        .await;

    let toggled = game_state
        .toggle_door(&toggler, &house.id, 0, WallDirection::North, 0)
        .await;
    assert_eq!(toggled, Some(true));

    // A reconnecting client re-fetches the house from disk (is_open false there);
    // the stamp overlays the in-memory open state.
    let mut served = vec![house.clone()];
    game_state.apply_open_door_state(&mut served).await;
    assert!(served[0].rooms[0].wall_north[0].is_open);

    // Closing the door clears the stamp.
    game_state
        .toggle_door(&toggler, &house.id, 0, WallDirection::North, 0)
        .await;
    let mut served = vec![house.clone()];
    game_state.apply_open_door_state(&mut served).await;
    assert!(!served[0].rooms[0].wall_north[0].is_open);

    // A house edit (passability reinstall) forgets its open doors.
    game_state
        .toggle_door(&toggler, &house.id, 0, WallDirection::North, 0)
        .await;
    game_state.passability_add_house(&house).await;
    let mut served = vec![house.clone()];
    game_state.apply_open_door_state(&mut served).await;
    assert!(!served[0].rooms[0].wall_north[0].is_open);
}
