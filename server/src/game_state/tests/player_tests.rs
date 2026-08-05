use super::*;
use onlinerpg_shared::inventory::ArmorConstruction;
use onlinerpg_shared::skills::{skill_xp_for_level, SkillId, SkillProgress, Skills};
use onlinerpg_shared::PhysicalProtection;

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
        .register_player_character(&player_id, 1, 0, attrs, 0, None)
        .await;
    let mut skills = Skills::default();
    skills.map.insert(
        SkillId::LeatherArmor,
        SkillProgress {
            level: 25,
            xp: skill_xp_for_level(25),
        },
    );
    skills.map.insert(
        SkillId::MailArmor,
        SkillProgress {
            level: 15,
            xp: skill_xp_for_level(15),
        },
    );
    skills.map.insert(
        SkillId::PlateArmor,
        SkillProgress {
            level: 5,
            xp: skill_xp_for_level(5),
        },
    );
    skills.map.insert(
        SkillId::PaddedArmor,
        SkillProgress {
            level: 15,
            xp: skill_xp_for_level(15),
        },
    );
    skills.map.insert(
        SkillId::HybridArmor,
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
                bag_item(5, "chain_mail", 1),
                bag_item(6, "breastplate", 1),
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
    assert_eq!(
        profile.primary_armor_protection,
        PhysicalProtection::default()
    );
    assert_eq!(profile.armor_coverage_percent, 0);
    assert_eq!(
        profile.weighted_armor_protection,
        PhysicalProtection::default()
    );
    assert_eq!(profile.effective_guard, 10);

    game_state.equip_item(&player_id, 3).await;
    drain(&mut rx);
    let profile = game_state.player_defense_profile(&player_id).await;
    assert_eq!(profile.armor_skill, Some(SkillId::PaddedArmor));
    assert_eq!(
        profile.primary_armor_construction,
        Some(onlinerpg_shared::inventory::ArmorConstruction::Padded)
    );
    assert_eq!(
        profile.primary_armor_protection,
        PhysicalProtection {
            slash: 1,
            pierce: 0,
            blunt: 2,
        }
    );
    assert_eq!(profile.armor_coverage_percent, 75);
    assert_eq!(
        profile.weighted_armor_protection,
        profile.primary_armor_protection
    );
    assert_eq!(profile.effective_guard, 12);

    game_state.equip_item(&player_id, 4).await;
    let profile = game_state.player_defense_profile(&player_id).await;
    assert_eq!(profile.armor_skill, Some(SkillId::HybridArmor));
    assert_eq!(
        profile.primary_armor_construction,
        Some(onlinerpg_shared::inventory::ArmorConstruction::Hybrid)
    );
    assert_eq!(
        profile.primary_armor_protection,
        PhysicalProtection {
            slash: 2,
            pierce: 2,
            blunt: 2,
        }
    );
    assert_eq!(profile.armor_coverage_percent, 55);
    assert_eq!(
        profile.weighted_armor_protection,
        profile.primary_armor_protection
    );
    assert_eq!(profile.effective_guard, 15);
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

    game_state.equip_item(&player_id, 5).await;
    let profile = game_state.player_defense_profile(&player_id).await;
    assert_eq!(profile.armor_skill, Some(SkillId::MailArmor));
    assert_eq!(
        profile.primary_armor_construction,
        Some(onlinerpg_shared::inventory::ArmorConstruction::Mail)
    );
    assert_eq!(
        profile.primary_armor_protection,
        PhysicalProtection {
            slash: 2,
            pierce: 1,
            blunt: 0,
        }
    );
    assert_eq!(profile.armor_coverage_percent, 75);
    assert_eq!(
        profile.weighted_armor_protection,
        profile.primary_armor_protection
    );
    assert_eq!(profile.effective_guard, 17);

    game_state.equip_item(&player_id, 6).await;
    let profile = game_state.player_defense_profile(&player_id).await;
    assert_eq!(profile.armor_skill, Some(SkillId::PlateArmor));
    assert_eq!(
        profile.primary_armor_construction,
        Some(onlinerpg_shared::inventory::ArmorConstruction::Plate)
    );
    assert_eq!(
        profile.primary_armor_protection,
        PhysicalProtection {
            slash: 3,
            pierce: 3,
            blunt: 1,
        }
    );
    assert_eq!(profile.armor_coverage_percent, 40);
    assert_eq!(
        profile.weighted_armor_protection,
        PhysicalProtection {
            slash: 2,
            pierce: 2,
            blunt: 1,
        }
    );
    assert_eq!(profile.effective_guard, 18);

    game_state.equip_item(&player_id, 1).await;
    let profile = game_state.player_defense_profile(&player_id).await;
    assert_eq!(profile.armor_skill, Some(SkillId::LeatherArmor));
    assert_eq!(
        profile.primary_armor_construction,
        Some(onlinerpg_shared::inventory::ArmorConstruction::Leather)
    );
    assert_eq!(
        profile.primary_armor_protection,
        PhysicalProtection {
            slash: 1,
            pierce: 1,
            blunt: 1,
        }
    );
    assert_eq!(profile.armor_coverage_percent, 40);
    assert_eq!(
        profile.weighted_armor_protection,
        profile.primary_armor_protection
    );
    assert_eq!(profile.effective_guard, 15);

    game_state.unequip_item(&player_id, EquipSlot::Chest).await;
    let profile = game_state.player_defense_profile(&player_id).await;
    assert_eq!(profile.effective_guard, 10);
    assert_eq!(
        profile.primary_armor_protection,
        PhysicalProtection::default()
    );
    assert_eq!(profile.armor_coverage_percent, 0);
}

