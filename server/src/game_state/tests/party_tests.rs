use super::*;

fn drain(rx: &mut DirectRx) {
    while rx.try_recv().is_ok() {}
}

async fn add(game_state: &GameState, name: &str, x: f32) -> DirectRx {
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
    for i in 2..=crate::game_state::party::PARTY_MAX_MEMBERS {
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
async fn positions_cover_members_beyond_aoi_including_self() {
    let game_state = make_test_game_state("party_positions");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    // Far outside the AOI on purpose: distance must not filter positions.
    let mut bob_rx = add(&game_state, "bob", 500.0).await;
    // In a party of their own: never visible to alice.
    add(&game_state, "carol", 10.0).await;
    add(&game_state, "dave", 15.0).await;
    form_party(&game_state, "alice", "bob").await;
    form_party(&game_state, "carol", "dave").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state.send_party_positions(&pid("alice")).await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyPositions { members }) => {
            // The payload is party-wide, requester included (clients filter
            // themselves), and never leaks other parties.
            let ids: Vec<_> = members.iter().map(|m| m.id).collect();
            assert_eq!(ids, [pid("alice"), pid("bob")]);
            assert_eq!(members[1].x, 500.0);
            assert_eq!(members[1].floor_level, 0);
        }
        other => panic!("Expected positions, got {:?}", other),
    }
    assert!(matches!(bob_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn position_tick_pushes_a_relocated_party_to_every_member() {
    let game_state = make_test_game_state("party_positions_tick");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 500.0).await;
    form_party(&game_state, "alice", "bob").await;
    // Flush the formation-queued push; an unchanged party then stays silent.
    game_state.tick_party_positions().await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    game_state.tick_party_positions().await;
    assert!(matches!(alice_rx.try_recv(), Err(MpscTryRecvError::Empty)));
    assert!(matches!(bob_rx.try_recv(), Err(MpscTryRecvError::Empty)));

    game_state
        .teleport_player(&pid("bob"), pos(40.0), 0.0, 0)
        .await;
    // Clear the teleport's own AOI fanout; the push comes from the tick.
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    game_state.tick_party_positions().await;
    for rx in [&mut alice_rx, &mut bob_rx] {
        match rx.try_recv() {
            Ok(ServerMessage::PartyPositions { members }) => {
                assert_eq!(members.len(), 2);
                let bob = members.iter().find(|m| m.id == pid("bob")).unwrap();
                assert_eq!(bob.x, 40.0);
            }
            other => panic!("Expected pushed positions, got {:?}", other),
        }
    }
}

#[tokio::test]
async fn position_tick_pushes_only_the_relocated_party() {
    let game_state = make_test_game_state("party_positions_tick_isolation");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 500.0).await;
    let mut carol_rx = add(&game_state, "carol", 10.0).await;
    let mut dave_rx = add(&game_state, "dave", 15.0).await;
    form_party(&game_state, "alice", "bob").await;
    form_party(&game_state, "carol", "dave").await;
    // Flush both formation pushes.
    game_state.tick_party_positions().await;
    for rx in [&mut alice_rx, &mut bob_rx, &mut carol_rx, &mut dave_rx] {
        drain(rx);
    }

    game_state
        .teleport_player(&pid("bob"), pos(40.0), 0.0, 0)
        .await;
    for rx in [&mut alice_rx, &mut bob_rx, &mut carol_rx, &mut dave_rx] {
        drain(rx);
    }
    game_state.tick_party_positions().await;
    // The relocated party gets exactly its own roster…
    for rx in [&mut alice_rx, &mut bob_rx] {
        match rx.try_recv() {
            Ok(ServerMessage::PartyPositions { members }) => {
                let ids: Vec<_> = members.iter().map(|m| m.id).collect();
                assert_eq!(ids, [pid("alice"), pid("bob")]);
            }
            other => panic!("Expected isolated push, got {:?}", other),
        }
    }
    // …and the unmoved party hears nothing.
    assert!(matches!(carol_rx.try_recv(), Err(MpscTryRecvError::Empty)));
    assert!(matches!(dave_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn leave_queues_a_position_push_without_the_leaver() {
    let game_state = make_test_game_state("party_positions_leave_push");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    add(&game_state, "carol", 10.0).await;
    form_party(&game_state, "alice", "bob").await;
    game_state.invite_to_party(&pid("alice"), "carol").await;
    game_state
        .respond_to_party_invite(&pid("carol"), &pid("alice"), true)
        .await;
    game_state.tick_party_positions().await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state.leave_party(&pid("carol")).await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    game_state.tick_party_positions().await;
    for rx in [&mut alice_rx, &mut bob_rx] {
        match rx.try_recv() {
            Ok(ServerMessage::PartyPositions { members }) => {
                let ids: Vec<_> = members.iter().map(|m| m.id).collect();
                assert_eq!(ids, [pid("alice"), pid("bob")]);
            }
            other => panic!("Expected post-leave push, got {:?}", other),
        }
    }
}

#[tokio::test]
async fn floor_only_relocation_queues_a_push() {
    let game_state = make_test_game_state("party_positions_floor_push");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    add(&game_state, "bob", 5.0).await;
    form_party(&game_state, "alice", "bob").await;
    game_state.tick_party_positions().await;
    drain(&mut alice_rx);

    // Same x/z, new floor: descending stairs must re-push (markers draw
    // x/z and floor).
    game_state
        .teleport_player(&pid("bob"), pos(5.0), 0.0, -1)
        .await;
    drain(&mut alice_rx);
    game_state.tick_party_positions().await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyPositions { members }) => {
            let bob = members.iter().find(|m| m.id == pid("bob")).unwrap();
            assert_eq!(bob.floor_level, -1);
        }
        other => panic!("Expected floor-change push, got {:?}", other),
    }
}

#[tokio::test]
async fn z_only_relocation_queues_a_push() {
    let game_state = make_test_game_state("party_positions_z_push");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    add(&game_state, "bob", 5.0).await;
    form_party(&game_state, "alice", "bob").await;
    game_state.tick_party_positions().await;
    drain(&mut alice_rx);

    // Due-north movement: x and floor unchanged, only z moves.
    game_state
        .teleport_player(
            &pid("bob"),
            Position {
                x: 5.0,
                y: 0.0,
                z: 30.0,
            },
            0.0,
            0,
        )
        .await;
    drain(&mut alice_rx);
    game_state.tick_party_positions().await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyPositions { members }) => {
            let bob = members.iter().find(|m| m.id == pid("bob")).unwrap();
            assert_eq!(bob.z, 30.0);
        }
        other => panic!("Expected z-change push, got {:?}", other),
    }
}

