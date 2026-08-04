use super::*;

#[tokio::test]
async fn chat_uses_direct_spatial_fanout_instead_of_global_broadcast() {
    let game_state = make_test_game_state("chat_spatial_fanout");
    let auth = make_test_auth("chat_spatial_fanout");
    let speaker_id = pid("speaker");
    let near_listener_id = pid("near_listener");
    let far_listener_id = pid("far_listener");

    game_state
        .add_player(make_player("speaker", 0.0, 0.0))
        .await;
    game_state
        .add_player(make_player("near_listener", 10.0, 0.0))
        .await;
    game_state
        .add_player(make_player("far_listener", 100.0, 0.0))
        .await;

    let mut speaker_rx = game_state.register_direct_channel(&speaker_id).await;
    let mut near_rx = game_state.register_direct_channel(&near_listener_id).await;
    let mut far_rx = game_state.register_direct_channel(&far_listener_id).await;

    let mut broadcast_rx = game_state.subscribe();
    game_state
        .send_chat_message(&speaker_id, "hello".to_string(), &auth)
        .await;

    match speaker_rx.try_recv() {
        Ok(ServerMessage::ChatMessage { player_id, message }) => {
            assert_eq!(player_id, pid("speaker"));
            assert_eq!(message, "hello");
        }
        other => panic!("Expected direct chat for speaker, got {:?}", other),
    }

    match near_rx.try_recv() {
        Ok(ServerMessage::ChatMessage { player_id, message }) => {
            assert_eq!(player_id, pid("speaker"));
            assert_eq!(message, "hello");
        }
        other => panic!("Expected direct chat for nearby listener, got {:?}", other),
    }

    match far_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Expected no direct chat for far listener, got {:?}", other),
    }

    match broadcast_rx.try_recv() {
        Err(TryRecvError::Empty) => {}
        Ok(msg) => {
            let server_msg: ServerMessage =
                rmp_serde::from_slice(&msg.bytes).expect("Failed to deserialize broadcast");
            panic!("Expected no chat broadcast, got {:?}", server_msg);
        }
        Err(err) => panic!("Expected empty broadcast channel, got {:?}", err),
    }
}

#[tokio::test]
async fn who_command_reports_online_counts_only_to_the_requester() {
    let game_state = make_test_game_state("who_command");
    let auth = make_test_auth("who_command");
    let asker_id = pid("asker");
    let bystander_id = pid("bystander");
    let mut asker = make_player("asker", 0.0, 0.0);
    asker.client_kind = ClientKind::Web;
    game_state.add_player(asker).await;
    let mut bystander = make_player("bystander", 5.0, 0.0);
    bystander.client_kind = ClientKind::Cli;
    game_state.add_player(bystander).await;
    let mut npc = make_player("npc_karl", 10.0, 0.0);
    npc.is_official_npc = true;
    npc.client_kind = ClientKind::Cli;
    game_state.add_player(npc).await;

    let mut asker_rx = game_state.register_direct_channel(&asker_id).await;
    let mut bystander_rx = game_state.register_direct_channel(&bystander_id).await;

    game_state
        .send_chat_message(&asker_id, "/who".to_string(), &auth)
        .await;

    match asker_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert_eq!(message, "Online: 3 (1 web, 1 cli, 1 npc)");
        }
        other => panic!("Expected online count reply, got {:?}", other),
    }

    match bystander_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("/who must not be relayed as chat, got {:?}", other),
    }
}

#[test]
fn whisper_command_parses_name_and_message() {
    use super::chat::parse_whisper_command;

    assert_eq!(
        parse_whisper_command("/w Rica hello there"),
        Some(("Rica", "hello there"))
    );
    assert_eq!(
        parse_whisper_command("/whisper Rica hi"),
        Some(("Rica", "hi"))
    );
    assert_eq!(
        parse_whisper_command(" /w  Rica   hi "),
        Some(("Rica", "hi"))
    );
    assert_eq!(parse_whisper_command("/w Rica"), Some(("Rica", "")));
    assert_eq!(parse_whisper_command("/w"), Some(("", "")));
    assert_eq!(parse_whisper_command("/who"), None);
    assert_eq!(parse_whisper_command("hello /w Rica"), None);
}

