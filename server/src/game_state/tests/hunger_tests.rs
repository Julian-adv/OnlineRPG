// ---- Hunger (doc/HUNGER.md) ------------------------------------------------
// Paused-time tests: tokio's clock is frozen, `advance` moves it, and the
// hunger ticks are driven by hand.

use super::*;
use onlinerpg_shared::hunger::{
    HungerState, CAMPFIRE_DURATION_MS, FOOD_POISONING_MS, GRILL_CAST_MS, SATIATION_MAX,
    SATIATION_RESPAWN, SATIATION_START, WELL_FED_MAX, WELL_FED_MIN,
};
use tokio::time::{advance, Duration};

/// Player on dry land (positive x) with hunger tracked at `satiation`.
async fn make_eater(game_state: &GameState, name: &str, satiation: u32) -> (PlayerId, DirectRx) {
    let id = pid(name);
    game_state.add_player(make_player(name, 100.0, 50.0)).await;
    game_state.inventories.write().await.insert(
        id,
        PlayerInventory {
            bag: vec![],
            equipped: std::collections::HashMap::new(),
        },
    );
    game_state
        .register_player_character(&id, 1, 0, attrs_with_cha(10), 0, Some(satiation))
        .await;
    let rx = game_state.register_direct_channel(&id).await;
    (id, rx)
}

async fn put_in_bag(game_state: &GameState, id: &PlayerId, instance_id: u64, def_id: &str) {
    game_state
        .inventories
        .write()
        .await
        .get_mut(id)
        .unwrap()
        .bag
        .push(bag_item(instance_id, def_id, 1));
}

async fn bag_ids(game_state: &GameState, id: &PlayerId) -> Vec<String> {
    game_state.inventories.read().await[id]
        .bag
        .iter()
        .map(|i| i.item_def_id.clone())
        .collect()
}

fn last_hunger_update(msgs: &[ServerMessage]) -> Option<(u32, HungerState, u64)> {
    msgs.iter().rev().find_map(|m| match m {
        ServerMessage::HungerUpdate {
            satiation,
            state,
            poisoned_ms,
            ..
        } => Some((*satiation, *state, *poisoned_ms)),
        _ => None,
    })
}

/// The tuning anchor: one full meal per game day. The Well-Fed band must
/// cover a day's decay, and a day's decay must be at least 90% of the band —
/// eat once a day (jerky-sized) and you stay fed, skip it and you go hungry.
#[test]
fn one_meal_per_game_day_keeps_you_fed_and_no_more() {
    let day_secs = super::super::time::REAL_DAY_DURATION_SECONDS as u64;
    let decay_per_day = day_secs / onlinerpg_shared::hunger::DECAY_INTERVAL_SECS;
    let band_width = u64::from(WELL_FED_MAX - WELL_FED_MIN);
    assert!(
        band_width >= decay_per_day,
        "a full stomach ({band_width}) must last a game day ({decay_per_day})"
    );
    assert!(
        decay_per_day * 10 >= band_width * 9,
        "a day's decay ({decay_per_day}) must nearly drain the band ({band_width}) or one meal a day stops mattering"
    );
    // The day's meal exists in the catalog: jerky covers a full day.
    let defs = ItemDefs::load();
    let jerky = defs.get("jerky").expect("jerky is in items.csv");
    assert!(u64::from(jerky.nutrition.unwrap()) >= decay_per_day);
}

#[tokio::test]
async fn eating_feeds_and_consumes_the_food() {
    let game_state = make_test_game_state("eat_feeds");
    let (id, mut rx) = make_eater(&game_state, "eater", 500).await;
    put_in_bag(&game_state, &id, 1, "bread").await;
    drain(&mut rx);

    game_state.use_item(&id, 1).await;

    assert_eq!(game_state.hunger_satiation(&id).await, Some(680));
    assert!(bag_ids(&game_state, &id).await.is_empty(), "bread is eaten");
    let msgs = drain(&mut rx);
    let (satiation, state, poisoned) = last_hunger_update(&msgs).expect("HungerUpdate sent");
    assert_eq!((satiation, state, poisoned), (680, HungerState::WellFed, 0));
}