#[tokio::test]
async fn rotation_only_updates_do_not_queue_a_push() {
    let game_state = make_test_game_state("party_positions_rotation");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    add(&game_state, "bob", 5.0).await;
    form_party(&game_state, "alice", "bob").await;
    game_state.tick_party_positions().await;
    drain(&mut alice_rx);

    // Turning in place (a fishing cast facing, e.g.) must not re-push.
    game_state
        .teleport_player(&pid("bob"), pos(5.0), 1.0, 0)
        .await;
    // Clear the teleport's own AOI fanout; only the tick sends positions.
    drain(&mut alice_rx);
    game_state.tick_party_positions().await;
    assert!(matches!(alice_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn position_tick_ignores_partyless_movers() {
    let game_state = make_test_game_state("party_positions_solo");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    game_state
        .teleport_player(&pid("alice"), pos(10.0), 0.0, 0)
        .await;
    drain(&mut alice_rx);
    game_state.tick_party_positions().await;
    assert!(matches!(alice_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn party_formation_queues_a_position_push() {
    let game_state = make_test_game_state("party_positions_form_push");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    add(&game_state, "bob", 500.0).await;
    form_party(&game_state, "alice", "bob").await;
    drain(&mut alice_rx);

    game_state.tick_party_positions().await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyPositions { members }) => {
            let ids: Vec<_> = members.iter().map(|m| m.id).collect();
            assert_eq!(ids, [pid("alice"), pid("bob")]);
        }
        other => panic!("Expected formation push, got {:?}", other),
    }
}

#[tokio::test]
async fn positions_without_party_are_empty() {
    let game_state = make_test_game_state("party_positions_none");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    drain(&mut alice_rx);

    game_state.send_party_positions(&pid("alice")).await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyPositions { members }) => assert!(members.is_empty()),
        other => panic!("Expected empty positions, got {:?}", other),
    }
}

#[tokio::test]
async fn positions_report_dungeon_floor() {
    let game_state = make_test_game_state("party_positions_floor");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    add(&game_state, "bob", 5.0).await;
    form_party(&game_state, "alice", "bob").await;
    game_state
        .players
        .write()
        .await
        .get_mut(&pid("bob"))
        .unwrap()
        .floor_level = -2;
    drain(&mut alice_rx);

    game_state.send_party_positions(&pid("alice")).await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyPositions { members }) => {
            let bob = members.iter().find(|m| m.id == pid("bob")).unwrap();
            assert_eq!(bob.floor_level, -2);
        }
        other => panic!("Expected positions, got {:?}", other),
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

// --- Party summoning scrolls ---

/// Put a stack of party summon scrolls (instance 9) in `name`'s bag.
async fn give_summon_scrolls(game_state: &GameState, name: &str, quantity: u32) {
    let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
    inv.bag
        .push(bag_item(9, "scroll_of_party_summon", quantity));
    game_state.inventories.write().await.insert(pid(name), inv);
}

async fn give_summon_scroll(game_state: &GameState, name: &str) {
    give_summon_scrolls(game_state, name, 1).await;
}

/// The remaining quantity of `name`'s summon scroll stack (0 = gone).
async fn summon_scrolls_left(game_state: &GameState, name: &str) -> u32 {
    game_state
        .get_player_inventory(&pid(name))
        .await
        .unwrap()
        .bag
        .iter()
        .find(|i| i.instance_id == 9)
        .map_or(0, |i| i.quantity)
}

/// Poke `name`'s combat clock (`GameState::now_ms()` = in combat, 0 = at peace).
async fn set_combat_at(game_state: &GameState, name: &str, at: u64) {
    game_state
        .players
        .write()
        .await
        .get_mut(&pid(name))
        .unwrap()
        .last_combat_at = at;
}

async fn set_health(game_state: &GameState, name: &str, health: u32) {
    game_state
        .players
        .write()
        .await
        .get_mut(&pid(name))
        .unwrap()
        .health = health;
}

/// `name`'s current x coordinate.
async fn player_x(game_state: &GameState, name: &str) -> f32 {
    player_xz(game_state, &pid(name)).await.0
}

#[tokio::test]
async fn summon_scroll_kept_without_party() {
    let game_state = make_test_game_state("summon_no_party");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    give_summon_scroll(&game_state, "alice").await;

    game_state.use_item(&pid("alice"), 9).await;

    let inv = game_state
        .get_player_inventory(&pid("alice"))
        .await
        .unwrap();
    assert_eq!(inv.bag.len(), 1, "the scroll should be kept");
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("no party members"), "{message}")
        }
        other => panic!("Expected a system reply, got {:?}", other),
    }
}

