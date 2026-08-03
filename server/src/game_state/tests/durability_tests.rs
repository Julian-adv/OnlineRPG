use super::*;
use onlinerpg_shared::inventory::ArmorConstruction;
use onlinerpg_shared::skills::{SkillId, Skills};

async fn setup_wearer(
    game_state: &GameState,
    name: &str,
    armor_durability: u32,
    kits: u32,
) -> DirectRx {
    let player_id = pid(name);
    let mut player = make_player(name, 0.0, 0.0);
    player.health = 1_000;
    player.max_health = 1_000;
    game_state.add_player(player).await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 0;
    game_state
        .register_player_character(&player_id, 1, 0, attrs, 0, None)
        .await;
    game_state
        .register_player_skills(&player_id, Skills::default())
        .await;
    let mut armor = bag_item(10, "leather_armor", 1);
    armor.durability = Some(armor_durability);
    let mut bag = Vec::new();
    if kits > 0 {
        bag.push(bag_item(20, "leather_repair_kit", kits));
    }
    game_state.inventories.write().await.insert(
        player_id,
        PlayerInventory {
            bag,
            equipped: [(EquipSlot::Chest, armor)].into_iter().collect(),
        },
    );
    game_state.register_direct_channel(&player_id).await
}

#[tokio::test]
async fn repair_kit_restores_broken_primary_armor_and_consumes_one_kit() {
    let game_state = make_test_game_state("repair_broken_armor");
    let player_id = pid("repairer");
    let mut rx = setup_wearer(&game_state, "repairer", 0, 1).await;

    let broken = game_state.player_defense_profile(&player_id).await;
    assert_eq!(broken.effective_guard, 0);
    assert_eq!(broken.primary_armor_construction, None);
    assert_eq!(broken.armor_skill, None);

    game_state.use_item(&player_id, 20).await;

    let inventory = game_state.get_player_inventory(&player_id).await.unwrap();
    assert!(inventory.bag.is_empty());
    assert_eq!(inventory.equipped[&EquipSlot::Chest].durability, Some(60));
    let repaired = game_state.player_defense_profile(&player_id).await;
    assert_eq!(repaired.effective_guard, 2);
    assert_eq!(
        repaired.primary_armor_construction,
        Some(ArmorConstruction::Leather)
    );
    assert_eq!(repaired.armor_skill, Some(SkillId::LeatherArmor));
    assert!(game_state.player_skills.read().await[&player_id]
        .map
        .is_empty());
    assert!(game_state
        .dirty_inventories
        .read()
        .await
        .contains(&player_id));
    assert!(drain(&mut rx).iter().any(|message| matches!(
        message,
        ServerMessage::SystemMessage { message }
            if message.contains("from 0/60 to full condition")
    )));
}

#[tokio::test]
async fn each_repair_family_restores_only_its_matching_armor() {
    for (case, armor_id, kit_id, max) in [
        ("cloth", "padded_battle_robe", "cloth_repair_kit", 40),
        ("leather", "leather_armor", "leather_repair_kit", 60),
        ("mail", "chain_mail", "metal_repair_kit", 90),
        ("plate", "breastplate", "metal_repair_kit", 120),
        ("hybrid", "brigandine_coat", "hybrid_repair_kit", 100),
    ] {
        let game_state = make_test_game_state(&format!("repair_family_{case}"));
        let name = format!("repair_family_{case}");
        let player_id = pid(&name);
        setup_wearer(&game_state, &name, 0, 1).await;
        {
            let mut inventories = game_state.inventories.write().await;
            let inventory = inventories.get_mut(&player_id).unwrap();
            inventory
                .equipped
                .get_mut(&EquipSlot::Chest)
                .unwrap()
                .item_def_id = armor_id.to_string();
            inventory.bag[0].item_def_id = kit_id.to_string();
        }

        game_state.use_item(&player_id, 20).await;

        let inventory = game_state.get_player_inventory(&player_id).await.unwrap();
        assert!(inventory.bag.is_empty(), "{case} kit consumed");
        assert_eq!(
            inventory.equipped[&EquipSlot::Chest].durability,
            Some(max),
            "{case} condition"
        );
    }
}

