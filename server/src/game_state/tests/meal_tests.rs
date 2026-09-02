// ---- Table meals (the inn maid's served plates) ------------------------------

use super::*;
use crate::game_state::meal::plate_spot;
use onlinerpg_shared::furniture::FurniturePlacement;
use onlinerpg_shared::hunger::SATIATION_MAX;
use onlinerpg_shared::meal::MealSlot;

fn placement(id: u32, type_id: &str, x: f32, z: f32, rotation_deg: f32) -> FurniturePlacement {
    FurniturePlacement {
        id,
        type_id: type_id.to_string(),
        x,
        y: 0.0,
        z,
        rotation_deg,
        floor_level: 0,
    }
}

/// The inn's table 31 (rot 90) with chairs 43 and 45 on opposite long sides,
/// re-based near the origin so the test region is (0, 0).
fn inn_table() -> Vec<FurniturePlacement> {
    vec![
        placement(31, "table", 100.0, 50.0, 90.0),
        placement(43, "chair", 101.13, 50.10, 270.0),
        placement(45, "chair", 98.83, 50.10, 90.0),
    ]
}

async fn make_maid(game_state: &GameState, name: &str, x: f32, z: f32) -> PlayerId {
    let mut p = make_player(name, x, z);
    p.is_official_npc = true;
    let id = p.id;
    game_state.add_player(p).await;
    id
}

async fn make_guest(
    game_state: &GameState,
    name: &str,
    x: f32,
    z: f32,
    satiation: u32,
) -> PlayerId {
    let id = pid(name);
    game_state.add_player(make_player(name, x, z)).await;
    game_state
        .inventories
        .write()
        .await
        .insert(id, PlayerInventory::default());
    game_state
        .register_player_character(&id, 1, 0, attrs_with_cha(10), 0, Some(satiation))
        .await;
    id
}

async fn sit(game_state: &GameState, id: &PlayerId, chair: u32) {
    game_state
        .set_player_interaction(id, Some("chair".to_string()), Some(chair))
        .await;
}

async fn meals(game_state: &GameState) -> Vec<onlinerpg_shared::meal::Meal> {
    game_state
        .meals
        .read()
        .await
        .values()
        .map(|e| e.meal.clone())
        .collect()
}

#[test]
fn two_chairs_on_one_rotated_table_get_two_edge_spots() {
    let t = inn_table();
    let (east, _) = plate_spot(&t[1], &t[0], MealSlot::Dish);
    let (west, _) = plate_spot(&t[2], &t[0], MealSlot::Dish);
    // The chairs sit on the long sides of a table turned 90°, so the plates
    // land at the short-axis edge inset, one each side, in front of each
    // seat — nudged to each guest's left (facing -x, left is +z; facing +x,
    // left is -z).
    assert!((east.x - 100.257).abs() < 0.01, "east plate x {}", east.x);
    assert!((west.x - 99.743).abs() < 0.01, "west plate x {}", west.x);
    assert!((east.z - 50.20).abs() < 0.01, "east plate z {}", east.z);
    assert!((west.z - 50.00).abs() < 0.01, "west plate z {}", west.z);
    assert!((east.y - 0.761).abs() < 1e-5, "table-top height");

    // The cup goes to the guest's right, clear of the plate.
    let (cup, _) = plate_spot(&t[1], &t[0], MealSlot::Drink);
    assert!((cup.x - 100.257).abs() < 0.01);
    assert!((cup.z - 49.85).abs() < 0.01, "east cup z {}", cup.z);
}

