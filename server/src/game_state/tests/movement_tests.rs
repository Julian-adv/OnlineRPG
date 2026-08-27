use super::*;

#[tokio::test]
async fn player_aoi_crosses_world_x_seam() {
    let game_state = make_test_game_state("player_aoi_x_wrap");
    let east_id = pid("east_player");
    let west_id = pid("west_player");

    game_state
        .add_player(make_player(
            "east_player",
            onlinerpg_shared::WORLD_MAX_X - 1.0,
            0.0,
        ))
        .await;
    game_state
        .add_player(make_player(
            "west_player",
            onlinerpg_shared::WORLD_MIN_X + 1.0,
            0.0,
        ))
        .await;

    let nearby = game_state
        .player_ids_within(&east_id, onlinerpg_shared::NPC_SIGHT_RADIUS)
        .await;
    assert!(nearby.contains(&east_id));
    assert!(nearby.contains(&west_id));
}

#[tokio::test]
async fn movement_into_aoi_sends_existing_monsters_and_ground_items() {
    let game_state = make_test_game_state("movement_world_entity_aoi");
    let player_id = pid("walker");
    let entity_position = Position {
        x: 50.0,
        y: 0.0,
        z: 0.0,
    };

    game_state.add_player(make_player("walker", 0.0, 0.0)).await;
    let mut direct_rx = game_state.register_direct_channel(&player_id).await;

    {
        let mut monsters = game_state.monsters.write().await;
        monsters.insert(
            "monster_a".to_string(),
            make_monster("monster_a", entity_position, 0),
        );
    }

    {
        let mut ground_items = game_state.ground_items.write().await;
        ground_items.insert(
            42,
            ServerGroundItem {
                item: GroundItem {
                    instance_id: 42,
                    item_def_id: "test_item".to_string(),
                    position: entity_position,
                    floor_level: 0,
                    quantity: 1,
                    enchant: 0,
                    dropped_by: None,
                    cape_color: None,
                    cape_texture: None,
                },
                dropped_at_ms: 0,
            },
        );
    }

    game_state
        .update_player_position(&player_id, move_cmd(entity_position, false), false)
        .await;
    game_state.tick_player_movement(60.0).await;

    match direct_rx.try_recv() {
        Ok(ServerMessage::MonsterSpawned { monster }) => {
            assert_eq!(monster.id, "monster_a");
        }
        other => panic!("Expected MonsterSpawned when entering AOI, got {:?}", other),
    }

    // The monster has no owner, so walking up to it adopts it on sight.
    match direct_rx.try_recv() {
        Ok(ServerMessage::MonsterAssigned { monster }) => {
            assert_eq!(monster.id, "monster_a");
        }
        other => panic!(
            "Expected MonsterAssigned for the ownerless monster, got {:?}",
            other
        ),
    }

    match direct_rx.try_recv() {
        Ok(ServerMessage::GroundItemAppeared { item }) => {
            assert_eq!(item.instance_id, 42);
        }
        other => panic!(
            "Expected GroundItemAppeared when entering AOI, got {:?}",
            other
        ),
    }

    match direct_rx.try_recv() {
        Ok(ServerMessage::PlayerMoved {
            player_id: moved_id,
            ..
        }) => {
            assert_eq!(moved_id, player_id);
        }
        other => panic!(
            "Expected self PlayerMoved after AOI snapshot, got {:?}",
            other
        ),
    }
}

/// The AOI diff visits only the monsters the cell index reports near the step,
/// so a monster's index entry has to follow its moves. The arrival point below
/// is picked to query the monster's new cell but not the one it left: with the
/// index left behind, the walker never hears about it.
#[tokio::test]
async fn movement_into_aoi_sends_a_monster_that_walked_there() {
    let game_state = make_test_game_state("moved_monster_aoi");
    let walker_id = pid("walker");
    let owner_id = pid("monster_owner");
    // All three spots hang off the AOI: the monster walks from outside it to a
    // spot the walker only reaches after moving.
    let aoi = onlinerpg_shared::EVENT_DELIVERY_RADIUS;
    let start = Position {
        x: 2.0 * aoi + 1.0,
        y: 0.0,
        z: 0.0,
    };
    let walked_to = Position {
        x: 2.0 * aoi - 6.0,
        y: 0.0,
        z: 0.0,
    };

    game_state.add_player(make_player("walker", 0.0, 0.0)).await;
    {
        let mut monsters = game_state.monsters.write().await;
        let mut monster = make_monster("wanderer", start, 0);
        monster.owner_id = Some(owner_id);
        monster.move_budget = 10.0;
        monster.last_move_at = GameState::now_ms();
        monsters.insert(monster.id.clone(), monster);
    }
    game_state
        .update_monster_position(
            &owner_id,
            "wanderer".to_string(),
            walked_to,
            0.0,
            MonsterState::Run,
            walked_to,
        )
        .await;
    assert_eq!(
        game_state.monsters.read().await["wanderer"].position.x,
        walked_to.x,
        "the move must be accepted for this to test anything"
    );

    let mut direct_rx = game_state.register_direct_channel(&walker_id).await;
    game_state
        .update_player_position(&walker_id, move_cmd(pos(aoi - 3.0), false), false)
        .await;
    game_state.tick_player_movement(60.0).await;

    let spawned = drain(&mut direct_rx).into_iter().any(
        |msg| matches!(msg, ServerMessage::MonsterSpawned { monster } if monster.id == "wanderer"),
    );
    assert!(
        spawned,
        "a monster that walked into range must be announced"
    );
}