#[tokio::test]
async fn mismatched_repair_family_keeps_the_kit_and_damage() {
    let game_state = make_test_game_state("repair_family_mismatch");
    let player_id = pid("repair_family_mismatch");
    let mut rx = setup_wearer(&game_state, "repair_family_mismatch", 7, 1).await;
    game_state
        .inventories
        .write()
        .await
        .get_mut(&player_id)
        .unwrap()
        .bag[0]
        .item_def_id = "cloth_repair_kit".to_string();

    game_state.use_item(&player_id, 20).await;

    let inventory = game_state.get_player_inventory(&player_id).await.unwrap();
    assert_eq!(inventory.bag[0].quantity, 1);
    assert_eq!(inventory.equipped[&EquipSlot::Chest].durability, Some(7));
    assert!(drain(&mut rx).iter().any(|message| matches!(
        message,
        ServerMessage::SystemMessage { message }
            if message.contains("requires a Leather kit")
    )));
}

#[tokio::test]
async fn repair_rejections_keep_the_kit() {
    let game_state = make_test_game_state("repair_rejections");
    let player_id = pid("careful_repairer");
    setup_wearer(&game_state, "careful_repairer", 60, 1).await;

    game_state.use_item(&player_id, 20).await;

    let inventory = game_state.get_player_inventory(&player_id).await.unwrap();
    assert_eq!(inventory.bag[0].quantity, 1);
    assert_eq!(inventory.equipped[&EquipSlot::Chest].durability, Some(60));

    {
        let mut inventories = game_state.inventories.write().await;
        inventories
            .get_mut(&player_id)
            .unwrap()
            .equipped
            .get_mut(&EquipSlot::Chest)
            .unwrap()
            .durability = Some(0);
    }
    game_state
        .players
        .write()
        .await
        .get_mut(&player_id)
        .unwrap()
        .last_combat_at = GameState::now_ms();
    game_state.use_item(&player_id, 20).await;
    assert_eq!(
        game_state.inventories.read().await[&player_id].bag[0].quantity,
        1
    );

    {
        let mut players = game_state.players.write().await;
        let player = players.get_mut(&player_id).unwrap();
        player.last_combat_at = 0;
        player.health = 0;
    }
    game_state.use_item(&player_id, 20).await;
    assert_eq!(
        game_state.inventories.read().await[&player_id].bag[0].quantity,
        1
    );
}

#[tokio::test]
async fn wear_is_instance_safe_and_broken_armor_stops_protecting() {
    let game_state = make_test_game_state("armor_wear_instance_safety");
    let player_id = pid("wearer");
    setup_wearer(&game_state, "wearer", 2, 0).await;

    game_state.wear_primary_armor(&player_id, 999).await;
    game_state.wear_primary_armor(&player_id, 10).await;
    assert_eq!(
        game_state.inventories.read().await[&player_id].equipped[&EquipSlot::Chest].durability,
        Some(1)
    );
    game_state.wear_primary_armor(&player_id, 10).await;

    let inventory = game_state.get_player_inventory(&player_id).await.unwrap();
    assert_eq!(inventory.equipped[&EquipSlot::Chest].durability, Some(0));
    let defense = game_state.player_defense_profile(&player_id).await;
    assert_eq!(defense.effective_guard, 0);
    assert_eq!(defense.primary_armor_construction, None);
    assert_eq!(defense.primary_armor_instance_id, None);
    assert_eq!(defense.armor_skill, None);
}