#[tokio::test]
async fn a_normal_range_meal_never_overshoots_into_stuffed() {
    let game_state = make_test_game_state("soft_cap");
    let (id, _rx) = make_eater(&game_state, "capped", 700).await;
    put_in_bag(&game_state, &id, 1, "jerky").await;

    game_state.use_item(&id, 1).await;

    // 700 + 540 clamps to the soft cap, not into the Stuffed band.
    assert_eq!(game_state.hunger_satiation(&id).await, Some(850));
}

#[tokio::test]
async fn deliberate_overeating_reaches_stuffed_and_the_cap_refuses() {
    let game_state = make_test_game_state("stuffed");
    let (id, mut rx) = make_eater(&game_state, "glutton", 820).await;
    put_in_bag(&game_state, &id, 1, "jerky").await;
    game_state.use_item(&id, 1).await;
    // 820 is above the 800 threshold: the full 540 lands, capped at 1000.
    assert_eq!(game_state.hunger_satiation(&id).await, Some(SATIATION_MAX));
    let msgs = drain(&mut rx);
    assert_eq!(last_hunger_update(&msgs).unwrap().1, HungerState::Stuffed);

    // At the hard cap another bite is refused and not consumed.
    put_in_bag(&game_state, &id, 2, "apple").await;
    game_state.use_item(&id, 2).await;
    assert_eq!(game_state.hunger_satiation(&id).await, Some(SATIATION_MAX));
    assert_eq!(bag_ids(&game_state, &id).await, vec!["apple".to_string()]);
}

#[tokio::test(start_paused = true)]
async fn raw_fish_poisoning_drains_four_times_faster_and_expires() {
    let game_state = make_test_game_state("poison");
    let (id, mut rx) = make_eater(&game_state, "risktaker", 500).await;
    put_in_bag(&game_state, &id, 1, "raw_trout").await;
    drain(&mut rx);

    // Forced poison keeps the 70% roll out of the assertion.
    game_state
        .use_eat_item(&id, 1, 40, None, true, Some(true))
        .await;
    let msgs = drain(&mut rx);
    let (satiation, _, poisoned_ms) = last_hunger_update(&msgs).unwrap();
    assert_eq!(satiation, 540);
    assert_eq!(poisoned_ms, FOOD_POISONING_MS);

    // Poisoned decay: 4 per bucket instead of 1.
    game_state.tick_hunger_decay().await;
    assert_eq!(game_state.hunger_satiation(&id).await, Some(536));

    // After the 5 minutes pass, the expiry announces itself and decay is 1.
    advance(Duration::from_millis(FOOD_POISONING_MS + 1)).await;
    drain(&mut rx);
    game_state.tick_hunger_decay().await;
    assert_eq!(game_state.hunger_satiation(&id).await, Some(535));
    let msgs = drain(&mut rx);
    assert_eq!(last_hunger_update(&msgs).unwrap().2, 0, "expiry is pushed");
}

#[tokio::test]
async fn an_unpoisoned_raw_fish_still_feeds_a_little() {
    let game_state = make_test_game_state("raw_ok");
    let (id, _rx) = make_eater(&game_state, "lucky", 500).await;
    put_in_bag(&game_state, &id, 1, "raw_minnow").await;

    game_state
        .use_eat_item(&id, 1, 40, Some("1d3".into()), true, Some(false))
        .await;

    assert_eq!(game_state.hunger_satiation(&id).await, Some(540));
    assert!(bag_ids(&game_state, &id).await.is_empty());
}

#[tokio::test]
async fn decay_announces_only_band_transitions() {
    let game_state = make_test_game_state("decay_bands");
    let (_id, mut rx) = make_eater(&game_state, "walker", WELL_FED_MIN).await;
    drain(&mut rx);

    game_state.tick_hunger_decay().await;
    let msgs = drain(&mut rx);
    assert_eq!(
        last_hunger_update(&msgs).map(|u| u.1),
        Some(HungerState::Hungry),
        "crossing 300 → 299 is a transition"
    );

    game_state.tick_hunger_decay().await;
    assert!(
        last_hunger_update(&drain(&mut rx)).is_none(),
        "299 → 298 stays quiet"
    );
}

