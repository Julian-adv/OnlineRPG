use super::*;

const CRYPT_ID: &str = "old_crypt";

/// The Old Crypt's deepest depth and the world position of its chest.
async fn crypt_chest_spot(game_state: &GameState) -> (u8, Position) {
    game_state.ensure_dungeon_runtime(CRYPT_ID).await;
    let entrance = game_state
        .dungeon_defs
        .get(CRYPT_ID)
        .expect("old_crypt def");
    let dungeons = game_state.dungeons.read().await;
    let rt = dungeons.get(CRYPT_ID).expect("crypt runtime");
    let deepest = rt.layouts.len() as u8;
    let chest = rt
        .layouts
        .last()
        .and_then(|l| l.chest)
        .expect("crypt's last floor has a chest");
    (deepest, cell_center(&entrance.position(), deepest, chest))
}

/// Mark the crypt's deepest-floor guardian dead (creating the floor runtime
/// if needed) and, when given, seat a player on that floor. `claimant` earns
/// the chest, as standing within `CHEST_CLAIM_RADIUS` of the boss would.
async fn kill_crypt_guardian(
    game_state: &GameState,
    deepest: u8,
    player_id: Option<PlayerId>,
    claimant: Option<i64>,
) {
    let mut dungeons = game_state.dungeons.write().await;
    let rt = dungeons.get_mut(CRYPT_ID).expect("crypt runtime");
    let floor = rt
        .floors
        .entry(deepest)
        .or_insert_with(|| super::dungeon::FloorRuntime {
            slots: Vec::new(),
            players: HashSet::new(),
            boss_defeated: false,
            chest_claimants: HashSet::new(),
        });
    if let Some(player_id) = player_id {
        floor.players.insert(player_id);
    }
    if let Some(character_id) = claimant {
        floor.chest_claimants.insert(character_id);
    }
    floor.boss_defeated = true;
}

/// Roll the game clock past sunset so the next tick resets the dungeons.
async fn advance_one_night(game_state: &GameState) {
    game_state.debug_set_time(0, 0);
    game_state.tick_dungeon_reset().await;
    game_state.debug_set_time(23, 0);
    game_state.tick_dungeon_reset().await;
}

/// Put a character next to the Old Crypt's chest on the deepest floor with
/// the guardian already dead — every check `open_dungeon_chest` makes before
/// the nightly refill gate.
async fn stage_chest_opener(game_state: &GameState, name: &str, character_id: i64) -> PlayerId {
    stage_opener(game_state, name, character_id, true).await
}

/// As above, but `earns_claim` decides whether they were beside the guardian.
async fn stage_opener(
    game_state: &GameState,
    name: &str,
    character_id: i64,
    earns_claim: bool,
) -> PlayerId {
    let player_id = pid(name);
    let (deepest, chest_pos) = crypt_chest_spot(game_state).await;

    let mut player = make_player(name, chest_pos.x, chest_pos.z);
    player.floor_level = -(deepest as i8);
    game_state.add_player(player).await;
    game_state
        .register_player_character(&player_id, character_id, 0, attrs_with_cha(12), 0, None)
        .await;
    kill_crypt_guardian(
        game_state,
        deepest,
        Some(player_id),
        earns_claim.then_some(character_id),
    )
    .await;
    player_id
}

/// Assert the next direct message refuses a chest interaction, with a reason
/// containing `expected`.
fn assert_chest_rejected(rx: &mut DirectRx, expected: &str) {
    match rx.try_recv() {
        Ok(ServerMessage::InteractionRejected { reason }) => {
            assert!(
                reason.contains(expected),
                "expected {expected:?}, got {reason}"
            );
        }
        other => panic!(
            "Expected a rejection containing {expected:?}, got {:?}",
            other
        ),
    }
}

/// Assert the next direct message is the item-less, gold-less chest open —
/// the "already claimed tonight" lid swing on an empty box.
fn assert_chest_empty_opened(rx: &mut DirectRx) {
    match rx.try_recv() {
        Ok(ServerMessage::DungeonChestOpened {
            item_def_ids, gold, ..
        }) => {
            assert!(item_def_ids.is_empty(), "empty open must carry no items");
            assert_eq!(gold, 0, "empty open must carry no gold");
        }
        other => panic!("Expected an empty chest open, got {:?}", other),
    }
}