#[tokio::test]
async fn whisper_reaches_only_the_target_regardless_of_distance() {
    let game_state = make_test_game_state("whisper_delivery");
    let auth = make_test_auth("whisper_delivery");
    let sender_id = pid("sender");
    let target_id = pid("far_target");
    let bystander_id = pid("bystander");
    game_state.add_player(make_player("sender", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("far_target", 500.0, 0.0))
        .await;
    game_state
        .add_player(make_player("bystander", 5.0, 0.0))
        .await;

    let mut sender_rx = game_state.register_direct_channel(&sender_id).await;
    let mut target_rx = game_state.register_direct_channel(&target_id).await;
    let mut bystander_rx = game_state.register_direct_channel(&bystander_id).await;
    let mut broadcast_rx = game_state.subscribe();

    // Mixed case on purpose: the name must match case-insensitively and the
    // delivered `to` must carry the canonical spelling.
    game_state
        .send_chat_message(&sender_id, "/w Far_Target psst".to_string(), &auth)
        .await;

    for rx in [&mut target_rx, &mut sender_rx] {
        match rx.try_recv() {
            Ok(ServerMessage::WhisperMessage { from, to, message }) => {
                assert_eq!(from, "sender");
                assert_eq!(to, "far_target");
                assert_eq!(message, "psst");
            }
            other => panic!("Expected whisper delivery, got {:?}", other),
        }
    }

    match bystander_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Whisper must not reach bystanders, got {:?}", other),
    }

    match broadcast_rx.try_recv() {
        Err(TryRecvError::Empty) => {}
        Ok(msg) => {
            let server_msg: ServerMessage =
                rmp_serde::from_slice(&msg.bytes).expect("Failed to deserialize broadcast");
            panic!("Expected no whisper broadcast, got {:?}", server_msg);
        }
        Err(err) => panic!("Expected empty broadcast channel, got {:?}", err),
    }
}

#[tokio::test]
async fn whisper_matches_names_ignoring_ascii_case() {
    let game_state = make_test_game_state("whisper_ci");
    let auth = make_test_auth("whisper_ci");
    let sender_id = pid("sender");
    let rica_id = pid("Rica");
    game_state.add_player(make_player("sender", 0.0, 0.0)).await;
    game_state.add_player(make_player("Rica", 5.0, 0.0)).await;

    let mut sender_rx = game_state.register_direct_channel(&sender_id).await;
    let mut rica_rx = game_state.register_direct_channel(&rica_id).await;

    game_state
        .send_chat_message(&sender_id, "/w rica psst".to_string(), &auth)
        .await;
    match rica_rx.try_recv() {
        Ok(ServerMessage::WhisperMessage { to, .. }) => assert_eq!(to, "Rica"),
        other => panic!("Expected case-insensitive whisper, got {:?}", other),
    }

    // The name index follows the roster: once Rica leaves, the lookup misses.
    game_state.remove_player(&rica_id).await;
    while sender_rx.try_recv().is_ok() {}
    game_state
        .send_chat_message(&sender_id, "/w RICA psst".to_string(), &auth)
        .await;
    match sender_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert_eq!(message, "Whisper: no one called RICA is here.")
        }
        other => panic!("Expected offline reply, got {:?}", other),
    }
}

#[tokio::test]
async fn whisper_errors_go_only_to_the_sender() {
    let game_state = make_test_game_state("whisper_errors");
    let auth = make_test_auth("whisper_errors");
    let sender_id = pid("sender");
    let listener_id = pid("listener");
    game_state.add_player(make_player("sender", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("listener", 5.0, 0.0))
        .await;

    let mut sender_rx = game_state.register_direct_channel(&sender_id).await;
    let mut listener_rx = game_state.register_direct_channel(&listener_id).await;

    let expect_reply =
        |result: Result<ServerMessage, MpscTryRecvError>, expected: &str| match result {
            Ok(ServerMessage::SystemMessage { message }) => {
                assert_eq!(message, expected);
            }
            other => panic!("Expected whisper error reply, got {:?}", other),
        };

    game_state
        .send_chat_message(&sender_id, "/w Nobody hi".to_string(), &auth)
        .await;
    expect_reply(
        sender_rx.try_recv(),
        "Whisper: no one called Nobody is here.",
    );

    game_state
        .send_chat_message(&sender_id, "/w listener".to_string(), &auth)
        .await;
    expect_reply(sender_rx.try_recv(), "Whisper: /w <name> <message>");

    game_state
        .send_chat_message(&sender_id, "/w sender hi".to_string(), &auth)
        .await;
    expect_reply(sender_rx.try_recv(), "Whisper: that's you.");

    match listener_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!(
            "Whisper errors must not leak into local chat, got {:?}",
            other
        ),
    }
}