#[tokio::test]
async fn a_plate_and_a_cup_share_a_chair_and_each_replaces_only_its_own_kind() {
    let game_state = make_test_game_state("meal_slots");
    game_state.sync_region_furniture(0, 0, &inn_table());
    let guest = make_guest(&game_state, "guest", 101.5, 50.0, 300).await;
    let maid = make_maid(&game_state, "miriel", 101.0, 51.0).await;
    sit(&game_state, &guest, 43).await;
    let served = || async {
        let mut ids: Vec<String> = meals(&game_state)
            .await
            .into_iter()
            .map(|m| m.item_def_id)
            .collect();
        ids.sort();
        ids
    };

    game_state.serve_meal(&maid, 43, "chicken_rice").await;
    game_state.serve_meal(&maid, 43, "beer").await;
    assert_eq!(served().await, ["beer", "chicken_rice"]);

    game_state.serve_meal(&maid, 43, "wine").await;
    assert_eq!(
        served().await,
        ["chicken_rice", "wine"],
        "the cup swaps, the plate stays"
    );
}

#[tokio::test]
async fn drinks_climb_the_stages_one_at_a_time() {
    let game_state = make_test_game_state("meal_alcohol");
    game_state.sync_region_furniture(0, 0, &inn_table());
    let guest = make_guest(&game_state, "guest", 101.5, 50.0, 300).await;
    let maid = make_maid(&game_state, "miriel", 101.0, 51.0).await;
    sit(&game_state, &guest, 43).await;

    // A beer is one unit, a wine two: beer, wine, beer climbs tipsy → wasted
    // in three cups and stays there.
    for (cup, expected) in [("beer", "tipsy"), ("wine", "wasted"), ("beer", "wasted")] {
        game_state.serve_meal(&maid, 43, cup).await;
        let fresh = meals(&game_state)
            .await
            .into_iter()
            .find(|m| !m.eaten)
            .expect("a fresh cup");
        game_state.eat_meal(&guest, fresh.id).await;
        let stages: Vec<String> = game_state.hunger.read().await[&guest]
            .debuffs
            .iter()
            .map(|d| d.def.id.clone())
            .collect();
        assert_eq!(stages, [expected], "one stage at a time");
    }
}

#[tokio::test]
async fn only_official_npcs_serve_and_only_to_a_seated_guest_in_reach() {
    let game_state = make_test_game_state("meal_serve_gate");
    game_state.sync_region_furniture(0, 0, &inn_table());
    let guest = make_guest(&game_state, "guest", 101.5, 50.0, 300).await;
    let stranger = make_guest(&game_state, "stranger", 101.0, 49.0, 300).await;
    let maid = make_maid(&game_state, "miriel", 101.0, 51.0).await;
    let mut maid_rx = game_state.register_direct_channel(&maid).await;

    game_state.serve_meal(&stranger, 43, "chicken_rice").await;
    assert!(meals(&game_state).await.is_empty(), "a player cannot serve");

    game_state.serve_meal(&maid, 43, "chicken_rice").await;
    assert!(meals(&game_state).await.is_empty(), "nobody is seated yet");

    sit(&game_state, &guest, 43).await;
    game_state.serve_meal(&maid, 43, "sword").await;
    assert!(meals(&game_state).await.is_empty(), "not a dish");

    game_state.serve_meal(&maid, 43, "chicken_rice").await;
    let served = meals(&game_state).await;
    assert_eq!(served.len(), 1);
    assert_eq!(served[0].for_player, guest);
    assert_eq!(served[0].chair_object_id, 43);
    assert!((served[0].position.x - 100.257).abs() < 0.01);
    assert!(drain(&mut maid_rx)
        .iter()
        .any(|m| matches!(m, ServerMessage::MealPlaced { .. })));

    // Serving the same chair again replaces the plate rather than stacking.
    game_state.serve_meal(&maid, 43, "bread").await;
    let served = meals(&game_state).await;
    assert_eq!(served.len(), 1);
    assert_eq!(served[0].item_def_id, "bread");
}

