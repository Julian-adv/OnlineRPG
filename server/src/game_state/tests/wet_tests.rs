// ---- Wet (doc/DEBUFF.md) ---------------------------------------------------
// `SplitWorldTiles` + `SeaOnlyWater` put the ocean at negative x (bed −5 m,
// surface 0) and dry land at positive x, so a step's x decides whether it
// soaks. Paused-time tests drive `soak_movers` by hand.

use super::*;
use crate::game_state::ambient_spawn::MoveStep;
use onlinerpg_shared::hunger::SATIATION_START;
use tokio::time::{advance, Duration};

use crate::game_state::WET_DEBUFF_ID as WET;

/// One round per water-check bucket, so the mover is sampled whatever their
/// id hashes to.
const BUCKETS: usize = 5;

async fn make_wader(game_state: &GameState, name: &str) -> (PlayerId, DirectRx) {
    let id = pid(name);
    game_state.add_player(make_player(name, 100.0, 50.0)).await;
    game_state
        .register_player_character(&id, 1, 0, attrs_with_cha(10), 0, Some(SATIATION_START))
        .await;
    let rx = game_state.register_direct_channel(&id).await;
    (id, rx)
}

fn step_to(player_id: PlayerId, x: f32, floor_level: i8) -> MoveStep {
    let to = Position { x, y: 0.0, z: 50.0 };
    MoveStep {
        player_id,
        from: to,
        to,
        floor_level,
        is_official_npc: false,
    }
}

async fn soak(game_state: &GameState, steps: &[MoveStep]) {
    for _ in 0..BUCKETS {
        game_state.soak_movers(steps).await;
    }
}

async fn wet_remaining(game_state: &GameState, id: &PlayerId) -> Option<Duration> {
    let now = tokio::time::Instant::now();
    let hunger = game_state.hunger.read().await;
    hunger
        .get(id)?
        .debuffs
        .iter()
        .find(|d| d.def.id == WET)
        .map(|d| d.until.saturating_duration_since(now))
}

async fn move_mult(game_state: &GameState, id: &PlayerId) -> f32 {
    game_state
        .hunger_movement_profiles_for(&[*id])
        .await
        .get(id)
        .map_or(1.0, |(m, _)| *m)
}

#[tokio::test(start_paused = true)]
async fn a_step_into_the_sea_soaks_and_slows() {
    let game_state = make_test_game_state("wet_sea_step");
    let (id, _rx) = make_wader(&game_state, "wader").await;

    soak(&game_state, &[step_to(id, -100.0, 0)]).await;

    assert_eq!(
        wet_remaining(&game_state, &id).await,
        Some(Duration::from_secs(450))
    );
    // Walk 3.0 → 2.49, sprint 4.5 → 3.735: between walking and running.
    assert!((move_mult(&game_state, &id).await - 0.83).abs() < 1e-6);
}

/// The deck is the server's own index and the mover's server-side Y, so a
/// client claiming a lofty Y beside the bridge gains nothing, and a wader
/// passing under the span still soaks.
#[tokio::test(start_paused = true)]
async fn a_bridge_deck_over_the_sea_stays_dry_beside_and_under_it_soaks() {
    let game_state = make_test_game_state("wet_bridge_deck");
    let (id, _rx) = make_wader(&game_state, "walker").await;
    game_state.sync_region_furniture(-1, 0, &[stone_bridge(-100.0, 3.0, 50.0)]);

    let mut on_deck = step_to(id, -92.0, 0);
    on_deck.to.y = 4.0;
    soak(&game_state, &[on_deck]).await;
    assert_eq!(wet_remaining(&game_state, &id).await, None);

    let mut under = step_to(id, -100.0, 0);
    under.to.y = -5.0;
    soak(&game_state, &[under]).await;
    assert_eq!(
        wet_remaining(&game_state, &id).await,
        Some(Duration::from_secs(450))
    );

    let (id2, _rx2) = make_wader(&game_state, "flier").await;
    let mut beside = step_to(id2, -112.0, 0);
    beside.to.y = 9.0;
    soak(&game_state, &[beside]).await;
    assert_eq!(
        wet_remaining(&game_state, &id2).await,
        Some(Duration::from_secs(450))
    );
}