#[tokio::test]
async fn accepted_landed_monster_hit_wears_the_resolved_armor() {
    let game_state = make_test_game_state("armor_wear_combat");
    let owner_id = pid("wear_owner");
    let defender_id = pid("combat_wearer");
    game_state
        .add_player(make_player("wear_owner", 0.0, 0.0))
        .await;
    setup_wearer(&game_state, "combat_wearer", 2, 0).await;

    let mut monster = make_monster("wear_hit", pos(0.0), 0);
    monster.owner_id = Some(owner_id);
    monster.level_override = Some(u8::MAX);
    game_state
        .monsters
        .write()
        .await
        .insert("wear_hit".to_string(), monster);

    game_state
        .broadcast_monster_attack(&owner_id, "wear_hit", &defender_id)
        .await;

    assert_eq!(
        game_state.inventories.read().await[&defender_id].equipped[&EquipSlot::Chest].durability,
        Some(1)
    );

    let mut rejected = make_monster("wear_rejected", pos(100.0), 0);
    rejected.owner_id = Some(owner_id);
    rejected.level_override = Some(u8::MAX);
    game_state
        .monsters
        .write()
        .await
        .insert("wear_rejected".to_string(), rejected);
    game_state
        .broadcast_monster_attack(&owner_id, "wear_rejected", &defender_id)
        .await;
    assert_eq!(
        game_state.inventories.read().await[&defender_id].equipped[&EquipSlot::Chest].durability,
        Some(1),
        "out-of-range requests do not wear armor"
    );
}

#[tokio::test]
async fn condition_survives_equip_unequip_drop_and_pickup() {
    let game_state = make_test_game_state("durability_item_paths");
    let player_id = pid("condition_carrier");
    game_state
        .add_player(make_player("condition_carrier", 0.0, 0.0))
        .await;
    let mut armor = bag_item(42, "leather_armor", 1);
    armor.durability = Some(17);
    game_state.inventories.write().await.insert(
        player_id,
        PlayerInventory {
            bag: vec![armor],
            ..Default::default()
        },
    );

    game_state.equip_item(&player_id, 42).await;
    assert_eq!(
        game_state.inventories.read().await[&player_id].equipped[&EquipSlot::Chest].durability,
        Some(17)
    );
    game_state.unequip_item(&player_id, EquipSlot::Chest).await;
    assert_eq!(
        game_state.inventories.read().await[&player_id].bag[0].durability,
        Some(17)
    );
    game_state.drop_item(&player_id, 42).await;
    assert_eq!(
        game_state.ground_items.read().await[&42].item.durability,
        Some(17)
    );
    game_state.pickup_item(&player_id, 42).await;
    assert_eq!(
        game_state.inventories.read().await[&player_id].bag[0].durability,
        Some(17)
    );
}

#[tokio::test]
async fn legacy_condition_hydrates_to_max_and_persists_on_next_save() {
    let auth = make_test_auth("legacy_durability_hydration");
    let account = auth.login_npc("npc_legacy_durability").unwrap();
    let record = create_test_character(&auth, &account, "LegacyDurability");
    auth.save_batch(
        &[],
        &[(
            record.id,
            vec![crate::auth::ItemRow {
                item_def_id: "leather_armor".to_string(),
                quantity: 1,
                equip_slot: Some("chest".to_string()),
                enchant: 0,
                durability: None,
            }],
        )],
        &[],
        &[],
        None,
    )
    .unwrap();

    let game_state = make_test_game_state("legacy_durability_hydration");
    let player_id = pid("legacy_durability_player");
    game_state
        .add_player(make_player("legacy_durability_player", 0.0, 0.0))
        .await;
    game_state
        .register_player_character(&player_id, record.id, 0, attrs_with_cha(10), 0, None)
        .await;
    game_state
        .load_player_inventory(&player_id, record.id, &auth)
        .await;

    assert_eq!(
        game_state.inventories.read().await[&player_id].equipped[&EquipSlot::Chest].durability,
        Some(60)
    );
    let (_, rows) = game_state.take_player_inventory(&player_id).await.unwrap();
    assert_eq!(rows[0].durability, Some(60));
    auth.save_batch(&[], &[(record.id, rows)], &[], &[], None)
        .unwrap();
    assert_eq!(
        auth.load_inventory(record.id).unwrap()[0].durability,
        Some(60)
    );
}