#[tokio::test]
async fn eating_fills_satiation_and_needs_the_plates_own_chair() {
    let game_state = make_test_game_state("meal_eat");
    game_state.sync_region_furniture(0, 0, &inn_table());
    let guest = make_guest(&game_state, "guest", 101.5, 50.0, 120).await;
    let other = make_guest(&game_state, "other", 98.5, 50.0, 120).await;
    let maid = make_maid(&game_state, "miriel", 101.0, 51.0).await;
    sit(&game_state, &guest, 43).await;
    sit(&game_state, &other, 45).await;
    game_state.serve_meal(&maid, 43, "chicken_rice").await;
    let meal_id = meals(&game_state).await[0].id;

    game_state.eat_meal(&other, meal_id).await;
    assert_eq!(meals(&game_state).await.len(), 1, "someone else's plate");
    assert_eq!(game_state.hunger.read().await[&other].satiation, 120);

    game_state.eat_meal(&guest, meal_id).await;
    let left = meals(&game_state).await;
    assert!(
        left.len() == 1 && left[0].eaten,
        "the empty plate stays for the maid"
    );
    assert_eq!(
        game_state.hunger.read().await[&guest].satiation,
        SATIATION_MAX
    );

    // Seconds are refused: the plate is empty, and only clearing removes it.
    game_state
        .hunger
        .write()
        .await
        .get_mut(&guest)
        .unwrap()
        .satiation = 100;
    game_state.eat_meal(&guest, meal_id).await;
    assert_eq!(game_state.hunger.read().await[&guest].satiation, 100);
    game_state.clear_meal(&maid, meal_id).await;
    assert!(meals(&game_state).await.is_empty());
}

#[tokio::test]
async fn a_plate_lingers_after_the_guest_stands_then_expires_or_is_cleared() {
    let game_state = make_test_game_state("meal_linger");
    game_state.sync_region_furniture(0, 0, &inn_table());
    let guest = make_guest(&game_state, "guest", 101.5, 50.0, 300).await;
    let maid = make_maid(&game_state, "miriel", 101.0, 51.0).await;
    sit(&game_state, &guest, 43).await;
    game_state.serve_meal(&maid, 43, "chicken_rice").await;
    let meal_id = meals(&game_state).await[0].id;

    let expires_at = || async { game_state.meals.read().await[&meal_id].expires_at };
    let untouched = expires_at().await;
    game_state.tick_meals().await;
    assert_eq!(expires_at().await, untouched, "still seated: no linger");

    game_state.set_player_interaction(&guest, None, None).await;
    game_state.tick_meals().await;
    assert!(
        expires_at().await <= std::time::Instant::now() + std::time::Duration::from_secs(90),
        "the guest left: the plate lingers, then goes"
    );

    // Back on the chair too late: the plate is already abandoned and the
    // maid clears it on arrival.
    game_state.clear_meal(&guest, meal_id).await;
    assert_eq!(meals(&game_state).await.len(), 1, "guests don't clear");
    game_state.clear_meal(&maid, meal_id).await;
    assert!(meals(&game_state).await.is_empty());

    // Expiry path: an abandoned plate nobody clears goes on its own.
    sit(&game_state, &guest, 43).await;
    game_state.serve_meal(&maid, 43, "chicken_rice").await;
    let meal_id = meals(&game_state).await[0].id;
    game_state
        .meals
        .write()
        .await
        .get_mut(&meal_id)
        .unwrap()
        .expires_at = std::time::Instant::now() - std::time::Duration::from_secs(1);
    game_state.tick_meals().await;
    assert!(meals(&game_state).await.is_empty());
}

#[tokio::test]
async fn a_drink_adds_only_its_own_nutrition() {
    let game_state = make_test_game_state("meal_drink");
    game_state.sync_region_furniture(0, 0, &inn_table());
    let guest = make_guest(&game_state, "guest", 101.5, 50.0, 300).await;
    let maid = make_maid(&game_state, "miriel", 101.0, 51.0).await;
    sit(&game_state, &guest, 43).await;
    game_state.serve_meal(&maid, 43, "wine").await;
    let meal_id = meals(&game_state).await[0].id;

    game_state.eat_meal(&guest, meal_id).await;
    assert_eq!(game_state.hunger.read().await[&guest].satiation, 420);
    assert!(meals(&game_state).await[0].eaten, "the cup stays, emptied");
}