#[tokio::test]
async fn player_movement_wraps_across_east_world_edge() {
    let game_state = make_test_game_state("movement_x_wrap");
    let player_id = pid("world_wrap_walker");
    game_state
        .add_player(make_player(
            "world_wrap_walker",
            onlinerpg_shared::WORLD_MAX_X - 0.25,
            0.0,
        ))
        .await;
    let mut direct_rx = game_state.register_direct_channel(&player_id).await;

    game_state
        .update_player_position(
            &player_id,
            MoveCommand {
                position: Position {
                    x: onlinerpg_shared::WORLD_MAX_X + 0.25,
                    y: 12.0,
                    z: 3.0,
                },
                rotation: 0.5,
                floor_level: 0,
                append: false,
                sprinting: false,
            },
            false,
        )
        .await;
    game_state.tick_player_movement(60.0).await;

    let players = game_state.get_all_players().await;
    let wrapped = &players[&player_id];
    assert_eq!(wrapped.position.x, onlinerpg_shared::WORLD_MIN_X + 0.25);

    match direct_rx.try_recv() {
        Ok(ServerMessage::PlayerMoved { position, .. }) => {
            assert_eq!(position.x, onlinerpg_shared::WORLD_MIN_X + 0.25);
        }
        other => panic!("Expected wrapped self PlayerMoved, got {other:?}"),
    }
}

#[tokio::test]
async fn seam_crossing_movement_checks_destination_edge_collision() {
    let game_state = make_test_game_state("movement_seam_collision");
    let player_id = pid("seam_walker");
    game_state
        .add_player(make_player(
            "seam_walker",
            onlinerpg_shared::WORLD_MAX_X - 0.5,
            5.5,
        ))
        .await;
    game_state.sync_region_furniture(
        -16,
        0,
        &[table_placement(onlinerpg_shared::WORLD_MIN_X + 0.5, 5.5)],
    );

    game_state
        .update_player_position(
            &player_id,
            move_cmd(
                Position {
                    x: onlinerpg_shared::WORLD_MIN_X + 1.5,
                    y: 0.0,
                    z: 5.5,
                },
                false,
            ),
            false,
        )
        .await;
    game_state.tick_player_movement(60.0).await;

    assert_eq!(
        player_xz(&game_state, &player_id).await,
        (onlinerpg_shared::WORLD_MAX_X - 0.5, 5.5)
    );
}

async fn player_x(game_state: &GameState, player_id: &PlayerId) -> f32 {
    player_xz(game_state, player_id).await.0
}

#[tokio::test]
async fn server_caps_player_movement_speed() {
    let game_state = make_test_game_state("movement_speed_cap");
    let player_id = pid("runner");
    game_state.add_player(make_player("runner", 0.0, 0.0)).await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(50.0), false), false)
        .await;

    assert_eq!(player_x(&game_state, &player_id).await, 0.0);

    game_state.tick_player_movement(1.0).await;
    let after_one_second = player_x(&game_state, &player_id).await;
    assert!(after_one_second > 2.0 && after_one_second < 4.0);

    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 50.0);
}

#[tokio::test]
async fn one_tick_budget_spans_queued_legs() {
    let game_state = make_test_game_state("movement_queue_budget");
    let player_id = pid("pathwalker");
    game_state
        .add_player(make_player("pathwalker", 0.0, 0.0))
        .await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(1.0), false), false)
        .await;
    for x in [2.0, 3.0] {
        game_state
            .update_player_position(&player_id, move_cmd(pos(x), true), false)
            .await;
    }

    // Budget 1.5m: leg 1 consumed whole, leg 2 partially — the budget holds
    // across legs, not per leg.
    game_state.tick_player_movement(0.5).await;
    let mid = player_x(&game_state, &player_id).await;
    assert!((1.5..2.1).contains(&mid), "mid was {mid}");

    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 3.0);
}

#[tokio::test]
async fn append_distance_guard_measures_from_queue_tail() {
    let game_state = make_test_game_state("movement_queue_tail_guard");
    let player_id = pid("longhauler");
    game_state
        .add_player(make_player("longhauler", 0.0, 0.0))
        .await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(50.0), false), false)
        .await;
    // 100 is >60m from the player but only 50m from the queue tail: accepted.
    game_state
        .update_player_position(&player_id, move_cmd(pos(100.0), true), false)
        .await;
    // 70m from the new tail: rejected.
    game_state
        .update_player_position(&player_id, move_cmd(pos(170.0), true), false)
        .await;

    game_state.tick_player_movement(600.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 100.0);
}

#[tokio::test]
async fn replace_drops_queued_waypoints() {
    let game_state = make_test_game_state("movement_queue_replace");
    let player_id = pid("rerouter");
    game_state
        .add_player(make_player("rerouter", 0.0, 0.0))
        .await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(10.0), false), false)
        .await;
    game_state
        .update_player_position(&player_id, move_cmd(pos(20.0), true), false)
        .await;
    game_state
        .update_player_position(&player_id, move_cmd(pos(5.0), false), false)
        .await;

    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 5.0);
}

#[tokio::test]
async fn full_waypoint_queue_drops_oldest_leg() {
    let game_state = make_test_game_state("movement_queue_cap");
    let player_id = pid("spammer");
    game_state
        .add_player(make_player("spammer", 0.0, 0.0))
        .await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(1.0), false), false)
        .await;
    for i in 2..=40 {
        game_state
            .update_player_position(&player_id, move_cmd(pos(i as f32), true), false)
            .await;
    }

    // Overflow evicts from the front, so the tail survives and the sim still
    // reaches the client's final position (a reject-newest policy would strand
    // the player at 32).
    game_state.tick_player_movement(600.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 40.0);
}

#[tokio::test]
async fn non_finite_move_is_rejected() {
    let game_state = make_test_game_state("movement_nan_reject");
    let player_id = pid("glitcher");
    game_state
        .add_player(make_player("glitcher", 1.0, 1.0))
        .await;

    for (bad, _) in NON_FINITE {
        game_state
            .update_player_position(&player_id, move_cmd(pos(bad), false), false)
            .await;
    }
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 1.0);
}