#[test]
fn block_command_parses_actions() {
    use super::chat::{parse_block_command, BlockCommand};

    assert_eq!(parse_block_command("/block"), Some(BlockCommand::List));
    assert_eq!(parse_block_command(" /block  "), Some(BlockCommand::List));
    assert_eq!(
        parse_block_command("/block Rica"),
        Some(BlockCommand::Block("Rica"))
    );
    assert_eq!(
        parse_block_command("/unblock Rica"),
        Some(BlockCommand::Unblock("Rica"))
    );
    assert_eq!(
        parse_block_command("/unblock"),
        Some(BlockCommand::Unblock(""))
    );
    assert_eq!(parse_block_command("/blocked"), None);
    assert_eq!(parse_block_command("hello /block Rica"), None);
}

#[tokio::test]
async fn blocked_players_chat_skips_the_blocker_only() {
    let game_state = make_test_game_state("block_chat_filter");
    let auth = make_test_auth("block_chat_filter");
    let speaker_id = pid("speaker");
    let blocker_id = pid("blocker");
    let listener_id = pid("listener");
    game_state
        .add_player(make_player("speaker", 0.0, 0.0))
        .await;
    game_state
        .add_player(make_player("blocker", 5.0, 0.0))
        .await;
    game_state
        .add_player(make_player("listener", 10.0, 0.0))
        .await;

    let mut blocker_rx = game_state.register_direct_channel(&blocker_id).await;
    let mut listener_rx = game_state.register_direct_channel(&listener_id).await;

    game_state
        .set_player_blocks(&blocker_id, vec!["speaker".to_string()])
        .await;
    game_state
        .send_chat_message(&speaker_id, "hello".to_string(), &auth)
        .await;

    match blocker_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Blocked chat must not reach the blocker, got {:?}", other),
    }
    match listener_rx.try_recv() {
        Ok(ServerMessage::ChatMessage { player_id, .. }) => assert_eq!(player_id, speaker_id),
        other => panic!("Bystander must still get the chat, got {:?}", other),
    }
}

#[tokio::test]
async fn blocked_senders_whisper_is_suppressed_but_still_echoed() {
    let game_state = make_test_game_state("block_whisper_filter");
    let auth = make_test_auth("block_whisper_filter");
    let sender_id = pid("sender");
    let blocker_id = pid("blocker");
    game_state.add_player(make_player("sender", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("blocker", 5.0, 0.0))
        .await;

    let mut sender_rx = game_state.register_direct_channel(&sender_id).await;
    let mut blocker_rx = game_state.register_direct_channel(&blocker_id).await;

    game_state
        .set_player_blocks(&blocker_id, vec!["sender".to_string()])
        .await;
    game_state
        .send_chat_message(&sender_id, "/w blocker psst".to_string(), &auth)
        .await;

    match blocker_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Whisper must not reach the blocker, got {:?}", other),
    }
    // The echo hides the block from the sender.
    match sender_rx.try_recv() {
        Ok(ServerMessage::WhisperMessage { from, to, message }) => {
            assert_eq!(from, "sender");
            assert_eq!(to, "blocker");
            assert_eq!(message, "psst");
        }
        other => panic!("Sender must still get the echo, got {:?}", other),
    }
}

