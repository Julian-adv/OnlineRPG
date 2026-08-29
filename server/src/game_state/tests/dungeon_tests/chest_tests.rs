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

/// The key the crypt's chest takes: its deepest (and only) locked floor's.
const CRYPT_CHEST_KEY: &str = "crypt_key_5";

/// Seat a player on the crypt's deepest floor (creating the floor runtime if
/// needed), with a slain guardian holding its slot until the reset.
async fn seat_on_deepest_floor(game_state: &GameState, deepest: u8, player_id: PlayerId) {
    let mut dungeons = game_state.dungeons.write().await;
    let rt = dungeons.get_mut(CRYPT_ID).expect("crypt runtime");
    let floor = rt
        .floors
        .entry(deepest)
        .or_insert_with(|| super::dungeon::FloorRuntime {
            slots: vec![super::dungeon::SpawnSlot {
                alive_monster_id: None,
                respawn_at_ms: super::dungeon::BOSS_RESPAWN_NEVER,
                is_boss: true,
            }],
            players: HashMap::new(),
        });
    floor.players.insert(player_id, 0);
}

/// How many `item_def_id` the player's bag holds.
async fn bag_count(game_state: &GameState, player_id: &PlayerId, item_def_id: &str) -> u32 {
    game_state
        .inventories
        .read()
        .await
        .get(player_id)
        .map(|inv| {
            inv.bag
                .iter()
                .filter(|i| i.item_def_id == item_def_id)
                .map(|i| i.quantity)
                .sum()
        })
        .unwrap_or(0)
}

/// Roll the game clock past sunset so the next tick resets the dungeons.
async fn advance_one_night(game_state: &GameState) {
    game_state.debug_set_time(0, 0);
    game_state.tick_dungeon_reset().await;
    game_state.debug_set_time(23, 0);
    game_state.tick_dungeon_reset().await;
}

/// Put a character next to the Old Crypt's chest on the deepest floor with
/// the chest's key in their bag — every check `open_dungeon_chest` makes
/// before the nightly refill gate.
async fn stage_chest_opener(game_state: &GameState, name: &str, character_id: i64) -> PlayerId {
    stage_opener(game_state, name, character_id, true).await
}

/// As above, but `has_key` decides whether the bag holds the chest's key.
async fn stage_opener(
    game_state: &GameState,
    name: &str,
    character_id: i64,
    has_key: bool,
) -> PlayerId {
    let player_id = pid(name);
    let (deepest, chest_pos) = crypt_chest_spot(game_state).await;

    let mut player = make_player(name, chest_pos.x, chest_pos.z);
    player.floor_level = -(deepest as i8);
    game_state.add_player(player).await;
    game_state
        .register_player_character(&player_id, character_id, 0, attrs_with_cha(12), 0, None)
        .await;
    give_bag(game_state, &player_id, has_key.then_some(CRYPT_CHEST_KEY)).await;
    seat_on_deepest_floor(game_state, deepest, player_id).await;
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

    // Past sunset — a new night, so one more open (with a fresh key).
    game_state.debug_set_time(23, 0);
    assert!(game_state.give_item(&player_id, CRYPT_CHEST_KEY).await);
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    let after_second = game_state.get_player_gold(&player_id).await;
    assert!(
        after_second > after_first,
        "the chest should refill at nightfall"
    );

    // Same night again: still one open only, even with another key.
    game_state.debug_set_time(23, 30);
    assert!(game_state.give_item(&player_id, CRYPT_CHEST_KEY).await);
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    assert_eq!(
        game_state.get_player_gold(&player_id).await,
        after_second,
        "one open per night, not one per visit"
    );
}

/// The chest takes the deepest locked floor's key; a slain guardian is no
/// substitute (doc/DUNGEON_REWARD.md).
#[tokio::test]
async fn dungeon_chest_stays_shut_without_the_key() {
    let auth = make_test_auth("chest_no_key");
    let account = auth.login_npc("npc_chest_no_key").unwrap();
    let character = create_test_character(&auth, &account, "Keyless");

    let game_state = make_test_game_state("chest_no_key");
    let player_id = stage_opener(&game_state, "Keyless", character.id, false).await;

    let mut direct_rx = game_state.register_direct_channel(&player_id).await;
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;

    assert_chest_rejected(&mut direct_rx, "Crypt Key");
    assert_eq!(game_state.get_player_gold(&player_id).await, 0);
    assert!(
        auth.load_dungeon_history(character.id)
            .map(|h| h.0)
            .unwrap()
            .is_empty(),
        "a refused open must not consume the night's chest"
    );
}

