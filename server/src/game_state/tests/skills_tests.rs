use super::*;

// Trained skills: XP grants update the map, notify the owner directly, and
// mark the player for the next dirty flush; a capped skill goes quiet.
#[tokio::test]
async fn skill_xp_grant_notifies_owner_and_marks_dirty() {
    use onlinerpg_shared::skills::{skill_xp_for_level, SkillId, SKILL_LEVEL_CAP};

    let game_state = make_test_game_state("skill_xp_grant");
    let player = pid("angler");
    game_state.add_player(make_player("angler", 0.0, 0.0)).await;
    game_state
        .register_player_character(&player, 42, 0, attrs_with_cha(10), 0, None)
        .await;
    game_state
        .register_player_skills(&player, Default::default())
        .await;
    let mut rx = game_state.register_direct_channel(&player).await;

    // 150 XP: crosses the level-1 threshold (100).
    let result = game_state
        .add_skill_xp(&player, SkillId::Fishing, 150)
        .await
        .expect("grant should apply");
    assert_eq!(result.new_level, 1);
    assert!(result.leveled_up);

    let msg = rx.try_recv().expect("owner should be notified");
    match msg {
        ServerMessage::SkillXpGained {
            skill,
            xp_amount,
            total_xp,
            new_level,
            leveled_up,
        } => {
            assert_eq!(skill, SkillId::Fishing);
            assert_eq!(xp_amount, 150);
            assert_eq!(total_xp, 150);
            assert_eq!(new_level, 1);
            assert!(leveled_up);
        }
        other => panic!("expected SkillXpGained, got {other:?}"),
    }

    // The dirty flush picks the player up exactly once, with the saved rows
    // and the drained id for the failed-save retry path.
    let (dirty_ids, dirty) = game_state.collect_dirty_skill_states().await;
    assert_eq!(dirty_ids, vec![player]);
    assert_eq!(dirty.len(), 1);
    assert_eq!(dirty[0].0, 42);
    assert_eq!(dirty[0].1.len(), 1);
    assert_eq!(dirty[0].1[0].skill_id, "fishing");
    assert_eq!(dirty[0].1[0].xp, 150);
    assert!(game_state.collect_dirty_skill_states().await.1.is_empty());
    // A failed save puts the dirtiness back for the next flush.
    game_state.restore_dirty_skills(vec![player]).await;
    assert_eq!(game_state.collect_dirty_skill_states().await.1.len(), 1);

    // Cap out: the clamp reports what was banked, then further grants no-op
    // (no message, no dirty flag).
    game_state
        .add_skill_xp(&player, SkillId::Fishing, u64::MAX)
        .await
        .expect("clamped grant still applies");
    assert!(game_state
        .add_skill_xp(&player, SkillId::Fishing, 10)
        .await
        .is_none());
    let _ = rx.try_recv().expect("cap-out grant notifies");
    assert!(matches!(rx.try_recv(), Err(MpscTryRecvError::Empty)));
    let (_, dirty) = game_state.collect_dirty_skill_states().await;
    assert_eq!(dirty.len(), 1);
    assert_eq!(dirty[0].1[0].xp, skill_xp_for_level(SKILL_LEVEL_CAP));
    assert_eq!(dirty[0].1[0].level, SKILL_LEVEL_CAP);
}

// Logout detach must snapshot skills for the save and drop the in-memory
// entry, exactly like inventories (F-015 shape).
#[tokio::test]
async fn take_player_skills_snapshots_and_detaches() {
    use onlinerpg_shared::skills::SkillId;

    let game_state = make_test_game_state("skill_detach");
    let player = pid("angler2");
    game_state
        .add_player(make_player("angler2", 0.0, 0.0))
        .await;
    game_state
        .register_player_character(&player, 7, 0, attrs_with_cha(10), 0, None)
        .await;
    game_state
        .register_player_skills(&player, Default::default())
        .await;
    game_state
        .add_skill_xp(&player, SkillId::Fishing, 600)
        .await
        .unwrap();
    game_state
        .add_skill_xp(&player, SkillId::OneHandedSword, 500)
        .await
        .unwrap();
    game_state
        .add_skill_xp(&player, SkillId::Dagger, 100)
        .await
        .unwrap();
    game_state
        .add_skill_xp(&player, SkillId::Spear, 20)
        .await
        .unwrap();
    game_state
        .add_skill_xp(&player, SkillId::Shield, 30)
        .await
        .unwrap();
    game_state
        .add_skill_xp(&player, SkillId::Healing, 40)
        .await
        .unwrap();
    game_state
        .add_skill_xp(&player, SkillId::LeatherArmor, 50)
        .await
        .unwrap();
    game_state
        .add_skill_xp(&player, SkillId::MailArmor, 60)
        .await
        .unwrap();
    game_state
        .add_skill_xp(&player, SkillId::PlateArmor, 70)
        .await
        .unwrap();
    game_state
        .add_skill_xp(&player, SkillId::PaddedArmor, 80)
        .await
        .unwrap();
    game_state
        .add_skill_xp(&player, SkillId::HybridArmor, 90)
        .await
        .unwrap();

    let (character_id, rows) = game_state
        .take_player_skills(&player)
        .await
        .expect("skills should detach");
    assert_eq!(character_id, 7);
    assert_eq!(rows.len(), 11);
    assert_eq!(rows[0].skill_id, "dagger");
    assert_eq!(rows[0].xp, 100);
    assert_eq!(rows[0].level, 1);
    assert_eq!(rows[1].skill_id, "fishing");
    assert_eq!(rows[1].xp, 600);
    assert_eq!(rows[1].level, 2);
    assert_eq!(rows[2].skill_id, "healing");
    assert_eq!(rows[2].xp, 40);
    assert_eq!(rows[2].level, 0);
    assert_eq!(rows[3].skill_id, "hybrid_armor");
    assert_eq!(rows[3].xp, 90);
    assert_eq!(rows[3].level, 0);
    assert_eq!(rows[4].skill_id, "leather_armor");
    assert_eq!(rows[4].xp, 50);
    assert_eq!(rows[4].level, 0);
    assert_eq!(rows[5].skill_id, "mail_armor");
    assert_eq!(rows[5].xp, 60);
    assert_eq!(rows[5].level, 0);
    assert_eq!(rows[6].skill_id, "one_handed_sword");
    assert_eq!(rows[6].xp, 500);
    assert_eq!(rows[6].level, 2);
    assert_eq!(rows[7].skill_id, "padded_armor");
    assert_eq!(rows[7].xp, 80);
    assert_eq!(rows[7].level, 0);
    assert_eq!(rows[8].skill_id, "plate_armor");
    assert_eq!(rows[8].xp, 70);
    assert_eq!(rows[8].level, 0);
    assert_eq!(rows[9].skill_id, "shield");
    assert_eq!(rows[9].xp, 30);
    assert_eq!(rows[9].level, 0);
    assert_eq!(rows[10].skill_id, "spear");
    assert_eq!(rows[10].xp, 20);
    assert_eq!(rows[10].level, 0);

    // Detached: nothing left to take or flush.
    assert!(game_state.take_player_skills(&player).await.is_none());
    assert!(game_state.collect_dirty_skill_states().await.1.is_empty());
}
