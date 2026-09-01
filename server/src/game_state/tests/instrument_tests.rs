use super::*;
use onlinerpg_shared::messages::{InstrumentNoteEvent, MUSIC_EMOTE};

async fn hand_instrument(game_state: &GameState, player: &str) {
    game_state.inventories.write().await.insert(
        pid(player),
        PlayerInventory {
            bag: vec![bag_item(1, "worn_mandolin", 1)],
            ..Default::default()
        },
    );
}

fn notes() -> Vec<InstrumentNoteEvent> {
    vec![
        InstrumentNoteEvent {
            note: 0,
            offset_ms: 0,
        },
        InstrumentNoteEvent {
            note: 7,
            offset_ms: 80,
        },
        InstrumentNoteEvent {
            note: 21,
            offset_ms: 249,
        },
    ]
}

#[test]
fn instrument_batch_validation_covers_every_wire_bound() {
    assert!(super::super::instrument::valid_instrument_batch(&notes()));
    assert!(!super::super::instrument::valid_instrument_batch(&[]));

    let too_many = vec![
        InstrumentNoteEvent {
            note: 0,
            offset_ms: 0,
        };
        17
    ];
    assert!(!super::super::instrument::valid_instrument_batch(&too_many));
    assert!(super::super::instrument::valid_instrument_batch(
        &too_many[..16]
    ));

    let mut invalid_note = notes();
    invalid_note[1].note = 22;
    assert!(!super::super::instrument::valid_instrument_batch(
        &invalid_note
    ));

    let mut leading_offset = notes();
    leading_offset[0].offset_ms = 1;
    assert!(!super::super::instrument::valid_instrument_batch(
        &leading_offset
    ));

    let mut late = notes();
    late[2].offset_ms = 250;
    assert!(!super::super::instrument::valid_instrument_batch(&late));

    let mut reversed = notes();
    reversed[2].offset_ms = 79;
    assert!(!super::super::instrument::valid_instrument_batch(&reversed));
}

#[tokio::test]
async fn instrument_notes_use_thirty_meter_same_floor_aoi_and_skip_the_player() {
    let game_state = make_test_game_state("live_instrument_aoi");
    let instrumentist = pid("instrumentist");
    let near = pid("near");
    let far = pid("far");
    let upstairs_id = pid("upstairs");

    game_state
        .add_player(make_player("instrumentist", 4.0, 6.0))
        .await;
    game_state.add_player(make_player("near", 33.9, 6.0)).await;
    game_state.add_player(make_player("far", 34.1, 6.0)).await;
    let mut upstairs = make_player("upstairs", 5.0, 6.0);
    upstairs.floor_level = 1;
    game_state.add_player(upstairs).await;
    hand_instrument(&game_state, "instrumentist").await;

    let mut instrumentist_rx = game_state.register_direct_channel(&instrumentist).await;
    let mut near_rx = game_state.register_direct_channel(&near).await;
    let mut far_rx = game_state.register_direct_channel(&far).await;
    let mut upstairs_rx = game_state.register_direct_channel(&upstairs_id).await;

    game_state.start_live_instrument(&instrumentist).await;

    let instrumentist_start = drain(&mut instrumentist_rx);
    assert!(instrumentist_start.iter().any(|message| matches!(
        message,
        ServerMessage::PlayerInteractionChanged { player_id, object_type, .. }
            if *player_id == instrumentist && object_type.as_deref() == Some(MUSIC_EMOTE)
    )));
    assert!(instrumentist_start.iter().any(|message| matches!(
        message,
        ServerMessage::PlayerInstrumentStarted { player_id } if *player_id == instrumentist
    )));

    let near_start = drain(&mut near_rx);
    assert!(near_start.iter().any(|message| matches!(
        message,
        ServerMessage::PlayerInstrumentStarted { player_id } if *player_id == instrumentist
    )));
    assert!(!drain(&mut far_rx)
        .iter()
        .any(|message| matches!(message, ServerMessage::PlayerInstrumentStarted { .. })));
    assert!(drain(&mut upstairs_rx).is_empty());

    let batch = notes();
    game_state
        .play_live_instrument_notes(&instrumentist, batch.clone())
        .await;

    assert!(drain(&mut instrumentist_rx).is_empty());
    match near_rx.try_recv() {
        Ok(ServerMessage::PlayerInstrumentNotes {
            player_id,
            position,
            floor_level,
            events,
        }) => {
            assert_eq!(player_id, instrumentist);
            assert_eq!((position.x, position.y, position.z), (4.0, 0.0, 6.0));
            assert_eq!(floor_level, 0);
            assert_eq!(events, batch);
        }
        other => panic!("Expected nearby instrument notes, got {other:?}"),
    }
    assert!(drain(&mut far_rx).is_empty());
    assert!(drain(&mut upstairs_rx).is_empty());
}

