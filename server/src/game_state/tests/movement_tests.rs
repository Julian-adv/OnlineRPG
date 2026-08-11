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
                },
                dropped_at_ms: 0,
            },
        );
    }

    game_state
        .update_player_position(&player_id, move_cmd(entity_position, false), false, false)
        .await;
    game_state.tick_player_movement(60.0).await;

    match direct_rx.try_recv() {
        Ok(ServerMessage::MonsterSpawned { monster }) => {
            assert_eq!(monster.id, "monster_a");
        }
        other => panic!("Expected MonsterSpawned when entering AOI, got {:?}", other),
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
        .update_player_position(&player_id, move_cmd(pos(50.0), false), false, false)
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
        .update_player_position(&player_id, move_cmd(pos(1.0), false), false, false)
        .await;
    for x in [2.0, 3.0] {
        game_state
            .update_player_position(&player_id, move_cmd(pos(x), true), false, false)
            .await;
    }

    // Budget ≈ 1.98m: leg 1 consumed whole, leg 2 partially — the cap holds
    // across legs, not per leg.
    game_state.tick_player_movement(0.5).await;
    let mid = player_x(&game_state, &player_id).await;
    assert!(mid > 1.5 && mid < 2.1, "mid was {mid}");

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
        .update_player_position(&player_id, move_cmd(pos(50.0), false), false, false)
        .await;
    // 100 is >60m from the player but only 50m from the queue tail: accepted.
    game_state
        .update_player_position(&player_id, move_cmd(pos(100.0), true), false, false)
        .await;
    // 70m from the new tail: rejected.
    game_state
        .update_player_position(&player_id, move_cmd(pos(170.0), true), false, false)
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
        .update_player_position(&player_id, move_cmd(pos(10.0), false), false, false)
        .await;
    game_state
        .update_player_position(&player_id, move_cmd(pos(20.0), true), false, false)
        .await;
    game_state
        .update_player_position(&player_id, move_cmd(pos(5.0), false), false, false)
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
        .update_player_position(&player_id, move_cmd(pos(1.0), false), false, false)
        .await;
    for i in 2..=40 {
        game_state
            .update_player_position(&player_id, move_cmd(pos(i as f32), true), false, false)
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
            .update_player_position(&player_id, move_cmd(pos(bad), false), false, false)
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

    game_state.update_player_floor(&player_id, max_floor).await;
    assert_eq!(
        game_state.players.read().await[&player_id].floor_level,
        max_floor
    );
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
        .update_player_position(
            &player_id,
            MoveCommand {
                position: pos(1.0),
                rotation: 0.0,
                floor_level: max_floor,
                append: false,
                sprinting: false,
            },
            false,
            false,
        )
        .await;
    game_state.tick_player_movement(1.0).await;
    let player = game_state.players.read().await[&player_id].clone();
    assert_eq!(player.position.x, 1.0);
    assert_eq!(player.floor_level, max_floor);
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
        .update_player_position(&player_id, move_cmd(pos(100.0), false), false, false)
        .await;
    game_state.tick_player_movement(600.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 0.0);
}

#[tokio::test]
async fn admin_move_applies_immediately() {
    let game_state = make_test_game_state("movement_admin_bypass");
    let player_id = pid("gm");
    game_state.add_player(make_player("gm", 0.0, 0.0)).await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(100.0), false), true, false)
        .await;
    assert_eq!(player_x(&game_state, &player_id).await, 100.0);
}

#[tokio::test]
async fn teleport_clears_pending_move_intent() {
    let game_state = make_test_game_state("movement_teleport_clears");
    let player_id = pid("traveler");
    game_state
        .add_player(make_player("traveler", 0.0, 0.0))
        .await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(50.0), false), false, false)
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
async fn rejected_far_move_snaps_client_back() {
    let game_state = make_test_game_state("movement_far_reject_snap");
    let player_id = pid("phantom");
    game_state
        .add_player(make_player("phantom", 0.0, 0.0))
        .await;
    let mut direct_rx = game_state.register_direct_channel(&player_id).await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(100.0), false), false, false)
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
        .update_player_position(&player_id, move_cmd(pos(101.0), false), false, false)
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
        .update_player_position(&player_id, move_cmd(pos(10.0), false), false, false)
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