/// A relog mints a fresh PlayerId, so the refill gate has to be keyed by
/// character and reloaded from the DB — otherwise the chest is farmable by
/// logging out and back in.
#[tokio::test]
async fn dungeon_chest_stays_empty_across_a_relog() {
    let auth = make_test_auth("chest_relog");
    let account = auth.login_npc("npc_chest_relog").unwrap();
    let character = create_test_character(&auth, &account, "Delver");

    let game_state = make_test_game_state("chest_relog");
    let player_id = stage_chest_opener(&game_state, "Delver", character.id).await;

    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    assert!(
        game_state.get_player_gold(&player_id).await > 0,
        "first open should pay out"
    );

    // Log out, then back in as a new session for the same character.
    game_state.unregister_player_character(&player_id).await;
    game_state.remove_player(&player_id).await;

    let opens = auth
        .load_dungeon_history(character.id)
        .map(|h| h.0)
        .unwrap();
    assert_eq!(opens.len(), 1, "the open should have been persisted");
    let rejoined_id = stage_chest_opener(&game_state, "Delver Rejoined", character.id).await;
    game_state.set_chest_opens(character.id, opens).await;

    let mut rejoined_rx = game_state.register_direct_channel(&rejoined_id).await;
    game_state
        .open_dungeon_chest(&rejoined_id, CRYPT_ID, &auth)
        .await;

    assert_chest_empty_opened(&mut rejoined_rx);
    assert_eq!(
        game_state.get_player_gold(&rejoined_id).await,
        0,
        "second open must not pay out"
    );
}

#[tokio::test]
async fn dungeon_chest_refills_once_per_night() {
    let auth = make_test_auth("chest_nightfall");
    let account = auth.login_npc("npc_chest_nightfall").unwrap();
    let character = create_test_character(&auth, &account, "Nightcaller");

    let game_state = make_test_game_state("chest_nightfall");
    let player_id = stage_chest_opener(&game_state, "Nightcaller", character.id).await;
    let mut direct_rx = game_state.register_direct_channel(&player_id).await;

    // Opened at midnight, before the day's sunset.
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    let after_first = game_state.get_player_gold(&player_id).await;
    assert!(after_first > 0, "first open should pay out");
    while direct_rx.try_recv().is_ok() {}

    // Still the same night: no refill.
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    assert_eq!(
        game_state.get_player_gold(&player_id).await,
        after_first,
        "the chest must not refill before nightfall"
    );

    // Past sunset — a new night, so one more open.
    game_state.debug_set_time(23, 0);
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    let after_second = game_state.get_player_gold(&player_id).await;
    assert!(
        after_second > after_first,
        "the chest should refill at nightfall"
    );

    // Same night again: still one open only.
    game_state.debug_set_time(23, 30);
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    assert_eq!(
        game_state.get_player_gold(&player_id).await,
        after_second,
        "one open per night, not one per visit"
    );
}

/// An unspawned boss slot (spawn pending, or refused by the global monster
/// cap) used to read as "no guardian alive" and open the gate.
#[tokio::test]
async fn dungeon_chest_stays_shut_until_the_guardian_dies() {
    let auth = make_test_auth("chest_guardian");
    let account = auth.login_npc("npc_chest_guardian").unwrap();
    let character = create_test_character(&auth, &account, "Challenger");

    let game_state = make_test_game_state("chest_guardian");
    let player_id = stage_chest_opener(&game_state, "Challenger", character.id).await;
    {
        let mut dungeons = game_state.dungeons.write().await;
        let rt = dungeons.get_mut(CRYPT_ID).expect("crypt runtime");
        let deepest = rt.layouts.len() as u8;
        rt.floors
            .get_mut(&deepest)
            .expect("staged floor")
            .boss_defeated = false;
    }

    let mut direct_rx = game_state.register_direct_channel(&player_id).await;
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;

    assert_chest_rejected(&mut direct_rx, "The guardian still lives");
    assert_eq!(game_state.get_player_gold(&player_id).await, 0);
    assert!(
        auth.load_dungeon_history(character.id)
            .map(|h| h.0)
            .unwrap()
            .is_empty(),
        "a refused open must not consume the night's chest"
    );
}

