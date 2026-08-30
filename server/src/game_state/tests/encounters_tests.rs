use super::*;
use crate::auth::{AuthService, CharacterRecord};
use onlinerpg_shared::messages::EncounterEntry;

fn character(auth: &AuthService, name: &str) -> CharacterRecord {
    let account = auth.login_npc(&format!("npc_{name}")).unwrap();
    create_test_character(auth, &account, name)
}

/// Enter the game as `record` at `x`, in the login path's order: character
/// registered (and any saved encounters seeded) before `add_player`, which
/// is what lets join-time meetings resolve the joiner's character id.
async fn enter(
    game_state: &GameState,
    auth: &AuthService,
    record: &CharacterRecord,
    x: f32,
) -> DirectRx {
    let name = record.name.as_str();
    game_state
        .register_player_character(
            &pid(name),
            record.id,
            record.xp,
            attrs_with_cha(12),
            record.gold,
            None,
        )
        .await;
    let saved = auth.load_encounters(record.id).unwrap();
    game_state.set_player_encounters(&pid(name), saved).await;
    game_state.add_player(make_player(name, x, 0.0)).await;
    game_state.register_direct_channel(&pid(name)).await
}

/// Request the list and return the entries of the answering message.
async fn encounter_list(
    game_state: &GameState,
    rx: &mut DirectRx,
    who: &str,
) -> Vec<EncounterEntry> {
    drain(rx);
    game_state.send_recent_encounters(&pid(who)).await;
    match drain(rx).as_slice() {
        [ServerMessage::RecentEncounters { entries }] => entries.clone(),
        other => panic!("expected one RecentEncounters, got {other:?}"),
    }
}

fn names(entries: &[EncounterEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.name.as_str()).collect()
}

#[tokio::test]
async fn joining_in_sight_is_a_meeting_for_both_sides() {
    let game_state = make_test_game_state("enc_join");
    let auth = make_test_auth("enc_join");
    let alice = character(&auth, "Alice");
    let bob = character(&auth, "Bob");

    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    let mut bob_rx = enter(&game_state, &auth, &bob, 5.0).await;

    let seen_by_alice = encounter_list(&game_state, &mut alice_rx, "Alice").await;
    assert_eq!(names(&seen_by_alice), ["Bob"]);
    assert_eq!(seen_by_alice[0].character_id, bob.id);
    assert_eq!(seen_by_alice[0].met_count, 1);
    assert_eq!(
        names(&encounter_list(&game_state, &mut bob_rx, "Bob").await),
        ["Alice"]
    );
}

#[tokio::test]
async fn walking_into_sight_records_and_the_list_is_newest_first() {
    let game_state = make_test_game_state("enc_walk");
    let auth = make_test_auth("enc_walk");
    let alice = character(&auth, "Alice");
    let bob = character(&auth, "Bob");
    let carol = character(&auth, "Carol");

    let far = EVENT_DELIVERY_RADIUS * 3.0;
    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    let _bob_rx = enter(&game_state, &auth, &bob, far).await;
    let _carol_rx = enter(&game_state, &auth, &carol, far * 2.0).await;
    assert!(
        encounter_list(&game_state, &mut alice_rx, "Alice")
            .await
            .is_empty(),
        "out-of-sight joins are not meetings"
    );

    // Walking through both: Bob first, Carol second → Carol newest.
    walk_player_to(&game_state, &pid("Alice"), far, 0.0).await;
    walk_player_to(&game_state, &pid("Alice"), far * 2.0, 0.0).await;
    let seen = encounter_list(&game_state, &mut alice_rx, "Alice").await;
    assert_eq!(names(&seen), ["Carol", "Bob"]);
}

#[tokio::test]
async fn official_npcs_neither_remember_nor_are_remembered() {
    let game_state = make_test_game_state("enc_npc");
    let auth = make_test_auth("enc_npc");
    let alice = character(&auth, "Alice");
    let merchant = character(&auth, "Merchant");

    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    game_state
        .register_player_character(
            &pid("Merchant"),
            merchant.id,
            merchant.xp,
            attrs_with_cha(12),
            merchant.gold,
            None,
        )
        .await;
    let mut npc = make_player("Merchant", 5.0, 0.0);
    npc.is_official_npc = true;
    game_state.add_player(npc).await;
    let mut npc_rx = game_state.register_direct_channel(&pid("Merchant")).await;

    assert!(encounter_list(&game_state, &mut alice_rx, "Alice")
        .await
        .is_empty());
    assert!(encounter_list(&game_state, &mut npc_rx, "Merchant")
        .await
        .is_empty());
}

#[tokio::test]
async fn logout_persists_and_relogin_restores_the_list() {
    let game_state = make_test_game_state("enc_persist");
    let auth = make_test_auth("enc_persist");
    let alice = character(&auth, "Alice");
    let bob = character(&auth, "Bob");

    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    let _bob_rx = enter(&game_state, &auth, &bob, 5.0).await;

    // Alice logs out through the real teardown order: persist first, then
    // unregister — take_player_encounters needs the character id.
    game_state
        .persist_and_detach_player(&pid("Alice"), &auth)
        .await;
    game_state.unregister_player_character(&pid("Alice")).await;
    game_state.remove_player(&pid("Alice")).await;
    drain(&mut alice_rx);

    let saved = auth.load_encounters(alice.id).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].character_id, bob.id);

    // Re-login far from Bob: the list comes back from the DB, not from a
    // new meeting.
    let far = EVENT_DELIVERY_RADIUS * 3.0;
    let mut alice_rx = enter(&game_state, &auth, &alice, far).await;
    let seen = encounter_list(&game_state, &mut alice_rx, "Alice").await;
    assert_eq!(names(&seen), ["Bob"]);
    assert_eq!(seen[0].met_count, 1);
}
