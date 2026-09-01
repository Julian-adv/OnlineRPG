use super::*;

async fn enter(game_state: &GameState, name: &str, character_id: i64) -> DirectRx {
    game_state.add_player(make_player(name, 0.0, 0.0)).await;
    game_state
        .register_player_character(&pid(name), character_id, 0, attrs_with_cha(10), 0, None)
        .await;
    game_state.set_player_titles(&pid(name), Vec::new()).await;
    game_state.register_direct_channel(&pid(name)).await
}

async fn boss(game_state: &GameState, id: &str) {
    game_state.dungeon_monsters.write().await.insert(
        id.to_string(),
        dungeon::DungeonMonsterRef {
            entrance_id: "old_crypt".to_string(),
            depth: 5,
            slot: 0,
            is_boss: true,
        },
    );
}

async fn titles_of(game_state: &GameState, name: &str) -> (Vec<String>, Option<String>) {
    let titles = game_state
        .player_titles
        .read()
        .await
        .get(&pid(name))
        .cloned()
        .unwrap_or_default();
    let shown = game_state.players.read().await[&pid(name)].title.clone();
    (titles, shown)
}

#[tokio::test]
async fn the_main_contributor_earns_the_title_and_a_near_solo_kill_the_solo_tier() {
    let game_state = make_test_game_state("titles_grant");
    let mut ann = enter(&game_state, "Ann", 1).await;
    let mut bob = enter(&game_state, "Bob", 2).await;
    boss(&game_state, "boss-1").await;

    game_state
        .record_boss_damage("boss-1", &pid("Ann"), 95)
        .await;
    game_state
        .record_boss_damage("boss-1", &pid("Bob"), 5)
        .await;
    drain(&mut ann);
    drain(&mut bob);

    game_state
        .grant_boss_kill_titles("boss-1", "goblin_boss", None)
        .await;

    let (titles, active) = titles_of(&game_state, "Ann").await;
    assert_eq!(titles, ["goblin_slayer", "goblin_slayer_solo"]);
    // Nothing was shown before, so the first grant shows, then the solo
    // tier of the same boss takes over.
    assert_eq!(active.as_deref(), Some("goblin_slayer_solo"));
    let shown = game_state.players.read().await[&pid("Ann")].title.clone();
    assert_eq!(shown.as_deref(), Some("goblin_slayer_solo"));

    let msgs = drain(&mut ann);
    let earned: Vec<&str> = msgs
        .iter()
        .filter_map(|m| match m {
            ServerMessage::TitleEarned { title } => Some(title.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(earned, ["goblin_slayer", "goblin_slayer_solo"]);
    assert!(msgs
        .iter()
        .any(|m| matches!(m, ServerMessage::PlayerTitleChanged { title: Some(a), .. } if a == "goblin_slayer_solo")));

    assert!(titles_of(&game_state, "Bob").await.0.is_empty());
    assert!(drain(&mut bob)
        .iter()
        .any(|m| matches!(m, ServerMessage::PlayerTitleChanged { player_id, title: Some(t) } if *player_id == pid("Ann") && t == "goblin_slayer_solo")));
    assert!(game_state.boss_damage.read().await.is_empty());
}

#[tokio::test]
async fn an_even_split_earns_nobody_a_title_and_a_regular_monster_logs_nothing() {
    let game_state = make_test_game_state("titles_split");
    enter(&game_state, "Ann", 1).await;
    enter(&game_state, "Bob", 2).await;
    enter(&game_state, "Cat", 3).await;
    boss(&game_state, "boss-1").await;

    for name in ["Ann", "Bob", "Cat"] {
        game_state
            .record_boss_damage("boss-1", &pid(name), 10)
            .await;
        game_state
            .record_boss_damage("grunt-1", &pid(name), 10)
            .await;
    }
    assert!(!game_state.boss_damage.read().await.contains_key("grunt-1"));

    game_state
        .grant_boss_kill_titles("boss-1", "goblin_boss", None)
        .await;
    for name in ["Ann", "Bob", "Cat"] {
        assert!(titles_of(&game_state, name).await.0.is_empty(), "{name}");
    }
}

#[tokio::test]
async fn landing_the_golden_sturgeon_earns_the_angler_title_once() {
    let game_state = make_test_game_state("titles_fishing");
    let mut ann = enter(&game_state, "Ann", 1).await;
    drain(&mut ann);

    game_state
        .grant_fishing_catch_title(&pid("Ann"), "golden_sturgeon", None)
        .await;
    let (titles, active) = titles_of(&game_state, "Ann").await;
    assert_eq!(titles, ["sturgeon_angler"]);
    // A first-ever title auto-shows.
    assert_eq!(active.as_deref(), Some("sturgeon_angler"));
    assert!(drain(&mut ann)
        .iter()
        .any(|m| matches!(m, ServerMessage::TitleEarned { title } if title == "sturgeon_angler")));

    // A repeat catch and an untitled fish grant nothing.
    game_state
        .grant_fishing_catch_title(&pid("Ann"), "golden_sturgeon", None)
        .await;
    game_state
        .grant_fishing_catch_title(&pid("Ann"), "raw_minnow", None)
        .await;
    assert_eq!(titles_of(&game_state, "Ann").await.0, ["sturgeon_angler"]);
    assert!(drain(&mut ann)
        .iter()
        .all(|m| !matches!(m, ServerMessage::TitleEarned { .. })));
}

#[tokio::test]
async fn a_player_picks_among_earned_titles_and_cannot_show_an_unearned_one() {
    let game_state = make_test_game_state("titles_pick");
    let mut ann = enter(&game_state, "Ann", 1).await;
    let mut bob = enter(&game_state, "Bob", 2).await;
    game_state
        .set_player_titles(
            &pid("Ann"),
            vec!["orc_slayer".into(), "goblin_slayer".into()],
        )
        .await;
    game_state
        .set_active_title(&pid("Ann"), Some("orc_slayer".into()), None)
        .await;
    // Definition order, not earned order.
    assert_eq!(
        titles_of(&game_state, "Ann").await.0,
        ["goblin_slayer", "orc_slayer"]
    );
    drain(&mut ann);
    drain(&mut bob);

    game_state
        .set_active_title(&pid("Ann"), Some("ogre_slayer".into()), None)
        .await;
    assert_eq!(
        titles_of(&game_state, "Ann").await.1.as_deref(),
        Some("orc_slayer")
    );
    assert!(drain(&mut bob).is_empty());

    game_state
        .handle_title_command(&pid("Ann"), "1", None)
        .await;
    assert_eq!(
        titles_of(&game_state, "Ann").await.1.as_deref(),
        Some("goblin_slayer")
    );
    assert!(drain(&mut bob)
        .iter()
        .any(|m| matches!(m, ServerMessage::PlayerTitleChanged { title: Some(t), .. } if t == "goblin_slayer")));

    game_state
        .handle_title_command(&pid("Ann"), "off", None)
        .await;
    assert_eq!(titles_of(&game_state, "Ann").await.1, None);
    assert_eq!(game_state.players.read().await[&pid("Ann")].title, None);

    // A later grant does not override the player's explicit "none".
    boss(&game_state, "boss-2").await;
    game_state
        .record_boss_damage("boss-2", &pid("Ann"), 40)
        .await;
    game_state
        .grant_boss_kill_titles("boss-2", "ogre_boss", None)
        .await;
    let (titles, active) = titles_of(&game_state, "Ann").await;
    assert_eq!(
        titles,
        [
            "goblin_slayer",
            "orc_slayer",
            "ogre_slayer",
            "ogre_slayer_solo"
        ]
    );
    assert_eq!(active, None);
}