/// The nightly claim is the durable anti-duplication boundary. A failed DB
/// write must reject the open without rewards and release only this attempt's
/// in-memory claim so a repaired storage layer can retry.
#[tokio::test(start_paused = true)]
async fn dungeon_chest_persistence_failure_rejects_without_reward_and_can_retry() {
    let (auth, db_path) = make_test_auth_with_path("chest_persist_fail");
    let account = auth.login_npc("npc_chest_persist_fail").unwrap();
    let character = create_test_character(&auth, &account, "CarefulDelver");

    let game_state = make_test_game_state("chest_persist_fail");
    let player_id = stage_chest_opener(&game_state, "CarefulDelver", character.id).await;
    let mut direct_rx = game_state.register_direct_channel(&player_id).await;

    rusqlite::Connection::open(&db_path)
        .unwrap()
        .execute("DROP TABLE character_dungeon_chests", [])
        .unwrap();

    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;

    assert_chest_rejected(&mut direct_rx, "saved");
    assert_eq!(
        game_state.get_player_gold(&player_id).await,
        0,
        "failed persistence must not pay gold"
    );
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    tokio::time::sleep(*super::combat::CHEST_LOOT_EJECT_DELAY).await;
    assert!(
        game_state.ground_items.read().await.is_empty(),
        "failed persistence must not eject chest loot"
    );

    let repaired_auth = crate::auth::AuthService::new(db_path).unwrap();

    while direct_rx.try_recv().is_ok() {}
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &repaired_auth)
        .await;

    assert!(
        game_state.get_player_gold(&player_id).await > 0,
        "retry after schema repair should pay out"
    );
    // Poll the eject task onto its timer, then cross the lid-swing delay
    // (virtual time; same idiom as pickup_tests).
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    assert!(
        game_state.ground_items.read().await.is_empty(),
        "chest loot must stay withheld while the lid swings open"
    );
    tokio::time::sleep(*super::combat::CHEST_LOOT_EJECT_DELAY).await;
    assert!(
        !game_state.ground_items.read().await.is_empty(),
        "retry after schema repair should eject chest loot onto the floor"
    );
    assert_eq!(
        repaired_auth
            .load_dungeon_history(character.id)
            .map(|h| h.0)
            .unwrap()
            .len(),
        1,
        "successful retry should persist exactly one chest claim"
    );
}

/// The nightly refill gate compares two of these, so the index has to move
/// forward exactly once a day and never backwards — including across the
/// seasonal swing in sunset time.
#[test]
fn night_epoch_advances_once_per_game_day() {
    use crate::game_state::time::{
        GAME_DAYS_PER_MONTH, GAME_DAYS_PER_YEAR, GAME_HOURS_PER_DAY, GAME_START_YEAR,
    };

    let start = GameState::night_epoch_at(GAME_START_YEAR as u32, 1, 1, 0, 0);
    let mut previous = start;
    let mut advances = 0;

    // A full game year, sampled every 10 minutes.
    for day in 0..GAME_DAYS_PER_YEAR {
        let month = (day / GAME_DAYS_PER_MONTH + 1) as u8;
        let day_of_month = (day % GAME_DAYS_PER_MONTH + 1) as u8;
        for step in 0..(GAME_HOURS_PER_DAY * 6) {
            let hour = (step / 6) as u8;
            let minute = ((step % 6) * 10) as u8;
            let epoch = GameState::night_epoch_at(
                GAME_START_YEAR as u32,
                month,
                day_of_month,
                hour,
                minute,
            );
            assert!(
                epoch == previous || epoch == previous + 1,
                "night epoch jumped from {previous} to {epoch} at {month}/{day_of_month} {hour}:{minute}"
            );
            if epoch != previous {
                advances += 1;
                previous = epoch;
            }
        }
    }

    assert_eq!(
        advances, GAME_DAYS_PER_YEAR,
        "one nightfall per game day over a year"
    );
}