#[tokio::test]
async fn block_command_persists_resolves_case_and_unblocks() {
    let auth = make_test_auth("block_command_flow");
    let account = auth.login_npc("npc_block_cmd_test").unwrap();
    let blocker_char = create_test_character(&auth, &account, "Blocker");
    let abuser_char = create_test_character(&auth, &account, "Abuser");

    let game_state = make_test_game_state("block_command_flow");
    let blocker_id = pid("Blocker");
    let abuser_id = pid("Abuser");
    game_state
        .add_player(make_player("Blocker", 0.0, 0.0))
        .await;
    game_state.add_player(make_player("Abuser", 5.0, 0.0)).await;
    game_state
        .register_player_character(&blocker_id, blocker_char.id, 0, attrs_with_cha(12), 0, None)
        .await;
    game_state
        .register_player_character(&abuser_id, abuser_char.id, 0, attrs_with_cha(12), 0, None)
        .await;

    let mut blocker_rx = game_state.register_direct_channel(&blocker_id).await;

    let expect_reply =
        |result: Result<ServerMessage, MpscTryRecvError>, expected: &str| match result {
            Ok(ServerMessage::SystemMessage { message }) => assert_eq!(message, expected),
            other => panic!("Expected block reply, got {:?}", other),
        };
    let command = |text: &str| game_state.send_chat_message(&blocker_id, text.to_string(), &auth);

    // Lowercase on purpose: resolution is case-insensitive.
    command("/block abuser").await;
    expect_reply(
        blocker_rx.try_recv(),
        "Block: Abuser is blocked; their chat and whispers are hidden. /unblock Abuser undoes this.",
    );
    assert_eq!(
        auth.load_blocked_names(blocker_char.id).unwrap(),
        vec!["Abuser".to_string()]
    );

    game_state
        .send_chat_message(&abuser_id, "spam".to_string(), &auth)
        .await;
    match blocker_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Blocked chat must not be delivered, got {:?}", other),
    }

    command("/block").await;
    expect_reply(blocker_rx.try_recv(), "Blocked: Abuser");

    command("/unblock ABUSER").await;
    expect_reply(
        blocker_rx.try_recv(),
        "Unblock: Abuser is no longer blocked.",
    );
    assert!(auth.load_blocked_names(blocker_char.id).unwrap().is_empty());

    game_state
        .send_chat_message(&abuser_id, "hello again".to_string(), &auth)
        .await;
    match blocker_rx.try_recv() {
        Ok(ServerMessage::ChatMessage { player_id, .. }) => assert_eq!(player_id, abuser_id),
        other => panic!("Chat must flow again after unblock, got {:?}", other),
    }

    // Guard rails: no self-block, no unknown names.
    command("/block Blocker").await;
    expect_reply(blocker_rx.try_recv(), "Block: that's you.");
    command("/block Nobody").await;
    expect_reply(blocker_rx.try_recv(), "Block: no character named Nobody.");
}

#[test]
fn notice_command_sets_or_clears_message() {
    use super::parse_notice_command;

    assert_eq!(
        parse_notice_command("/notice Server maintenance"),
        Some(Some("Server maintenance"))
    );
    assert_eq!(parse_notice_command(" /notice "), Some(None));
    assert_eq!(parse_notice_command("/noticeboard"), None);
}

#[tokio::test]
async fn server_notice_is_stored_broadcast_and_clearable() {
    let game_state = make_test_game_state("server_notice");
    let auth = make_test_auth("server_notice");
    let player_id = pid("notice_admin");
    let mut receiver = game_state.subscribe();

    game_state
        .send_chat_message(&player_id, "/notice Server maintenance".to_string(), &auth)
        .await;
    assert_eq!(
        game_state.server_notice().await.as_deref(),
        Some("Server maintenance")
    );
    let message = receiver.recv().await.expect("notice broadcast");
    assert!(matches!(
        rmp_serde::from_slice::<ServerMessage>(&message.bytes),
        Ok(ServerMessage::ServerNotice {
            message: Some(message)
        }) if message == "Server maintenance"
    ));

    game_state
        .send_chat_message(&player_id, "/notice".to_string(), &auth)
        .await;
    assert_eq!(game_state.server_notice().await, None);
    let message = receiver.recv().await.expect("notice clear broadcast");
    assert!(matches!(
        rmp_serde::from_slice::<ServerMessage>(&message.bytes),
        Ok(ServerMessage::ServerNotice { message: None })
    ));
}

