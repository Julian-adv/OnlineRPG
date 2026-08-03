use super::*;

const CRYPT_ID: &str = "old_crypt";

/// Put a character next to the Old Crypt's chest on the deepest floor with
/// the guardian already dead — every check `open_dungeon_chest` makes before
/// the nightly refill gate.
async fn stage_chest_opener(game_state: &GameState, name: &str, character_id: i64) -> PlayerId {
    let player_id = pid(name);
    game_state.ensure_dungeon_runtime(CRYPT_ID).await;
    let entrance = game_state
        .dungeon_defs
        .get(CRYPT_ID)
        .expect("old_crypt def");

    let (deepest, chest_pos) = {
        let dungeons = game_state.dungeons.read().await;
        let rt = dungeons.get(CRYPT_ID).expect("crypt runtime");
        let deepest = rt.layouts.len() as u8;
        let chest = rt
            .layouts
            .last()
            .and_then(|l| l.chest)
            .expect("crypt's last floor has a chest");
        (deepest, cell_center(&entrance.position(), deepest, chest))
    };

    let mut player = make_player(name, chest_pos.x, chest_pos.z);
    player.floor_level = -(deepest as i8);
    game_state.add_player(player).await;
    game_state
        .register_player_character(&player_id, character_id, 0, attrs_with_cha(12), 0, None)
        .await;
    {
        let mut dungeons = game_state.dungeons.write().await;
        let rt = dungeons.get_mut(CRYPT_ID).expect("crypt runtime");
        let floor = rt
            .floors
            .entry(deepest)
            .or_insert_with(|| super::dungeon::FloorRuntime {
                slots: Vec::new(),
                players: HashSet::new(),
                boss_defeated: false,
            });
        floor.players.insert(player_id);
        floor.boss_defeated = true;
    }
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

    assert_chest_rejected(&mut rejoined_rx, "nightfall");
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
#[tokio::test]
async fn dungeon_chest_persistence_failure_rejects_without_reward_and_can_retry() {
    let (auth, db_path) = make_test_auth_with_path("chest_persist_fail");
    let account = auth.login_npc("npc_chest_persist_fail").unwrap();
    let character = create_test_character(&auth, &account, "CarefulDelver");

    let game_state = make_test_game_state("chest_persist_fail");
    let player_id = stage_chest_opener(&game_state, "CarefulDelver", character.id).await;
    game_state
        .inventories
        .write()
        .await
        .insert(player_id, PlayerInventory::default());
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
    assert!(
        game_state
            .get_player_inventory(&player_id)
            .await
            .expect("staged inventory")
            .bag
            .is_empty(),
        "failed persistence must not grant chest items"
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
    assert!(
        !game_state
            .get_player_inventory(&player_id)
            .await
            .expect("staged inventory")
            .bag
            .is_empty(),
        "retry after schema repair should grant chest items"
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

    assert_chest_rejected(&mut second_rx, "nightfall");
    assert_eq!(game_state.get_player_gold(&second_id).await, 0);
}