#[tokio::test]
async fn invalid_or_inactive_batches_never_reach_a_listener() {
    let game_state = make_test_game_state("live_instrument_invalid");
    let instrumentist = pid("instrumentist");
    let listener = pid("listener");
    game_state
        .add_player(make_player("instrumentist", 0.0, 0.0))
        .await;
    game_state
        .add_player(make_player("listener", 2.0, 0.0))
        .await;
    hand_instrument(&game_state, "instrumentist").await;
    let mut listener_rx = game_state.register_direct_channel(&listener).await;

    game_state
        .play_live_instrument_notes(&instrumentist, notes())
        .await;
    assert!(drain(&mut listener_rx).is_empty());

    game_state.start_live_instrument(&instrumentist).await;
    drain(&mut listener_rx);
    for events in [
        Vec::new(),
        vec![InstrumentNoteEvent {
            note: 22,
            offset_ms: 0,
        }],
        vec![InstrumentNoteEvent {
            note: 0,
            offset_ms: 250,
        }],
        vec![
            InstrumentNoteEvent {
                note: 0,
                offset_ms: 0,
            },
            InstrumentNoteEvent {
                note: 1,
                offset_ms: 20,
            },
            InstrumentNoteEvent {
                note: 2,
                offset_ms: 19,
            },
        ],
    ] {
        game_state
            .play_live_instrument_notes(&instrumentist, events)
            .await;
    }
    assert!(drain(&mut listener_rx).is_empty());
}

#[tokio::test]
async fn stop_movement_and_instrument_loss_end_the_live_session() {
    let game_state = make_test_game_state("live_instrument_endings");
    let instrumentist = pid("instrumentist");
    let listener = pid("listener");
    game_state
        .add_player(make_player("instrumentist", 0.0, 0.0))
        .await;
    game_state
        .add_player(make_player("listener", 2.0, 0.0))
        .await;
    hand_instrument(&game_state, "instrumentist").await;
    let mut listener_rx = game_state.register_direct_channel(&listener).await;

    game_state.start_live_instrument(&instrumentist).await;
    drain(&mut listener_rx);
    game_state
        .set_player_interaction(&instrumentist, None, None)
        .await;
    assert!(!game_state
        .live_instrument_players
        .read()
        .await
        .contains(&instrumentist));
    drain(&mut listener_rx);

    game_state.start_live_instrument(&instrumentist).await;
    drain(&mut listener_rx);
    game_state
        .update_player_position(&instrumentist, move_cmd(pos(1.0), false), false)
        .await;
    game_state.tick_player_movement(1.0).await;
    assert!(!game_state
        .live_instrument_players
        .read()
        .await
        .contains(&instrumentist));
    drain(&mut listener_rx);

    game_state.start_live_instrument(&instrumentist).await;
    drain(&mut listener_rx);
    game_state.inventories.write().await.remove(&instrumentist);
    game_state
        .play_live_instrument_notes(&instrumentist, notes())
        .await;
    assert!(!game_state
        .live_instrument_players
        .read()
        .await
        .contains(&instrumentist));
    let ended = drain(&mut listener_rx);
    assert!(ended.iter().any(|message| matches!(
        message,
        ServerMessage::PlayerInteractionChanged { player_id, object_type, .. }
            if *player_id == instrumentist && object_type.is_none()
    )));
    assert!(!ended
        .iter()
        .any(|message| matches!(message, ServerMessage::PlayerInstrumentNotes { .. })));
}

