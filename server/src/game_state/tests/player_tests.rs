use super::*;

/// `GameState.players` is a list (numeric ids can't key a wasm-serialized
/// map), so snapshot assertions look their player up by id.
fn find_player(players: &[Player], id: PlayerId) -> &Player {
    players
        .iter()
        .find(|p| p.id == id)
        .expect("player missing from snapshot")
}

#[tokio::test]
async fn equipped_torch_syncs_live_and_late_join_player_state() {
    let game_state = make_test_game_state("late_join_torch_snapshot");
    let torch_holder_id = pid("torch_holder");

    game_state
        .add_player(make_player("torch_holder", 0.0, 0.0))
        .await;
    game_state.inventories.write().await.insert(
        torch_holder_id,
        PlayerInventory {
            bag: vec![bag_item(1, "torch", 1)],
            equipped: Default::default(),
        },
    );

    game_state.equip_item(&torch_holder_id, 1).await;
    assert!(game_state.get_all_players().await[&torch_holder_id].torch_on);

    let snapshot = game_state
        .add_player(make_player("late_joiner", 1.0, 0.0))
        .await
        .expect("nearby existing player should produce a GameState snapshot");
    match snapshot {
        ServerMessage::GameState { players, .. } => {
            assert!(find_player(&players, torch_holder_id).torch_on);
        }
        other => panic!("expected GameState, got {other:?}"),
    }

    game_state
        .unequip_item(&torch_holder_id, EquipSlot::OffHand)
        .await;

    assert!(!game_state.get_all_players().await[&torch_holder_id].torch_on);
}

#[tokio::test]
async fn equipped_main_hand_syncs_live_and_late_join_player_state() {
    let game_state = make_test_game_state("late_join_main_hand_snapshot");
    let angler_id = pid("angler");

    game_state.add_player(make_player("angler", 0.0, 0.0)).await;
    game_state.inventories.write().await.insert(
        angler_id,
        PlayerInventory {
            bag: vec![bag_item(1, "fishing_rod", 1)],
            equipped: Default::default(),
        },
    );

    game_state.equip_item(&angler_id, 1).await;
    assert_eq!(
        game_state.get_all_players().await[&angler_id].main_hand,
        Some("fishing_rod".to_string())
    );

    let snapshot = game_state
        .add_player(make_player("late_joiner", 1.0, 0.0))
        .await
        .expect("nearby existing player should produce a GameState snapshot");
    match snapshot {
        ServerMessage::GameState { players, .. } => {
            assert_eq!(
                find_player(&players, angler_id).main_hand.as_deref(),
                Some("fishing_rod")
            );
        }
        other => panic!("expected GameState, got {other:?}"),
    }

    game_state
        .unequip_item(&angler_id, EquipSlot::MainHand)
        .await;

    assert_eq!(
        game_state.get_all_players().await[&angler_id].main_hand,
        None
    );
}

#[tokio::test]
async fn respawn_player_revives_dead_player_only() {
    let game_state = make_test_game_state("respawn_dead");

    let player = Player {
        id: pid("player_dead"),
        name: "DeadPlayer".to_string(),
        position: Position {
            x: 12.0,
            y: 0.0,
            z: -4.0,
        },
        rotation: 1.25,
        level: 3,
        health: 0,
        max_health: 30,
        class: CharacterClass::Knight,
        gender: Gender::default(),
        is_official_npc: false,
        torch_on: false,
        floor_level: 0,
        object_type: None,
        main_hand: None,
        object_id: None,
        last_combat_at: 0,
        client_kind: Default::default(),
    };
    let player_id = player.id;
    game_state.add_player(player).await;

    let mut direct_rx = game_state.register_direct_channel(&player_id).await;
    let mut broadcast_rx = game_state.subscribe();
    game_state.respawn_player(&player_id).await;

    let players = game_state.get_all_players().await;
    let revived = players
        .get(&player_id)
        .expect("Player should still exist after respawn");
    let spawn = &world_config().spawn_position;
    assert_eq!(revived.health, revived.max_health);
    assert_eq!(revived.position.x, spawn.x);
    assert_eq!(revived.position.y, spawn.y);
    assert_eq!(revived.position.z, spawn.z);
    assert_eq!(revived.rotation, spawn.rotation);

    // The spawn point sits within discovery range of a dungeon entrance, so
    // an unseeded respawner may receive DungeonDiscoveries alongside this.
    let respawned = drain(&mut direct_rx)
        .into_iter()
        .find_map(|msg| match msg {
            ServerMessage::PlayerRespawned { player } => Some(player),
            _ => None,
        })
        .expect("Expected direct PlayerRespawned");
    assert_eq!(respawned.id, player_id);
    assert_eq!(respawned.health, respawned.max_health);

    match broadcast_rx.try_recv() {
        Err(TryRecvError::Empty) => {}
        Ok(msg) => {
            let server_msg: ServerMessage =
                rmp_serde::from_slice(&msg.bytes).expect("Failed to deserialize broadcast");
            panic!("Expected no respawn broadcast, got {:?}", server_msg);
        }
        Err(err) => panic!("Expected empty broadcast channel, got {:?}", err),
    }
}

