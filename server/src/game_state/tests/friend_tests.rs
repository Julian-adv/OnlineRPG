use super::*;
use crate::auth::{AuthService, CharacterRecord};
use onlinerpg_shared::messages::FriendEntry;

fn character(auth: &AuthService, name: &str) -> CharacterRecord {
    let account = auth.login_npc(&format!("npc_{name}")).unwrap();
    create_test_character(auth, &account, name)
}

/// Enter the game as `record`, seeded from the DB exactly like the login path.
async fn enter(
    game_state: &GameState,
    auth: &AuthService,
    record: &CharacterRecord,
    x: f32,
) -> DirectRx {
    let name = record.name.as_str();
    game_state.add_player(make_player(name, x, 0.0)).await;
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
    let friends = auth.load_friends(record.id).unwrap();
    game_state.set_player_friends(&pid(name), friends).await;
    game_state.register_direct_channel(&pid(name)).await
}

/// The names on the newest `FriendList` in a batch, or None if it holds none.
fn friend_names(msgs: &[ServerMessage]) -> Option<Vec<String>> {
    msgs.iter().rev().find_map(|msg| match msg {
        ServerMessage::FriendList { friends } => {
            Some(friends.iter().map(|f| f.name.clone()).collect())
        }
        _ => None,
    })
}