#[tokio::test]
async fn current_armor_loadouts_match_weighted_coverage_contract() {
    let game_state = make_test_game_state("armor_loadout_contract");
    let player_id = pid("coverage_tester");
    game_state
        .add_player(make_player("coverage_tester", 0.0, 0.0))
        .await;

    let cases = [
        (
            "clothing",
            vec![(EquipSlot::Chest, "traveler_robe")],
            None,
            None,
            0,
            PhysicalProtection::default(),
            PhysicalProtection::default(),
        ),
        (
            "padded",
            vec![(EquipSlot::Chest, "padded_battle_robe")],
            Some(ArmorConstruction::Padded),
            Some(SkillId::PaddedArmor),
            75,
            PhysicalProtection {
                slash: 1,
                pierce: 0,
                blunt: 2,
            },
            PhysicalProtection {
                slash: 1,
                pierce: 0,
                blunt: 2,
            },
        ),
        (
            "leather",
            vec![
                (EquipSlot::Head, "leather_helmet"),
                (EquipSlot::Chest, "leather_armor"),
                (EquipSlot::Hands, "leather_gloves"),
                (EquipSlot::Pants, "leather_pants"),
                (EquipSlot::Boots, "leather_boots"),
            ],
            Some(ArmorConstruction::Leather),
            Some(SkillId::LeatherArmor),
            85,
            PhysicalProtection {
                slash: 1,
                pierce: 1,
                blunt: 1,
            },
            PhysicalProtection {
                slash: 1,
                pierce: 1,
                blunt: 1,
            },
        ),
        (
            "mail",
            vec![
                (EquipSlot::Head, "iron_helmet"),
                (EquipSlot::Chest, "chain_mail"),
                (EquipSlot::Hands, "iron_gauntlets"),
                (EquipSlot::Boots, "iron_boots"),
            ],
            Some(ArmorConstruction::Mail),
            Some(SkillId::MailArmor),
            100,
            PhysicalProtection {
                slash: 2,
                pierce: 1,
                blunt: 0,
            },
            PhysicalProtection {
                slash: 2,
                pierce: 1,
                blunt: 0,
            },
        ),
        (
            "plate",
            vec![
                (EquipSlot::Head, "plate_helmet"),
                (EquipSlot::Chest, "breastplate"),
                (EquipSlot::Hands, "plate_gauntlets"),
                (EquipSlot::Pants, "plate_greaves"),
                (EquipSlot::Boots, "plate_boots"),
            ],
            Some(ArmorConstruction::Plate),
            Some(SkillId::PlateArmor),
            85,
            PhysicalProtection {
                slash: 3,
                pierce: 3,
                blunt: 1,
            },
            PhysicalProtection {
                slash: 3,
                pierce: 3,
                blunt: 1,
            },
        ),
        (
            "hybrid",
            vec![(EquipSlot::Chest, "brigandine_coat")],
            Some(ArmorConstruction::Hybrid),
            Some(SkillId::HybridArmor),
            55,
            PhysicalProtection {
                slash: 2,
                pierce: 2,
                blunt: 2,
            },
            PhysicalProtection {
                slash: 2,
                pierce: 2,
                blunt: 2,
            },
        ),
    ];

    for (
        case,
        loadout,
        expected_construction,
        expected_skill,
        expected_coverage,
        expected_authored,
        expected_effective,
    ) in cases
    {
        let equipped = loadout
            .into_iter()
            .enumerate()
            .map(|(index, (slot, item_def_id))| (slot, bag_item(index as u64 + 1, item_def_id, 1)))
            .collect();
        game_state.inventories.write().await.insert(
            player_id,
            PlayerInventory {
                bag: vec![],
                equipped,
            },
        );

        let profile = game_state.player_defense_profile(&player_id).await;
        assert_eq!(
            profile.primary_armor_construction, expected_construction,
            "{case} construction"
        );
        assert_eq!(profile.armor_skill, expected_skill, "{case} skill");
        assert_eq!(
            profile.armor_coverage_percent, expected_coverage,
            "{case} coverage"
        );
        assert_eq!(
            profile.primary_armor_protection, expected_authored,
            "{case} authored protection"
        );
        assert_eq!(
            profile.weighted_armor_protection, expected_effective,
            "{case} effective protection"
        );
    }
}

#[tokio::test]
async fn replacement_login_kicks_the_previous_account_session() {
    let auth = make_test_auth("account_session_replacement");
    let account = auth.login_npc("npc_account_session").unwrap();
    let game_state = make_test_game_state("account_session_replacement");
    let (first_tx, mut first_rx) = tokio::sync::mpsc::unbounded_channel();
    let first_id = game_state
        .register_account_session(&account, first_tx, &auth)
        .await;

    let (second_tx, _second_rx) = tokio::sync::mpsc::unbounded_channel();
    let second_id = game_state
        .register_account_session(&account, second_tx, &auth)
        .await;

    assert!(matches!(
        first_rx.try_recv(),
        Ok(ServerMessage::Kicked { player_id, .. }) if player_id == PlayerId::from(0)
    ));
    assert!(
        !game_state
            .is_current_account_session(&account, first_id)
            .await
    );
    assert!(
        game_state
            .is_current_account_session(&account, second_id)
            .await
    );

    game_state
        .end_account_session(&account, first_id, &auth)
        .await;
    assert!(
        game_state
            .is_current_account_session(&account, second_id)
            .await
    );
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

    // Deletion must wait for admission, then reject the registered character.
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
        .register_player_character(&player_id, record.id, 0, attrs_with_cha(12), 0, None)
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