#[tokio::test]
async fn respawn_player_ignores_alive_player() {
    let game_state = make_test_game_state("respawn_alive");

    let player = Player {
        id: pid("player_alive"),
        name: "AlivePlayer".to_string(),
        position: Position {
            x: 5.0,
            y: 0.0,
            z: 6.0,
        },
        rotation: 0.75,
        level: 2,
        health: 18,
        max_health: 20,
        class: CharacterClass::Knight,
        gender: Gender::default(),
        is_official_npc: false,
        torch_on: false,
        floor_level: 0,
        object_type: None,
        main_hand: None,
        object_id: None,
        last_combat_at: 0,
        client_kind: Default::default(),
    };
    let player_id = player.id;
    game_state.add_player(player).await;

    let mut rx = game_state.subscribe();
    game_state.respawn_player(&player_id).await;

    let players = game_state.get_all_players().await;
    let unchanged = players
        .get(&player_id)
        .expect("Player should still exist after ignored respawn");
    assert_eq!(unchanged.health, 18);
    assert_eq!(unchanged.position.x, 5.0);
    assert_eq!(unchanged.position.y, 0.0);
    assert_eq!(unchanged.position.z, 6.0);
    assert_eq!(unchanged.rotation, 0.75);

    match rx.try_recv() {
        Err(TryRecvError::Empty) => {}
        Ok(msg) => {
            let server_msg: ServerMessage =
                rmp_serde::from_slice(&msg.bytes).expect("Failed to deserialize broadcast");
            panic!(
                "Expected no broadcast for alive respawn, got {:?}",
                server_msg
            );
        }
        Err(err) => panic!("Expected empty channel, got {:?}", err),
    }
}

#[tokio::test]
async fn active_character_cannot_be_deleted_from_another_session() {
    let auth = make_test_auth("active_character_delete_guard");
    let account = auth.login_npc("npc_active_delete").unwrap();
    let record = create_test_character(&auth, &account, "StillPlaying");
    let game_state = Arc::new(make_test_game_state("active_character_delete_guard"));
    let player_id = pid("active_character");

    // EnterGame takes the admission lock before reading the durable row. A
    // concurrent delete must wait until that session appears in the live map,
    // then reject instead of winning the check/delete race.
    let admission = game_state.lock_character_sessions().await;
    let deleting_state = Arc::clone(&game_state);
    let deleting_auth = auth.clone();
    let deleting_account = account.clone();
    let character_id = record.id;
    let delete = tokio::spawn(async move {
        deleting_state
            .delete_character_if_inactive(&deleting_auth, &deleting_account, character_id)
            .await
            .unwrap()
    });
    tokio::task::yield_now().await;
    assert!(!delete.is_finished());

    game_state
        .register_player_character(&player_id, record.id, 0, attrs_with_cha(12), 0)
        .await;
    drop(admission);

    assert!(!delete.await.unwrap());
    assert!(auth.get_character_for_account(&account, record.id).is_ok());

    game_state.unregister_player_character(&player_id).await;
    assert!(game_state
        .delete_character_if_inactive(&auth, &account, record.id)
        .await
        .unwrap());
    assert!(matches!(
        auth.get_character_for_account(&account, record.id),
        Err(crate::auth::AuthError::CharacterNotFound)
    ));
}