#[tokio::test]
async fn summon_accept_teleports_to_casters_side() {
    let game_state = make_test_game_state("summon_accept");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 500.0).await;
    form_party(&game_state, "alice", "bob").await;
    give_summon_scroll(&game_state, "alice").await;
    // The caster waits on a dungeon floor: the arrival must match it.
    game_state
        .players
        .write()
        .await
        .get_mut(&pid("alice"))
        .unwrap()
        .floor_level = -2;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state.use_item(&pid("alice"), 9).await;
    let inv = game_state
        .get_player_inventory(&pid("alice"))
        .await
        .unwrap();
    assert!(inv.bag.is_empty(), "the scroll should be consumed");
    match bob_rx.try_recv() {
        Ok(ServerMessage::PartySummonReceived {
            caster_id,
            caster_name,
        }) => {
            assert_eq!(caster_id, pid("alice"));
            assert_eq!(caster_name, "alice");
        }
        other => panic!("Expected summon for bob, got {:?}", other),
    }

    game_state
        .respond_to_party_summon(&pid("bob"), &pid("alice"), true)
        .await;
    let players = game_state.players.read().await;
    let alice = players.get(&pid("alice")).unwrap();
    let bob = players.get(&pid("bob")).unwrap();
    let (dx, dz) = (
        bob.position.x - alice.position.x,
        bob.position.z - alice.position.z,
    );
    let dist = (dx * dx + dz * dz).sqrt();
    assert!(dist <= 2.0, "bob should arrive beside alice, {dist}m away");
    assert!(dist > 0.5, "arrivals should not stack on the caster");
    assert_eq!(bob.floor_level, -2);
}