#[tokio::test]
async fn positive_floor_above_limit_is_rejected() {
    let game_state = make_test_game_state("positive_floor_limit");
    let player_id = pid("ghost");
    let observer_id = pid("observer");
    let max_floor = onlinerpg_shared::housing::MAX_FLOOR_LEVEL as i8;
    game_state.add_player(make_player("ghost", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("observer", 1.0, 0.0))
        .await;
    let mut observer_rx = game_state.register_direct_channel(&observer_id).await;

    for invalid_floor in [max_floor + 1, 120] {
        game_state
            .update_player_floor(&player_id, invalid_floor)
            .await;
        assert_eq!(
            game_state.players.read().await[&player_id].floor_level,
            0,
            "floor {invalid_floor} must be rejected"
        );
        assert!(matches!(
            observer_rx.try_recv(),
            Err(MpscTryRecvError::Empty)
        ));
    }
}

#[tokio::test]
async fn player_move_cannot_bypass_positive_floor_limit() {
    let game_state = make_test_game_state("positive_floor_move_limit");
    let player_id = pid("ghost");
    let observer_id = pid("observer");
    let max_floor = onlinerpg_shared::housing::MAX_FLOOR_LEVEL as i8;
    game_state.add_player(make_player("ghost", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("observer", 1.0, 0.0))
        .await;
    let mut observer_rx = game_state.register_direct_channel(&observer_id).await;

    game_state
        .update_player_position(
            &player_id,
            MoveCommand {
                position: pos(1.0),
                rotation: 0.0,
                floor_level: max_floor + 1,
                append: false,
                sprinting: false,
            },
            false,
        )
        .await;

    assert!(
        !game_state
            .movement_intents
            .read()
            .await
            .contains_key(&player_id),
        "an invalid floor move must not enqueue an intent"
    );
    let player = game_state.players.read().await[&player_id].clone();
    assert_eq!(player.position.x, 0.0);
    assert_eq!(player.floor_level, 0);
    assert!(matches!(
        observer_rx.try_recv(),
        Err(MpscTryRecvError::Empty)
    ));

    game_state
        .update_player_position(&player_id, move_cmd(pos(1.0), false), false)
        .await;
    game_state.tick_player_movement(1.0).await;
    let player = game_state.players.read().await[&player_id].clone();
    assert_eq!(player.position.x, 1.0);
    assert_eq!(player.floor_level, 0);
}

#[tokio::test]
async fn a_rejected_floor_change_snaps_the_client_back() {
    let game_state = make_test_game_state("positive_floor_correction");
    let player_id = pid("ghost");
    let max_floor = onlinerpg_shared::housing::MAX_FLOOR_LEVEL as i8;
    game_state.add_player(make_player("ghost", 2.0, 3.0)).await;
    let mut rx = game_state.register_direct_channel(&player_id).await;

    game_state
        .update_player_floor(&player_id, max_floor + 1)
        .await;
    let (position, _, floor_level) =
        first_correction(&mut rx).expect("a rejected floor must snap the client back");
    assert_eq!(floor_level, 0);
    assert_eq!((position.x, position.z), (2.0, 3.0));

    // The snap rides the refused-move throttle: a client that keeps reporting
    // the bad floor is not yanked once per packet.
    game_state
        .update_player_position(
            &player_id,
            MoveCommand {
                floor_level: max_floor + 1,
                ..move_cmd(pos(1.0), false)
            },
            false,
        )
        .await;
    assert!(first_correction(&mut rx).is_none());
}

#[tokio::test]
async fn a_rejected_move_floor_snaps_the_client_back() {
    let game_state = make_test_game_state("positive_floor_move_correction");
    let player_id = pid("ghost");
    let max_floor = onlinerpg_shared::housing::MAX_FLOOR_LEVEL as i8;
    game_state.add_player(make_player("ghost", 2.0, 3.0)).await;
    let mut rx = game_state.register_direct_channel(&player_id).await;

    game_state
        .update_player_position(
            &player_id,
            MoveCommand {
                floor_level: max_floor + 1,
                ..move_cmd(pos(1.0), false)
            },
            false,
        )
        .await;
    let (position, _, floor_level) =
        first_correction(&mut rx).expect("a rejected move floor must snap the client back");
    assert_eq!(floor_level, 0);
    assert_eq!((position.x, position.z), (2.0, 3.0));
}

#[test]
fn restored_floor_falls_back_to_surface() {
    let max_floor = onlinerpg_shared::housing::MAX_FLOOR_LEVEL as i8;

    for saved in [max_floor + 1, 120, i8::MAX] {
        assert_eq!(
            super::restored_floor_level(saved),
            0,
            "floor {saved} must fall back to the surface"
        );
    }

    // Negative floors use the existing dungeon rehydration path.
    for saved in [i8::MIN, -1, 0, max_floor] {
        assert_eq!(
            super::restored_floor_level(saved),
            saved,
            "floor {saved} must be restored as-is"
        );
    }
}

#[tokio::test]
async fn far_move_target_is_rejected() {
    let game_state = make_test_game_state("movement_far_reject");
    let player_id = pid("warper");
    game_state.add_player(make_player("warper", 0.0, 0.0)).await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(100.0), false), false)
        .await;
    game_state.tick_player_movement(600.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 0.0);
}

#[tokio::test]
async fn teleport_clears_pending_move_intent() {
    let game_state = make_test_game_state("movement_teleport_clears");
    let player_id = pid("traveler");
    game_state
        .add_player(make_player("traveler", 0.0, 0.0))
        .await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(50.0), false), false)
        .await;
    game_state
        .teleport_player(
            &player_id,
            Position {
                x: 5.0,
                y: 0.0,
                z: 5.0,
            },
            0.0,
            0,
        )
        .await;
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 5.0);
}