#[tokio::test]
async fn weak_and_poisoned_players_do_not_regenerate() {
    let game_state = make_test_game_state("regen_gate");
    let (weak_id, _rx1) = make_eater(&game_state, "starving", 50).await;
    let (fed_id, _rx2) = make_eater(&game_state, "healthy", 500).await;
    {
        let mut players = game_state.players.write().await;
        for id in [&weak_id, &fed_id] {
            let p = players.get_mut(id).unwrap();
            p.health = 5;
            p.last_combat_at = 0;
        }
    }

    game_state.tick_regeneration().await;

    let players = game_state.players.read().await;
    assert_eq!(players[&weak_id].health, 5, "Weak: no natural healing");
    assert!(players[&fed_id].health > 5, "Well-Fed heals normally");
}

#[tokio::test]
async fn weak_hunger_shrinks_carry_weight() {
    let game_state = make_test_game_state("carry");
    let (id, _rx) = make_eater(&game_state, "porter", 50).await;
    // STR 10 → base 150, Weak ×0.6.
    assert_eq!(game_state.max_carry_weight(&id).await, 90.0);

    let (fed, _rx2) = make_eater(&game_state, "fed_porter", 500).await;
    // Well-Fed ×1.15.
    assert!((game_state.max_carry_weight(&fed).await - 172.5).abs() < 0.01);
}

#[tokio::test]
async fn respawn_resets_satiation_to_the_well_fed_floor() {
    let game_state = make_test_game_state("respawn");
    let (id, _rx) = make_eater(&game_state, "casualty", 30).await;
    game_state
        .players
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .health = 0;

    game_state.respawn_player(&id).await;

    assert_eq!(
        game_state.hunger_satiation(&id).await,
        Some(SATIATION_RESPAWN)
    );
}

#[tokio::test(start_paused = true)]
async fn a_raw_fish_near_a_campfire_grills_instead_of_being_eaten() {
    let game_state = make_test_game_state("grill");
    let (id, mut rx) = make_eater(&game_state, "cook", 500).await;
    put_in_bag(&game_state, &id, 1, "raw_trout").await;
    let pos = game_state.get_player_position(&id).await.unwrap().0;
    game_state.spawn_campfire(pos, 0).await;
    drain(&mut rx);

    game_state.use_item(&id, 1).await;
    assert!(
        drain(&mut rx)
            .iter()
            .any(|m| matches!(m, ServerMessage::GrillStarted)),
        "the cast starts instead of eating"
    );
    assert_eq!(game_state.hunger_satiation(&id).await, Some(500));

    advance(Duration::from_millis(GRILL_CAST_MS + 1)).await;
    game_state.tick_grills().await;

    assert_eq!(
        bag_ids(&game_state, &id).await,
        vec!["grilled_trout".to_string()]
    );
    let msgs = drain(&mut rx);
    assert!(msgs.iter().any(|m| matches!(
        m,
        ServerMessage::GrillEnded { grilled_item_def_id: Some(id) } if id == "grilled_trout"
    )));
}

#[tokio::test(start_paused = true)]
async fn moving_cancels_the_grill_and_keeps_the_raw_fish() {
    let game_state = make_test_game_state("grill_move");
    let (id, mut rx) = make_eater(&game_state, "fidget", 500).await;
    put_in_bag(&game_state, &id, 1, "raw_trout").await;
    let pos = game_state.get_player_position(&id).await.unwrap().0;
    game_state.spawn_campfire(pos, 0).await;
    game_state.use_item(&id, 1).await;
    drain(&mut rx);

    game_state
        .update_player_position(
            &id,
            move_cmd(
                Position {
                    x: pos.x + 1.0,
                    y: pos.y,
                    z: pos.z,
                },
                false,
            ),
            false,
            false,
        )
        .await;
    game_state.tick_player_movement(1.0).await;

    advance(Duration::from_millis(GRILL_CAST_MS + 1)).await;
    game_state.tick_grills().await;

    assert_eq!(
        bag_ids(&game_state, &id).await,
        vec!["raw_trout".to_string()],
        "the raw fish survives a cancelled cast"
    );
    assert!(drain(&mut rx).iter().any(|m| matches!(
        m,
        ServerMessage::GrillEnded {
            grilled_item_def_id: None
        }
    )));
}