#[tokio::test]
async fn summon_scroll_kept_while_caster_in_combat() {
    let game_state = make_test_game_state("summon_caster_combat");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 500.0).await;
    form_party(&game_state, "alice", "bob").await;
    give_summon_scroll(&game_state, "alice").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    set_combat_at(&game_state, "alice", GameState::now_ms()).await;
    game_state.use_item(&pid("alice"), 9).await;

    let inv = game_state
        .get_player_inventory(&pid("alice"))
        .await
        .unwrap();
    assert_eq!(inv.bag.len(), 1, "the scroll should be kept");
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("in combat"), "{message}")
        }
        other => panic!("Expected a combat refusal, got {:?}", other),
    }
    assert!(matches!(bob_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn summon_recast_while_call_is_out_keeps_scroll() {
    let game_state = make_test_game_state("summon_recast");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 500.0).await;
    form_party(&game_state, "alice", "bob").await;
    give_summon_scrolls(&game_state, "alice", 2).await;
    game_state.use_item(&pid("alice"), 9).await;
    assert_eq!(summon_scrolls_left(&game_state, "alice").await, 1);
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    // The call is still out: re-reading must burn nothing and re-pop nobody.
    game_state.use_item(&pid("alice"), 9).await;
    assert_eq!(summon_scrolls_left(&game_state, "alice").await, 1);
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("still out"), "{message}")
        }
        other => panic!("Expected still-out notice, got {:?}", other),
    }
    assert!(matches!(bob_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn summon_accept_waits_out_casters_combat() {
    let game_state = make_test_game_state("summon_caster_accept_combat");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 500.0).await;
    form_party(&game_state, "alice", "bob").await;
    give_summon_scroll(&game_state, "alice").await;
    game_state.use_item(&pid("alice"), 9).await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    // The caster pulled a fight after casting: delivery must wait it out.
    set_combat_at(&game_state, "alice", GameState::now_ms()).await;
    game_state
        .respond_to_party_summon(&pid("bob"), &pid("alice"), true)
        .await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("alice is in combat"), "{message}")
        }
        other => panic!("Expected caster-combat refusal, got {:?}", other),
    }
    assert_eq!(player_x(&game_state, "bob").await, 500.0);

    // The fight ends: the surviving entry delivers.
    set_combat_at(&game_state, "alice", 0).await;
    game_state
        .respond_to_party_summon(&pid("bob"), &pid("alice"), true)
        .await;
    let x = player_x(&game_state, "bob").await;
    assert!(
        x.abs() < 3.0,
        "bob should arrive once alice is at peace, x={x}"
    );
}

#[tokio::test]
async fn summon_accept_refused_while_caster_fallen() {
    let game_state = make_test_game_state("summon_caster_fallen");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 500.0).await;
    form_party(&game_state, "alice", "bob").await;
    give_summon_scroll(&game_state, "alice").await;
    game_state.use_item(&pid("alice"), 9).await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    set_health(&game_state, "alice", 0).await;
    game_state
        .respond_to_party_summon(&pid("bob"), &pid("alice"), true)
        .await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("has fallen"), "{message}")
        }
        other => panic!("Expected fallen-caster refusal, got {:?}", other),
    }
    assert_eq!(player_x(&game_state, "bob").await, 500.0);
}

#[tokio::test]
async fn summon_accept_waits_out_combat() {
    let game_state = make_test_game_state("summon_combat");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 500.0).await;
    form_party(&game_state, "alice", "bob").await;
    give_summon_scroll(&game_state, "alice").await;
    game_state.use_item(&pid("alice"), 9).await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    set_combat_at(&game_state, "bob", GameState::now_ms()).await;
    game_state
        .respond_to_party_summon(&pid("bob"), &pid("alice"), true)
        .await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("not while in combat"), "{message}")
        }
        other => panic!("Expected combat refusal, got {:?}", other),
    }
    assert_eq!(player_x(&game_state, "bob").await, 500.0);

    // Out of combat again: the pending summon survived the refusal.
    set_combat_at(&game_state, "bob", 0).await;
    game_state
        .respond_to_party_summon(&pid("bob"), &pid("alice"), true)
        .await;
    let x = player_x(&game_state, "bob").await;
    assert!(x.abs() < 3.0, "bob should reach alice after combat, x={x}");
}

#[tokio::test]
async fn summon_decline_reports_and_spends_the_ask() {
    let game_state = make_test_game_state("summon_decline");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 500.0).await;
    form_party(&game_state, "alice", "bob").await;
    give_summon_scroll(&game_state, "alice").await;
    game_state.use_item(&pid("alice"), 9).await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state
        .respond_to_party_summon(&pid("bob"), &pid("alice"), false)
        .await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("bob declined"), "{message}")
        }
        other => panic!("Expected decline report, got {:?}", other),
    }

    // The declined entry is gone: a change of heart finds nothing to accept.
    game_state
        .respond_to_party_summon(&pid("bob"), &pid("alice"), true)
        .await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("expired"), "{message}")
        }
        other => panic!("Expected expired notice, got {:?}", other),
    }
    assert_eq!(player_x(&game_state, "bob").await, 500.0);
}