#[tokio::test]
async fn teleport_rejects_non_finite_position() {
    let game_state = make_test_game_state("movement_teleport_non_finite");
    let player_id = pid("gm");
    game_state.add_player(make_player("gm", 3.0, 4.0)).await;

    for bad_x in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
        game_state
            .teleport_player(
                &player_id,
                Position {
                    x: bad_x,
                    y: 0.0,
                    z: 0.0,
                },
                0.0,
                0,
            )
            .await;
        assert_eq!(player_x(&game_state, &player_id).await, 3.0);
    }
}

#[tokio::test]
async fn rejected_far_move_snaps_client_back() {
    let game_state = make_test_game_state("movement_far_reject_snap");
    let player_id = pid("phantom");
    game_state
        .add_player(make_player("phantom", 0.0, 0.0))
        .await;
    let mut direct_rx = game_state.register_direct_channel(&player_id).await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(100.0), false), false)
        .await;

    let corrections = |msgs: Vec<ServerMessage>| {
        msgs.into_iter()
            .filter(|m| matches!(m, ServerMessage::PositionCorrected { .. }))
            .count()
    };
    assert_eq!(corrections(drain(&mut direct_rx)), 1);

    // A client that keeps predicting along a rejected path resends every
    // second; the snap rides the correction throttle instead of matching it.
    game_state
        .update_player_position(&player_id, move_cmd(pos(101.0), false), false)
        .await;
    assert_eq!(corrections(drain(&mut direct_rx)), 0);
}

#[tokio::test]
async fn implausible_dungeon_floor_move_is_refused_and_snapped() {
    let game_state = make_test_game_state("movement_floor_coerce_snap");
    let player_id = pid("faller");
    game_state.add_player(make_player("faller", 0.0, 0.0)).await;
    let mut rx = game_state.register_direct_channel(&player_id).await;

    // No dungeon footprint at the origin, so the claimed floor is coerced;
    // the mover must be told rather than left predicting floor -1.
    game_state
        .update_player_position(
            &player_id,
            MoveCommand {
                floor_level: -1,
                ..move_cmd(pos(1.0), false)
            },
            false,
        )
        .await;

    let (position, _, floor_level) =
        first_correction(&mut rx).expect("a coerced floor must snap the client back");
    assert_eq!(floor_level, 0);
    assert_eq!((position.x, position.z), (0.0, 0.0));
    assert!(
        !game_state
            .movement_intents
            .read()
            .await
            .contains_key(&player_id),
        "a refused floor move must not enqueue an intent"
    );
}

#[tokio::test]
async fn plausible_dungeon_floor_move_is_not_snapped() {
    use onlinerpg_shared::dungeon::{floor_height_at, generate_dungeon_for};

    let game_state = make_test_game_state("movement_floor_plausible");
    let entrance = first_dungeon(&game_state);
    let player_id = pid("delver");
    game_state
        .add_player(make_player("delver", entrance.x, entrance.z))
        .await;
    let mut rx = game_state.register_direct_channel(&player_id).await;

    // The ground Y an honest client reports for the claimed floor.
    let layouts = generate_dungeon_for(&entrance.id);
    let (x, z) = (entrance.x + 1.0, entrance.z);
    let y = floor_height_at(&entrance.position(), &layouts, 1, x, z).expect("depth 1 exists");
    game_state
        .update_player_position(
            &player_id,
            MoveCommand {
                floor_level: -1,
                ..move_cmd(Position { x, y, z }, false)
            },
            false,
        )
        .await;

    assert!(
        first_correction(&mut rx).is_none(),
        "a valid descent must not be snapped"
    );
    assert!(game_state
        .movement_intents
        .read()
        .await
        .contains_key(&player_id));
}

/// Keyboard descents stream mid-ramp positions, and the client flips its
/// claimed floor near the top of the shaft — long before the flat floor Y is
/// within tolerance. Those honest packets must validate, not snap.
#[tokio::test]
async fn mid_ramp_descent_claim_is_not_snapped() {
    use onlinerpg_shared::dungeon::{dungeon_origin, floor_height_at, generate_dungeon_for};

    let game_state = make_test_game_state("movement_floor_mid_ramp");
    let entrance = first_dungeon(&game_state);
    let layouts = generate_dungeon_for(&entrance.id);
    let e_pos = entrance.position();
    let (ox, oz) = dungeon_origin(entrance.x, entrance.z);
    // A point on the entrance shaft just past the client's depth-switch
    // fraction: barely descended, so far from floor -1's flat Y.
    let (x, y, z) = (0..80)
        .flat_map(|i| (0..80).map(move |j| (i, j)))
        .find_map(|(i, j)| {
            let (x, z) = (ox + i as f32 + 0.5, oz + j as f32 + 0.5);
            let y = floor_height_at(&e_pos, &layouts, 1, x, z)?;
            let descended = entrance.y - y;
            (descended > 0.7 && descended < 1.4).then_some((x, y, z))
        })
        .expect("a mid-ramp cell on the entrance shaft");

    let player_id = pid("stepper");
    game_state.add_player(make_player("stepper", x, z)).await;
    let mut rx = game_state.register_direct_channel(&player_id).await;

    game_state
        .update_player_position(
            &player_id,
            MoveCommand {
                floor_level: -1,
                ..move_cmd(Position { x, y, z }, false)
            },
            false,
        )
        .await;

    assert!(
        first_correction(&mut rx).is_none(),
        "an honest mid-ramp packet must not be snapped"
    );
    assert!(game_state
        .movement_intents
        .read()
        .await
        .contains_key(&player_id));
}

