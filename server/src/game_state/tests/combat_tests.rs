use super::*;

/// Ground height equals the tile X index, so crossing a tile seam is a 1m step.
struct SteppedHeightTiles;

#[async_trait::async_trait]
impl onlinerpg_terrain::height::HeightTiles for SteppedHeightTiles {
    async fn read_heightmap(&self, tx: i32, _tz: i32) -> std::io::Result<Vec<u8>> {
        Ok(uniform_heightmap(tx as f32))
    }
}

/// The next direct message must be the rejection ack for `expected_id`.
fn expect_attack_rejected(
    rx: &mut DirectRx,
    expected_id: &str,
    expected_reason: AttackRejectReason,
) {
    match rx.try_recv() {
        Ok(ServerMessage::PlayerAttackRejected { monster_id, reason }) => {
            assert_eq!(monster_id, expected_id);
            assert_eq!(reason, expected_reason);
        }
        other => panic!("Expected a {expected_reason} rejection ack, got {other:?}"),
    }
}

/// `base` with one component replaced by each non-finite value, labelled.
fn non_finite_positions(base: Position) -> Vec<(String, Position)> {
    let mut out = Vec::new();
    for (value, value_name) in NON_FINITE {
        for (axis, axis_name) in [(0, "x"), (1, "y"), (2, "z")] {
            let mut position = base;
            match axis {
                0 => position.x = value,
                1 => position.y = value,
                _ => position.z = value,
            }
            out.push((format!("{axis_name} {value_name}"), position));
        }
    }
    out
}

#[tokio::test]
async fn monster_events_do_not_cross_floors() {
    let game_state = make_test_game_state("monster_floor_segregation");

    // A surface guard and a dungeon delver share the exact XZ footprint: the
    // guard stands directly above the dungeon floor the delver is on.
    let mut guard = make_player("guard", 0.0, 0.0);
    guard.floor_level = 0;
    let mut delver = make_player("delver", 0.0, 0.0);
    delver.floor_level = -1;
    game_state.add_player(guard).await;
    game_state.add_player(delver).await;

    // Channels registered after join so the AOI snapshots don't pollute them.
    let mut guard_rx = game_state.register_direct_channel(&pid("guard")).await;
    let mut delver_rx = game_state.register_direct_channel(&pid("delver")).await;

    let monster_pos = Position {
        x: 0.0,
        y: -40.0,
        z: 0.0,
    };
    {
        let mut monsters = game_state.monsters.write().await;
        let mut monster = make_monster("dungeon_monster", monster_pos, -1);
        monster.owner_id = Some(pid("keeper"));
        monsters.insert("dungeon_monster".to_string(), monster);
    }

    game_state
        .update_monster_position(
            &pid("keeper"),
            "dungeon_monster".to_string(),
            monster_pos,
            0.0,
            MonsterState::Walk,
            monster_pos,
        )
        .await;

    // Same-floor delver sees the movement; the surface guard above never does.
    match delver_rx.try_recv() {
        Ok(ServerMessage::MonsterMoved { monster_id, .. }) => {
            assert_eq!(monster_id, "dungeon_monster");
        }
        other => panic!(
            "Expected MonsterMoved for same-floor delver, got {:?}",
            other
        ),
    }
    match guard_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!(
            "Surface guard must not receive dungeon monster events, got {:?}",
            other
        ),
    }
}