/// A real open spends the key; the same-night empty re-open does not — so a
/// second key found later waits in the bag for tomorrow's chest.
#[tokio::test]
async fn dungeon_chest_spends_one_key_and_an_empty_reopen_keeps_it() {
    let auth = make_test_auth("chest_key_spent");
    let account = auth.login_npc("npc_chest_key_spent").unwrap();
    let character = create_test_character(&auth, &account, "Keyholder");

    let game_state = make_test_game_state("chest_key_spent");
    let player_id = stage_chest_opener(&game_state, "Keyholder", character.id).await;
    assert!(game_state.give_item(&player_id, CRYPT_CHEST_KEY).await);
    assert_eq!(bag_count(&game_state, &player_id, CRYPT_CHEST_KEY).await, 2);

    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    assert!(game_state.get_player_gold(&player_id).await > 0);
    assert_eq!(
        bag_count(&game_state, &player_id, CRYPT_CHEST_KEY).await,
        1,
        "one key per open, the stockpile stays"
    );

    let mut rx = game_state.register_direct_channel(&player_id).await;
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    assert_chest_empty_opened(&mut rx);
    assert_eq!(
        bag_count(&game_state, &player_id, CRYPT_CHEST_KEY).await,
        1,
        "an empty box costs nothing"
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

/// The key outlives leaving the floor — a party that steps out to rest can
/// come back for the chest.
#[tokio::test]
async fn chest_key_survives_leaving_the_floor() {
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
        "the chest paid out to its keyholder after a round trip"
    );
}

/// Sunset empties the dungeons: occupants surface at the entrance and the
/// guardian's slot is freed.
#[tokio::test(start_paused = true)]
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

    let mut rx = game_state.register_direct_channel(&player_id).await;
    let reset_state = game_state.clone();
    let reset = tokio::spawn(async move { advance_one_night(&reset_state).await });
    tokio::task::yield_now().await;
    assert!(
        drain(&mut rx)
            .iter()
            .any(|m| matches!(m, ServerMessage::DungeonReset)),
        "the delver hears the roar before being evicted"
    );

    tokio::time::advance(
        crate::game_state::dungeon::DUNGEON_RESET_WARNING_DURATION
            - std::time::Duration::from_millis(1),
    )
    .await;
    assert_eq!(
        game_state.players.read().await[&player_id].floor_level,
        -(deepest as i8),
        "the reset waits for the full roar"
    );
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    reset.await.expect("dungeon reset task");

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
        floor.slots.iter().all(|s| s.respawn_at_ms == 0),
        "the guardian rises with the new night"
    );
}

/// Someone descending while the sweep runs entered on the new night, so their
/// floor keeps its guardian rather than being freed under their feet — which
/// would orphan that floor's live monsters.
#[tokio::test(start_paused = true)]
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
    seat_on_deepest_floor(&game_state, deepest, player_id).await;

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
        floor.slots.iter().all(|s| s.respawn_at_ms == 0),
        "an emptied floor is reset like any other"
    );
}