#[tokio::test]
async fn summon_leave_voids_the_call() {
    let game_state = make_test_game_state("summon_leave");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 500.0).await;
    form_party(&game_state, "alice", "bob").await;
    give_summon_scroll(&game_state, "alice").await;
    game_state.use_item(&pid("alice"), 9).await;
    game_state.leave_party(&pid("alice")).await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    // Leaving voided the entry, exactly as the clients' roster prune assumes.
    game_state
        .respond_to_party_summon(&pid("bob"), &pid("alice"), true)
        .await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("expired"), "{message}")
        }
        other => panic!("Expected expired notice, got {:?}", other),
    }
    assert_eq!(player_x(&game_state, "bob").await, 500.0);
}

#[tokio::test]
async fn summon_teleport_voids_calls_aimed_at_the_mover() {
    let game_state = make_test_game_state("summon_teleport_voids");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 500.0).await;
    let mut carol_rx = add(&game_state, "carol", 900.0).await;
    form_party(&game_state, "alice", "bob").await;
    form_party(&game_state, "alice", "carol").await;
    give_summon_scroll(&game_state, "alice").await;
    give_summon_scrolls(&game_state, "carol", 2).await;
    game_state.use_item(&pid("alice"), 9).await;
    game_state.use_item(&pid("carol"), 9).await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    drain(&mut carol_rx);

    // Bob answers alice; the teleport voids carol's call at him — the
    // clients wiped it from their queues, so the server must match.
    game_state
        .respond_to_party_summon(&pid("bob"), &pid("alice"), true)
        .await;
    let bob_x = player_x(&game_state, "bob").await;
    assert!(bob_x.abs() < 3.0, "bob should stand by alice, x={bob_x}");
    drain(&mut bob_rx);

    game_state
        .respond_to_party_summon(&pid("bob"), &pid("carol"), true)
        .await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("expired"), "{message}")
        }
        other => panic!("Expected expired notice for carol's call, got {:?}", other),
    }

    // And carol can call him again at once: her re-read reaches exactly bob
    // (alice still holds carol's live entry, so she is excluded).
    drain(&mut carol_rx);
    game_state.use_item(&pid("carol"), 9).await;
    assert_eq!(summon_scrolls_left(&game_state, "carol").await, 0);
    let call_notice = std::iter::from_fn(|| carol_rx.try_recv().ok())
        .find_map(|msg| match msg {
            ServerMessage::SystemMessage { message } => Some(message),
            _ => None,
        })
        .expect("carol should get a call notice");
    assert!(call_notice.contains("calling 1"), "{call_notice}");
    assert!(matches!(
        bob_rx.try_recv(),
        Ok(ServerMessage::PartySummonReceived { .. })
    ));
}

#[tokio::test]
async fn party_chat_reaches_every_member_with_sender_echo() {
    let game_state = make_test_game_state("party_chat_all");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    // Far outside the AOI on purpose: party chat ignores distance.
    let mut bob_rx = add(&game_state, "bob", 500.0).await;
    let mut carol_rx = add(&game_state, "carol", 1000.0).await;
    form_party(&game_state, "alice", "bob").await;
    form_party(&game_state, "alice", "carol").await;
    for rx in [&mut alice_rx, &mut bob_rx, &mut carol_rx] {
        drain(rx);
    }

    game_state
        .send_party_chat(&pid("bob"), "to the dungeon!".to_string())
        .await;
    for rx in [&mut alice_rx, &mut bob_rx, &mut carol_rx] {
        match rx.try_recv() {
            Ok(ServerMessage::PartyChatMessage { from, message }) => {
                assert_eq!(from, "bob");
                assert_eq!(message, "to the dungeon!");
            }
            other => panic!("Expected party chat, got {:?}", other),
        }
    }
}