#[tokio::test]
async fn monster_move_requires_ownership() {
    let game_state = make_test_game_state("monster_move_ownership");
    let owner_id = pid("owner");
    let hijacker_id = pid("hijacker");

    game_state.add_player(make_player("owner", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("hijacker", 0.0, 0.0))
        .await;
    let mut hijacker_rx = game_state.register_direct_channel(&hijacker_id).await;

    {
        let mut monsters = game_state.monsters.write().await;
        let mut monster = make_monster("victim_monster", pos(1.0), 0);
        monster.owner_id = Some(owner_id);
        monsters.insert("victim_monster".to_string(), monster);
    }

    game_state
        .update_monster_position(
            &hijacker_id,
            "victim_monster".to_string(),
            pos(50.0),
            0.0,
            MonsterState::Walk,
            pos(50.0),
        )
        .await;

    assert_eq!(
        game_state.monsters.read().await["victim_monster"]
            .position
            .x,
        1.0,
        "a non-owner move must not change the monster position"
    );
    // The non-owner is told to drop its brain and, still being in sight,
    // gets the monster back as a bystander view.
    match hijacker_rx.try_recv() {
        Ok(ServerMessage::MonsterRemoved { monster_id }) => {
            assert_eq!(monster_id, "victim_monster")
        }
        other => panic!("a non-owner move must be answered by a release, got {other:?}"),
    }
    match hijacker_rx.try_recv() {
        Ok(ServerMessage::MonsterSpawned { monster }) => {
            assert_eq!(monster.owner_id, Some(owner_id))
        }
        other => panic!("Expected a bystander respawn for the watcher, got {other:?}"),
    }

    game_state
        .update_monster_position(
            &owner_id,
            "victim_monster".to_string(),
            pos(2.0),
            0.0,
            MonsterState::Walk,
            pos(2.0),
        )
        .await;

    assert_eq!(
        game_state.monsters.read().await["victim_monster"]
            .position
            .x,
        2.0,
        "the owner's move must apply"
    );
}

#[tokio::test]
async fn non_finite_monster_move_is_rejected() {
    let game_state = make_test_game_state("monster_move_non_finite");
    let owner_id = pid("owner");
    let observer_id = pid("observer");
    let authoritative_position = Position {
        x: 1.0,
        y: 2.0,
        z: 3.0,
    };

    game_state.add_player(make_player("owner", 1.0, 3.0)).await;
    game_state
        .add_player(make_player("observer", 1.0, 3.0))
        .await;
    let mut owner_rx = game_state.register_direct_channel(&owner_id).await;
    let mut observer_rx = game_state.register_direct_channel(&observer_id).await;

    {
        let mut monsters = game_state.monsters.write().await;
        let mut monster = make_monster("owned_monster", authoritative_position, 0);
        monster.owner_id = Some(owner_id);
        monster.rotation = 0.25;
        monster.move_budget = 7.0;
        monster.last_move_at = 42;
        monsters.insert("owned_monster".to_string(), monster);
    }

    let mut cases = Vec::new();
    for (label, position) in non_finite_positions(authoritative_position) {
        cases.push((
            format!("position {label}"),
            position,
            0.5,
            authoritative_position,
        ));
        cases.push((
            format!("target {label}"),
            authoritative_position,
            0.5,
            position,
        ));
    }
    for (rotation, value_name) in NON_FINITE {
        cases.push((
            format!("rotation {value_name}"),
            authoritative_position,
            rotation,
            authoritative_position,
        ));
    }

    for (case, position, rotation, target_position) in cases {
        let before = game_state.monsters.read().await["owned_monster"].clone();

        game_state
            .update_monster_position(
                &owner_id,
                "owned_monster".to_string(),
                position,
                rotation,
                MonsterState::Run,
                target_position,
            )
            .await;

        let after = game_state.monsters.read().await["owned_monster"].clone();
        assert_eq!(after.position, before.position, "{case}");
        assert_eq!(after.rotation, before.rotation, "{case}");
        assert_eq!(after.state, before.state, "{case}");
        assert_eq!(after.move_budget, before.move_budget, "{case}");
        assert_eq!(after.last_move_at, before.last_move_at, "{case}");

        match observer_rx.try_recv() {
            Err(MpscTryRecvError::Empty) => {}
            other => panic!("{case} must not fan out, got {other:?}"),
        }
        match owner_rx.try_recv() {
            Ok(ServerMessage::MonsterMoved {
                monster_id,
                position,
                rotation,
                state,
                target_position,
                ..
            }) => {
                assert_eq!(monster_id, "owned_monster", "{case}");
                assert_eq!(position, before.position, "{case}");
                assert_eq!(rotation, before.rotation, "{case}");
                assert_eq!(state, before.state, "{case}");
                assert_eq!(target_position, before.position, "{case}");
            }
            other => panic!("{case} must correct the owner, got {other:?}"),
        }
        assert!(matches!(owner_rx.try_recv(), Err(MpscTryRecvError::Empty)));
    }
}

#[tokio::test]
async fn monster_move_cannot_report_dead_state() {
    let game_state = make_test_game_state("monster_move_dead_state");
    let owner_id = pid("owner");
    let observer_id = pid("observer");
    let authoritative_position = pos(1.0);

    game_state.add_player(make_player("owner", 1.0, 0.0)).await;
    game_state
        .add_player(make_player("observer", 1.0, 0.0))
        .await;
    let mut owner_rx = game_state.register_direct_channel(&owner_id).await;
    let mut observer_rx = game_state.register_direct_channel(&observer_id).await;

    {
        let mut monsters = game_state.monsters.write().await;
        let mut monster = make_monster("owned_monster", authoritative_position, 0);
        monster.owner_id = Some(owner_id);
        monster.move_budget = 7.0;
        monster.last_move_at = 42;
        monsters.insert(monster.id.clone(), monster);
    }

    game_state
        .update_monster_position(
            &owner_id,
            "owned_monster".to_string(),
            authoritative_position,
            0.5,
            MonsterState::Dead,
            authoritative_position,
        )
        .await;

    let monster = game_state.monsters.read().await["owned_monster"].clone();
    assert_eq!(monster.position, authoritative_position);
    assert_eq!(monster.rotation, 0.0);
    assert_eq!(monster.state, MonsterState::Idle);
    assert_eq!(monster.move_budget, 7.0);
    assert_eq!(monster.last_move_at, 42);

    match owner_rx.try_recv() {
        Ok(ServerMessage::MonsterMoved {
            monster_id,
            position,
            rotation,
            state,
            target_position,
            ..
        }) => {
            assert_eq!(monster_id, "owned_monster");
            assert_eq!(position, authoritative_position);
            assert_eq!(rotation, 0.0);
            assert_eq!(state, MonsterState::Idle);
            assert_eq!(target_position, authoritative_position);
        }
        other => panic!("a client-reported death must correct the owner, got {other:?}"),
    }
    assert!(matches!(owner_rx.try_recv(), Err(MpscTryRecvError::Empty)));
    assert!(matches!(
        observer_rx.try_recv(),
        Err(MpscTryRecvError::Empty)
    ));
}

#[tokio::test]
async fn ambient_spawn_stores_authoritative_world_position() {
    let game_state = make_test_game_state("canonical_spawn_position");
    let player_id = pid("spawner");
    game_state
        .add_player(make_player("spawner", 1.0, 0.0))
        .await;
    // What the placement hands `spawn_monster`: a seam-wrapped X and the
    // ground the server samples, never the reported pose.
    let position = Position {
        x: onlinerpg_shared::wrap_world_x(onlinerpg_shared::WORLD_WIDTH_X * 2.0 + 1.0),
        y: 5.0,
        z: 0.0,
    };

    let monster = game_state
        .spawn_monster(
            "goblin".to_string(),
            position,
            0.0,
            Some(player_id),
            0,
            MonsterLifecycle::Ambient,
            None,
            false,
        )
        .await
        .expect("spawn should fit the test cap");
    assert_eq!(monster.position.x, 1.0);
    assert_eq!(monster.position.y, 5.0);
    assert_eq!(
        game_state.monsters.read().await[&monster.id].position.x,
        1.0
    );
}

/// Spawn canonicalization only holds if moves keep it. The budget and the
/// terrain sweep both measure periodic distance, so an owner could otherwise
/// report a whole-world-width offset as a short move and park the monster
/// outside every spatial-hash lookup — invisible to watchers while its
/// periodic attack reach still landed.
#[tokio::test]
async fn client_monster_move_stores_canonical_world_x() {
    let game_state = make_test_game_state("monster_move_canonical_x");
    let owner_id = pid("owner");
    let start = pos(1.0);

    game_state.add_player(make_player("owner", 1.0, 0.0)).await;
    game_state
        .add_player(make_player("observer", 1.0, 0.0))
        .await;
    let mut observer_rx = game_state.register_direct_channel(&pid("observer")).await;

    {
        let mut monsters = game_state.monsters.write().await;
        let mut monster = make_monster("wrapping_monster", start, 0);
        monster.owner_id = Some(owner_id);
        monster.move_budget = 10.0;
        monster.last_move_at = GameState::now_ms();
        monsters.insert(monster.id.clone(), monster);
    }

    let wrapped = pos(1.5 + onlinerpg_shared::WORLD_WIDTH_X * 2.0);
    game_state
        .update_monster_position(
            &owner_id,
            "wrapping_monster".to_string(),
            wrapped,
            0.0,
            MonsterState::Run,
            wrapped,
        )
        .await;

    assert_eq!(
        game_state.monsters.read().await["wrapping_monster"]
            .position
            .x,
        1.5
    );
    // The watcher still sees it move rather than leave: a non-canonical X
    // would fall outside every queried spatial cell and read as a departure.
    match observer_rx.try_recv() {
        Ok(ServerMessage::MonsterMoved {
            position,
            target_position,
            ..
        }) => {
            assert_eq!(position.x, 1.5);
            assert_eq!(target_position.x, 1.5);
        }
        other => panic!("a periodic-equivalent move must fan out, got {other:?}"),
    }
}

#[tokio::test]
async fn client_monster_move_charges_vertical_displacement() {
    let game_state = make_game_state_with(
        "monster_move_vertical_budget",
        SteppedHeightTiles,
        SeaOnlyWater,
    );
    let owner_id = pid("owner");
    let observer_id = pid("observer");
    let start = Position {
        x: 31.9,
        y: 0.0,
        z: 1.0,
    };

    game_state
        .add_player(make_player("owner", start.x, start.z))
        .await;
    game_state
        .add_player(make_player("observer", start.x, start.z))
        .await;
    let mut owner_rx = game_state.register_direct_channel(&owner_id).await;
    let mut observer_rx = game_state.register_direct_channel(&observer_id).await;

    {
        let mut monsters = game_state.monsters.write().await;
        let mut monster = make_monster("vertical_monster", start, 0);
        monster.owner_id = Some(owner_id);
        monster.move_budget = 1.0;
        monster.last_move_at = GameState::now_ms();
        monsters.insert(monster.id.clone(), monster);
    }

    let forged = Position { y: -40.0, ..start };
    game_state
        .update_monster_position(
            &owner_id,
            "vertical_monster".to_string(),
            forged,
            0.0,
            MonsterState::Run,
            forged,
        )
        .await;

    let after = game_state.monsters.read().await["vertical_monster"].clone();
    assert_eq!(after.position, start);
    assert!(after.move_budget >= 1.0, "a rejected move keeps its refill");
    match owner_rx.try_recv() {
        Ok(ServerMessage::MonsterMoved { position, .. }) => assert_eq!(position, start),
        other => panic!("a vertical teleport must correct its owner, got {other:?}"),
    }
    assert!(matches!(
        observer_rx.try_recv(),
        Err(MpscTryRecvError::Empty)
    ));

    {
        let mut monsters = game_state.monsters.write().await;
        let monster = monsters.get_mut("vertical_monster").unwrap();
        monster.move_budget = 12.0;
        monster.last_move_at = GameState::now_ms();
    }
    let legal = Position {
        x: 32.1,
        y: 1.0,
        z: 1.0,
    };
    let expected = legal.wrapped_x();
    game_state
        .update_monster_position(
            &owner_id,
            "vertical_monster".to_string(),
            legal,
            0.0,
            MonsterState::Walk,
            legal,
        )
        .await;

    let after = game_state.monsters.read().await["vertical_monster"].clone();
    assert_eq!(after.position, expected);
    assert!(
        after.move_budget < 11.5,
        "a vertical-dominant move must spend its vertical displacement, got {}",
        after.move_budget
    );
    match observer_rx.try_recv() {
        Ok(ServerMessage::MonsterMoved { position, .. }) => assert_eq!(position, expected),
        other => panic!("a bounded vertical move must fan out, got {other:?}"),
    }
}

/// A client-owned monster is still untrusted movement. Staying within the
/// speed bucket must not let its owner walk it through the same solid cell that
/// blocks server-simulated player movement — while an equally long move that
/// clears the furniture still goes through.
#[tokio::test]
async fn client_owned_monster_cannot_cross_solid_furniture() {
    let game_state = make_test_game_state("monster_furniture_collision");
    let owner_id = pid("monster_owner");
    let observer_id = pid("monster_observer");
    let start = Position {
        x: 0.5,
        y: 0.0,
        z: 4.5,
    };
    let destination = Position {
        x: 0.5,
        y: 0.0,
        z: 6.5,
    };

    game_state
        .add_player(make_player("monster_owner", start.x, start.z))
        .await;
    game_state
        .add_player(make_player("monster_observer", start.x, start.z))
        .await;
    let mut owner_rx = game_state.register_direct_channel(&owner_id).await;
    let mut observer_rx = game_state.register_direct_channel(&observer_id).await;
    game_state.sync_region_furniture(0, 0, &[table_placement(0.5, 5.5)]);

    {
        let mut monsters = game_state.monsters.write().await;
        let mut monster = make_monster("wall_walking_monster", start, 0);
        monster.owner_id = Some(owner_id);
        monster.move_budget = 10.0;
        monster.last_move_at = GameState::now_ms();
        monsters.insert(monster.id.clone(), monster);
    }

    game_state
        .update_monster_position(
            &owner_id,
            "wall_walking_monster".to_string(),
            destination,
            0.0,
            MonsterState::Run,
            destination,
        )
        .await;

    let after = game_state.monsters.read().await["wall_walking_monster"].clone();
    assert_eq!(
        after.position, start,
        "solid furniture must block a client-owned monster move"
    );
    // Banked, not spent: a block keeps the refill like the speed-reject path.
    assert!(
        after.move_budget >= 10.0,
        "a blocked move must not consume budget, got {}",
        after.move_budget
    );
    assert!(matches!(
        observer_rx.try_recv(),
        Err(MpscTryRecvError::Empty)
    ));
    match owner_rx.try_recv() {
        Ok(ServerMessage::MonsterMoved { position, .. }) => assert_eq!(position, start),
        other => panic!("a blocked monster move must correct its owner, got {other:?}"),
    }

    let forged_height = Position {
        x: start.x + 0.25,
        y: 2.0,
        ..start
    };
    game_state
        .update_monster_position(
            &owner_id,
            "wall_walking_monster".to_string(),
            forged_height,
            0.0,
            MonsterState::Run,
            forged_height,
        )
        .await;
    assert_eq!(
        game_state.monsters.read().await["wall_walking_monster"].position,
        start,
        "a client-selected height must not clear the furniture"
    );
    match owner_rx.try_recv() {
        Ok(ServerMessage::MonsterMoved { position, .. }) => assert_eq!(position, start),
        other => panic!("a forged height must correct its owner, got {other:?}"),
    }
    assert!(matches!(
        observer_rx.try_recv(),
        Err(MpscTryRecvError::Empty)
    ));

    // The sweep's other side. An owner reports whole path legs at once, so a
    // sweep that over-refuses would freeze legitimate monsters at every corner:
    // the same 2m hop away from the table must apply and reach watchers.
    let clear = Position {
        x: 0.5,
        y: 0.0,
        z: 2.5,
    };
    game_state
        .update_monster_position(
            &owner_id,
            "wall_walking_monster".to_string(),
            clear,
            0.0,
            MonsterState::Run,
            clear,
        )
        .await;

    assert_eq!(
        game_state.monsters.read().await["wall_walking_monster"].position,
        clear,
        "a clear move must apply"
    );
    match observer_rx.try_recv() {
        Ok(ServerMessage::MonsterMoved { position, .. }) => assert_eq!(position, clear),
        other => panic!("a clear move must fan out to watchers, got {other:?}"),
    }
    // The owner applied it optimistically and gets no correction.
    assert!(matches!(owner_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

/// Even the owner can't teleport a monster onto a distant victim: a move is
/// capped to what the monster could run since its last accepted move, so an
/// owned monster stays a melee threat only where it could actually walk.
#[tokio::test]
async fn monster_move_is_speed_capped() {
    let game_state = make_test_game_state("monster_move_speed_cap");
    let owner_id = pid("owner");

    game_state.add_player(make_player("owner", 0.0, 0.0)).await;
    // A bystander next to the monster observes fanout: the owner is skipped in
    // the position broadcast, so it can't tell an applied move from a refused
    // one on its own.
    game_state
        .add_player(make_player("observer", 0.0, 0.0))
        .await;
    let mut observer_rx = game_state.register_direct_channel(&pid("observer")).await;
    let mut owner_rx = game_state.register_direct_channel(&owner_id).await;

    {
        let mut monsters = game_state.monsters.write().await;
        let mut monster = make_monster("owned_monster", pos(0.0), 0);
        monster.owner_id = Some(owner_id);
        // A large elapsed budget (last_move_at = 0) still can't clear the
        // absolute per-move cap.
        monsters.insert("owned_monster".to_string(), monster);
    }

    // A 50m jump exceeds the absolute step cap and is refused outright.
    game_state
        .update_monster_position(
            &owner_id,
            "owned_monster".to_string(),
            pos(50.0),
            0.0,
            MonsterState::Run,
            pos(50.0),
        )
        .await;
    assert_eq!(
        game_state.monsters.read().await["owned_monster"].position.x,
        0.0,
        "a teleport past the step cap must not move the monster"
    );
    match observer_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("a rejected move must not fan out, got {other:?}"),
    }
    // The mover applies its moves optimistically, so a reject must echo the
    // authoritative position back to it (and only to it).
    match owner_rx.try_recv() {
        Ok(ServerMessage::MonsterMoved { position, .. }) => {
            assert_eq!(position.x, 0.0, "correction must carry the real position");
        }
        other => panic!("a rejected move must send a correction to the mover, got {other:?}"),
    }

    // A short hop within the cap still applies normally.
    game_state
        .update_monster_position(
            &owner_id,
            "owned_monster".to_string(),
            pos(5.0),
            0.0,
            MonsterState::Run,
            pos(5.0),
        )
        .await;
    assert_eq!(
        game_state.monsters.read().await["owned_monster"].position.x,
        5.0,
        "a move within the cap must apply"
    );
    match observer_rx.try_recv() {
        Ok(ServerMessage::MonsterMoved { monster_id, .. }) => {
            assert_eq!(monster_id, "owned_monster");
        }
        other => panic!("an accepted move must fan out to bystanders, got {other:?}"),
    }
    match owner_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("an accepted move must not echo to the mover, got {other:?}"),
    }

    // Drain the bucket and reset its clock: an under-cap jump (8m < 15m) is now
    // refused by the refill rate, not the cap, since no time has passed to
    // refill it.
    {
        let mut monsters = game_state.monsters.write().await;
        let monster = monsters.get_mut("owned_monster").unwrap();
        monster.move_budget = 0.0;
        monster.last_move_at = GameState::now_ms();
    }
    game_state
        .update_monster_position(
            &owner_id,
            "owned_monster".to_string(),
            pos(13.0),
            0.0,
            MonsterState::Run,
            pos(13.0),
        )
        .await;
    assert_eq!(
        game_state.monsters.read().await["owned_monster"].position.x,
        5.0,
        "an 8m jump with an empty budget must be refused by the refill rate"
    );
    match owner_rx.try_recv() {
        Ok(ServerMessage::MonsterMoved { position, .. }) => {
            assert_eq!(position.x, 5.0, "correction must carry the real position");
        }
        other => panic!("a rate-refused move must send a correction to the mover, got {other:?}"),
    }
}

/// Owning a monster must not let a player damage arbitrary targets at range.
/// Anyone can spawn a monster beside themselves and become its owner, so the
/// ownership check alone would leave `target_player_id` as a world-wide damage
/// primitive against any id the attacker can name.
#[tokio::test]
async fn monster_attack_requires_proximity_to_target() {
    let game_state = make_test_game_state("monster_attack_range");
    let owner_id = pid("owner");
    let victim_id = pid("victim");

    game_state.add_player(make_player("owner", 0.0, 0.0)).await;
    // Far out of any monster's reach, but well within the attacker's ability to
    // name: an id is all the exploit needed.
    game_state
        .add_player(make_player("victim", 500.0, 0.0))
        .await;

    // Each half uses its own monster: a rejected attack still consumes the
    // cooldown, so reusing one would block the in-range case for 1.5s.
    {
        let mut monsters = game_state.monsters.write().await;
        for (id, x) in [("far_monster", 0.0), ("near_monster", 499.0)] {
            let mut monster = make_monster(id, pos(x), 0);
            monster.owner_id = Some(owner_id);
            monsters.insert(id.to_string(), monster);
        }
    }

    game_state
        .broadcast_monster_attack(&owner_id, "far_monster", &victim_id)
        .await;

    // `last_combat_at` is stamped for any in-range attack, hit or miss, so it
    // records that the swing was processed without depending on a damage roll.
    assert_eq!(
        game_state.players.read().await[&victim_id].last_combat_at,
        0,
        "a monster 500m from its target must not reach it"
    );
    assert_eq!(
        game_state.players.read().await[&victim_id].health,
        10,
        "an out-of-range monster attack must not deal damage"
    );

    game_state
        .broadcast_monster_attack(&owner_id, "near_monster", &victim_id)
        .await;

    assert_ne!(
        game_state.players.read().await[&victim_id].last_combat_at,
        0,
        "a monster standing next to its target must still land its attack"
    );
}

/// A monster and its target must share a floor, so a surface monster cannot
/// strike a player on the dungeon floor directly beneath it.
#[tokio::test]
async fn cross_floor_monster_attack_is_rejected() {
    let game_state = make_test_game_state("cross_floor_monster_attack");
    let owner_id = pid("owner");
    let delver_id = pid("delver");

    game_state.add_player(make_player("owner", 0.0, 0.0)).await;
    let mut delver = make_player("delver", 0.0, 0.0);
    delver.floor_level = -1;
    delver.position.y = -40.0;
    game_state.add_player(delver).await;

    {
        let mut monsters = game_state.monsters.write().await;
        let mut monster = make_monster("surface_monster", pos(0.0), 0);
        monster.owner_id = Some(owner_id);
        monsters.insert("surface_monster".to_string(), monster);
    }

    game_state
        .broadcast_monster_attack(&owner_id, "surface_monster", &delver_id)
        .await;

    assert_eq!(
        game_state.players.read().await[&delver_id].last_combat_at,
        0,
        "a surface monster must not reach a player one floor below it"
    );
}

#[tokio::test]
async fn cross_floor_player_attack_is_rejected() {
    let game_state = make_test_game_state("cross_floor_attack");

    let mut guard = make_player("guard", 0.0, 0.0);
    guard.floor_level = 0;
    game_state.add_player(guard).await;
    let mut guard_rx = game_state.register_direct_channel(&pid("guard")).await;

    {
        let mut monsters = game_state.monsters.write().await;
        monsters.insert(
            "dungeon_monster".to_string(),
            make_monster(
                "dungeon_monster",
                Position {
                    x: 0.0,
                    y: -40.0,
                    z: 0.0,
                },
                -1,
            ),
        );
    }

    game_state
        .broadcast_player_attack(&pid("guard"), "dungeon_monster".to_string())
        .await;

    // The attack is dropped server-side: the monster keeps full HP and the
    // attacker gets a coarse rejection that must not name the floor.
    let health = game_state
        .monsters
        .read()
        .await
        .get("dungeon_monster")
        .map(|m| m.health)
        .unwrap();
    assert_eq!(health, 10, "cross-floor attack must not damage the monster");
    expect_attack_rejected(
        &mut guard_rx,
        "dungeon_monster",
        AttackRejectReason::InvalidTarget,
    );
}

#[tokio::test]
async fn out_of_range_player_attack_only_provokes_monster() {
    let game_state = make_test_game_state("out_of_range_attack");
    let player_id = pid("attacker");
    let controller_id = pid("monster_controller");

    game_state
        .add_player(make_player("attacker", 0.0, 0.0))
        .await;
    let mut attacker_rx = game_state.register_direct_channel(&player_id).await;
    let mut controller_rx = game_state.register_direct_channel(&controller_id).await;

    {
        let mut monsters = game_state.monsters.write().await;
        let mut monster = make_monster("distant_monster", pos(3.01), 0);
        monster.owner_id = Some(controller_id);
        monsters.insert("distant_monster".to_string(), monster);
    }

    game_state
        .broadcast_player_attack(&player_id, "distant_monster".to_string())
        .await;

    let monsters = game_state.monsters.read().await;
    assert_eq!(
        monsters["distant_monster"].health, 10,
        "an out-of-range attack must not damage the monster"
    );
    drop(monsters);
    assert_eq!(
        game_state.players.read().await[&player_id].last_combat_at,
        0,
        "a rejected attack must not enter combat"
    );
    match controller_rx.try_recv() {
        Ok(ServerMessage::MonsterProvoked {
            player_id: actual_player_id,
            monster_id,
        }) => {
            assert_eq!(actual_player_id, player_id);
            assert_eq!(monster_id, "distant_monster");
        }
        other => panic!("Expected only an aggro event outside melee range, got {other:?}"),
    }
    expect_attack_rejected(
        &mut attacker_rx,
        "distant_monster",
        AttackRejectReason::OutOfRange,
    );
}

#[tokio::test]
async fn player_attack_beyond_provoke_range_is_fully_rejected() {
    let game_state = make_test_game_state("beyond_provoke_range_attack");
    let player_id = pid("attacker");
    let controller_id = pid("monster_controller");

    game_state
        .add_player(make_player("attacker", 0.0, 0.0))
        .await;
    let mut attacker_rx = game_state.register_direct_channel(&player_id).await;
    let mut controller_rx = game_state.register_direct_channel(&controller_id).await;

    {
        let mut monster = make_monster(
            "remote_monster",
            pos(super::combat::PLAYER_ATTACK_PROVOKE_RANGE_METERS + 0.01),
            0,
        );
        monster.owner_id = Some(controller_id);
        game_state
            .monsters
            .write()
            .await
            .insert("remote_monster".to_string(), monster);
    }

    game_state
        .broadcast_player_attack(&player_id, "remote_monster".to_string())
        .await;

    assert_eq!(
        game_state.monsters.read().await["remote_monster"].health,
        10
    );
    assert_eq!(
        game_state.players.read().await[&player_id].last_combat_at,
        0
    );
    match controller_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Expected no provoke event beyond 10m, got {other:?}"),
    }
    expect_attack_rejected(
        &mut attacker_rx,
        "remote_monster",
        AttackRejectReason::OutOfRange,
    );
}

#[tokio::test]
async fn player_attack_at_melee_range_is_allowed() {
    let game_state = make_test_game_state("melee_range_attack");
    let player_id = pid("attacker");

    game_state
        .add_player(make_player("attacker", 0.0, 0.0))
        .await;
    let mut attacker_rx = game_state.register_direct_channel(&player_id).await;

    {
        let mut monsters = game_state.monsters.write().await;
        monsters.insert(
            "nearby_monster".to_string(),
            make_monster("nearby_monster", pos(2.0), 0),
        );
    }

    game_state
        .broadcast_player_attack(&player_id, "nearby_monster".to_string())
        .await;

    match attacker_rx.try_recv() {
        Ok(ServerMessage::PlayerAttacked {
            player_id: actual_player_id,
            monster_id,
            ..
        }) => {
            assert_eq!(actual_player_id, player_id);
            assert_eq!(monster_id, "nearby_monster");
        }
        other => panic!("Expected an attack echo at melee range, got {other:?}"),
    }
    assert_ne!(
        game_state.players.read().await[&player_id].last_combat_at,
        0,
        "an allowed attack must enter combat"
    );
}

#[tokio::test]
async fn player_attack_interval_is_server_enforced() {
    let game_state = make_test_game_state("player_attack_interval");
    let player_id = pid("attacker");

    game_state
        .add_player(make_player("attacker", 0.0, 0.0))
        .await;
    let mut attacker_rx = game_state.register_direct_channel(&player_id).await;
    game_state.monsters.write().await.insert(
        "nearby_monster".to_string(),
        make_monster("nearby_monster", pos(1.0), 0),
    );

    game_state
        .broadcast_player_attack(&player_id, "nearby_monster".to_string())
        .await;
    game_state
        .broadcast_player_attack(&player_id, "nearby_monster".to_string())
        .await;

    let attack_count = drain(&mut attacker_rx)
        .into_iter()
        .filter(|message| matches!(message, ServerMessage::PlayerAttacked { .. }))
        .count();
    assert_eq!(
        attack_count, 1,
        "back-to-back requests must produce one authoritative attack roll"
    );

    game_state.last_player_attacks.write().await.insert(
        player_id,
        GameState::now_ms().saturating_sub(*super::combat::PLAYER_ATTACK_INTERVAL_MS),
    );
    game_state
        .broadcast_player_attack(&player_id, "nearby_monster".to_string())
        .await;

    assert!(drain(&mut attacker_rx)
        .into_iter()
        .any(|message| matches!(message, ServerMessage::PlayerAttacked { .. })));
}

#[tokio::test]
async fn rejected_player_attack_does_not_consume_interval() {
    let game_state = make_test_game_state("rejected_player_attack_interval");
    let player_id = pid("attacker");

    game_state
        .add_player(make_player("attacker", 0.0, 0.0))
        .await;
    let mut attacker_rx = game_state.register_direct_channel(&player_id).await;
    game_state.monsters.write().await.insert(
        "nearby_monster".to_string(),
        make_monster("nearby_monster", pos(1.0), 0),
    );

    game_state
        .broadcast_player_attack(&player_id, "missing_monster".to_string())
        .await;
    expect_attack_rejected(
        &mut attacker_rx,
        "missing_monster",
        AttackRejectReason::InvalidTarget,
    );
    game_state
        .broadcast_player_attack(&player_id, "nearby_monster".to_string())
        .await;

    assert!(drain(&mut attacker_rx)
        .into_iter()
        .any(|message| matches!(message, ServerMessage::PlayerAttacked { .. })));
}

/// A player at 0 HP (awaiting respawn) must not be able to keep attacking.
#[tokio::test]
async fn dead_player_cannot_attack() {
    let game_state = make_test_game_state("dead_player_attack");
    let player_id = pid("attacker");

    game_state
        .add_player(make_player("attacker", 0.0, 0.0))
        .await;
    game_state
        .players
        .write()
        .await
        .get_mut(&player_id)
        .unwrap()
        .health = 0;
    let mut attacker_rx = game_state.register_direct_channel(&player_id).await;

    {
        let mut monsters = game_state.monsters.write().await;
        monsters.insert(
            "nearby_monster".to_string(),
            make_monster("nearby_monster", pos(2.0), 0),
        );
    }

    game_state
        .broadcast_player_attack(&player_id, "nearby_monster".to_string())
        .await;

    expect_attack_rejected(
        &mut attacker_rx,
        "nearby_monster",
        AttackRejectReason::AttackerDead,
    );
    assert_eq!(
        game_state.monsters.read().await["nearby_monster"].health,
        10,
        "a dead player's attack must deal no damage"
    );
}

/// A stale id — dead or never known — earns the same coarse rejection, so
/// probing ids reveals nothing about hidden monster state.
#[tokio::test]
async fn stale_monster_attack_is_rejected_as_invalid_target() {
    let game_state = make_test_game_state("stale_monster_attack");
    let player_id = pid("attacker");

    game_state
        .add_player(make_player("attacker", 0.0, 0.0))
        .await;
    let mut attacker_rx = game_state.register_direct_channel(&player_id).await;

    {
        let mut monsters = game_state.monsters.write().await;
        let mut monster = make_monster("dead_monster", pos(1.0), 0);
        monster.state = MonsterState::Dead;
        monsters.insert("dead_monster".to_string(), monster);
    }

    for target in ["dead_monster", "unknown_monster"] {
        game_state
            .broadcast_player_attack(&player_id, target.to_string())
            .await;
        expect_attack_rejected(&mut attacker_rx, target, AttackRejectReason::InvalidTarget);
    }
}