/// Logging in twice kicks the first session, whose cleanup then runs *after*
/// the replacement already loaded its chest history. Dropping character-keyed
/// state on that late cleanup would hand the live session a fresh chest.
#[tokio::test]
async fn session_replacement_keeps_the_live_session_chest_claim() {
    let auth = make_test_auth("chest_kick");
    let account = auth.login_npc("npc_chest_kick").unwrap();
    let character = create_test_character(&auth, &account, "Doubler");

    let game_state = make_test_game_state("chest_kick");
    let first_id = stage_chest_opener(&game_state, "Doubler", character.id).await;
    game_state
        .open_dungeon_chest(&first_id, CRYPT_ID, &auth)
        .await;
    assert!(game_state.get_player_gold(&first_id).await > 0);

    // Replacement session logs in and loads the claim from the DB...
    let second_id = stage_chest_opener(&game_state, "Doubler Again", character.id).await;
    let opens = auth
        .load_dungeon_history(character.id)
        .map(|h| h.0)
        .unwrap();
    game_state.set_chest_opens(character.id, opens).await;
    // ...then the kicked connection finally tears itself down.
    game_state.unregister_player_character(&first_id).await;
    game_state.remove_player(&first_id).await;

    let mut second_rx = game_state.register_direct_channel(&second_id).await;
    game_state
        .open_dungeon_chest(&second_id, CRYPT_ID, &auth)
        .await;

    assert_chest_empty_opened(&mut second_rx);
    assert_eq!(game_state.get_player_gold(&second_id).await, 0);
}

/// A real open arms the vault's return: firing it carries the opener to town;
/// the empty re-open arms nothing; and once fired (or pre-empted by a
/// disconnect) it does not fire again.
#[tokio::test]
async fn dungeon_chest_open_arms_a_town_return() {
    let auth = make_test_auth("chest_return");
    let account = auth.login_npc("npc_chest_return").unwrap();
    let character = create_test_character(&auth, &account, "Homeward");

    let game_state = make_test_game_state("chest_return");
    let player_id = stage_chest_opener(&game_state, "Homeward", character.id).await;
    let mut rx = game_state.register_direct_channel(&player_id).await;

    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    assert!(
        game_state.chest_returns.read().await.contains(&player_id),
        "a real open should arm the return"
    );

    game_state.fire_chest_return(&player_id).await;
    let (position, _, floor, _) = game_state.player_pose(&player_id).await.unwrap();
    let spawn = &crate::world_config::world_config().spawn_position;
    assert_eq!(floor, 0, "the return lands on the surface");
    assert!(
        (position.x - spawn.position().x).abs() < 0.01
            && (position.z - spawn.position().z).abs() < 0.01,
        "the return lands at the town spawn, got {position:?}"
    );
    assert!(
        !game_state.chest_returns.read().await.contains(&player_id),
        "a fired return is spent"
    );

    // Back on the deepest floor: the same-night re-open is empty and arms nothing.
    let (deepest, chest_pos) = crypt_chest_spot(&game_state).await;
    game_state
        .teleport_player(&player_id, chest_pos, 0.0, -(deepest as i8))
        .await;
    // The guardian stays down and the claim with it until the dungeon resets.
    while rx.try_recv().is_ok() {}
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    assert_chest_empty_opened(&mut rx);
    assert!(
        !game_state.chest_returns.read().await.contains(&player_id),
        "an empty open must not arm a return"
    );
}

/// Standing by the chest is not enough: the chest belongs to whoever was
/// beside the guardian when it fell, so a character parked on the last floor
/// no longer collects without fighting.
#[tokio::test]
async fn dungeon_chest_refuses_a_bystander_who_missed_the_kill() {
    let auth = make_test_auth("chest_bystander");
    let account = auth.login_npc("npc_chest_bystander").unwrap();
    let character = create_test_character(&auth, &account, "Parked");

    let game_state = make_test_game_state("chest_bystander");
    let player_id = stage_opener(&game_state, "Parked", character.id, false).await;

    let mut direct_rx = game_state.register_direct_channel(&player_id).await;
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;

    assert_chest_rejected(&mut direct_rx, "felled the guardian");
    assert_eq!(game_state.get_player_gold(&player_id).await, 0);
    assert!(
        auth.load_dungeon_history(character.id)
            .map(|h| h.0)
            .unwrap()
            .is_empty(),
        "a refused open must not consume the night's chest"
    );
}