#[tokio::test(start_paused = true)]
async fn steps_on_dry_land_never_soak() {
    let game_state = make_test_game_state("wet_dry_step");
    let (id, _rx) = make_wader(&game_state, "walker").await;

    soak(&game_state, &[step_to(id, 100.0, 0)]).await;

    assert_eq!(wet_remaining(&game_state, &id).await, None);
    assert_eq!(move_mult(&game_state, &id).await, 1.0);
}

#[tokio::test(start_paused = true)]
async fn water_above_a_surface_floor_is_never_sampled() {
    let game_state = make_test_game_state("wet_upper_floor");
    let (id, _rx) = make_wader(&game_state, "upstairs").await;

    soak(&game_state, &[step_to(id, -100.0, 1)]).await;

    assert_eq!(wet_remaining(&game_state, &id).await, None);
}

#[tokio::test(start_paused = true)]
async fn a_player_without_a_hunger_entry_is_exempt() {
    let game_state = make_test_game_state("wet_official_exempt");
    let id = pid("official");
    game_state
        .add_player(make_player("official", 100.0, 50.0))
        .await;

    soak(&game_state, &[step_to(id, -100.0, 0)]).await;

    assert_eq!(wet_remaining(&game_state, &id).await, None);
}

#[tokio::test(start_paused = true)]
async fn wading_refreshes_only_once_the_soaking_is_wearing_off() {
    let game_state = make_test_game_state("wet_refresh_gate");
    let (id, _rx) = make_wader(&game_state, "swimmer").await;
    let in_water = [step_to(id, -100.0, 0)];

    soak(&game_state, &in_water).await;
    advance(Duration::from_secs(100)).await;

    // Still 350 s left: wading costs no terrain sample and no refresh.
    soak(&game_state, &in_water).await;
    assert_eq!(
        wet_remaining(&game_state, &id).await,
        Some(Duration::from_secs(350))
    );

    advance(Duration::from_secs(100)).await;
    soak(&game_state, &in_water).await;
    assert_eq!(
        wet_remaining(&game_state, &id).await,
        Some(Duration::from_secs(450))
    );
}

#[tokio::test(start_paused = true)]
async fn leaving_the_water_dries_off_after_a_game_hour() {
    let game_state = make_test_game_state("wet_dries_off");
    let (id, _rx) = make_wader(&game_state, "beachcomber").await;

    soak(&game_state, &[step_to(id, -100.0, 0)]).await;
    advance(Duration::from_secs(451)).await;
    game_state.tick_debuffs().await;

    assert_eq!(wet_remaining(&game_state, &id).await, None);
    assert_eq!(move_mult(&game_state, &id).await, 1.0);
}

/// A lit fire at the wader's feet, on their floor.
async fn light_fire_at(game_state: &GameState, x: f32, floor_level: i8) {
    game_state
        .spawn_campfire(
            Position { x, y: 0.0, z: 50.0 },
            floor_level,
            onlinerpg_shared::hunger::CAMPFIRE_DURATION_MS,
        )
        .await;
}

#[tokio::test(start_paused = true)]
async fn a_campfire_pulls_the_soaking_down_ten_times_faster() {
    let game_state = make_test_game_state("wet_campfire_dries");
    let (id, _rx) = make_wader(&game_state, "camper").await;
    soak(&game_state, &[step_to(id, -100.0, 0)]).await;
    light_fire_at(&game_state, 100.0, 0).await;

    // One sweep second by the fire burns ten off the soaking.
    game_state
        .tick_campfire_drying(Duration::from_secs(1))
        .await;
    assert_eq!(
        wet_remaining(&game_state, &id).await,
        Some(Duration::from_secs(441))
    );

    for _ in 0..49 {
        game_state
            .tick_campfire_drying(Duration::from_secs(1))
            .await;
    }
    game_state.tick_debuffs().await;
    assert_eq!(wet_remaining(&game_state, &id).await, None);
    assert_eq!(move_mult(&game_state, &id).await, 1.0);
}

