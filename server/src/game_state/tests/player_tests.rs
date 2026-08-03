use super::*;
use onlinerpg_shared::skills::{skill_xp_for_level, SkillId, SkillProgress, Skills};

/// `GameState.players` is a list (numeric ids can't key a wasm-serialized
/// map), so snapshot assertions look their player up by id.
fn find_player(players: &[Player], id: PlayerId) -> &Player {
    players
        .iter()
        .find(|p| p.id == id)
        .expect("player missing from snapshot")
}

#[tokio::test]
async fn primary_chest_garments_replace_each_other_without_leaking_armor_skills() {
    let game_state = make_test_game_state("primary_chest_layers");
    let player_id = pid("layer_tester");
    game_state
        .add_player(make_player("layer_tester", 0.0, 0.0))
        .await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 10;
    game_state
        .register_player_character(&player_id, 1, 0, attrs, 0)
        .await;
    let mut skills = Skills::default();
    skills.map.insert(
        SkillId::LeatherArmor,
        SkillProgress {
            level: 25,
            xp: skill_xp_for_level(25),
        },
    );
    game_state.register_player_skills(&player_id, skills).await;
    game_state.inventories.write().await.insert(
        player_id,
        PlayerInventory {
            bag: vec![
                bag_item(2, "traveler_robe", 1),
                bag_item(3, "padded_battle_robe", 1),
                bag_item(4, "brigandine_coat", 1),
            ],
            equipped: [(EquipSlot::Chest, bag_item(1, "leather_armor", 1))]
                .into_iter()
                .collect(),
        },
    );
    let mut rx = game_state.register_direct_channel(&player_id).await;

    game_state.equip_item(&player_id, 2).await;
    drain(&mut rx);
    let profile = game_state.player_defense_profile(&player_id).await;
    assert_eq!(profile.armor_skill, None);
    assert_eq!(profile.primary_armor_construction, None);
    assert_eq!(profile.effective_guard, 10);

    game_state.equip_item(&player_id, 3).await;
    drain(&mut rx);
    let profile = game_state.player_defense_profile(&player_id).await;
    assert_eq!(profile.armor_skill, None);
    assert_eq!(
        profile.primary_armor_construction,
        Some(onlinerpg_shared::inventory::ArmorConstruction::Padded)
    );
    assert_eq!(profile.effective_guard, 10);

    game_state.equip_item(&player_id, 4).await;
    let profile = game_state.player_defense_profile(&player_id).await;
    assert_eq!(profile.armor_skill, None);
    assert_eq!(
        profile.primary_armor_construction,
        Some(onlinerpg_shared::inventory::ArmorConstruction::Hybrid)
    );
    assert_eq!(profile.effective_guard, 12);
    let burden = drain(&mut rx)
        .into_iter()
        .find_map(|message| match message {
            ServerMessage::EquipmentBurdenUpdated { burden } => Some(burden),
            _ => None,
        })
        .expect("equipment mutation publishes burden");
    assert_eq!(burden.equipped_weight, 14.0);
    assert_eq!(burden.max_carry_weight, 150.0);
    assert_eq!(
        burden.tier,
        onlinerpg_shared::EquipmentBurdenTier::Unburdened
    );
    assert_eq!(burden.movement_speed, 3.0);

    game_state.equip_item(&player_id, 1).await;
    let profile = game_state.player_defense_profile(&player_id).await;
    assert_eq!(profile.armor_skill, Some(SkillId::LeatherArmor));
    assert_eq!(
        profile.primary_armor_construction,
        Some(onlinerpg_shared::inventory::ArmorConstruction::Leather)
    );
    assert_eq!(profile.effective_guard, 14);

    game_state.unequip_item(&player_id, EquipSlot::Chest).await;
    assert_eq!(game_state.effective_guard(&player_id).await, 10);
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

    match direct_rx.try_recv() {
        Ok(ServerMessage::PlayerRespawned { player }) => {
            assert_eq!(player.id, player_id);
            assert_eq!(player.health, player.max_health);
        }
        other => panic!("Expected direct PlayerRespawned, got {:?}", other),
    }

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