#[tokio::test]
async fn dead_player_move_is_refused_and_snapped() {
    let game_state = make_test_game_state("movement_dead_reject_snap");
    let player_id = pid("corpse");
    game_state.add_player(make_player("corpse", 0.0, 0.0)).await;
    game_state
        .players
        .write()
        .await
        .get_mut(&player_id)
        .unwrap()
        .health = 0;
    let mut direct_rx = game_state.register_direct_channel(&player_id).await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(10.0), false), false)
        .await;

    assert!(drain(&mut direct_rx)
        .iter()
        .any(|m| matches!(m, ServerMessage::PositionCorrected { .. })));
    assert!(game_state
        .movement_intents
        .read()
        .await
        .get(&player_id)
        .is_none());
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 0.0);
}

/// `SpatialCell::within_radius` is a conservative superset: every point within
/// the radius must land in one of the cells it yields. Under-scanning is
/// silent — the ownership sweep would despawn a monster a player is standing
/// next to — so pin the property directly rather than through a behaviour.
#[test]
fn within_radius_covers_every_point_inside_the_radius() {
    use onlinerpg_shared::{wrap_world_x, WORLD_MIN_X, WORLD_WIDTH_X};

    let radius = super::super::EVENT_DELIVERY_RADIUS;
    let mut seed = 0x5EED_F00Du64;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as f32 / (u32::MAX as f32 / 2.0)
    };

    // Both world edges, the middle, and random interior points: the seam is
    // where a naive implementation breaks.
    let centers: Vec<f32> = [
        WORLD_MIN_X,
        WORLD_MIN_X + 1.0,
        WORLD_MIN_X + radius,
        0.0,
        WORLD_MIN_X + WORLD_WIDTH_X - radius,
        WORLD_MIN_X + WORLD_WIDTH_X - 1.0,
    ]
    .into_iter()
    .chain((0..40).map(|_| WORLD_MIN_X + next() * WORLD_WIDTH_X))
    .collect();

    for center_x in centers {
        let center = Position {
            x: wrap_world_x(center_x),
            y: 0.0,
            z: next() * 400.0 - 200.0,
        };
        let cells: std::collections::HashSet<_> =
            super::super::SpatialCell::within_radius(&center, radius).collect();

        for _ in 0..80 {
            // Points ringing the boundary, inside and just outside.
            let angle = next() * std::f32::consts::TAU;
            let dist = next() * radius * 1.2;
            let probe = Position {
                x: wrap_world_x(center.x + angle.cos() * dist),
                y: 0.0,
                z: center.z + angle.sin() * dist,
            };
            if center.dist_xz_sq(&probe) > radius * radius {
                continue;
            }
            assert!(
                cells.contains(&super::super::SpatialCell::from_position(&probe)),
                "{:?} is {:.1}m from {:?} but its cell was not enumerated",
                probe,
                center.dist_xz_sq(&probe).sqrt(),
                center
            );
        }
    }
}

mod dungeon_floor_gate {
    use super::*;
    use onlinerpg_shared::dungeon::{
        cell_center, floor_height_at, generate_dungeon_for, FloorLayout, Room,
        DUNGEON_FLOOR_HEIGHT, GRID, SHAFT_CHANGE_MARGIN,
    };

    /// The shaft joining depth `shallow` to the floor below it.
    fn shaft_rect(layouts: &[FloorLayout], shallow: u8) -> Room {
        layouts[shallow as usize].up_shaft.rect()
    }

    /// First carved cell of `layout` (row-major) passing `pred`.
    fn carved_cell(layout: &FloorLayout, pred: impl Fn(i32, i32) -> bool) -> (i32, i32) {
        (0..GRID)
            .flat_map(|z| (0..GRID).map(move |x| (x, z)))
            .find(|&(x, z)| layout.is_carved(x, z) && pred(x, z))
            .expect("a carved cell")
    }

    /// A carved cell on `depth` clear of every shaft's margin, and at least
    /// `min_cells` (Chebyshev) from the floor's down shaft.
    fn room_cell_off_stairs(layouts: &[FloorLayout], depth: u8, min_cells: i32) -> (i32, i32) {
        let layout = &layouts[depth as usize - 1];
        let clear = SHAFT_CHANGE_MARGIN + 2;
        let mut shafts = vec![layout.up_shaft.rect().expanded(clear)];
        shafts.extend(layout.down_shaft.map(|s| s.rect().expanded(clear)));
        let down = layout.down_shaft.expect("a floor below").rect().center();
        carved_cell(layout, |x, z| {
            !shafts.iter().any(|r| r.contains(x, z))
                && (x - down.0).abs().max((z - down.1).abs()) >= min_cells
        })
    }

    /// A carved cell on `depth` inside the down shaft's change margin but off
    /// its footprint, carved on the floor below too — where a stair descent's
    /// racing floor-change message lands.
    fn room_cell_beside_stairs(layouts: &[FloorLayout], depth: u8) -> (i32, i32) {
        let shaft = layouts[depth as usize - 1]
            .down_shaft
            .expect("a floor below")
            .rect();
        let margin = shaft.expanded(SHAFT_CHANGE_MARGIN);
        let below = &layouts[depth as usize];
        carved_cell(&layouts[depth as usize - 1], |x, z| {
            margin.contains(x, z) && !shaft.contains(x, z) && below.is_carved(x, z)
        })
    }

    /// A carved cell on `depth` within a short leg of the down shaft, but
    /// outside its margin.
    fn room_cell_near_stairs(layouts: &[FloorLayout], depth: u8) -> (i32, i32) {
        let layout = &layouts[depth as usize - 1];
        let shaft = layout.down_shaft.expect("a floor below").rect();
        let near = shaft.expanded(SHAFT_CHANGE_MARGIN + 3);
        let margin = shaft.expanded(SHAFT_CHANGE_MARGIN + 1);
        carved_cell(layout, |x, z| near.contains(x, z) && !margin.contains(x, z))
    }

    struct Delver {
        game_state: GameState,
        player_id: PlayerId,
        rx: DirectRx,
        entrance: Position,
        layouts: Vec<FloorLayout>,
        cell: (i32, i32),
    }

