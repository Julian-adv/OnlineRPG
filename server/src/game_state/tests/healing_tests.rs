use super::*;
use onlinerpg_shared::skills::{skill_xp_for_level, SkillId, SkillProgress, Skills};

async fn setup_healer(
    game_state: &GameState,
    name: &str,
    health: u32,
    max_health: u32,
    skill_level: u32,
    item_def_id: &str,
    quantity: u32,
) -> DirectRx {
    let player_id = pid(name);
    let mut player = make_player(name, 0.0, 0.0);
    player.health = health;
    player.max_health = max_health;
    game_state.add_player(player).await;
    game_state
        .register_player_character(&player_id, 1, 0, attrs_with_cha(10), 0, None)
        .await;
    let mut skills = Skills::default();
    if skill_level > 0 {
        skills.map.insert(
            SkillId::Healing,
            SkillProgress {
                level: skill_level,
                xp: skill_xp_for_level(skill_level),
            },
        );
    }
    game_state.register_player_skills(&player_id, skills).await;
    game_state.inventories.write().await.insert(
        player_id,
        PlayerInventory {
            bag: vec![bag_item(100, item_def_id, quantity)],
            equipped: Default::default(),
        },
    );
    game_state.register_direct_channel(&player_id).await
}

#[tokio::test]
async fn bandaging_restores_with_skill_bonus_and_awards_actual_hp() {
    let game_state = make_test_game_state("healing_skill_use");
    let player_id = pid("field_medic");
    let mut rx = setup_healer(&game_state, "field_medic", 10, 100, 15, "bandage", 1).await;

    game_state.use_item(&player_id, 100).await;

    let health = game_state.players.read().await[&player_id].health;
    let restored = health - 10;
    assert!(
        (4..=10).contains(&restored),
        "2d4 Bandage plus level-15 Healing +2 restored {restored}"
    );
    assert!(game_state.inventories.read().await[&player_id]
        .bag
        .is_empty());
    let progress = game_state.player_skills.read().await[&player_id].get(SkillId::Healing);
    assert_eq!(progress.level, 15);
    assert_eq!(progress.xp, skill_xp_for_level(15) + u64::from(restored));

    let messages = drain(&mut rx);
    assert!(messages.iter().any(|message| matches!(
        message,
        ServerMessage::PlayerHealthUpdate { health: updated, .. } if *updated == health
    )));
    assert!(messages.iter().any(|message| matches!(
        message,
        ServerMessage::SkillXpGained {
            skill: SkillId::Healing,
            xp_amount,
            total_xp,
            ..
        } if *xp_amount == u64::from(restored)
            && *total_xp == skill_xp_for_level(15) + u64::from(restored)
    )));

    let metrics = game_state.skill_balance_snapshot();
    assert_eq!(metrics.healing.uses, 1);
    assert_eq!(metrics.healing.restored_hp, u64::from(restored));
    assert_eq!(metrics.healing.xp, u64::from(restored));
    assert_eq!(metrics.healing_by_skill_band[2].uses, 1);
    assert_eq!(metrics.healing_xp_messages, 1);
    assert_eq!(metrics.healing_rows_created, 0);
}

#[tokio::test]
async fn capped_heal_trains_only_the_hp_actually_restored() {
    let game_state = make_test_game_state("healing_actual_hp");
    let player_id = pid("scratched_medic");
    let mut rx = setup_healer(&game_state, "scratched_medic", 99, 100, 15, "bandage", 1).await;

    game_state.use_item(&player_id, 100).await;

    assert_eq!(game_state.players.read().await[&player_id].health, 100);
    assert_eq!(
        game_state.player_skills.read().await[&player_id]
            .get(SkillId::Healing)
            .xp,
        skill_xp_for_level(15) + 1
    );
    assert!(drain(&mut rx).iter().any(|message| matches!(
        message,
        ServerMessage::SkillXpGained {
            skill: SkillId::Healing,
            xp_amount: 1,
            ..
        }
    )));
}

#[tokio::test]
async fn rejected_bandages_and_finished_products_do_not_train_healing() {
    let game_state = make_test_game_state("healing_rejections");
    let full_id = pid("full_health");
    let mut full_rx = setup_healer(&game_state, "full_health", 100, 100, 0, "bandage", 1).await;
    game_state.use_item(&full_id, 100).await;
    assert_eq!(game_state.inventories.read().await[&full_id].bag.len(), 1);
    assert!(!game_state.player_skills.read().await[&full_id]
        .map
        .contains_key(&SkillId::Healing));
    assert!(drain(&mut full_rx).iter().any(|message| matches!(
        message,
        ServerMessage::SystemMessage { message } if message.contains("full health")
    )));

    let potion_id = pid("potion_drinker");
    let mut potion_rx = setup_healer(
        &game_state,
        "potion_drinker",
        10,
        100,
        15,
        "healing_potion",
        1,
    )
    .await;
    game_state.use_item(&potion_id, 100).await;
    let potion_restored = game_state.players.read().await[&potion_id].health - 10;
    assert!((6..=24).contains(&potion_restored));
    assert_eq!(
        game_state.player_skills.read().await[&potion_id].get(SkillId::Healing),
        SkillProgress {
            level: 15,
            xp: skill_xp_for_level(15),
        }
    );
    assert!(!drain(&mut potion_rx).iter().any(|message| matches!(
        message,
        ServerMessage::SkillXpGained {
            skill: SkillId::Healing,
            ..
        }
    )));
    assert_eq!(game_state.skill_balance_snapshot().healing.uses, 0);

    let fish_id = pid("fish_eater");
    let _fish_rx = setup_healer(&game_state, "fish_eater", 10, 100, 0, "raw_trout", 1).await;
    game_state.use_item(&fish_id, 100).await;
    assert_eq!(game_state.players.read().await[&fish_id].health, 10);
    for _ in 0..onlinerpg_shared::hunger::FOOD_REGEN_DURATION_SECS {
        game_state.tick_food_regeneration().await;
    }
    assert!(game_state.players.read().await[&fish_id].health > 10);
    assert!(!game_state.player_skills.read().await[&fish_id]
        .map
        .contains_key(&SkillId::Healing));
}

#[tokio::test]
async fn duplicate_use_packets_cannot_reuse_one_bandage() {
    let game_state = make_test_game_state("healing_duplicate_use");
    let player_id = pid("packet_medic");
    let mut rx = setup_healer(&game_state, "packet_medic", 10, 100, 0, "bandage", 1).await;

    tokio::join!(
        game_state.use_item(&player_id, 100),
        game_state.use_item(&player_id, 100)
    );

    let restored = game_state.players.read().await[&player_id].health - 10;
    assert!((2..=8).contains(&restored));
    assert!(game_state.inventories.read().await[&player_id]
        .bag
        .is_empty());
    assert_eq!(
        game_state.player_skills.read().await[&player_id]
            .get(SkillId::Healing)
            .xp,
        u64::from(restored)
    );
    let xp_messages = drain(&mut rx)
        .into_iter()
        .filter(|message| {
            matches!(
                message,
                ServerMessage::SkillXpGained {
                    skill: SkillId::Healing,
                    ..
                }
            )
        })
        .count();
    assert_eq!(xp_messages, 1);
    assert_eq!(game_state.skill_balance_snapshot().healing.uses, 1);
}