#[tokio::test]
async fn escape_command_returns_a_stuck_player_to_spawn() {
    let game_state = make_test_game_state("escape_to_spawn");
    let auth = make_test_auth("escape_to_spawn");
    let stuck_id = pid("stuck");
    let bystander_id = pid("bystander");
    game_state.add_player(make_player("stuck", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("bystander", 5.0, 0.0))
        .await;
    let mut bystander_rx = game_state.register_direct_channel(&bystander_id).await;

    game_state
        .send_chat_message(&stuck_id, "/escape".to_string(), &auth)
        .await;

    let spawn = &world_config().spawn_position;
    let (position, _, floor_level) = game_state
        .get_player_position(&stuck_id)
        .await
        .expect("player should still exist");
    // Approximate: the teleport runs the X through `wrap_world_x`, which is a
    // no-op away from the seam but not bit-exact.
    let expected = spawn.position();
    assert!(
        (position.x - expected.x).abs() < 0.01
            && (position.y - expected.y).abs() < 0.01
            && (position.z - expected.z).abs() < 0.01,
        "expected spawn {expected:?}, got {position:?}"
    );
    // Escaping a dungeon has to land on the surface, not carry its floor along.
    assert_eq!(floor_level, 0);

    // The command is consumed, never relayed as chat to anyone nearby. The
    // bystander does still get the movement traffic the teleport generates.
    for msg in drain(&mut bystander_rx) {
        assert!(
            !matches!(
                msg,
                ServerMessage::ChatMessage { .. } | ServerMessage::SystemMessage { .. }
            ),
            "/escape must not be echoed as chat: {msg:?}"
        );
    }
}

#[test]
fn admin_command_parses_actions() {
    use super::chat::{parse_admin_command, AdminCommand};

    assert_eq!(
        parse_admin_command("/kick Abuser"),
        Some(AdminCommand::Kick("Abuser"))
    );
    assert_eq!(
        parse_admin_command("/mute Abuser"),
        Some(AdminCommand::Mute {
            name: "Abuser",
            minutes: None
        })
    );
    assert_eq!(
        parse_admin_command("/mute Abuser 30"),
        Some(AdminCommand::Mute {
            name: "Abuser",
            minutes: Some("30")
        })
    );
    assert_eq!(
        parse_admin_command("/unmute Abuser"),
        Some(AdminCommand::Unmute("Abuser"))
    );
    assert_eq!(
        parse_admin_command("/summon Abuser"),
        Some(AdminCommand::Summon("Abuser"))
    );
    assert_eq!(
        parse_admin_command("/goto Abuser"),
        Some(AdminCommand::Goto("Abuser"))
    );
    assert_eq!(
        parse_admin_command("/ban Abuser"),
        Some(AdminCommand::Ban {
            name: "Abuser",
            minutes: None
        })
    );
    assert_eq!(
        parse_admin_command("/ban Abuser 60"),
        Some(AdminCommand::Ban {
            name: "Abuser",
            minutes: Some("60")
        })
    );
    // /unban must not be read as /ban: the shorter prefix is checked second.
    assert_eq!(
        parse_admin_command("/unban Abuser"),
        Some(AdminCommand::Unban("Abuser"))
    );
    // Whole-word rule: /kickstart is not /kick.
    assert_eq!(parse_admin_command("/kickstart party"), None);
    assert_eq!(parse_admin_command("/banish Abuser"), None);
    assert_eq!(parse_admin_command("hello /kick Abuser"), None);
    // Unvalidated on purpose: a bare command parses (and is admin-gated),
    // then draws a usage reply instead of leaking into local chat.
    assert_eq!(parse_admin_command("/kick"), Some(AdminCommand::Kick("")));
}

/// The gap review found: the ban is stored per account, so eviction has to be
/// per account too. A session playing an alt — or sitting at character select
/// with no character at all — must still be closed.
#[tokio::test]
async fn ban_evicts_the_account_session_playing_any_character() {
    let game_state = make_test_game_state("admin_ban_alt");
    let (auth, _db) = make_test_auth_with_path("admin_ban_alt");
    let admin_id = pid("admin");
    let alt_id = pid("Bob");
    game_state.add_player(make_player("admin", 0.0, 0.0)).await;
    game_state.add_player(make_player("Bob", 5.0, 0.0)).await;
    let mut admin_rx = game_state.register_direct_channel(&admin_id).await;

    // One account owning two characters; only the alt is online.
    let account = auth.login_npc("npc_ban_alt").unwrap();
    let attributes = test_attributes();
    for name in ["Alice", "Bob"] {
        auth.create_character(
            &account,
            name,
            &attributes,
            16,
            CharacterClass::Knight,
            Gender::Male,
        )
        .unwrap();
    }
    let (kick_tx, mut kick_rx) = tokio::sync::mpsc::unbounded_channel();
    let session_id = game_state
        .register_account_session(&account, kick_tx, &auth)
        .await;
    assert!(
        game_state
            .attach_player_to_account_session(&account, session_id, alt_id)
            .await
    );

    // Banning the *offline* character has to close the session held by the alt.
    game_state
        .send_chat_message(&admin_id, "/ban Alice 30".to_string(), &auth)
        .await;

    match kick_rx.try_recv() {
        Ok(ServerMessage::Kicked { reason, .. }) => assert!(
            reason.contains("30 minute"),
            "the kick carries the remaining time: {reason}"
        ),
        other => panic!("expected the alt's session to be kicked, got {other:?}"),
    }
    assert!(
        !game_state
            .is_current_account_session(&account, session_id)
            .await,
        "the banned account must hold no session"
    );
    assert!(
        auth.active_ban(&account).unwrap().is_some(),
        "and the ban itself is recorded"
    );
    let _ = drain(&mut admin_rx);
}

/// A session that authenticated but has not entered the game has no player id,
/// so a character-name lookup cannot see it — the account key can.
#[tokio::test]
async fn ban_evicts_a_session_still_at_character_select() {
    let game_state = make_test_game_state("admin_ban_select");
    let (auth, _db) = make_test_auth_with_path("admin_ban_select");
    let admin_id = pid("admin");
    game_state.add_player(make_player("admin", 0.0, 0.0)).await;
    let mut admin_rx = game_state.register_direct_channel(&admin_id).await;

    let account = auth.login_npc("npc_ban_select").unwrap();
    auth.create_character(
        &account,
        "Idler",
        &test_attributes(),
        16,
        CharacterClass::Knight,
        Gender::Male,
    )
    .unwrap();
    let (kick_tx, mut kick_rx) = tokio::sync::mpsc::unbounded_channel();
    let session_id = game_state
        .register_account_session(&account, kick_tx, &auth)
        .await;
    // Deliberately no attach_player_to_account_session: still at select.

    game_state
        .send_chat_message(&admin_id, "/ban Idler".to_string(), &auth)
        .await;

    assert!(
        matches!(kick_rx.try_recv(), Ok(ServerMessage::Kicked { .. })),
        "a pre-game session must be closed too"
    );
    assert!(
        !game_state
            .is_current_account_session(&account, session_id)
            .await
    );
    let _ = drain(&mut admin_rx);
}

/// Self-ban has to compare accounts: an operator's own alt is a different
/// character name on the same account.
#[tokio::test]
async fn ban_refuses_an_operators_own_alt() {
    let game_state = make_test_game_state("admin_ban_self");
    let (auth, _db) = make_test_auth_with_path("admin_ban_self");
    let admin_id = pid("admin");
    game_state.add_player(make_player("admin", 0.0, 0.0)).await;
    let mut admin_rx = game_state.register_direct_channel(&admin_id).await;

    let account = auth.login_npc("npc_ban_self").unwrap();
    auth.create_character(
        &account,
        "AdminAlt",
        &test_attributes(),
        16,
        CharacterClass::Knight,
        Gender::Male,
    )
    .unwrap();
    let (kick_tx, _kick_rx) = tokio::sync::mpsc::unbounded_channel();
    let session_id = game_state
        .register_account_session(&account, kick_tx, &auth)
        .await;
    assert!(
        game_state
            .attach_player_to_account_session(&account, session_id, admin_id)
            .await
    );

    game_state
        .send_chat_message(&admin_id, "/ban AdminAlt".to_string(), &auth)
        .await;

    assert!(
        drain(&mut admin_rx).iter().any(|m| matches!(
            m,
            ServerMessage::SystemMessage { message } if message.contains("your own account")
        )),
        "banning an alt of your own account must be refused"
    );
    assert!(
        auth.active_ban(&account).unwrap().is_none(),
        "and must not have written a ban"
    );
}

#[tokio::test]
async fn kick_command_disconnects_the_named_player() {
    let game_state = make_test_game_state("admin_kick");
    let auth = make_test_auth("admin_kick");
    let admin_id = pid("admin");
    let target_id = pid("target");
    game_state.add_player(make_player("admin", 0.0, 0.0)).await;
    game_state.add_player(make_player("target", 5.0, 0.0)).await;
    let mut admin_rx = game_state.register_direct_channel(&admin_id).await;
    // The kick rides the account session channel (it closes the socket), so
    // the target needs one, like a real login.
    let (kick_tx, mut kick_rx) = tokio::sync::mpsc::unbounded_channel();
    let session_id = game_state
        .register_account_session("target-account", kick_tx, &auth)
        .await;
    assert!(
        game_state
            .attach_player_to_account_session("target-account", session_id, target_id)
            .await
    );

    // Mixed case on purpose: resolution is case-insensitive.
    game_state
        .send_chat_message(&admin_id, "/kick TARGET".to_string(), &auth)
        .await;

    match kick_rx.try_recv() {
        Ok(ServerMessage::Kicked { player_id, reason }) => {
            assert_eq!(player_id, target_id);
            assert_eq!(reason, "Removed by an operator");
        }
        other => panic!("Expected a kick over the account session, got {:?}", other),
    }
    assert!(
        !game_state
            .is_current_account_session("target-account", session_id)
            .await,
        "the kicked account session must be gone"
    );
    assert!(
        drain(&mut admin_rx).iter().any(|m| matches!(
            m,
            ServerMessage::SystemMessage { message } if message == "Kick: target was disconnected."
        )),
        "the admin must get a confirmation"
    );
    assert!(
        game_state.get_player_position(&target_id).await.is_none(),
        "the kicked player must leave the roster"
    );

    game_state
        .send_chat_message(&admin_id, "/kick Nobody".to_string(), &auth)
        .await;
    match admin_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert_eq!(message, "Kick: no one called Nobody is here.")
        }
        other => panic!("Expected a kick error, got {:?}", other),
    }
}