    /// A player standing on `depth` at `cell`, as a stair descent leaves them.
    async fn delver(tag: &str, depth: u8, cell: impl Fn(&[FloorLayout]) -> (i32, i32)) -> Delver {
        let game_state = make_test_game_state(tag);
        let def = first_dungeon(&game_state);
        let entrance = def.position();
        let layouts = generate_dungeon_for(&def.id);
        let cell = cell(&layouts);
        let at = cell_center(&entrance, depth, cell);
        let mut player = make_player("delver", at.x, at.z);
        player.position.y = at.y;
        player.floor_level = -(depth as i8);
        game_state.add_player(player).await;
        let player_id = pid("delver");
        let rx = game_state.register_direct_channel(&player_id).await;
        Delver {
            game_state,
            player_id,
            rx,
            entrance,
            layouts,
            cell,
        }
    }

    impl Delver {
        fn ground(&self, depth: u8, cell: (i32, i32)) -> Position {
            let p = cell_center(&self.entrance, depth, cell);
            let y = floor_height_at(&self.entrance, &self.layouts, depth, p.x, p.z).unwrap();
            Position { y, ..p }
        }

        async fn send_move(&self, position: Position, floor_level: i8) {
            self.game_state
                .update_player_position(
                    &self.player_id,
                    MoveCommand {
                        floor_level,
                        ..move_cmd(position, false)
                    },
                    false,
                )
                .await;
        }

        async fn floor(&self) -> i8 {
            self.game_state.players.read().await[&self.player_id].floor_level
        }

        async fn queued_target(&self) -> Option<(Position, i8)> {
            self.game_state
                .movement_intents
                .read()
                .await
                .get(&self.player_id)
                .and_then(|q| q.back())
                .map(|w| (w.target, w.floor_level))
        }
    }

    #[tokio::test]
    async fn floor_change_off_the_stairs_is_refused_and_snapped() {
        let mut d = delver("gate_off_stairs_move", 1, |l| room_cell_off_stairs(l, 1, 0)).await;
        // Same spot, the floor below's flat Y: a forged descent through rock.
        d.send_move(d.ground(2, d.cell), -2).await;

        let (_, _, floor) = first_correction(&mut d.rx).expect("a refused floor change snaps");
        assert_eq!(floor, -1);
        assert!(d.queued_target().await.is_none());
    }

    #[tokio::test]
    async fn floor_change_message_off_the_stairs_is_ignored() {
        let mut d = delver("gate_off_stairs_msg", 1, |l| room_cell_off_stairs(l, 1, 0)).await;
        d.game_state.update_player_floor(&d.player_id, -2).await;
        assert_eq!(d.floor().await, -1);
        // The client renders the floor it asked for and re-announces it every
        // frame, so a refusal has to reach it.
        let (_, _, floor) =
            first_correction(&mut d.rx).expect("a refused floor change must correct the client");
        assert_eq!(floor, -1);
        d.game_state.update_player_floor(&d.player_id, 0).await;
        assert_eq!(d.floor().await, -1);
    }

    #[tokio::test]
    async fn floor_change_on_the_shaft_is_accepted() {
        let d = delver("gate_on_shaft_msg", 1, |l| shaft_rect(l, 1).center()).await;
        d.game_state.update_player_floor(&d.player_id, -2).await;
        assert_eq!(d.floor().await, -2);
    }

    /// The floor-change message races the walk, landing while the stored Y
    /// still reads the departing floor's ground. Beside the shaft that is a
    /// mid-transition claim: accepted, with Y snapped to the claimed floor —
    /// refusing latched the player on the old floor, cut off from the new
    /// floor's broadcasts.
    #[tokio::test]
    async fn floor_change_beside_the_shaft_with_departing_y_is_accepted() {
        let d = delver("gate_beside_shaft_msg", 1, |l| {
            room_cell_beside_stairs(l, 1)
        })
        .await;
        d.game_state.update_player_floor(&d.player_id, -2).await;
        assert_eq!(d.floor().await, -2);
        let snapped = d.ground(2, d.cell).y;
        let y = d.game_state.players.read().await[&d.player_id].position.y;
        assert!(
            (y - snapped).abs() < 0.01,
            "Y must snap to the claimed floor"
        );
    }

    /// A shaft's ramp is shared geometry, so a stored Y trailing the climb
    /// matches neither floor. Refusing on it latched the climber below.
    #[tokio::test]
    async fn floor_change_on_the_shaft_survives_a_trailing_y() {
        let d = delver("gate_shaft_trailing_y", 2, |l| shaft_rect(l, 1).center()).await;
        let ramp_y = d.ground(1, d.cell).y;
        {
            let mut players = d.game_state.players.write().await;
            players.get_mut(&d.player_id).unwrap().position.y = ramp_y - DUNGEON_FLOOR_HEIGHT;
        }

        d.game_state.update_player_floor(&d.player_id, -1).await;

        assert_eq!(d.floor().await, -1);
        let y = d.game_state.players.read().await[&d.player_id].position.y;
        assert!(
            (y - ramp_y).abs() < 0.01,
            "Y must snap to the ramp under the player, got {y} for {ramp_y}"
        );
    }

    #[tokio::test]
    async fn short_leg_through_the_shaft_may_change_floor() {
        let mut d = delver("gate_leg_through_shaft", 1, |l| room_cell_near_stairs(l, 1)).await;
        let shaft = shaft_rect(&d.layouts, 1);
        d.send_move(d.ground(2, shaft.center()), -2).await;
        assert!(first_correction(&mut d.rx).is_none());
        assert_eq!(d.queued_target().await.map(|(_, f)| f), Some(-2));
    }