/// The claim outlives leaving the floor — a party that steps out to rest can
/// come back for the chest they earned.
#[tokio::test]
async fn chest_claim_survives_leaving_the_floor() {
    let auth = make_test_auth("chest_claim_persists");
    let account = auth.login_npc("npc_chest_claim").unwrap();
    let character = create_test_character(&auth, &account, "Returner");

    let game_state = make_test_game_state("chest_claim_persists");
    let player_id = stage_chest_opener(&game_state, "Returner", character.id).await;
    let (deepest, chest_pos) = crypt_chest_spot(&game_state).await;

    game_state.teleport_to_town(&player_id).await;
    game_state
        .teleport_player(&player_id, chest_pos, 0.0, -(deepest as i8))
        .await;

    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;

    assert!(
        game_state.get_player_gold(&player_id).await > 0,
        "the chest paid out to its claimant after a round trip"
    );
}

/// Sunset empties the dungeons: occupants surface at the entrance and the
/// guardian's slot is freed, which is the only thing that lifts the claim.
#[tokio::test]
async fn dungeon_reset_evicts_occupants_and_wakes_the_guardian() {
    let auth = make_test_auth("dungeon_reset");
    let account = auth.login_npc("npc_dungeon_reset").unwrap();
    let character = create_test_character(&auth, &account, "Delver");

    let game_state = make_test_game_state("dungeon_reset");
    let player_id = stage_chest_opener(&game_state, "Delver", character.id).await;
    let deepest = crypt_chest_spot(&game_state).await.0;

    // First tick after boot only records the epoch — a restart must not
    // empty every dungeon.
    game_state.tick_dungeon_reset().await;
    assert!(
        game_state
            .players
            .read()
            .await
            .get(&player_id)
            .is_some_and(|p| p.floor_level == -(deepest as i8)),
        "the boot tick must leave delvers where they stand"
    );

    advance_one_night(&game_state).await;

    let entrance = game_state
        .dungeon_defs
        .get(CRYPT_ID)
        .expect("old_crypt def")
        .position();
    let players = game_state.players.read().await;
    let player = players.get(&player_id).expect("delver still online");
    assert_eq!(player.floor_level, 0, "the reset surfaces every occupant");
    assert!(
        player.position.dist_xz_sq(&entrance) < 0.01,
        "the reset lands them at the entrance, got {:?}",
        player.position
    );
    drop(players);

    let dungeons = game_state.dungeons.read().await;
    let floor = dungeons
        .get(CRYPT_ID)
        .and_then(|rt| rt.floors.get(&deepest))
        .expect("staged floor");
    assert!(
        !floor.boss_defeated,
        "the guardian rises with the new night"
    );
    assert!(floor.chest_claimants.is_empty(), "the night's claims lapse");
}

/// Someone descending while the sweep runs entered on the new night, so their
/// floor keeps its guardian rather than being freed under their feet — which
/// would orphan that floor's live monsters.
#[tokio::test]
async fn dungeon_reset_sweeps_a_floor_someone_walked_into() {
    let auth = make_test_auth("reset_latecomer");
    let account = auth.login_npc("npc_reset_latecomer").unwrap();
    let character = create_test_character(&auth, &account, "Latecomer");

    let game_state = make_test_game_state("reset_latecomer");
    let player_id = stage_chest_opener(&game_state, "Latecomer", character.id).await;
    let deepest = crypt_chest_spot(&game_state).await.0;

    game_state.tick_dungeon_reset().await;
    // Re-seat them as if they had just descended.
    let chest_pos = crypt_chest_spot(&game_state).await.1;
    game_state
        .teleport_player(&player_id, chest_pos, 0.0, -(deepest as i8))
        .await;
    kill_crypt_guardian(&game_state, deepest, Some(player_id), None).await;

    advance_one_night(&game_state).await;

    let dungeons = game_state.dungeons.read().await;
    let floor = dungeons
        .get(CRYPT_ID)
        .and_then(|rt| rt.floors.get(&deepest))
        .expect("staged floor");
    assert!(
        floor.players.is_empty(),
        "the second sweep pass evicts a latecomer it can still see"
    );
    assert!(
        !floor.boss_defeated,
        "an emptied floor is reset like any other"
    );
}