#[tokio::test]
async fn mute_command_silences_chat_and_whispers_until_unmute() {
    let game_state = make_test_game_state("admin_mute");
    let auth = make_test_auth("admin_mute");
    let admin_id = pid("admin");
    let spammer_id = pid("spammer");
    let listener_id = pid("listener");
    game_state.add_player(make_player("admin", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("spammer", 5.0, 0.0))
        .await;
    game_state
        .add_player(make_player("listener", 10.0, 0.0))
        .await;
    let mut admin_rx = game_state.register_direct_channel(&admin_id).await;
    let mut spammer_rx = game_state.register_direct_channel(&spammer_id).await;
    let mut listener_rx = game_state.register_direct_channel(&listener_id).await;

    game_state
        .send_chat_message(&admin_id, "/mute SPAMMER 5".to_string(), &auth)
        .await;
    match admin_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => assert_eq!(
            message,
            "Mute: spammer cannot chat for 5m. /unmute spammer undoes this."
        ),
        other => panic!("Expected a mute reply, got {:?}", other),
    }
    match spammer_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert_eq!(message, "You are muted for 5m.")
        }
        other => panic!("Expected a mute notice, got {:?}", other),
    }

    game_state
        .send_chat_message(&spammer_id, "buy gold".to_string(), &auth)
        .await;
    match spammer_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert_eq!(message, "Muted: you cannot chat for another 5m.")
        }
        other => panic!("Expected a muted reply, got {:?}", other),
    }
    match listener_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Muted chat must not be delivered, got {:?}", other),
    }

    game_state
        .send_chat_message(&spammer_id, "/w listener psst".to_string(), &auth)
        .await;
    match spammer_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert_eq!(message, "Muted: you cannot whisper for another 5m.")
        }
        other => panic!("Expected a muted whisper reply, got {:?}", other),
    }
    match listener_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Muted whisper must not be delivered, got {:?}", other),
    }

    // Keyed by name, not session: a relog must not clear it.
    game_state.remove_player(&spammer_id).await;
    game_state
        .add_player(make_player("spammer", 5.0, 0.0))
        .await;
    let mut spammer_rx = game_state.register_direct_channel(&spammer_id).await;
    while listener_rx.try_recv().is_ok() {}
    while admin_rx.try_recv().is_ok() {}
    game_state
        .send_chat_message(&spammer_id, "still here".to_string(), &auth)
        .await;
    match spammer_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert_eq!(message, "Muted: you cannot chat for another 5m.")
        }
        other => panic!("Expected the mute to survive a relog, got {:?}", other),
    }

    game_state
        .send_chat_message(&admin_id, "/unmute spammer".to_string(), &auth)
        .await;
    match admin_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert_eq!(message, "Unmute: spammer can chat again.")
        }
        other => panic!("Expected an unmute reply, got {:?}", other),
    }
    game_state
        .send_chat_message(&spammer_id, "hello".to_string(), &auth)
        .await;
    match listener_rx.try_recv() {
        Ok(ServerMessage::ChatMessage { player_id, .. }) => assert_eq!(player_id, spammer_id),
        other => panic!("Chat must flow again after unmute, got {:?}", other),
    }
}