    /// Y is interpolated along the whole leg, so a long one clipping the
    /// shaft would cross onto the other floor's grid far from the stairs.
    #[tokio::test]
    async fn long_leg_through_the_shaft_is_refused() {
        let mut d = delver("gate_long_leg", 1, |l| room_cell_off_stairs(l, 1, 20)).await;
        let shaft = shaft_rect(&d.layouts, 1);
        let target = d.ground(2, shaft.center());
        let leg = cell_center(&d.entrance, 1, d.cell)
            .dist_xz_sq(&target)
            .sqrt();
        assert!(
            leg < onlinerpg_shared::MAX_MOVE_TARGET_DISTANCE,
            "the leg must be refused by the stair gate, not the distance guard"
        );
        d.send_move(target, -2).await;
        assert_eq!(first_correction(&mut d.rx).map(|c| c.2), Some(-1));
        assert!(d.queued_target().await.is_none());
    }

    #[tokio::test]
    async fn depth_jump_is_refused_even_on_the_shaft() {
        let d = delver("gate_depth_jump", 1, |l| shaft_rect(l, 1).center()).await;
        assert!(d.layouts.len() >= 3, "test dungeon needs three floors");
        d.game_state.update_player_floor(&d.player_id, -3).await;
        assert_eq!(d.floor().await, -1);
    }

    #[tokio::test]
    async fn surface_exit_needs_the_entrance_shaft() {
        let mut d = delver("gate_surface_exit", 1, |l| room_cell_off_stairs(l, 1, 0)).await;
        let mut up = cell_center(&d.entrance, 1, d.cell);
        up.y = d.entrance.y;
        d.send_move(up, 0).await;
        assert_eq!(first_correction(&mut d.rx).map(|c| c.2), Some(-1));
        assert_eq!(d.floor().await, -1);

        let d = delver("gate_surface_exit_ok", 1, |l| shaft_rect(l, 0).center()).await;
        d.game_state.update_player_floor(&d.player_id, 0).await;
        assert_eq!(d.floor().await, 0);
    }

    #[tokio::test]
    async fn reported_y_is_replaced_by_the_floors_own_height() {
        let mut d = delver("gate_forged_y", 1, |l| room_cell_off_stairs(l, 1, 0)).await;
        let honest = d.ground(1, d.cell);
        let forged = Position {
            y: honest.y + 2.0,
            ..honest
        };
        d.send_move(forged, -1).await;
        assert!(first_correction(&mut d.rx).is_none());
        let (target, floor) = d.queued_target().await.expect("the move is queued");
        assert_eq!(floor, -1);
        assert_eq!(target.y, honest.y);
    }
}

/// Two adoptions racing on the same monster: the loser must be released by a
/// MonsterRemoved for the owner it *actually* was, not the one planned, and be
/// respawned as a bystander since it is still watching.
#[tokio::test]
async fn outraced_adopter_is_released_and_kept_watching() {
    let game_state = make_test_game_state("double_adopt_release");
    let first = pid("first_adopter");
    let second = pid("second_adopter");
    let position = Position {
        x: 10.0,
        y: 0.0,
        z: 10.0,
    };

    game_state
        .add_player(make_player("first_adopter", 10.0, 10.0))
        .await;
    game_state
        .add_player(make_player("second_adopter", 10.0, 10.0))
        .await;
    let mut first_rx = game_state.register_direct_channel(&first).await;
    let mut second_rx = game_state.register_direct_channel(&second).await;

    {
        let mut monsters = game_state.monsters.write().await;
        let mut monster = make_monster("contested", position, 0);
        monster.owner_id = Some(pid("long_gone"));
        monsters.insert("contested".to_string(), monster);
    }

    // Both adoptions were planned against the same unattended monster.
    use crate::game_state::monster::Handoff;
    game_state
        .hand_off_monsters(vec![Handoff {
            monster_id: "contested".to_string(),
            new_owner: first,
        }])
        .await;
    game_state
        .hand_off_monsters(vec![Handoff {
            monster_id: "contested".to_string(),
            new_owner: second,
        }])
        .await;

    match first_rx.try_recv() {
        Ok(ServerMessage::MonsterAssigned { monster }) => assert_eq!(monster.id, "contested"),
        other => panic!("Expected MonsterAssigned to the first adopter, got {other:?}"),
    }
    match first_rx.try_recv() {
        Ok(ServerMessage::MonsterRemoved { monster_id }) => assert_eq!(monster_id, "contested"),
        other => panic!("Expected the outraced adopter's release, got {other:?}"),
    }
    match first_rx.try_recv() {
        Ok(ServerMessage::MonsterSpawned { monster }) => {
            assert_eq!(monster.id, "contested");
            assert_eq!(monster.owner_id, Some(second));
        }
        other => panic!("Expected a bystander respawn for the watcher, got {other:?}"),
    }
    match second_rx.try_recv() {
        Ok(ServerMessage::MonsterAssigned { monster }) => {
            assert_eq!(monster.id, "contested");
            assert_eq!(monster.owner_id, Some(second));
        }
        other => panic!("Expected MonsterAssigned to the final owner, got {other:?}"),
    }
}

/// A move from a client that lost ownership long ago is answered with a
/// release (plus a bystander respawn while it can still see the monster)
/// instead of being silently dropped, so its stale brain shuts down.
#[tokio::test]
async fn stale_controller_move_gets_released() {
    let game_state = make_test_game_state("stale_controller_release");
    let stale = pid("stale_controller");
    let owner = pid("real_owner");
    let position = Position {
        x: 10.0,
        y: 0.0,
        z: 10.0,
    };

    game_state
        .add_player(make_player("stale_controller", 10.0, 10.0))
        .await;
    game_state
        .add_player(make_player("real_owner", 10.0, 10.0))
        .await;
    let mut stale_rx = game_state.register_direct_channel(&stale).await;

    {
        let mut monsters = game_state.monsters.write().await;
        let mut monster = make_monster("held", position, 0);
        monster.owner_id = Some(owner);
        monsters.insert("held".to_string(), monster);
    }

    let target = Position {
        x: 11.0,
        y: 0.0,
        z: 10.0,
    };
    game_state
        .update_monster_position(
            &stale,
            "held".to_string(),
            target,
            0.0,
            MonsterState::Walk,
            target,
        )
        .await;

    match stale_rx.try_recv() {
        Ok(ServerMessage::MonsterRemoved { monster_id }) => assert_eq!(monster_id, "held"),
        other => panic!("Expected the stale controller's release, got {other:?}"),
    }
    match stale_rx.try_recv() {
        Ok(ServerMessage::MonsterSpawned { monster }) => {
            assert_eq!(monster.id, "held");
            assert_eq!(monster.owner_id, Some(owner));
        }
        other => panic!("Expected a bystander respawn for the watcher, got {other:?}"),
    }
}