#[tokio::test(start_paused = true)]
async fn a_fire_out_of_reach_or_on_another_floor_dries_nobody() {
    let game_state = make_test_game_state("wet_campfire_out_of_reach");
    let (id, _rx) = make_wader(&game_state, "loner").await;
    soak(&game_state, &[step_to(id, -100.0, 0)]).await;

    // 10 m away, and a second fire underfoot but on the storey above.
    light_fire_at(&game_state, 110.0, 0).await;
    light_fire_at(&game_state, 100.0, 1).await;
    game_state
        .tick_campfire_drying(Duration::from_secs(1))
        .await;

    assert_eq!(
        wet_remaining(&game_state, &id).await,
        Some(Duration::from_secs(450))
    );
}

#[tokio::test(start_paused = true)]
async fn drying_leaves_a_dry_player_alone() {
    let game_state = make_test_game_state("wet_campfire_dry_player");
    let (id, _rx) = make_wader(&game_state, "dry_camper").await;
    light_fire_at(&game_state, 100.0, 0).await;

    game_state
        .tick_campfire_drying(Duration::from_secs(1))
        .await;

    assert_eq!(wet_remaining(&game_state, &id).await, None);
}

/// An official NPC's `/light_campfire` fire is the same entry as a player's,
/// so it dries the people standing around it (doc/HUNGER.md).
#[tokio::test(start_paused = true)]
async fn an_npc_lit_campfire_dries_too() {
    let game_state = make_test_game_state("wet_npc_campfire");
    let auth = make_test_auth("wet_npc_campfire");
    let (id, _rx) = make_wader(&game_state, "guest").await;
    soak(&game_state, &[step_to(id, -100.0, 0)]).await;

    let npc_id = pid("npc_signe");
    let mut npc = make_player("npc_signe", 100.0, 50.0);
    npc.is_official_npc = true;
    game_state.add_player(npc).await;
    game_state
        .send_chat_message(&npc_id, "/light_campfire".to_string(), &auth)
        .await;
    assert_eq!(game_state.campfires.read().await.len(), 1);

    game_state
        .tick_campfire_drying(Duration::from_secs(1))
        .await;

    assert_eq!(
        wet_remaining(&game_state, &id).await,
        Some(Duration::from_secs(441))
    );
}

async fn broadcast_wet_flag(game_state: &GameState, id: &PlayerId) -> bool {
    game_state.players.read().await[id].wet
}

#[tokio::test(start_paused = true)]
async fn the_soaking_rides_the_broadcast_player_for_nearby_clients() {
    let game_state = make_test_game_state("wet_broadcast_flag");
    let (id, _rx) = make_wader(&game_state, "splasher").await;
    assert!(!broadcast_wet_flag(&game_state, &id).await);

    soak(&game_state, &[step_to(id, -100.0, 0)]).await;
    assert!(broadcast_wet_flag(&game_state, &id).await);

    advance(Duration::from_secs(451)).await;
    game_state.tick_debuffs().await;
    assert!(!broadcast_wet_flag(&game_state, &id).await);
}

#[tokio::test(start_paused = true)]
async fn death_drops_the_broadcast_soaking_too() {
    let game_state = make_test_game_state("wet_broadcast_death");
    let (id, _rx) = make_wader(&game_state, "drowner").await;
    soak(&game_state, &[step_to(id, -100.0, 0)]).await;

    game_state.clear_debuffs(&id).await;

    assert!(!broadcast_wet_flag(&game_state, &id).await);
    assert_eq!(wet_remaining(&game_state, &id).await, None);
}

async fn carry(game_state: &GameState, id: &PlayerId, inventory: PlayerInventory) {
    game_state.inventories.write().await.insert(*id, inventory);
}

async fn carried_weight(game_state: &GameState, id: &PlayerId) -> f32 {
    let armor_mult = game_state.armor_weight_mult(id).await;
    let inventories = game_state.inventories.read().await;
    game_state.calc_total_weight(&inventories[id], armor_mult)
}