#[tokio::test]
async fn summon_and_goto_teleport_beside_the_target() {
    let game_state = make_test_game_state("admin_summon_goto");
    let auth = make_test_auth("admin_summon_goto");
    let admin_id = pid("admin");
    let wanderer_id = pid("wanderer");
    game_state.add_player(make_player("admin", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("wanderer", 500.0, 0.0))
        .await;
    let mut admin_rx = game_state.register_direct_channel(&admin_id).await;
    let mut wanderer_rx = game_state.register_direct_channel(&wanderer_id).await;

    game_state
        .send_chat_message(&admin_id, "/summon WANDERER".to_string(), &auth)
        .await;
    let (position, _, floor) = game_state
        .get_player_position(&wanderer_id)
        .await
        .expect("wanderer should still be online");
    let distance = (position.x.powi(2) + position.z.powi(2)).sqrt();
    assert!(
        distance <= 2.0,
        "summon must land beside the admin, got {position:?}"
    );
    assert_eq!(floor, 0);
    assert!(
        drain(&mut wanderer_rx).iter().any(|m| matches!(
            m,
            ServerMessage::SystemMessage { message }
                if message == "An operator summoned you to admin's side."
        )),
        "the summoned player must be told what happened"
    );
    assert!(drain(&mut admin_rx).iter().any(|m| matches!(
        m,
        ServerMessage::SystemMessage { message } if message == "Summon: wanderer is at your side."
    )));

    // Send the wanderer far away again, then walk the admin over instead.
    game_state
        .teleport_player(
            &wanderer_id,
            Position {
                x: -300.0,
                y: 0.0,
                z: 40.0,
            },
            0.0,
            0,
        )
        .await;
    game_state
        .send_chat_message(&admin_id, "/goto wanderer".to_string(), &auth)
        .await;
    let (position, _, _) = game_state
        .get_player_position(&admin_id)
        .await
        .expect("admin should still be online");
    let distance = ((position.x + 300.0).powi(2) + (position.z - 40.0).powi(2)).sqrt();
    assert!(
        distance <= 2.0,
        "goto must land beside the wanderer, got {position:?}"
    );
    assert!(drain(&mut admin_rx).iter().any(|m| matches!(
        m,
        ServerMessage::SystemMessage { message } if message == "Goto: you are at wanderer's side."
    )));
}

#[tokio::test]
async fn escape_command_refused_while_in_combat() {
    let game_state = make_test_game_state("escape_in_combat");
    let auth = make_test_auth("escape_in_combat");
    let fighter_id = pid("fighter");
    game_state
        .add_player(make_player("fighter", 20.0, 30.0))
        .await;
    {
        let mut players = game_state.players.write().await;
        players.get_mut(&fighter_id).unwrap().last_combat_at = GameState::now_ms();
    }

    game_state
        .send_chat_message(&fighter_id, "/escape".to_string(), &auth)
        .await;

    let (position, _, _) = game_state
        .get_player_position(&fighter_id)
        .await
        .expect("player should still exist");
    assert_eq!(
        (position.x, position.z),
        (20.0, 30.0),
        "/escape must not double as a combat disengage"
    );
}