#[tokio::test]
async fn party_chat_outside_a_party_is_refused() {
    let game_state = make_test_game_state("party_chat_solo");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;

    game_state
        .send_party_chat(&pid("alice"), "anyone?".to_string())
        .await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("not in a party"), "{message}")
        }
        other => panic!("Expected refusal, got {:?}", other),
    }
    assert!(matches!(alice_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn party_chat_skips_members_who_blocked_the_sender() {
    let game_state = make_test_game_state("party_chat_block");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    let mut carol_rx = add(&game_state, "carol", 10.0).await;
    form_party(&game_state, "alice", "bob").await;
    form_party(&game_state, "alice", "carol").await;
    game_state
        .set_player_blocks(&pid("carol"), vec!["bob".to_string()])
        .await;
    for rx in [&mut alice_rx, &mut bob_rx, &mut carol_rx] {
        drain(rx);
    }

    game_state
        .send_party_chat(&pid("bob"), "hello?".to_string())
        .await;
    // The block must stay invisible: bob still gets his own echo.
    for rx in [&mut alice_rx, &mut bob_rx] {
        assert!(matches!(
            rx.try_recv(),
            Ok(ServerMessage::PartyChatMessage { .. })
        ));
    }
    assert!(matches!(carol_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn party_chat_from_muted_player_is_refused() {
    use std::time::{Duration, Instant};

    let game_state = make_test_game_state("party_chat_muted");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    form_party(&game_state, "alice", "bob").await;
    game_state.muted_until.write().await.insert(
        "bob".to_string(),
        ("bob".to_string(), Instant::now() + Duration::from_secs(600)),
    );
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state
        .send_party_chat(&pid("bob"), "psst".to_string())
        .await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("Muted"), "{message}")
        }
        other => panic!("Expected mute refusal, got {:?}", other),
    }
    assert!(matches!(alice_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn blank_party_chat_sends_nothing() {
    let game_state = make_test_game_state("party_chat_blank");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    form_party(&game_state, "alice", "bob").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state
        .send_party_chat(&pid("bob"), "   ".to_string())
        .await;
    assert!(matches!(alice_rx.try_recv(), Err(MpscTryRecvError::Empty)));
    assert!(matches!(bob_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

/// A client without the web UI's channel state types the command instead, so
/// the server has to route it — otherwise the line lands in local chat.
#[tokio::test]
async fn typed_party_chat_routes_to_the_party_and_not_to_a_bystander() {
    let game_state = make_test_game_state("party_chat_typed");
    let auth = make_test_auth("party_chat_typed");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    // Standing next to the sender, deliberately out of the party.
    let mut dave_rx = add(&game_state, "dave", 1.0).await;
    form_party(&game_state, "alice", "bob").await;
    for rx in [&mut alice_rx, &mut bob_rx, &mut dave_rx] {
        drain(rx);
    }

    game_state
        .send_chat_message(&pid("bob"), "/p meet at the west gate".to_string(), &auth)
        .await;
    for rx in [&mut alice_rx, &mut bob_rx] {
        match rx.try_recv() {
            Ok(ServerMessage::PartyChatMessage { from, message }) => {
                assert_eq!(from, "bob");
                assert_eq!(message, "meet at the west gate");
            }
            other => panic!("Expected party chat, got {:?}", other),
        }
    }
    assert!(matches!(dave_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn bare_typed_party_chat_draws_a_usage_reply() {
    let game_state = make_test_game_state("party_chat_usage");
    let auth = make_test_auth("party_chat_usage");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    form_party(&game_state, "alice", "bob").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state
        .send_chat_message(&pid("bob"), "/p".to_string(), &auth)
        .await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("/p <message>"), "{message}")
        }
        other => panic!("Expected usage reply, got {:?}", other),
    }
    assert!(matches!(alice_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

// --- Kick and leadership handover ---

#[tokio::test]
async fn leader_kicks_member_with_notices() {
    let game_state = make_test_game_state("party_kick");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    let mut carol_rx = add(&game_state, "carol", 10.0).await;
    form_party(&game_state, "alice", "bob").await;
    form_party(&game_state, "alice", "carol").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    drain(&mut carol_rx);

    game_state.kick_from_party(&pid("alice"), &pid("bob")).await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::PartyState { members, .. }) => assert!(members.is_empty()),
        other => panic!("Expected cleared state for bob, got {:?}", other),
    }
    match bob_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("you were removed"), "{message}")
        }
        other => panic!("Expected removal notice for bob, got {:?}", other),
    }
    for rx in [&mut alice_rx, &mut carol_rx] {
        match rx.try_recv() {
            Ok(ServerMessage::PartyState { leader_id, members }) => {
                assert_eq!(leader_id, pid("alice"));
                let names: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
                assert_eq!(names, ["alice", "carol"]);
            }
            other => panic!("Expected shrunk roster, got {:?}", other),
        }
        match rx.try_recv() {
            Ok(ServerMessage::SystemMessage { message }) => {
                assert!(message.contains("bob was removed"), "{message}")
            }
            other => panic!("Expected removal line, got {:?}", other),
        }
    }
}

#[tokio::test]
async fn only_the_leader_kicks() {
    let game_state = make_test_game_state("party_kick_leader_only");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    let mut carol_rx = add(&game_state, "carol", 10.0).await;
    form_party(&game_state, "alice", "bob").await;
    form_party(&game_state, "alice", "carol").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    drain(&mut carol_rx);

    game_state.kick_from_party(&pid("bob"), &pid("carol")).await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("only the party leader"), "{message}")
        }
        other => panic!("Expected refusal for bob, got {:?}", other),
    }
    assert!(matches!(carol_rx.try_recv(), Err(MpscTryRecvError::Empty)));
    assert!(matches!(alice_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn kick_rejects_self_outsiders_and_the_partyless() {
    let game_state = make_test_game_state("party_kick_rejects");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    let mut dave_rx = add(&game_state, "dave", 15.0).await;
    form_party(&game_state, "alice", "bob").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    drain(&mut dave_rx);

    game_state
        .kick_from_party(&pid("alice"), &pid("alice"))
        .await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("that's you"), "{message}")
        }
        other => panic!("Expected self-kick refusal, got {:?}", other),
    }

    game_state
        .kick_from_party(&pid("alice"), &pid("dave"))
        .await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("not in your party"), "{message}")
        }
        other => panic!("Expected outsider refusal, got {:?}", other),
    }
    assert!(matches!(dave_rx.try_recv(), Err(MpscTryRecvError::Empty)));

    game_state
        .kick_from_party(&pid("dave"), &pid("alice"))
        .await;
    match dave_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("not in a party"), "{message}")
        }
        other => panic!("Expected partyless refusal, got {:?}", other),
    }
}

