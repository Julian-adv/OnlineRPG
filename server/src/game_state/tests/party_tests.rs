use super::*;

fn drain(rx: &mut UnboundedReceiver<ServerMessage>) {
    while rx.try_recv().is_ok() {}
}

async fn add(game_state: &GameState, name: &str, x: f32) -> UnboundedReceiver<ServerMessage> {
    game_state.add_player(make_player(name, x, 0.0)).await;
    game_state.register_direct_channel(&pid(name)).await
}

async fn form_party(game_state: &GameState, leader: &str, member: &str) {
    game_state.invite_to_party(&pid(leader), member).await;
    game_state
        .respond_to_party_invite(&pid(member), &pid(leader), true)
        .await;
}

#[tokio::test]
async fn invite_and_accept_forms_party() {
    let game_state = make_test_game_state("party_form");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    // Far outside the AOI on purpose: invites are name-based like whisper.
    let mut bob_rx = add(&game_state, "bob", 500.0).await;

    game_state.invite_to_party(&pid("alice"), "bob").await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::PartyInviteReceived {
            inviter_id,
            inviter_name,
        }) => {
            assert_eq!(inviter_id, pid("alice"));
            assert_eq!(inviter_name, "alice");
        }
        other => panic!("Expected invite for bob, got {:?}", other),
    }
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("invited bob"), "{message}")
        }
        other => panic!("Expected ack for alice, got {:?}", other),
    }

    game_state
        .respond_to_party_invite(&pid("bob"), &pid("alice"), true)
        .await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyInviteResult { accepted, .. }) => assert!(accepted),
        other => panic!("Expected accepted result, got {:?}", other),
    }
    for rx in [&mut alice_rx, &mut bob_rx] {
        match rx.try_recv() {
            Ok(ServerMessage::PartyState { leader_id, members }) => {
                assert_eq!(leader_id, pid("alice"));
                let names: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
                assert_eq!(names, ["alice", "bob"]);
            }
            other => panic!("Expected party state, got {:?}", other),
        }
    }
}

#[tokio::test]
async fn invites_match_names_ignoring_ascii_case() {
    let game_state = make_test_game_state("party_ci_invite");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "Bob", 5.0).await;
    drain(&mut alice_rx);

    game_state.invite_to_party(&pid("alice"), "bob").await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::PartyInviteReceived { inviter_name, .. }) => {
            assert_eq!(inviter_name, "alice")
        }
        other => panic!("Expected case-insensitive invite, got {:?}", other),
    }
    // The ack echoes the canonical spelling, not the typed one.
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("invited Bob"), "{message}")
        }
        other => panic!("Expected ack, got {:?}", other),
    }
}

#[tokio::test]
async fn decline_reports_to_inviter_and_forms_nothing() {
    let game_state = make_test_game_state("party_decline");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;

    game_state.invite_to_party(&pid("alice"), "bob").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    game_state
        .respond_to_party_invite(&pid("bob"), &pid("alice"), false)
        .await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyInviteResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("declined"), "{message}");
        }
        other => panic!("Expected declined result, got {:?}", other),
    }
    assert!(matches!(bob_rx.try_recv(), Err(MpscTryRecvError::Empty)));
    // A declined invite cannot be accepted afterwards.
    game_state
        .respond_to_party_invite(&pid("bob"), &pid("alice"), true)
        .await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("expired"), "{message}")
        }
        other => panic!("Expected expired notice, got {:?}", other),
    }
}

#[tokio::test]
async fn respond_without_invite_is_expired() {
    let game_state = make_test_game_state("party_no_invite");
    let _alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;

    game_state
        .respond_to_party_invite(&pid("bob"), &pid("alice"), true)
        .await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("expired"), "{message}")
        }
        other => panic!("Expected expired notice, got {:?}", other),
    }
}

#[tokio::test]
async fn leader_leave_promotes_earliest_member() {
    let game_state = make_test_game_state("party_promote");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    let mut carol_rx = add(&game_state, "carol", 10.0).await;
    form_party(&game_state, "alice", "bob").await;
    form_party(&game_state, "alice", "carol").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    drain(&mut carol_rx);

    game_state.leave_party(&pid("alice")).await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyState { members, .. }) => assert!(members.is_empty()),
        other => panic!("Expected cleared state for alice, got {:?}", other),
    }
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("left"), "{message}")
        }
        other => panic!("Expected leave notice, got {:?}", other),
    }
    for rx in [&mut bob_rx, &mut carol_rx] {
        match rx.try_recv() {
            Ok(ServerMessage::PartyState { leader_id, members }) => {
                assert_eq!(leader_id, pid("bob"));
                let names: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
                assert_eq!(names, ["bob", "carol"]);
            }
            other => panic!("Expected promoted roster, got {:?}", other),
        }
    }
}