/// Inside the grace window after a handoff, a non-owner move is just an
/// in-flight packet from the old owner — dropped silently, no release.
#[tokio::test]
async fn in_flight_move_after_handoff_stays_silent() {
    let game_state = make_test_game_state("handoff_grace_window");
    let old_owner = pid("old_owner");
    let new_owner = pid("new_owner");
    let position = Position {
        x: 10.0,
        y: 0.0,
        z: 10.0,
    };

    game_state
        .add_player(make_player("old_owner", 10.0, 10.0))
        .await;
    game_state
        .add_player(make_player("new_owner", 10.0, 10.0))
        .await;
    let mut old_rx = game_state.register_direct_channel(&old_owner).await;

    {
        let mut monsters = game_state.monsters.write().await;
        let mut monster = make_monster("fresh_handoff", position, 0);
        monster.owner_id = Some(old_owner);
        monsters.insert("fresh_handoff".to_string(), monster);
        monsters.reassign_owner("fresh_handoff", new_owner, GameState::now_ms());
    }

    game_state
        .update_monster_position(
            &old_owner,
            "fresh_handoff".to_string(),
            position,
            0.0,
            MonsterState::Walk,
            position,
        )
        .await;

    match old_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => {
            panic!("an in-flight move within the grace window must stay silent, got {other:?}")
        }
    }
}

#[tokio::test]
async fn sim_tracks_nominal_progress_instead_of_racing_to_the_leg_end() {
    let game_state = make_test_game_state("movement_nominal_progress");
    let player_id = pid("walker");
    game_state.add_player(make_player("walker", 0.0, 0.0)).await;
    let target = Position {
        x: 0.0,
        y: 0.0,
        z: 30.0,
    };
    game_state
        .update_player_position(&player_id, move_cmd(target, false), false)
        .await;
    for _ in 0..5 {
        game_state.tick_player_movement(0.2).await;
    }
    let z = game_state.players.read().await[&player_id].position.z;
    let nominal = onlinerpg_shared::PLAYER_MOVE_SPEED * 1.0;
    assert!(
        (z - nominal).abs() < 1e-3,
        "sim should walk at client speed: {z}"
    );
    game_state.tick_player_movement(60.0).await;
    let z = game_state.players.read().await[&player_id].position.z;
    assert!((z - 30.0).abs() < 1e-3, "leg should still complete: {z}");
}

async fn move_to(game_state: &GameState, id: &PlayerId, x: f32, y: f32, z: f32) -> f32 {
    game_state
        .update_player_position(
            id,
            MoveCommand {
                position: Position { x, y, z },
                rotation: 0.0,
                floor_level: 0,
                append: false,
                sprinting: false,
            },
            false,
        )
        .await;
    game_state.tick_player_movement(60.0).await;
    game_state.get_all_players().await[id].position.y
}

/// On open terrain the stored Y is the terrain's, not the client's — a
/// forged height neither flies nor sinks the player.
#[tokio::test]
async fn open_terrain_y_comes_from_the_heightmap() {
    let game_state = make_test_game_state("movement_ground_y");
    let id = pid("grounded");
    game_state
        .add_player(make_player("grounded", 100.0, 50.0))
        .await;

    assert_eq!(move_to(&game_state, &id, 102.0, 40.0, 50.0).await, 5.0);
    assert_eq!(move_to(&game_state, &id, 104.0, -40.0, 50.0).await, 5.0);
}

/// A bridge deck lifts the mover to the deck curve from the catalog.
#[tokio::test]
async fn bridge_deck_y_comes_from_the_deck_index() {
    let game_state = make_test_game_state("movement_deck_y");
    let id = pid("crosser");
    game_state
        .add_player(make_player("crosser", 100.0, 50.0))
        .await;
    game_state.sync_region_furniture(0, 0, &[stone_bridge(100.0, 5.0, 50.0)]);

    assert_eq!(move_to(&game_state, &id, 112.0, 0.0, 50.0).await, 5.0);
    let end = move_to(&game_state, &id, 109.5, 0.0, 50.0).await;
    assert!(end > 5.0 && end < 6.0, "{end}");
    let crown = move_to(&game_state, &id, 100.0, 0.0, 50.0).await;
    assert!((crown - 7.4951).abs() < 1e-3, "{crown}");
    assert_eq!(move_to(&game_state, &id, 112.0, 0.0, 50.0).await, 5.0);
}

/// A wader in the river under the span stays on the bed: the deck is only
/// taken by a mover no lower than its abutments.
#[tokio::test]
async fn under_a_bridge_the_mover_keeps_the_river_bed() {
    let game_state = make_test_game_state("movement_under_deck");
    let id = pid("wader");
    game_state
        .add_player(make_player("wader", -100.0, 60.0))
        .await;
    game_state.sync_region_furniture(-1, 0, &[stone_bridge(-100.0, 3.0, 50.0)]);

    assert_eq!(move_to(&game_state, &id, -100.0, 0.0, 55.0).await, -5.0);
    assert_eq!(move_to(&game_state, &id, -100.0, 9.0, 50.0).await, -5.0);
}