#[tokio::test]
async fn kick_to_one_disbands() {
    let game_state = make_test_game_state("party_kick_disband");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    form_party(&game_state, "alice", "bob").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state.kick_from_party(&pid("alice"), &pid("bob")).await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::PartyState { members, .. }) => assert!(members.is_empty()),
        other => panic!("Expected cleared state for bob, got {:?}", other),
    }
    match bob_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("you were removed"), "{message}")
        }
        other => panic!("Expected removal notice for bob, got {:?}", other),
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
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("bob was removed"), "{message}")
        }
        other => panic!("Expected removal line, got {:?}", other),
    }
}

#[tokio::test]
async fn promote_hands_over_leadership() {
    let game_state = make_test_game_state("party_promote_cmd");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    let mut carol_rx = add(&game_state, "carol", 10.0).await;
    form_party(&game_state, "alice", "bob").await;
    form_party(&game_state, "alice", "carol").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    drain(&mut carol_rx);

    game_state
        .promote_party_leader(&pid("alice"), &pid("bob"))
        .await;
    for rx in [&mut alice_rx, &mut bob_rx, &mut carol_rx] {
        match rx.try_recv() {
            Ok(ServerMessage::PartyState { leader_id, members }) => {
                assert_eq!(leader_id, pid("bob"));
                // The roster order is untouched by a handover.
                let names: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
                assert_eq!(names, ["alice", "bob", "carol"]);
            }
            other => panic!("Expected handed-over roster, got {:?}", other),
        }
        match rx.try_recv() {
            Ok(ServerMessage::SystemMessage { message }) => {
                assert!(message.contains("bob is now the party leader"), "{message}")
            }
            other => panic!("Expected handover line, got {:?}", other),
        }
    }

    // The lead actually moved: the new leader may kick.
    game_state.kick_from_party(&pid("bob"), &pid("carol")).await;
    match carol_rx.try_recv() {
        Ok(ServerMessage::PartyState { members, .. }) => assert!(members.is_empty()),
        other => panic!("Expected carol kicked by the new leader, got {:?}", other),
    }
}