#[tokio::test(start_paused = true)]
async fn campfires_burn_out_after_ten_minutes() {
    let game_state = make_test_game_state("burnout");
    let (id, mut rx) = make_eater(&game_state, "bystander", 500).await;
    let pos = game_state.get_player_position(&id).await.unwrap().0;
    let campfire = game_state.spawn_campfire(pos, 0).await;
    drain(&mut rx);

    advance(Duration::from_millis(CAMPFIRE_DURATION_MS + 1)).await;
    game_state.tick_campfires().await;

    assert!(game_state.campfires.read().await.is_empty());
    assert!(drain(&mut rx).iter().any(|m| matches!(
        m,
        ServerMessage::CampfireRemoved { campfire_id } if *campfire_id == campfire.id
    )));
}

#[tokio::test]
async fn using_a_campfire_kit_lights_a_fire_on_land() {
    let game_state = make_test_game_state("kit");
    let (id, mut rx) = make_eater(&game_state, "scout", 500).await;
    put_in_bag(&game_state, &id, 1, "campfire_kit").await;

    game_state.use_item(&id, 1).await;

    assert_eq!(game_state.campfires.read().await.len(), 1);
    assert!(bag_ids(&game_state, &id).await.is_empty(), "kit consumed");
    assert!(drain(&mut rx)
        .iter()
        .any(|m| matches!(m, ServerMessage::CampfireSpawned { .. })));
}

#[tokio::test]
async fn npcs_are_exempt_from_hunger_but_can_still_eat() {
    let game_state = make_test_game_state("npc_exempt");
    let id = pid("npc_rica");
    let mut npc = make_player("npc_rica", 100.0, 50.0);
    npc.is_official_npc = true;
    game_state.add_player(npc).await;
    game_state.inventories.write().await.insert(
        id,
        PlayerInventory {
            bag: vec![bag_item(1, "bread", 1)],
            equipped: std::collections::HashMap::new(),
        },
    );
    // NPC registration passes None: no hunger entry.
    game_state
        .register_player_character(&id, 7, 0, attrs_with_cha(10), 0, None)
        .await;
    let mut rx = game_state.register_direct_channel(&id).await;

    assert_eq!(game_state.hunger_satiation(&id).await, None);
    game_state.use_item(&id, 1).await;
    assert!(bag_ids(&game_state, &id).await.is_empty(), "still consumed");
    assert!(
        last_hunger_update(&drain(&mut rx)).is_none(),
        "no HungerUpdate for the exempt"
    );
    game_state.tick_hunger_decay().await;
    assert_eq!(game_state.hunger_satiation(&id).await, None);
}

#[test]
fn satiation_survives_a_save_and_reload() {
    let auth = make_test_auth("hunger_persist");
    let account = auth.login_npc("npc_hunger_persist").unwrap();
    let record = create_test_character(&auth, &account, "Ripknight");
    assert_eq!(record.satiation, SATIATION_START);

    let save = crate::auth::CharacterSaveData {
        character_id: record.id,
        x: 1.0,
        y: 2.0,
        z: 3.0,
        rotation: 0.0,
        xp: 0,
        level: 1,
        max_hp: 16,
        health: 16,
        floor_level: 0,
        gold: 0,
        satiation: 123,
    };
    auth.save_batch(&[save], &[], &[], &[], None).unwrap();

    let reloaded = auth.get_character_for_account(&account, record.id).unwrap();
    assert_eq!(reloaded.satiation, 123);
}