#[tokio::test(start_paused = true)]
async fn soaked_armour_drags_but_the_rest_of_the_pack_does_not() {
    let game_state = make_test_game_state("wet_armour_weight");
    let (id, _rx) = make_wader(&game_state, "porter").await;
    // chain_mail (30) + wooden_shield (6) armour, plus a non-armour torch (1).
    carry(
        &game_state,
        &id,
        PlayerInventory {
            active_ammo: None,
            bag: vec![bag_item(801, "chain_mail", 1), bag_item(802, "torch", 1)],
            equipped: std::collections::HashMap::from([(
                EquipSlot::OffHand,
                bag_item(803, "wooden_shield", 1),
            )]),
        },
    )
    .await;

    let dry = carried_weight(&game_state, &id).await;
    assert!((dry - 37.0).abs() < 1e-3, "expected 30+6+1, got {dry}");

    soak(&game_state, &[step_to(id, -100.0, 0)]).await;

    // Only the 36 of armour scales; the torch is untouched. Derived from the
    // constant so retuning it stays a one-line change.
    let mult = crate::debuff_defs::debuff_def(WET)
        .unwrap()
        .armor_weight_mult;
    let expected = 36.0 * mult + 1.0;
    let soaked = carried_weight(&game_state, &id).await;
    assert!(
        (soaked - expected).abs() < 1e-3,
        "expected {expected}, got {soaked}"
    );
}

#[tokio::test(start_paused = true)]
async fn an_unarmoured_player_carries_the_same_soaked_or_dry() {
    let game_state = make_test_game_state("wet_no_armour_weight");
    let (id, _rx) = make_wader(&game_state, "streaker").await;
    carry(
        &game_state,
        &id,
        PlayerInventory {
            active_ammo: None,
            bag: vec![bag_item(811, "torch", 4)],
            equipped: std::collections::HashMap::new(),
        },
    )
    .await;

    let dry = carried_weight(&game_state, &id).await;
    soak(&game_state, &[step_to(id, -100.0, 0)]).await;

    assert_eq!(carried_weight(&game_state, &id).await, dry);
}

/// Scale measurement for the wet path, sized against the 5,000 concurrent-user
/// target. Not an assertion — run with --nocapture to read the timings.
#[tokio::test]
#[ignore = "measurement, not an assertion; run explicitly with --nocapture"]
async fn wet_path_cost_at_scale() {
    const USERS: usize = 5_000;
    let game_state = make_test_game_state("wet_scale");

    // Cache-hit sampling cost: the fixture serves tiles from memory, so this
    // is the steady-state path, not first-touch tile IO.
    for i in 0..64 {
        let _ = game_state.water_depth_at(-100.0 + i as f32, 50.0).await;
    }
    let start = std::time::Instant::now();
    for i in 0..USERS {
        let x = -100.0 + (i % 64) as f32;
        let z = 50.0 + ((i / 64) % 64) as f32;
        let _ = game_state.water_depth_at(x, z).await;
    }
    let elapsed = start.elapsed();
    println!(
        "water_depth_at x{USERS}: {elapsed:?} ({:.2} us/sample)",
        elapsed.as_secs_f64() * 1e6 / USERS as f64
    );

    let mut ids = Vec::with_capacity(USERS);
    for i in 0..USERS {
        let name = format!("wet_bot{i}");
        let id = pid(&name);
        game_state.add_player(make_player(&name, 100.0, 50.0)).await;
        game_state
            .register_player_character(&id, 1, 0, attrs_with_cha(10), 0, Some(700))
            .await;
        ids.push(id);
    }
    let steps: Vec<_> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| step_to(*id, 100.0 + (i % 64) as f32, 0))
        .collect();

    // One movement tick with everybody walking: a fifth of them fall in this
    // tick's water-check bucket.
    let start = std::time::Instant::now();
    game_state.soak_movers(&steps).await;
    println!(
        "soak_movers, {USERS} movers, none wet: {:?}",
        start.elapsed()
    );

    let start = std::time::Instant::now();
    game_state
        .tick_campfire_drying(Duration::from_secs(1))
        .await;
    println!("tick_campfire_drying, nobody wet: {:?}", start.elapsed());

    for id in &ids {
        game_state.inflict_debuff(id, WET, Some(true)).await;
    }
    // The refresh gate should now skip the sampling for all of them.
    let start = std::time::Instant::now();
    game_state.soak_movers(&steps).await;
    println!(
        "soak_movers, {USERS} movers, all already wet: {:?}",
        start.elapsed()
    );

    for i in 0..200 {
        light_fire_at(&game_state, 300.0 + i as f32, 0).await;
    }
    let start = std::time::Instant::now();
    game_state
        .tick_campfire_drying(Duration::from_secs(1))
        .await;
    println!(
        "tick_campfire_drying, {USERS} wet x 200 fires: {:?}",
        start.elapsed()
    );
}