#[tokio::test]
async fn play_instrument_command_and_song_playback_are_mutually_exclusive() {
    let game_state = make_test_game_state("live_instrument_chat");
    let auth = make_test_auth("live_instrument_chat");
    let instrumentist = pid("instrumentist");
    game_state
        .add_player(make_player("instrumentist", 0.0, 0.0))
        .await;
    hand_instrument(&game_state, "instrumentist").await;

    game_state
        .send_chat_message(&instrumentist, "/play_instrument".to_string(), &auth)
        .await;
    assert!(game_state
        .live_instrument_players
        .read()
        .await
        .contains(&instrumentist));
    assert!(!game_state
        .music_performances
        .read()
        .await
        .contains_key(&instrumentist));

    game_state
        .send_chat_message(&instrumentist, "/play_music twilight".to_string(), &auth)
        .await;
    assert!(!game_state
        .live_instrument_players
        .read()
        .await
        .contains(&instrumentist));
    assert!(game_state
        .music_performances
        .read()
        .await
        .contains_key(&instrumentist));

    game_state.start_live_instrument(&instrumentist).await;
    assert!(game_state
        .live_instrument_players
        .read()
        .await
        .contains(&instrumentist));
    assert!(!game_state
        .music_performances
        .read()
        .await
        .contains_key(&instrumentist));

    game_state
        .cancel_concentration_if_active(&instrumentist)
        .await;
    assert!(!game_state
        .live_instrument_players
        .read()
        .await
        .contains(&instrumentist));
}

#[tokio::test]
async fn changing_equipment_ends_the_live_session() {
    let game_state = make_test_game_state("live_instrument_equip");
    let instrumentist = pid("instrumentist");
    game_state
        .add_player(make_player("instrumentist", 0.0, 0.0))
        .await;
    game_state.inventories.write().await.insert(
        instrumentist,
        PlayerInventory {
            bag: vec![
                bag_item(1, "worn_mandolin", 1),
                bag_item(2, "iron_sword", 1),
            ],
            ..Default::default()
        },
    );

    game_state.start_live_instrument(&instrumentist).await;
    assert!(game_state
        .live_instrument_players
        .read()
        .await
        .contains(&instrumentist));

    game_state.equip_item(&instrumentist, 2).await;
    assert!(!game_state
        .live_instrument_players
        .read()
        .await
        .contains(&instrumentist));
}

#[tokio::test]
async fn listeners_who_blocked_the_performer_hear_no_notes() {
    let game_state = make_test_game_state("live_instrument_blocked");
    let performer = pid("performer");
    let fan = pid("fan");
    let hater = pid("hater");

    game_state
        .add_player(make_player("performer", 4.0, 6.0))
        .await;
    game_state.add_player(make_player("fan", 6.0, 6.0)).await;
    game_state.add_player(make_player("hater", 8.0, 6.0)).await;
    hand_instrument(&game_state, "performer").await;
    game_state
        .set_player_blocks(&hater, vec!["performer".to_string()])
        .await;

    let mut fan_rx = game_state.register_direct_channel(&fan).await;
    let mut hater_rx = game_state.register_direct_channel(&hater).await;

    game_state.start_live_instrument(&performer).await;
    drain(&mut fan_rx);
    drain(&mut hater_rx);

    game_state
        .play_live_instrument_notes(&performer, notes())
        .await;

    assert!(drain(&mut fan_rx)
        .iter()
        .any(|message| matches!(message, ServerMessage::PlayerInstrumentNotes { .. })));
    assert!(drain(&mut hater_rx).is_empty());
}