#[tokio::test]
async fn party_of_two_disbands_on_leave() {
    let game_state = make_test_game_state("party_disband");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    form_party(&game_state, "alice", "bob").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state.leave_party(&pid("bob")).await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::PartyState { members, .. }) => assert!(members.is_empty()),
        other => panic!("Expected cleared state for bob, got {:?}", other),
    }
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyState { members, .. }) => assert!(members.is_empty()),
        other => panic!("Expected cleared state for alice, got {:?}", other),
    }
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("disbanded"), "{message}")
        }
        other => panic!("Expected disband notice, got {:?}", other),
    }
}

#[tokio::test]
async fn disconnect_leaves_party() {
    let game_state = make_test_game_state("party_disconnect");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    let mut carol_rx = add(&game_state, "carol", 10.0).await;
    form_party(&game_state, "alice", "bob").await;
    form_party(&game_state, "alice", "carol").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    drain(&mut carol_rx);

    game_state.remove_player(&pid("bob")).await;
    for rx in [&mut alice_rx, &mut carol_rx] {
        let state = loop {
            match rx.try_recv() {
                Ok(ServerMessage::PartyState { leader_id, members }) => break (leader_id, members),
                Ok(_) => continue,
                Err(err) => panic!("Expected roster after disconnect, got {:?}", err),
            }
        };
        assert_eq!(state.0, pid("alice"));
        let names: Vec<&str> = state.1.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["alice", "carol"]);
    }
}

#[tokio::test]
async fn only_the_leader_invites() {
    let game_state = make_test_game_state("party_leader_only");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    let mut carol_rx = add(&game_state, "carol", 10.0).await;
    form_party(&game_state, "alice", "bob").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state.invite_to_party(&pid("bob"), "carol").await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::PartyInviteResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("leader"), "{message}");
        }
        other => panic!("Expected leader-only rejection, got {:?}", other),
    }
    assert!(matches!(carol_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn inviting_a_partied_player_gives_no_membership_oracle() {
    let game_state = make_test_game_state("party_taken");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    let mut dave_rx = add(&game_state, "dave", 10.0).await;
    form_party(&game_state, "alice", "bob").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    // Delivered like any other invite: an inviter-visible difference keyed
    // on bob's membership would let anyone poll who is grouped with whom.
    game_state.invite_to_party(&pid("dave"), "bob").await;
    match dave_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("invited bob"), "{message}")
        }
        other => panic!("Expected plain ack, got {:?}", other),
    }
    assert!(matches!(
        bob_rx.try_recv(),
        Ok(ServerMessage::PartyInviteReceived { .. })
    ));

    // The accept path sorts it out instead.
    game_state
        .respond_to_party_invite(&pid("bob"), &pid("dave"), true)
        .await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("already in a party"), "{message}")
        }
        other => panic!("Expected already-in-party notice, got {:?}", other),
    }
    match dave_rx.try_recv() {
        Ok(ServerMessage::PartyInviteResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("can't accept"), "{message}");
        }
        other => panic!("Expected neutral failure, got {:?}", other),
    }
}

#[tokio::test]
async fn cannot_invite_your_own_member() {
    let game_state = make_test_game_state("party_own_member");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    form_party(&game_state, "alice", "bob").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state.invite_to_party(&pid("alice"), "bob").await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyInviteResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("already in your party"), "{message}");
        }
        other => panic!("Expected own-member rejection, got {:?}", other),
    }
    assert!(matches!(bob_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn decline_does_not_reset_the_spam_brake() {
    let game_state = make_test_game_state("party_decline_brake");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state.invite_to_party(&pid("alice"), "bob").await;
    game_state
        .respond_to_party_invite(&pid("bob"), &pid("alice"), false)
        .await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    // Re-inviting right after the decline must not pop a fresh toast.
    game_state.invite_to_party(&pid("alice"), "bob").await;
    assert!(matches!(bob_rx.try_recv(), Err(MpscTryRecvError::Empty)));
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("invited bob"), "{message}")
        }
        other => panic!("Expected plain ack, got {:?}", other),
    }
}