fn system_lines(msgs: &[ServerMessage]) -> Vec<String> {
    msgs.iter()
        .filter_map(|msg| match msg {
            ServerMessage::SystemMessage { message } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

fn has_line(msgs: &[ServerMessage], needle: &str) -> bool {
    system_lines(msgs).iter().any(|line| line.contains(needle))
}

async fn befriend(game_state: &GameState, auth: &AuthService, a: &str, b: &str) {
    game_state.request_friend(&pid(a), b, auth).await;
    game_state
        .respond_to_friend_request(&pid(b), &pid(a), true, auth)
        .await;
}

#[tokio::test]
async fn request_and_accept_forms_friendship_on_both_sides() {
    let game_state = make_test_game_state("friend_form");
    let auth = make_test_auth("friend_form");
    let alice = character(&auth, "alice");
    let bob = character(&auth, "bob");
    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    // Far outside the AOI on purpose: requests are name-based like whispers.
    let mut bob_rx = enter(&game_state, &auth, &bob, 500.0).await;

    game_state.request_friend(&pid("alice"), "bob", &auth).await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::FriendRequestReceived {
            requester_id,
            requester_name,
        }) => {
            assert_eq!(requester_id, pid("alice"));
            assert_eq!(requester_name, "alice");
        }
        other => panic!("Expected a request for bob, got {:?}", other),
    }
    assert!(has_line(&drain(&mut alice_rx), "asked bob"));

    game_state
        .respond_to_friend_request(&pid("bob"), &pid("alice"), true, &auth)
        .await;

    assert_eq!(
        friend_names(&drain(&mut alice_rx)).as_deref(),
        Some(["bob".to_string()].as_slice())
    );
    assert_eq!(
        friend_names(&drain(&mut bob_rx)).as_deref(),
        Some(["alice".to_string()].as_slice())
    );
    // Both rows, so either side's next login loads the friendship.
    assert_eq!(
        auth.load_friends(alice.id).unwrap(),
        vec![FriendEntry {
            character_id: bob.id,
            name: "bob".to_string(),
            level: 1,
            class: bob.class.clone(),
        }]
    );
    assert_eq!(
        auth.load_friends(bob.id).unwrap(),
        vec![FriendEntry {
            character_id: alice.id,
            name: "alice".to_string(),
            level: 1,
            class: alice.class.clone(),
        }]
    );
}

#[tokio::test]
async fn reciprocal_requests_form_friendship_with_no_answer() {
    let game_state = make_test_game_state("friend_reciprocal");
    let auth = make_test_auth("friend_reciprocal");
    let alice = character(&auth, "alice");
    let bob = character(&auth, "bob");
    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    let mut bob_rx = enter(&game_state, &auth, &bob, 5.0).await;

    game_state.request_friend(&pid("alice"), "bob", &auth).await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    // Bob asks back instead of answering: both sides have said yes already.
    game_state.request_friend(&pid("bob"), "alice", &auth).await;

    let bob_msgs = drain(&mut bob_rx);
    assert_eq!(
        friend_names(&bob_msgs).as_deref(),
        Some(["alice".to_string()].as_slice())
    );
    assert!(has_line(&bob_msgs, "now friends"));
    assert_eq!(
        friend_names(&drain(&mut alice_rx)).as_deref(),
        Some(["bob".to_string()].as_slice())
    );
    // No leftover request: answering one afterwards must find nothing.
    game_state
        .respond_to_friend_request(&pid("bob"), &pid("alice"), true, &auth)
        .await;
    assert!(has_line(&drain(&mut bob_rx), "expired"));
}

#[tokio::test]
async fn declining_reports_it_and_cannot_be_undone() {
    let game_state = make_test_game_state("friend_decline");
    let auth = make_test_auth("friend_decline");
    let alice = character(&auth, "alice");
    let bob = character(&auth, "bob");
    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    let mut bob_rx = enter(&game_state, &auth, &bob, 5.0).await;

    game_state.request_friend(&pid("alice"), "bob", &auth).await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    game_state
        .respond_to_friend_request(&pid("bob"), &pid("alice"), false, &auth)
        .await;
    assert!(has_line(&drain(&mut alice_rx), "declined"));

    // The declined entry survives until it expires, so a change of heart
    // cannot mint a friendship the requester never re-offered.
    game_state
        .respond_to_friend_request(&pid("bob"), &pid("alice"), true, &auth)
        .await;
    assert!(has_line(&drain(&mut bob_rx), "expired"));
    assert!(auth.load_friends(alice.id).unwrap().is_empty());
}

#[tokio::test]
async fn re_asking_after_a_decline_says_so_instead_of_acking_nothing() {
    let game_state = make_test_game_state("friend_redecline");
    let auth = make_test_auth("friend_redecline");
    let alice = character(&auth, "alice");
    let bob = character(&auth, "bob");
    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    let mut bob_rx = enter(&game_state, &auth, &bob, 5.0).await;

    game_state.request_friend(&pid("alice"), "bob", &auth).await;
    game_state
        .respond_to_friend_request(&pid("bob"), &pid("alice"), false, &auth)
        .await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state.request_friend(&pid("alice"), "bob", &auth).await;

    // The decline still suppresses the toast for its window — but alice is
    // told that, rather than being acked as though bob would see it.
    let alice_msgs = drain(&mut alice_rx);
    assert!(has_line(&alice_msgs, "declined"), "{alice_msgs:?}");
    assert!(!has_line(&alice_msgs, "asked bob"), "{alice_msgs:?}");
    assert!(matches!(bob_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn re_asking_while_still_unanswered_acks_without_a_second_toast() {
    let game_state = make_test_game_state("friend_reask");
    let auth = make_test_auth("friend_reask");
    let alice = character(&auth, "alice");
    let bob = character(&auth, "bob");
    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    let mut bob_rx = enter(&game_state, &auth, &bob, 5.0).await;

    game_state.request_friend(&pid("alice"), "bob", &auth).await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    game_state.request_friend(&pid("alice"), "bob", &auth).await;

    // Bob's toast is already up: re-acking is honest, re-delivering would let
    // alice re-pop it at will.
    assert!(has_line(&drain(&mut alice_rx), "asked bob"));
    assert!(matches!(bob_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn removing_a_friend_drops_both_sides() {
    let game_state = make_test_game_state("friend_remove");
    let auth = make_test_auth("friend_remove");
    let alice = character(&auth, "alice");
    let bob = character(&auth, "bob");
    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    let mut bob_rx = enter(&game_state, &auth, &bob, 5.0).await;
    befriend(&game_state, &auth, "alice", "bob").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    // Typed spelling need not match the stored one.
    game_state
        .remove_friend_by_name(&pid("alice"), "BOB", &auth)
        .await;

    let alice_msgs = drain(&mut alice_rx);
    assert_eq!(friend_names(&alice_msgs).as_deref(), Some([].as_slice()));
    assert!(has_line(&alice_msgs, "no longer your friend"));
    // Bob's live roster is corrected too, but he is told nothing.
    let bob_msgs = drain(&mut bob_rx);
    assert_eq!(friend_names(&bob_msgs).as_deref(), Some([].as_slice()));
    assert!(system_lines(&bob_msgs).is_empty(), "{bob_msgs:?}");
    assert!(auth.load_friends(alice.id).unwrap().is_empty());
    assert!(auth.load_friends(bob.id).unwrap().is_empty());
}

#[tokio::test]
async fn blocking_a_friend_ends_the_friendship() {
    let game_state = make_test_game_state("friend_block");
    let auth = make_test_auth("friend_block");
    let alice = character(&auth, "alice");
    let bob = character(&auth, "bob");
    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    let mut bob_rx = enter(&game_state, &auth, &bob, 5.0).await;
    befriend(&game_state, &auth, "alice", "bob").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state
        .send_chat_message(&pid("alice"), "/block bob".to_string(), &auth)
        .await;

    let alice_msgs = drain(&mut alice_rx);
    assert!(has_line(&alice_msgs, "no longer your friend"));
    assert_eq!(friend_names(&alice_msgs).as_deref(), Some([].as_slice()));
    assert_eq!(
        friend_names(&drain(&mut bob_rx)).as_deref(),
        Some([].as_slice())
    );
    assert!(auth.load_friends(alice.id).unwrap().is_empty());
    assert!(auth.load_friends(bob.id).unwrap().is_empty());
}

#[tokio::test]
async fn requesting_someone_you_blocked_is_refused() {
    let game_state = make_test_game_state("friend_block_refuse");
    let auth = make_test_auth("friend_block_refuse");
    let alice = character(&auth, "alice");
    let bob = character(&auth, "bob");
    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    let mut bob_rx = enter(&game_state, &auth, &bob, 5.0).await;
    game_state
        .set_player_blocks(&pid("alice"), vec!["bob".to_string()])
        .await;
    drain(&mut alice_rx);

    game_state.request_friend(&pid("alice"), "bob", &auth).await;

    assert!(has_line(&drain(&mut alice_rx), "/unblock bob"));
    assert!(matches!(bob_rx.try_recv(), Err(MpscTryRecvError::Empty)));
    assert!(auth.load_friends(alice.id).unwrap().is_empty());
}

#[tokio::test]
async fn a_blocked_requester_gets_the_normal_ack_and_no_toast() {
    let game_state = make_test_game_state("friend_block_suppress");
    let auth = make_test_auth("friend_block_suppress");
    let alice = character(&auth, "alice");
    let bob = character(&auth, "bob");
    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    let mut bob_rx = enter(&game_state, &auth, &bob, 5.0).await;
    game_state
        .set_player_blocks(&pid("bob"), vec!["alice".to_string()])
        .await;

    game_state.request_friend(&pid("alice"), "bob", &auth).await;

    // The ack is word-for-word the unblocked one: any difference would let
    // alice detect the block.
    assert!(has_line(&drain(&mut alice_rx), "asked bob"));
    assert!(matches!(bob_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn requests_are_capped_and_a_full_list_refuses() {
    let game_state = make_test_game_state("friend_caps");
    let auth = make_test_auth("friend_caps");
    let alice = character(&auth, "alice");
    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;

    // Six targets: the sixth request exceeds the pending cap.
    let mut targets = Vec::new();
    for index in 0..6 {
        let name = format!("target{index}");
        let record = character(&auth, &name);
        enter(&game_state, &auth, &record, 10.0 + index as f32).await;
        targets.push(name);
    }
    for name in &targets[..5] {
        game_state.request_friend(&pid("alice"), name, &auth).await;
    }
    drain(&mut alice_rx);
    game_state
        .request_friend(&pid("alice"), &targets[5], &auth)
        .await;
    assert!(has_line(&drain(&mut alice_rx), "too many pending requests"));

    // A full list refuses before the request is even recorded. Seeded
    // directly: what is under test is the cap, not 100 more DB rows.
    let filler = (0..super::super::friends::MAX_FRIENDS as i64)
        .map(|i| FriendEntry {
            character_id: 10_000 + i,
            name: format!("filler{i}"),
            level: 1,
            class: CharacterClass::Knight,
        })
        .collect();
    game_state.set_player_friends(&pid("alice"), filler).await;
    game_state
        .request_friend(&pid("alice"), &targets[0], &auth)
        .await;
    assert!(has_line(&drain(&mut alice_rx), "friend list is full"));
}

#[tokio::test]
async fn a_logout_voids_pending_requests_in_both_directions() {
    let game_state = make_test_game_state("friend_logout");
    let auth = make_test_auth("friend_logout");
    let alice = character(&auth, "alice");
    let bob = character(&auth, "bob");
    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    let mut bob_rx = enter(&game_state, &auth, &bob, 5.0).await;

    game_state.request_friend(&pid("alice"), "bob", &auth).await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    game_state.unregister_player_character(&pid("bob")).await;
    let mut bob_rx = {
        drop(bob_rx);
        enter(&game_state, &auth, &bob, 5.0).await
    };

    game_state
        .respond_to_friend_request(&pid("bob"), &pid("alice"), true, &auth)
        .await;
    assert!(has_line(&drain(&mut bob_rx), "expired"));
    assert!(auth.load_friends(alice.id).unwrap().is_empty());
}

#[tokio::test]
async fn friendships_are_reloaded_on_the_next_login() {
    let game_state = make_test_game_state("friend_relog");
    let auth = make_test_auth("friend_relog");
    let alice = character(&auth, "alice");
    let bob = character(&auth, "bob");
    let alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    let bob_rx = enter(&game_state, &auth, &bob, 5.0).await;
    befriend(&game_state, &auth, "alice", "bob").await;
    drop(alice_rx);
    drop(bob_rx);

    game_state.unregister_player_character(&pid("alice")).await;
    game_state.remove_player(&pid("alice")).await;
    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;

    game_state.send_friend_list(&pid("alice")).await;
    assert_eq!(
        friend_names(&drain(&mut alice_rx)).as_deref(),
        Some(["bob".to_string()].as_slice())
    );
}

#[tokio::test]
async fn presence_lists_only_the_friends_who_are_online() {
    let game_state = make_test_game_state("friend_presence");
    let auth = make_test_auth("friend_presence");
    let alice = character(&auth, "alice");
    let bob = character(&auth, "bob");
    let carol = character(&auth, "carol");
    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    let bob_rx = enter(&game_state, &auth, &bob, 5.0).await;
    let carol_rx = enter(&game_state, &auth, &carol, 9.0).await;
    befriend(&game_state, &auth, "alice", "bob").await;
    befriend(&game_state, &auth, "alice", "carol").await;
    drop(bob_rx);
    drop(carol_rx);

    game_state.unregister_player_character(&pid("carol")).await;
    game_state.remove_player(&pid("carol")).await;
    drain(&mut alice_rx);
    game_state.send_friends_online(&pid("alice")).await;

    match drain(&mut alice_rx).as_slice() {
        [ServerMessage::FriendsOnline { friends }] => {
            assert_eq!(
                friends
                    .iter()
                    .map(|f| (f.character_id, f.level))
                    .collect::<Vec<_>>(),
                vec![(bob.id, 1)]
            );
        }
        other => panic!("Expected one presence answer, got {:?}", other),
    }
}

#[tokio::test]
async fn npcs_are_out_of_the_friend_system() {
    let game_state = make_test_game_state("friend_npc");
    let auth = make_test_auth("friend_npc");
    let alice = character(&auth, "alice");
    let mut alice_rx = enter(&game_state, &auth, &alice, 0.0).await;
    let mut npc = make_player("rica", 5.0, 0.0);
    npc.is_official_npc = true;
    game_state.add_player(npc).await;

    game_state
        .request_friend(&pid("alice"), "rica", &auth)
        .await;

    assert!(has_line(&drain(&mut alice_rx), "is an NPC"));
    assert!(auth.load_friends(alice.id).unwrap().is_empty());
}