/// Kills above a locked floor may carry its key; a killer who already has it
/// or has opened the chest tonight gets nothing (the key would be dead
/// weight), and the locked floor's own monsters never carry one.
#[tokio::test]
async fn dungeon_key_drops_only_where_they_are_useful() {
    let auth = make_test_auth("key_candidate");
    let account = auth.login_npc("npc_key_candidate").unwrap();
    let character = create_test_character(&auth, &account, "Hunter");

    let game_state = make_test_game_state("key_candidate");
    let player_id = stage_opener(&game_state, "Hunter", character.id, false).await;
    let total = crypt_chest_spot(&game_state).await.0;

    assert_eq!(
        game_state
            .dungeon_key_candidate(&player_id, CRYPT_ID, 1, total)
            .await
            .as_deref(),
        Some(CRYPT_CHEST_KEY)
    );
    assert_eq!(
        game_state
            .dungeon_key_candidate(&player_id, CRYPT_ID, total, total)
            .await,
        None,
        "the locked floor itself drops no key"
    );

    assert!(game_state.give_item(&player_id, CRYPT_CHEST_KEY).await);
    assert_eq!(
        game_state
            .dungeon_key_candidate(&player_id, CRYPT_ID, 1, total)
            .await,
        None,
        "one key in the bag is enough"
    );

    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    assert_eq!(bag_count(&game_state, &player_id, CRYPT_CHEST_KEY).await, 0);
    assert_eq!(
        game_state
            .dungeon_key_candidate(&player_id, CRYPT_ID, 1, total)
            .await,
        None,
        "tonight's chest is spent, so no key until the refill"
    );

    game_state.debug_set_time(23, 0);
    assert_eq!(
        game_state
            .dungeon_key_candidate(&player_id, CRYPT_ID, 1, total)
            .await
            .as_deref(),
        Some(CRYPT_CHEST_KEY),
        "a new night, a new chest, so keys drop again"
    );
}

/// On a locked floor the arrival room is sealed until the door is opened, so
/// its occupants are invisible to monster AI until then; on any floor a fresh
/// arrival gets `STAIR_ROOM_ARRIVAL_GRACE_MS` unseen in the stair room.
#[tokio::test]
async fn players_behind_a_shut_locked_door_are_unseen() {
    let game_state = make_test_game_state("sealed_stair_room");
    game_state.ensure_dungeon_runtime(CRYPT_ID).await;
    let entrance = game_state.dungeon_defs.get(CRYPT_ID).expect("old_crypt");
    let (depth, landing, door_id) = {
        let dungeons = game_state.dungeons.read().await;
        let rt = dungeons.get(CRYPT_ID).expect("crypt runtime");
        let depth = rt.layouts.len() as u8;
        let layout = rt.layouts.last().unwrap();
        let door_id = rt.locked_doors[depth as usize - 1][0];
        (
            depth,
            cell_center(&entrance.position(), depth, layout.up_shaft.exit_cell()),
            door_id,
        )
    };
    let arrival = (pid("arrival"), landing, -(depth as i8));
    let shallow = (pid("shallow"), landing, -1);

    let sealed = game_state
        .players_hidden_in_stair_rooms(&[arrival, shallow])
        .await;
    assert!(sealed.contains(&pid("arrival")), "shut door: unseen");
    assert!(!sealed.contains(&pid("shallow")), "floor 1 has no lock");

    // Floor 1: just arrived → unseen; a stale arrival → seen.
    let floor1_landing = {
        let dungeons = game_state.dungeons.read().await;
        let layout = &dungeons.get(CRYPT_ID).unwrap().layouts[0];
        cell_center(&entrance.position(), 1, layout.up_shaft.exit_cell())
    };
    game_state
        .handle_player_floor_change(&pid("shallow"), 0, -1, &floor1_landing, &floor1_landing)
        .await;
    let shallow = (pid("shallow"), floor1_landing, -1);
    let sealed = game_state.players_hidden_in_stair_rooms(&[shallow]).await;
    assert!(sealed.contains(&pid("shallow")), "fresh arrival: unseen");
    game_state
        .dungeons
        .write()
        .await
        .get_mut(CRYPT_ID)
        .unwrap()
        .floors
        .get_mut(&1)
        .unwrap()
        .players
        .insert(
            pid("shallow"),
            GameState::now_ms() - super::dungeon::STAIR_ROOM_ARRIVAL_GRACE_MS,
        );
    let sealed = game_state.players_hidden_in_stair_rooms(&[shallow]).await;
    assert!(sealed.is_empty(), "grace over: seen");

    game_state
        .dungeons
        .write()
        .await
        .get_mut(CRYPT_ID)
        .unwrap()
        .open_doors
        .entry(depth)
        .or_default()
        .insert(door_id);
    let sealed = game_state.players_hidden_in_stair_rooms(&[arrival]).await;
    assert!(sealed.is_empty(), "open door: seen");
}
