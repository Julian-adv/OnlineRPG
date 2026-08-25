// ---- Hunger (doc/HUNGER.md) ------------------------------------------------
// Paused-time tests: tokio's clock is frozen, `advance` moves it, and the
// hunger ticks are driven by hand.

use super::*;
use onlinerpg_shared::hunger::{
    HungerState, CAMPFIRE_DURATION_MS, GRILL_CAST_MS, MOVEMENT_DRAIN_INTERVAL_SECS, SATIATION_MAX,
    SATIATION_START, SPRINT_DRAIN_INTERVAL_SECS,
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

fn last_hunger_update(msgs: &[ServerMessage]) -> Option<(u32, HungerState, f32)> {
    msgs.iter().rev().find_map(|m| match m {
        ServerMessage::HungerUpdate {
            satiation,
            state,
            move_mult,
            ..
        } => Some((*satiation, *state, *move_mult)),
        _ => None,
    })
}

/// (id, remaining_ms) pairs of the last DebuffUpdate.
fn last_debuffs(msgs: &[ServerMessage]) -> Option<Vec<(String, u64)>> {
    msgs.iter().rev().find_map(|m| match m {
        ServerMessage::DebuffUpdate { debuffs } => Some(
            debuffs
                .iter()
                .map(|d| (d.id.clone(), d.remaining_ms))
                .collect(),
        ),
        _ => None,
    })
}

#[test]
fn one_meal_covers_the_active_day_budget() {
    let half_day_secs = super::super::time::REAL_DAY_DURATION_SECONDS as f32 / 2.0;
    let half_day_movement = half_day_secs / MOVEMENT_DRAIN_INTERVAL_SECS;
    let kills = 150.0;
    let sprint = 180.0 / SPRINT_DRAIN_INTERVAL_SECS;
    let active_day_budget = half_day_movement + kills + sprint;
    let defs = ItemDefs::load();
    let bread = defs.get("bread").expect("bread is in items.csv");
    assert!(bread.nutrition.unwrap() as f32 >= active_day_budget);
}

#[tokio::test]
async fn eating_feeds_and_consumes_the_food() {
    let game_state = make_test_game_state("eat_feeds");
    let (id, mut rx) = make_eater(&game_state, "eater", 200).await;
    put_in_bag(&game_state, &id, 1, "bread").await;
    drain(&mut rx);

    game_state.use_item(&id, 1).await;

    assert_eq!(game_state.hunger_satiation(&id).await, Some(740));
    assert!(bag_ids(&game_state, &id).await.is_empty(), "bread is eaten");
    let msgs = drain(&mut rx);
    let (satiation, state, move_mult) = last_hunger_update(&msgs).expect("HungerUpdate sent");
    assert_eq!(
        (satiation, state, move_mult),
        (740, HungerState::Normal, 1.0)
    );
}

#[tokio::test]
async fn meals_only_clamp_at_the_hard_cap() {
    let game_state = make_test_game_state("hard_cap");
    let (id, _rx) = make_eater(&game_state, "capped", 700).await;
    put_in_bag(&game_state, &id, 1, "jerky").await;

    game_state.use_item(&id, 1).await;

    assert_eq!(game_state.hunger_satiation(&id).await, Some(SATIATION_MAX));
}

#[tokio::test]
async fn eating_at_the_cap_is_refused_and_keeps_the_food() {
    let game_state = make_test_game_state("overeat");
    let (id, mut rx) = make_eater(&game_state, "glutton", 820).await;
    put_in_bag(&game_state, &id, 1, "jerky").await;
    game_state.use_item(&id, 1).await;
    assert_eq!(game_state.hunger_satiation(&id).await, Some(SATIATION_MAX));
    let msgs = drain(&mut rx);
    assert_eq!(last_hunger_update(&msgs).unwrap().1, HungerState::Normal);

    put_in_bag(&game_state, &id, 2, "apple").await;
    game_state.use_item(&id, 2).await;
    assert_eq!(game_state.hunger_satiation(&id).await, Some(SATIATION_MAX));
    assert_eq!(bag_ids(&game_state, &id).await, vec!["apple"]);
}

#[tokio::test(start_paused = true)]
async fn raw_fish_poisoning_drains_four_times_faster_and_expires() {
    let game_state = make_test_game_state("poison");
    let (id, mut rx) = make_eater(&game_state, "risktaker", 500).await;
    put_in_bag(&game_state, &id, 1, "raw_trout").await;
    drain(&mut rx);

    let poison = crate::debuff_defs::debuff_def("food_poisoning").unwrap();
    // Forced poison keeps the 70% roll out of the assertion.
    game_state
        .use_eat_item(&id, 1, 40, true, Some("food_poisoning"), Some(true))
        .await;
    let msgs = drain(&mut rx);
    let (satiation, _, move_mult) = last_hunger_update(&msgs).unwrap();
    assert_eq!(satiation, 540);
    assert_eq!(move_mult, poison.move_mult, "debuff multiplier is shipped");
    assert_eq!(
        last_debuffs(&msgs).unwrap(),
        vec![("food_poisoning".to_string(), poison.duration_secs * 1000)]
    );

    game_state
        .record_movement_activity(&[(id, MOVEMENT_DRAIN_INTERVAL_SECS, false)])
        .await;
    assert_eq!(game_state.hunger_satiation(&id).await, Some(536));

    advance(Duration::from_secs(poison.duration_secs + 1)).await;
    drain(&mut rx);
    game_state.tick_debuffs().await;
    assert_eq!(game_state.hunger_satiation(&id).await, Some(536));
    let msgs = drain(&mut rx);
    assert!(last_debuffs(&msgs).unwrap().is_empty(), "expiry is pushed");
    assert_eq!(last_hunger_update(&msgs).unwrap().2, 1.0);
}

#[tokio::test(start_paused = true)]
async fn bleeding_ticks_damage_each_second_and_expires() {
    let game_state = make_test_game_state("bleed");
    let (id, mut rx) = make_eater(&game_state, "clawed", 500).await;
    game_state
        .players
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .health = 20;
    drain(&mut rx);

    let bleed = crate::debuff_defs::debuff_def("bleed").unwrap();
    assert!(game_state.inflict_debuff(&id, "bleed", Some(true)).await);
    assert_eq!(
        last_debuffs(&drain(&mut rx)).unwrap(),
        vec![("bleed".to_string(), bleed.duration_secs * 1000)]
    );

    // One extra sweep past the duration: it must expire, not tick again.
    for _ in 0..=bleed.duration_secs {
        advance(Duration::from_secs(1)).await;
        game_state.tick_debuffs().await;
    }
    let health = game_state.players.read().await[&id].health;
    assert_eq!(
        health,
        20 - bleed.duration_secs as u32 * bleed.dps,
        "one tick per second for the duration"
    );
    assert!(last_debuffs(&drain(&mut rx)).unwrap().is_empty(), "expired");
    assert!(
        !game_state.inflict_debuff(&id, "bleed", Some(false)).await,
        "a failed roll applies nothing"
    );
}

#[tokio::test(start_paused = true)]
async fn bleeding_out_kills_and_clears_the_debuff() {
    let game_state = make_test_game_state("bleed_death");
    let (id, mut rx) = make_eater(&game_state, "doomed", 500).await;
    game_state
        .players
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .health = 1;
    game_state.inflict_debuff(&id, "bleed", Some(true)).await;
    drain(&mut rx);

    advance(Duration::from_secs(1)).await;
    game_state.tick_debuffs().await;

    assert_eq!(game_state.players.read().await[&id].health, 0);
    let msgs = drain(&mut rx);
    assert!(msgs
        .iter()
        .any(|m| matches!(m, ServerMessage::PlayerDead { player_id } if *player_id == id)));
    assert!(
        last_debuffs(&msgs).unwrap().is_empty(),
        "death drops the debuff"
    );
}

#[tokio::test(start_paused = true)]
async fn reapplying_a_debuff_refreshes_instead_of_stacking() {
    let game_state = make_test_game_state("bleed_refresh");
    let (id, mut rx) = make_eater(&game_state, "scratched", 500).await;
    let bleed = crate::debuff_defs::debuff_def("bleed").unwrap();
    game_state.inflict_debuff(&id, "bleed", Some(true)).await;
    advance(Duration::from_secs(3)).await;
    game_state.inflict_debuff(&id, "bleed", Some(true)).await;
    let debuffs = last_debuffs(&drain(&mut rx)).unwrap();
    assert_eq!(
        debuffs,
        vec![("bleed".to_string(), bleed.duration_secs * 1000)]
    );
}

#[tokio::test]
async fn an_unpoisoned_raw_fish_still_feeds_a_little() {
    let game_state = make_test_game_state("raw_ok");
    let (id, _rx) = make_eater(&game_state, "lucky", 500).await;
    put_in_bag(&game_state, &id, 1, "raw_minnow").await;

    game_state
        .use_eat_item(&id, 1, 40, true, Some("food_poisoning"), Some(false))
        .await;

    assert_eq!(game_state.hunger_satiation(&id).await, Some(540));
    assert!(bag_ids(&game_state, &id).await.is_empty());
}

#[tokio::test]
async fn sustenance_gear_slows_the_activity_drain() {
    let game_state = make_test_game_state("sustenance");
    let (id, _rx) = make_eater(&game_state, "wearer", 500).await;
    put_in_bag(&game_state, &id, 1, "silver_necklace").await;
    game_state.equip_item(&id, 1).await;

    for _ in 0..4 {
        game_state
            .record_movement_activity(&[(id, MOVEMENT_DRAIN_INTERVAL_SECS, false)])
            .await;
    }

    // 0.75× drain: four intervals' walking costs three points, not four.
    assert_eq!(game_state.hunger_satiation(&id).await, Some(497));
}

#[tokio::test]
async fn activity_drain_announces_only_band_transitions() {
    let game_state = make_test_game_state("decay_bands");
    let (id, mut rx) = make_eater(&game_state, "walker", 300).await;
    drain(&mut rx);

    game_state
        .record_movement_activity(&[(id, MOVEMENT_DRAIN_INTERVAL_SECS, false)])
        .await;
    let msgs = drain(&mut rx);
    assert_eq!(
        last_hunger_update(&msgs).map(|u| u.1),
        Some(HungerState::Hungry),
        "crossing 300 → 299 is a transition"
    );

    game_state
        .record_movement_activity(&[(id, MOVEMENT_DRAIN_INTERVAL_SECS, false)])
        .await;
    assert!(
        last_hunger_update(&drain(&mut rx)).is_none(),
        "299 → 298 stays quiet"
    );
}

/// The sprint gate (`can_sprint`) flips inside the Normal band; clients
/// mirror it, so the flip must be announced even though the band holds.
#[tokio::test]
async fn a_walking_drain_that_closes_the_sprint_gate_announces_it() {
    let game_state = make_test_game_state("gate_flip");
    let (id, mut rx) = make_eater(&game_state, "edge_walker", 302).await;
    drain(&mut rx);

    game_state
        .record_movement_activity(&[(id, MOVEMENT_DRAIN_INTERVAL_SECS, false)])
        .await;
    assert!(
        last_hunger_update(&drain(&mut rx)).is_none(),
        "302 → 301 keeps the gate open, stays quiet"
    );

    game_state
        .record_movement_activity(&[(id, MOVEMENT_DRAIN_INTERVAL_SECS, false)])
        .await;
    let (satiation, state, _) =
        last_hunger_update(&drain(&mut rx)).expect("301 → 300 closes the sprint gate");
    assert_eq!(satiation, 300);
    assert_eq!(state, HungerState::Normal, "the band did not move");
}

#[tokio::test]
async fn sprint_stops_at_300_without_draining_into_weakness() {
    let game_state = make_test_game_state("sprint_floor");
    let (id, mut rx) = make_eater(&game_state, "runner", 301).await;
    drain(&mut rx);

    game_state
        .record_movement_activity(&[(id, SPRINT_DRAIN_INTERVAL_SECS, true)])
        .await;
    assert_eq!(game_state.hunger_satiation(&id).await, Some(300));
    assert_eq!(last_hunger_update(&drain(&mut rx)).unwrap().0, 300);

    game_state
        .record_movement_activity(&[(id, 10.0, true)])
        .await;
    assert_eq!(game_state.hunger_satiation(&id).await, Some(300));

    game_state
        .record_movement_activity(&[(id, MOVEMENT_DRAIN_INTERVAL_SECS, false)])
        .await;
    assert_eq!(game_state.hunger_satiation(&id).await, Some(299));
}

#[tokio::test]
async fn server_applies_sprint_speed_and_rejects_it_without_fuel() {
    let game_state = make_test_game_state("sprint_speed");
    let (runner, _rx1) = make_eater(&game_state, "fast_runner", 500).await;
    let (hungry, _rx2) = make_eater(&game_state, "hungry_runner", 300).await;

    for id in [runner, hungry] {
        let start = game_state.players.read().await[&id].position;
        game_state
            .update_player_position(
                &id,
                MoveCommand {
                    position: Position {
                        x: start.x + 10.0,
                        ..start
                    },
                    rotation: 0.0,
                    floor_level: 0,
                    append: false,
                    sprinting: true,
                },
                false,
                false,
            )
            .await;
    }

    game_state.tick_player_movement(1.0).await;

    let walk_step = onlinerpg_shared::PLAYER_MOVE_SPEED;
    let sprint_step = walk_step * onlinerpg_shared::hunger::SPRINT_MOVE_MULT;
    let players = game_state.players.read().await;
    assert!((players[&runner].position.x - (100.0 + sprint_step)).abs() < 0.001);
    assert!((players[&hungry].position.x - (100.0 + walk_step)).abs() < 0.001);
    drop(players);
    assert_eq!(game_state.hunger_satiation(&runner).await, Some(499));
    assert_eq!(game_state.hunger_satiation(&hungry).await, Some(300));
}

#[tokio::test]
async fn food_restores_hp_over_ten_seconds() {
    let game_state = make_test_game_state("food_regen");
    let (id, _rx) = make_eater(&game_state, "diner", 500).await;
    put_in_bag(&game_state, &id, 1, "bread").await;
    game_state
        .players
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .health = 1;

    game_state.use_item(&id, 1).await;
    assert_eq!(game_state.players.read().await[&id].health, 1);

    game_state.tick_food_regeneration().await;
    assert_eq!(game_state.players.read().await[&id].health, 3);
    game_state.cancel_food_regeneration(&id).await;
    game_state.tick_food_regeneration().await;
    assert_eq!(game_state.players.read().await[&id].health, 3);
}

#[tokio::test]
async fn healing_scales_with_satiation_gained() {
    let game_state = make_test_game_state("partial_heal");
    let (id, _rx) = make_eater(&game_state, "nibbler", 900).await;
    put_in_bag(&game_state, &id, 1, "bread").await;
    game_state
        .players
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .health = 1;

    game_state.use_item(&id, 1).await;
    for _ in 0..10 {
        game_state.tick_food_regeneration().await;
    }
    // Only 100 of the bread's 540 nutrition fit, so it heals 100/20 = 5.
    assert_eq!(game_state.players.read().await[&id].health, 6);
}

#[tokio::test]
async fn eating_while_full_is_refused() {
    let game_state = make_test_game_state("full_no_heal");
    let (id, _rx) = make_eater(&game_state, "glutton", 1000).await;
    put_in_bag(&game_state, &id, 1, "bread").await;
    game_state
        .players
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .health = 1;

    game_state.use_item(&id, 1).await;
    game_state.tick_food_regeneration().await;
    assert_eq!(game_state.players.read().await[&id].health, 1);
    let inventories = game_state.inventories.read().await;
    assert!(
        inventories[&id]
            .bag
            .iter()
            .any(|i| i.item_def_id == "bread"),
        "the refused meal stays in the bag"
    );
}

#[tokio::test]
async fn weak_and_debuffed_players_do_not_regenerate() {
    let game_state = make_test_game_state("regen_gate");
    let (weak_id, _rx1) = make_eater(&game_state, "starving", 50).await;
    let (bleeding_id, _rx2) = make_eater(&game_state, "bleeding", 500).await;
    let (fed_id, _rx3) = make_eater(&game_state, "healthy", 500).await;
    {
        let mut players = game_state.players.write().await;
        for id in [&weak_id, &bleeding_id, &fed_id] {
            let p = players.get_mut(id).unwrap();
            p.health = 5;
            p.last_combat_at = 0;
        }
    }
    game_state
        .inflict_debuff(&bleeding_id, "bleed", Some(true))
        .await;

    game_state.tick_regeneration().await;

    let players = game_state.players.read().await;
    assert_eq!(players[&weak_id].health, 5, "Weak: no natural healing");
    assert_eq!(
        players[&bleeding_id].health, 5,
        "blocksRegen debuff: no natural healing"
    );
    assert!(players[&fed_id].health > 5, "normal hunger heals normally");
}

#[tokio::test]
async fn hungry_players_regenerate_every_other_natural_tick() {
    let game_state = make_test_game_state("hungry_regen");
    let (id, _rx) = make_eater(&game_state, "slow_healer", 200).await;
    {
        let mut players = game_state.players.write().await;
        let player = players.get_mut(&id).unwrap();
        player.health = 5;
        player.last_combat_at = 0;
    }

    game_state.tick_regeneration().await;
    assert_eq!(game_state.players.read().await[&id].health, 5);
    game_state.tick_regeneration().await;
    assert!(game_state.players.read().await[&id].health > 5);
}

#[tokio::test]
async fn weak_hunger_shrinks_carry_weight() {
    let game_state = make_test_game_state("carry");
    let (id, _rx) = make_eater(&game_state, "porter", 50).await;
    // STR 10 → base 150, Weak ×0.6.
    assert_eq!(game_state.max_carry_weight(&id).await, 90.0);

    let (fed, _rx2) = make_eater(&game_state, "fed_porter", 500).await;
    assert!((game_state.max_carry_weight(&fed).await - 150.0).abs() < 0.01);
}

#[tokio::test]
async fn weak_hunger_slows_the_authoritative_attack_interval() {
    let game_state = make_test_game_state("weak_attack_speed");
    let (id, mut rx) = make_eater(&game_state, "tired_fighter", 50).await;
    let mut monster = make_monster(
        "training_target",
        Position {
            x: 101.0,
            y: 0.0,
            z: 50.0,
        },
        0,
    );
    monster.health = 100;
    monster.max_health = 100;
    game_state
        .monsters
        .write()
        .await
        .insert("training_target".into(), monster);

    game_state
        .broadcast_player_attack(&id, "training_target".into())
        .await;
    drain(&mut rx);

    game_state.last_player_attacks.write().await.insert(
        id,
        GameState::now_ms().saturating_sub(*super::super::combat::PLAYER_ATTACK_INTERVAL_MS),
    );
    game_state
        .broadcast_player_attack(&id, "training_target".into())
        .await;
    assert!(!drain(&mut rx)
        .iter()
        .any(|msg| matches!(msg, ServerMessage::PlayerAttacked { .. })));

    let weak_interval = (*super::super::combat::PLAYER_ATTACK_INTERVAL_MS as f32
        / onlinerpg_shared::hunger::WEAK_ATTACK_MULT)
        .ceil() as u64;
    game_state
        .last_player_attacks
        .write()
        .await
        .insert(id, GameState::now_ms().saturating_sub(weak_interval));
    game_state
        .broadcast_player_attack(&id, "training_target".into())
        .await;
    assert!(drain(&mut rx)
        .iter()
        .any(|msg| matches!(msg, ServerMessage::PlayerAttacked { .. })));
}

#[tokio::test]
async fn respawn_preserves_satiation() {
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

    assert_eq!(game_state.hunger_satiation(&id).await, Some(30));
}

#[tokio::test(start_paused = true)]
async fn a_raw_fish_near_a_campfire_grills_instead_of_being_eaten() {
    let game_state = make_test_game_state("grill");
    let (id, mut rx) = make_eater(&game_state, "cook", 500).await;
    put_in_bag(&game_state, &id, 1, "raw_trout").await;
    let pos = game_state.get_player_position(&id).await.unwrap().0;
    game_state
        .spawn_campfire(pos, 0, onlinerpg_shared::hunger::CAMPFIRE_DURATION_MS)
        .await;
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
    game_state
        .spawn_campfire(pos, 0, onlinerpg_shared::hunger::CAMPFIRE_DURATION_MS)
        .await;
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
    let campfire = game_state
        .spawn_campfire(pos, 0, onlinerpg_shared::hunger::CAMPFIRE_DURATION_MS)
        .await;
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

    let position = {
        let campfires = game_state.campfires.read().await;
        assert_eq!(campfires.len(), 1);
        campfires.values().next().unwrap().campfire.position
    };
    assert_eq!(
        position,
        Position {
            x: 100.0,
            y: 0.0,
            z: 51.0,
        }
    );
    assert!(bag_ids(&game_state, &id).await.is_empty(), "kit consumed");
    assert!(drain(&mut rx)
        .iter()
        .any(|m| matches!(m, ServerMessage::CampfireSpawned { .. })));
}

#[tokio::test]
async fn campfire_placement_falls_back_to_the_player_when_blocked() {
    let game_state = make_test_game_state("kit_blocked");
    let (id, _rx) = make_eater(&game_state, "blocked_scout", 500).await;
    put_in_bag(&game_state, &id, 1, "campfire_kit").await;
    game_state.sync_region_furniture(0, 0, &[table_placement(100.5, 51.5)]);

    game_state.use_item(&id, 1).await;

    let position = game_state
        .campfires
        .read()
        .await
        .values()
        .next()
        .unwrap()
        .campfire
        .position;
    assert_eq!(
        position,
        Position {
            x: 100.0,
            y: 0.0,
            z: 50.0,
        }
    );
}

/// An NPC lights its own fire — a bard busking through the night cannot run
/// to the merchant for a kit every ten minutes, and earns nothing to pay with.
#[tokio::test]
async fn only_an_npc_lights_a_campfire_without_a_kit_and_only_one_of_them() {
    let game_state = make_test_game_state("npc_fire");
    let auth = make_test_auth("npc_fire");
    let (player_id, _prx) = make_eater(&game_state, "busker_fan", 500).await;
    let npc_id = pid("npc_signe");
    let mut npc = make_player("npc_signe", 100.0, 50.0);
    npc.is_official_npc = true;
    game_state.add_player(npc).await;

    game_state
        .send_chat_message(&player_id, "/light_campfire".to_string(), &auth)
        .await;
    assert!(
        game_state.campfires.read().await.is_empty(),
        "players still need a kit"
    );

    game_state
        .send_chat_message(&npc_id, "/light_campfire".to_string(), &auth)
        .await;
    assert_eq!(game_state.campfires.read().await.len(), 1);

    // The world starts at midnight, so this one burns until sunrise: hours of
    // game time, well under the three real hours a whole game day takes.
    let burn = {
        let campfires = game_state.campfires.read().await;
        let entry = campfires.values().next().unwrap();
        entry
            .expires_at
            .saturating_duration_since(tokio::time::Instant::now())
    };
    assert!(
        burn.as_millis() as u64 > CAMPFIRE_DURATION_MS * 2
            && burn.as_secs_f64() < super::super::time::REAL_DAY_DURATION_SECONDS / 2.0,
        "night fire burns to sunrise, not on the kit's timer: {burn:?}"
    );

    game_state
        .send_chat_message(&npc_id, "/light_campfire".to_string(), &auth)
        .await;
    assert_eq!(
        game_state.campfires.read().await.len(),
        1,
        "a second fire on the same pitch is refused"
    );
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
    assert!(
        !game_state.inflict_debuff(&id, "bleed", Some(true)).await,
        "no debuffs for the exempt"
    );
    game_state.tick_debuffs().await;
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