#[tokio::test]
async fn promote_requires_leadership_and_membership() {
    let game_state = make_test_game_state("party_promote_rejects");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    let mut dave_rx = add(&game_state, "dave", 15.0).await;
    form_party(&game_state, "alice", "bob").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    drain(&mut dave_rx);

    game_state
        .promote_party_leader(&pid("bob"), &pid("alice"))
        .await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("only the party leader"), "{message}")
        }
        other => panic!("Expected refusal for bob, got {:?}", other),
    }

    game_state
        .promote_party_leader(&pid("alice"), &pid("alice"))
        .await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("already lead"), "{message}")
        }
        other => panic!("Expected self-promote refusal, got {:?}", other),
    }

    game_state
        .promote_party_leader(&pid("alice"), &pid("dave"))
        .await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("not in your party"), "{message}")
        }
        other => panic!("Expected outsider refusal, got {:?}", other),
    }
    assert!(matches!(dave_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

#[tokio::test]
async fn party_kick_and_leader_chat_commands() {
    let game_state = make_test_game_state("party_subcommands");
    let auth = make_test_auth("party_subcommands");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    let mut carol_rx = add(&game_state, "carol", 10.0).await;
    form_party(&game_state, "alice", "bob").await;
    form_party(&game_state, "alice", "carol").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    drain(&mut carol_rx);

    game_state
        .send_chat_message(&pid("alice"), "/party kick".to_string(), &auth)
        .await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("/party kick <name>"), "{message}")
        }
        other => panic!("Expected usage reply, got {:?}", other),
    }

    game_state
        .send_chat_message(&pid("alice"), "/party kick nobody".to_string(), &auth)
        .await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("no one called nobody"), "{message}")
        }
        other => panic!("Expected unknown-name reply, got {:?}", other),
    }

    // Names resolve ignoring case, like invites.
    game_state
        .send_chat_message(&pid("alice"), "/party leader BOB".to_string(), &auth)
        .await;
    match alice_rx.try_recv() {
        Ok(ServerMessage::PartyState { leader_id, .. }) => assert_eq!(leader_id, pid("bob")),
        other => panic!("Expected handover via command, got {:?}", other),
    }
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    drain(&mut carol_rx);

    game_state
        .send_chat_message(&pid("bob"), "/party kick carol".to_string(), &auth)
        .await;
    match carol_rx.try_recv() {
        Ok(ServerMessage::PartyState { members, .. }) => assert!(members.is_empty()),
        other => panic!("Expected carol kicked via command, got {:?}", other),
    }
    drain(&mut alice_rx);
    drain(&mut bob_rx);

    game_state
        .send_chat_message(&pid("bob"), "/party leave".to_string(), &auth)
        .await;
    match bob_rx.try_recv() {
        Ok(ServerMessage::PartyState { members, .. }) => assert!(members.is_empty()),
        other => panic!("Expected bob out via /party leave, got {:?}", other),
    }
}

// --- Roster vitals ---

#[tokio::test]
async fn party_state_carries_hp_and_class() {
    let game_state = make_test_game_state("party_state_vitals");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 5.0).await;
    set_health(&game_state, "bob", 4).await;
    form_party(&game_state, "alice", "bob").await;
    drain(&mut bob_rx);

    let members = loop {
        match alice_rx.try_recv() {
            Ok(ServerMessage::PartyState { members, .. }) => break members,
            Ok(_) => continue,
            Err(err) => panic!("Expected roster, got {:?}", err),
        }
    };
    let bob = members.iter().find(|m| m.name == "bob").unwrap();
    assert_eq!(bob.hp, 4);
    assert_eq!(bob.max_hp, 10);
    assert_eq!(bob.class, CharacterClass::Knight);
}

#[tokio::test]
async fn vitals_tick_pushes_only_dirty_parties() {
    let game_state = make_test_game_state("party_vitals_tick");
    let mut alice_rx = add(&game_state, "alice", 0.0).await;
    let mut bob_rx = add(&game_state, "bob", 500.0).await;
    let mut dave_rx = add(&game_state, "dave", 15.0).await;
    form_party(&game_state, "alice", "bob").await;
    drain(&mut alice_rx);
    drain(&mut bob_rx);
    drain(&mut dave_rx);

    // Nothing queued: the tick sends nothing.
    game_state.tick_party_vitals().await;
    assert!(matches!(alice_rx.try_recv(), Err(MpscTryRecvError::Empty)));

    // A queued member pushes the whole party's vitals to every member,
    // beyond any AOI (bob is 500m away).
    set_health(&game_state, "bob", 3).await;
    game_state.mark_party_vitals_dirty(&pid("bob")).await;
    game_state.tick_party_vitals().await;
    for rx in [&mut alice_rx, &mut bob_rx] {
        match rx.try_recv() {
            Ok(ServerMessage::PartyVitals { members }) => {
                assert_eq!(members.len(), 2);
                let bob = members.iter().find(|m| m.id == pid("bob")).unwrap();
                assert_eq!(bob.hp, 3);
                assert_eq!(bob.max_hp, 10);
            }
            other => panic!("Expected vitals push, got {:?}", other),
        }
    }

    // A partyless player's queued change is dropped by the tick.
    set_health(&game_state, "dave", 2).await;
    game_state.mark_party_vitals_dirty(&pid("dave")).await;
    game_state.tick_party_vitals().await;
    assert!(matches!(dave_rx.try_recv(), Err(MpscTryRecvError::Empty)));
    assert!(matches!(alice_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}