#[tokio::test]
async fn official_npcs_cannot_be_invited() {
    let game_state = make_test_game_state("party_npc");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut npc = make_player("rica", 5.0, 0.0);
    npc.is_official_npc = true;
    game_state.add_player(npc).await;
    let mut npc_rx = game_state.register_direct_channel(&pid("rica")).await;
    drain(&mut alice_rx);

    game_state.invite_to_party(&pid("alice"), "rica").await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyInviteResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("NPC"), "{message}");
        }
        other => panic!("Expected NPC rejection, got {:?}", other),
    }
    assert!(matches!(npc_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn official_npcs_cannot_invite_either() {
    let game_state = make_test_game_state("party_npc_inviter");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut npc = make_player("karl", 5.0, 0.0);
    npc.is_official_npc = true;
    game_state.add_player(npc).await;
    let mut npc_rx = game_state.register_direct_channel(&pid("karl")).await;
    drain(&mut alice_rx);

    game_state.invite_to_party(&pid("karl"), "alice").await;
    match npc_rx.try_recv() {
        Ok(ServerMessage::PartyInviteResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("player travelers"), "{message}");
        }
        other => panic!("Expected NPC-inviter rejection, got {:?}", other),
    }
    assert!(matches!(alice_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn repeat_invite_is_acked_but_not_redelivered() {
    let game_state = make_test_game_state("party_repeat");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state.invite_to_party(&pid("alice"), "bob").await;
    game_state.invite_to_party(&pid("alice"), "bob").await;
    assert!(matches!(
        bob_rx.try_recv(),
        Ok(ServerMessage::PartyInviteReceived { .. })
    ));
    // The re-invite must not pop a second toast on bob's screen.
    assert!(matches!(bob_rx.try_recv(), Err(MpscTryRecvError::Empty)));
    for _ in 0..2 {
        match alice_rx.try_recv() {
            Ok(ServerMessage::SystemMessage { message }) => {
                assert!(message.contains("invited bob"), "{message}")
            }
            other => panic!("Expected ack, got {:?}", other),
        }
    }
}

#[tokio::test]
async fn pending_invites_are_capped() {
    let game_state = make_test_game_state("party_invite_cap");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    for i in 1..=6 {
        add(&game_state, &format!("target{i}"), i as f32).await;
    }
    drain(&mut alice_rx);

    for i in 1..=5 {
        game_state
            .invite_to_party(&pid("alice"), &format!("target{i}"))
            .await;
    }
    drain(&mut alice_rx);
    game_state.invite_to_party(&pid("alice"), "target6").await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyInviteResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("too many pending"), "{message}");
        }
        other => panic!("Expected pending-cap rejection, got {:?}", other),
    }
}

#[tokio::test]
async fn leaving_no_party_keeps_received_invites() {
    let game_state = make_test_game_state("party_leave_keeps_invite");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state.invite_to_party(&pid("alice"), "bob").await;
    game_state.leave_party(&pid("bob")).await;
    drain(&mut bob_rx);
    game_state
        .respond_to_party_invite(&pid("bob"), &pid("alice"), true)
        .await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::PartyState { members, .. }) => assert_eq!(members.len(), 2),
        other => panic!("Expected party formed after stray /leave, got {:?}", other),
    }
}

#[tokio::test]
async fn full_party_rejects_further_invites() {
    let game_state = make_test_game_state("party_full");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    for i in 2..=8 {
        let name = format!("member{i}");
        add(&game_state, &name, i as f32).await;
        form_party(&game_state, "alice", &name).await;
    }
    let mut extra_rx = add(&game_state, "extra", 50.0).await;
    drain(&mut alice_rx);

    game_state.invite_to_party(&pid("alice"), "extra").await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyInviteResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("full"), "{message}");
        }
        other => panic!("Expected full-party rejection, got {:?}", other),
    }
    assert!(matches!(extra_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn blocked_inviter_gets_a_silent_expiry() {
    let game_state = make_test_game_state("party_blocked");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    game_state
        .set_player_blocks(&pid("bob"), vec!["alice".to_string()])
        .await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state.invite_to_party(&pid("alice"), "bob").await;
    // The block is invisible: alice gets the normal ack, bob hears nothing.
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("invited bob"), "{message}")
        }
        other => panic!("Expected normal ack, got {:?}", other),
    }
    assert!(matches!(bob_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn self_and_unknown_invites_are_rejected() {
    let game_state = make_test_game_state("party_bad_targets");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;

    game_state.invite_to_party(&pid("alice"), "alice").await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyInviteResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("that's you"), "{message}");
        }
        other => panic!("Expected self-invite rejection, got {:?}", other),
    }

    game_state.invite_to_party(&pid("alice"), "nobody").await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyInviteResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("no one called"), "{message}");
        }
        other => panic!("Expected unknown-name rejection, got {:?}", other),
    }
}

#[tokio::test]
async fn party_chat_command_invites_and_reports() {
    let game_state = make_test_game_state("party_command");
    let auth = make_test_auth("party_command");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state
        .send_chat_message(&pid("alice"), "/party".to_string(), &auth)
        .await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("not in a party"), "{message}")
        }
        other => panic!("Expected empty-party status, got {:?}", other),
    }

    game_state
        .send_chat_message(&pid("alice"), "/party bob".to_string(), &auth)
        .await;
    assert!(matches!(
        bob_rx.try_recv(),
        Ok(ServerMessage::PartyInviteReceived { .. })
    ));
    drain(&mut alice_rx);
    game_state
        .respond_to_party_invite(&pid("bob"), &pid("alice"), true)
        .await;
    drain(&mut alice_rx);

    game_state
        .send_chat_message(&pid("alice"), "/party".to_string(), &auth)
        .await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("alice (leader)"), "{message}");
            assert!(message.contains("bob"), "{message}");
        }
        other => panic!("Expected roster status, got {:?}", other),
    }
}
