use super::*;
use onlinerpg_shared::skills::{
    armor_skill_guard_bonus, shield_skill_guard_bonus, skill_xp_for_level, SkillId, SkillProgress,
    Skills, DEFAULT_WEAPON_ATTACK_COOLDOWN_MS, DEFAULT_WEAPON_MELEE_RANGE_METERS, SKILL_LEVEL_CAP,
    SPEAR_ATTACK_COOLDOWN_MS, SPEAR_MELEE_RANGE_METERS,
};
use onlinerpg_shared::{PhysicalDamageType, PhysicalProtection};

async fn setup_trained_weapon_attacker(
    game_state: &GameState,
    name: &str,
    item_def_id: Option<&str>,
    enchant: i32,
    skill: SkillId,
    skill_level: u32,
) -> DirectRx {
    let player_id = pid(name);
    game_state.add_player(make_player(name, 0.0, 0.0)).await;
    game_state
        .register_player_character(&player_id, 1, 0, attrs_with_cha(10), 0, None)
        .await;
    let mut skills = Skills::default();
    if skill_level > 0 {
        skills.map.insert(
            skill,
            SkillProgress {
                level: skill_level,
                xp: skill_xp_for_level(skill_level),
            },
        );
    }
    game_state.register_player_skills(&player_id, skills).await;
    let mut inventory = PlayerInventory::default();
    if let Some(item_def_id) = item_def_id {
        inventory.equipped.insert(
            EquipSlot::MainHand,
            ItemInstance {
                instance_id: 1,
                item_def_id: item_def_id.to_string(),
                quantity: 1,
                enchant,
                durability: None,
            },
        );
    }
    game_state
        .inventories
        .write()
        .await
        .insert(player_id, inventory);
    game_state.register_direct_channel(&player_id).await
}

async fn setup_weapon_attacker(
    game_state: &GameState,
    name: &str,
    item_def_id: Option<&str>,
    enchant: i32,
    sword_level: u32,
) -> DirectRx {
    setup_trained_weapon_attacker(
        game_state,
        name,
        item_def_id,
        enchant,
        SkillId::OneHandedSword,
        sword_level,
    )
    .await
}

async fn insert_combat_monster(
    game_state: &GameState,
    id: &str,
    position: Position,
    floor_level: i8,
    health: u32,
) {
    let mut monster = make_monster(id, position, floor_level);
    monster.monster_type = "kobold".to_string();
    monster.health = health;
    monster.max_health = health;
    game_state
        .monsters
        .write()
        .await
        .insert(id.to_string(), monster);
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
    match hijacker_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("a rejected move must not fan out, got {other:?}"),
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
async fn non_finite_spawn_request_is_rejected() {
    let game_state = make_test_game_state("spawn_request_non_finite");
    let player_id = pid("spawner");
    game_state
        .add_player(make_player("spawner", 10.0, 20.0))
        .await;

    let valid = Position {
        x: 12.0,
        y: 0.5,
        z: 22.0,
    };
    assert!(game_state
        .validate_spawn_request(&player_id, "goblin", &valid, 1.5)
        .await
        .is_some());

    for (label, position) in non_finite_positions(valid) {
        assert!(
            game_state
                .validate_spawn_request(&player_id, "goblin", &position, 1.5)
                .await
                .is_none(),
            "position {label}"
        );
    }
    for (rotation, value_name) in NON_FINITE {
        assert!(
            game_state
                .validate_spawn_request(&player_id, "goblin", &valid, rotation)
                .await
                .is_none(),
            "rotation {value_name}"
        );
    }
}

#[tokio::test]
async fn ambient_spawn_requires_unconsumed_server_allowance() {
    let game_state = make_test_game_state("ambient_spawn_allowance");
    let player_id = pid("spawner");
    game_state
        .add_player(make_player("spawner", 10.0, 20.0))
        .await;
    let mut player_rx = game_state.register_direct_channel(&player_id).await;

    assert!(
        !game_state.take_spawn_allowance(&player_id, "goblin").await,
        "an unsolicited ambient spawn must be rejected"
    );

    game_state.tick_monster_spawns().await;
    assert_eq!(spawn_requests(&mut player_rx, "goblin"), 1);
    assert!(game_state.take_spawn_allowance(&player_id, "goblin").await);
    assert!(
        !game_state.take_spawn_allowance(&player_id, "goblin").await,
        "one allowance must authorize at most one spawn"
    );
}

#[tokio::test]
async fn ambient_spawn_allowance_is_bounded_and_expires() {
    let game_state = make_test_game_state("ambient_spawn_allowance_expiry");
    let player_id = pid("spawner");
    game_state
        .add_player(make_player("spawner", 10.0, 20.0))
        .await;
    let mut player_rx = game_state.register_direct_channel(&player_id).await;

    game_state.tick_monster_spawns().await;
    game_state.tick_monster_spawns().await;
    assert_eq!(spawn_requests(&mut player_rx, "goblin"), 1);

    game_state
        .ambient_spawn_allowances
        .write()
        .await
        .insert((player_id, "goblin".to_string()), GameState::now_ms());
    assert!(
        !game_state.take_spawn_allowance(&player_id, "goblin").await,
        "an expired allowance must not authorize a spawn"
    );

    game_state.tick_monster_spawns().await;
    assert_eq!(spawn_requests(&mut player_rx, "goblin"), 1);
    game_state.remove_player(&player_id).await;
    assert!(game_state
        .ambient_spawn_allowances
        .read()
        .await
        .keys()
        .all(|(owner_id, _)| owner_id != &player_id));
}

#[tokio::test]
async fn wrapped_spawn_cannot_bypass_no_spawn_zone() {
    let zone = onlinerpg_shared::NoSpawnZone {
        min_x: -1.0,
        min_z: -1.0,
        max_x: 1.0,
        max_z: 1.0,
    };
    let game_state = make_game_state_with_zones(
        "wrapped_spawn_zone",
        SplitWorldTiles,
        SeaOnlyWater,
        vec![zone],
    );
    let player_id = pid("spawner");
    game_state
        .add_player(make_player("spawner", 0.0, 0.0))
        .await;
    let wrapped_zone_position = Position {
        x: onlinerpg_shared::WORLD_WIDTH_X,
        y: 0.0,
        z: 0.0,
    };

    assert!(game_state
        .validate_spawn_request(&player_id, "goblin", &wrapped_zone_position, 0.0)
        .await
        .is_none());

    // Positive control: the same fixture accepts a periodic-equivalent
    // position clear of the zone's margin, so the rejection above is the zone
    // and not a missing rule or player.
    let wrapped_clear_position = Position {
        x: onlinerpg_shared::WORLD_WIDTH_X,
        y: 0.0,
        z: 40.0,
    };
    assert_eq!(
        game_state
            .validate_spawn_request(&player_id, "goblin", &wrapped_clear_position, 0.0)
            .await
            .expect("clear of the zone and within range")
            .x,
        0.0
    );
}

#[tokio::test]
async fn ambient_spawn_stores_canonical_world_x() {
    let game_state = make_test_game_state("canonical_spawn_position");
    let player_id = pid("spawner");
    game_state
        .add_player(make_player("spawner", 1.0, 0.0))
        .await;
    let raw_position = Position {
        x: onlinerpg_shared::WORLD_WIDTH_X * 2.0 + 1.0,
        y: 0.0,
        z: 0.0,
    };

    let position = game_state
        .validate_spawn_request(&player_id, "goblin", &raw_position, 0.0)
        .await
        .expect("the periodic position is in range");
    assert_eq!(position.x, 1.0);

    let monster = game_state
        .spawn_monster(
            "goblin".to_string(),
            position,
            0.0,
            Some(player_id),
            0,
            None,
            false,
        )
        .await
        .expect("spawn should fit the test cap");
    assert_eq!(monster.position.x, 1.0);
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
async fn shield_guard_bonus_requires_an_explicitly_mapped_equipped_shield() {
    let game_state = make_test_game_state("shield_guard_profile");
    let defender_id = pid("shield_guardian");
    game_state
        .add_player(make_player("shield_guardian", 0.0, 0.0))
        .await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 10;
    game_state
        .register_player_character(&defender_id, 1, 0, attrs, 0, None)
        .await;
    let mut skills = Skills::default();
    skills.map.insert(
        SkillId::Shield,
        SkillProgress {
            level: 15,
            xp: skill_xp_for_level(15),
        },
    );
    game_state
        .register_player_skills(&defender_id, skills)
        .await;

    let mut inventory = PlayerInventory::default();
    inventory
        .equipped
        .insert(EquipSlot::OffHand, bag_item(1, "wooden_shield", 1));
    inventory
        .equipped
        .insert(EquipSlot::Head, bag_item(2, "leather_helmet", 1));
    game_state
        .inventories
        .write()
        .await
        .insert(defender_id, inventory);

    let profile = game_state.player_defense_profile(&defender_id).await;
    assert_eq!(profile.shield_skill, Some(SkillId::Shield));
    assert_eq!(profile.shield_skill_level, 15);
    assert_eq!(profile.shield_skill_guard_bonus, 2);
    assert_eq!(profile.armor_skill, None);
    assert_eq!(profile.armor_skill_guard_bonus, 0);
    assert_eq!(profile.effective_guard, 14); // base 10 + shield 1 + helm 1 + skill 2
    assert_eq!(game_state.effective_guard(&defender_id).await, 14);

    game_state
        .inventories
        .write()
        .await
        .get_mut(&defender_id)
        .unwrap()
        .equipped
        .insert(EquipSlot::OffHand, bag_item(3, "torch", 1));
    let without_shield = game_state.player_defense_profile(&defender_id).await;
    assert_eq!(without_shield.shield_skill, None);
    assert_eq!(without_shield.shield_skill_guard_bonus, 0);
    assert_eq!(without_shield.effective_guard, 11);
    assert_eq!(shield_skill_guard_bonus(15), 2);
}

#[tokio::test]
async fn accepted_monster_attacks_train_shield_without_packet_shortcuts() {
    let game_state = make_test_game_state("shield_training");
    let owner_id = pid("shield_monster_owner");
    let defender_id = pid("shield_defender");
    game_state
        .add_player(make_player("shield_monster_owner", 0.0, 0.0))
        .await;
    let mut defender = make_player("shield_defender", 0.0, 0.0);
    defender.health = 100;
    defender.max_health = 100;
    game_state.add_player(defender).await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 20; // A level-1 monster cannot beat 20 with its d20 roll.
    game_state
        .register_player_character(&defender_id, 2, 0, attrs, 0, None)
        .await;
    game_state
        .register_player_skills(&defender_id, Skills::default())
        .await;
    let mut inventory = PlayerInventory::default();
    inventory
        .equipped
        .insert(EquipSlot::OffHand, bag_item(10, "wooden_shield", 1));
    game_state
        .inventories
        .write()
        .await
        .insert(defender_id, inventory);
    let mut defender_rx = game_state.register_direct_channel(&defender_id).await;

    {
        let mut monsters = game_state.monsters.write().await;
        for (id, x, level_override) in [
            ("far_shield_attack", 100.0, None),
            ("shield_miss", 0.0, None),
            ("shield_hit", 0.0, Some(u8::MAX)),
            ("unshielded_hit", 0.0, Some(u8::MAX)),
        ] {
            let mut monster = make_monster(id, pos(x), 0);
            monster.owner_id = Some(owner_id);
            monster.level_override = level_override;
            monsters.insert(id.to_string(), monster);
        }
    }

    // Range validation runs before the defense event and grants nothing.
    game_state
        .broadcast_monster_attack(&owner_id, "far_shield_attack", &defender_id)
        .await;
    assert_eq!(
        game_state.skill_level(&defender_id, SkillId::Shield).await,
        0
    );
    assert!(!game_state.player_skills.read().await[&defender_id]
        .map
        .contains_key(&SkillId::Shield));

    game_state
        .broadcast_monster_attack(&owner_id, "shield_miss", &defender_id)
        .await;
    assert_eq!(
        game_state.player_skills.read().await[&defender_id]
            .get(SkillId::Shield)
            .xp,
        10
    );

    // Replaying the same request inside the monster cooldown is ignored.
    game_state
        .broadcast_monster_attack(&owner_id, "shield_miss", &defender_id)
        .await;
    assert_eq!(
        game_state.player_skills.read().await[&defender_id]
            .get(SkillId::Shield)
            .xp,
        10
    );

    game_state
        .broadcast_monster_attack(&owner_id, "shield_hit", &defender_id)
        .await;
    assert_eq!(
        game_state.player_skills.read().await[&defender_id]
            .get(SkillId::Shield)
            .xp,
        15
    );

    // An off-hand item without defenseSkill cannot keep training Shield.
    game_state
        .inventories
        .write()
        .await
        .get_mut(&defender_id)
        .unwrap()
        .equipped
        .insert(EquipSlot::OffHand, bag_item(11, "torch", 1));
    game_state
        .broadcast_monster_attack(&owner_id, "unshielded_hit", &defender_id)
        .await;
    assert_eq!(
        game_state.player_skills.read().await[&defender_id]
            .get(SkillId::Shield)
            .xp,
        15
    );

    let xp_messages: Vec<u64> = drain(&mut defender_rx)
        .into_iter()
        .filter_map(|message| match message {
            ServerMessage::SkillXpGained {
                skill: SkillId::Shield,
                xp_amount,
                ..
            } => Some(xp_amount),
            _ => None,
        })
        .collect();
    assert_eq!(xp_messages, vec![10, 5]);

    let metrics = game_state.skill_balance_snapshot();
    assert_eq!(metrics.defense.defenses, 2);
    assert_eq!(metrics.defense.hits_taken, 1);
    assert_eq!(metrics.defense.avoids, 1);
    assert_eq!(metrics.defense.xp, 15);
    assert_eq!(metrics.defense_by_skill["shield"].defenses, 2);
    assert_eq!(metrics.defense_by_skill_band[0].defenses, 2);
    assert_eq!(metrics.defense_xp_messages, 2);
    assert_eq!(metrics.defense_rows_created, 1);
}

#[tokio::test]
async fn shield_bonus_threshold_pushes_an_immediate_guard_update() {
    let game_state = make_test_game_state("shield_guard_level_up");
    let owner_id = pid("threshold_owner");
    let defender_id = pid("threshold_defender");
    game_state
        .add_player(make_player("threshold_owner", 0.0, 0.0))
        .await;
    game_state
        .add_player(make_player("threshold_defender", 0.0, 0.0))
        .await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 20;
    game_state
        .register_player_character(&defender_id, 3, 0, attrs, 0, None)
        .await;
    let mut skills = Skills::default();
    skills.map.insert(
        SkillId::Shield,
        SkillProgress {
            level: 4,
            xp: skill_xp_for_level(5) - 5,
        },
    );
    game_state
        .register_player_skills(&defender_id, skills)
        .await;
    let mut inventory = PlayerInventory::default();
    inventory
        .equipped
        .insert(EquipSlot::OffHand, bag_item(20, "wooden_shield", 1));
    game_state
        .inventories
        .write()
        .await
        .insert(defender_id, inventory);
    let mut rx = game_state.register_direct_channel(&defender_id).await;
    let mut monster = make_monster("threshold_miss", pos(0.0), 0);
    monster.owner_id = Some(owner_id);
    game_state
        .monsters
        .write()
        .await
        .insert(monster.id.clone(), monster);

    assert_eq!(game_state.effective_guard(&defender_id).await, 21);
    game_state
        .broadcast_monster_attack(&owner_id, "threshold_miss", &defender_id)
        .await;

    let messages = drain(&mut rx);
    assert!(matches!(
        messages.first(),
        Some(ServerMessage::SkillXpGained {
            skill: SkillId::Shield,
            new_level: 5,
            leveled_up: true,
            ..
        })
    ));
    assert!(messages
        .iter()
        .any(|message| matches!(message, ServerMessage::GuardUpdated { guard: 22 })));
    assert_eq!(game_state.effective_guard(&defender_id).await, 22);
}

#[tokio::test]
async fn armor_skill_bonus_is_anchored_to_the_mapped_primary_chest() {
    let game_state = make_test_game_state("leather_armor_profile");
    let defender_id = pid("leather_guardian");
    game_state
        .add_player(make_player("leather_guardian", 0.0, 0.0))
        .await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 10;
    game_state
        .register_player_character(&defender_id, 4, 0, attrs, 0, None)
        .await;
    let mut skills = Skills::default();
    for (skill, level) in [
        (SkillId::Shield, 15),
        (SkillId::LeatherArmor, 25),
        (SkillId::MailArmor, 15),
        (SkillId::PlateArmor, 5),
        (SkillId::PaddedArmor, 15),
        (SkillId::HybridArmor, 25),
    ] {
        skills.map.insert(
            skill,
            SkillProgress {
                level,
                xp: skill_xp_for_level(level),
            },
        );
    }
    game_state
        .register_player_skills(&defender_id, skills)
        .await;

    let mut inventory = PlayerInventory::default();
    inventory
        .equipped
        .insert(EquipSlot::OffHand, bag_item(30, "wooden_shield", 1));
    inventory
        .equipped
        .insert(EquipSlot::Chest, bag_item(31, "leather_armor", 1));
    game_state
        .inventories
        .write()
        .await
        .insert(defender_id, inventory);

    let profile = game_state.player_defense_profile(&defender_id).await;
    assert_eq!(profile.shield_skill, Some(SkillId::Shield));
    assert_eq!(profile.shield_skill_guard_bonus, 2);
    assert_eq!(profile.armor_skill, Some(SkillId::LeatherArmor));
    assert_eq!(profile.armor_skill_level, 25);
    assert_eq!(profile.armor_skill_guard_bonus, 3);
    assert_eq!(profile.effective_guard, 18); // base 10 + gear 3 + skills 2 + 3

    game_state
        .inventories
        .write()
        .await
        .get_mut(&defender_id)
        .unwrap()
        .equipped
        .insert(EquipSlot::Chest, bag_item(32, "chain_mail", 1));
    let mail_profile = game_state.player_defense_profile(&defender_id).await;
    assert_eq!(mail_profile.armor_skill, Some(SkillId::MailArmor));
    assert_eq!(mail_profile.armor_skill_level, 15);
    assert_eq!(mail_profile.armor_skill_guard_bonus, 2);
    assert_eq!(mail_profile.effective_guard, 20); // base 10 + gear 6 + skills 2 + 2

    game_state
        .inventories
        .write()
        .await
        .get_mut(&defender_id)
        .unwrap()
        .equipped
        .insert(EquipSlot::Chest, bag_item(33, "breastplate", 1));
    let plate_profile = game_state.player_defense_profile(&defender_id).await;
    assert_eq!(plate_profile.armor_skill, Some(SkillId::PlateArmor));
    assert_eq!(plate_profile.armor_skill_level, 5);
    assert_eq!(plate_profile.armor_skill_guard_bonus, 1);
    assert_eq!(plate_profile.effective_guard, 21); // base 10 + gear 8 + skills 2 + 1

    game_state
        .inventories
        .write()
        .await
        .get_mut(&defender_id)
        .unwrap()
        .equipped
        .insert(EquipSlot::Chest, bag_item(34, "padded_battle_robe", 1));
    let padded_profile = game_state.player_defense_profile(&defender_id).await;
    assert_eq!(padded_profile.armor_skill, Some(SkillId::PaddedArmor));
    assert_eq!(padded_profile.armor_skill_level, 15);
    assert_eq!(padded_profile.armor_skill_guard_bonus, 2);
    assert_eq!(padded_profile.effective_guard, 15); // base 10 + shield 1 + skills 2 + 2

    game_state
        .inventories
        .write()
        .await
        .get_mut(&defender_id)
        .unwrap()
        .equipped
        .insert(EquipSlot::Chest, bag_item(35, "brigandine_coat", 1));
    let hybrid_profile = game_state.player_defense_profile(&defender_id).await;
    assert_eq!(hybrid_profile.armor_skill, Some(SkillId::HybridArmor));
    assert_eq!(hybrid_profile.armor_skill_level, 25);
    assert_eq!(hybrid_profile.armor_skill_guard_bonus, 3);
    assert_eq!(hybrid_profile.effective_guard, 18); // base 10 + gear 3 + skills 2 + 3
    assert_eq!(armor_skill_guard_bonus(SkillId::LeatherArmor, 25), 3);
    assert_eq!(armor_skill_guard_bonus(SkillId::MailArmor, 15), 2);
    assert_eq!(armor_skill_guard_bonus(SkillId::PlateArmor, 5), 1);
    assert_eq!(armor_skill_guard_bonus(SkillId::PaddedArmor, 15), 2);
    assert_eq!(armor_skill_guard_bonus(SkillId::HybridArmor, 25), 3);
}

#[tokio::test]
async fn padded_primary_armor_combines_typed_mitigation_and_skill_training() {
    let game_state = make_test_game_state("padded_typed_mitigation");
    let owner_id = pid("typed_monster_owner");
    let defender_id = pid("padded_defender");
    game_state
        .add_player(make_player("typed_monster_owner", 0.0, 0.0))
        .await;
    let mut defender = make_player("padded_defender", 0.0, 0.0);
    defender.health = 1_000;
    defender.max_health = 1_000;
    game_state.add_player(defender).await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 100;
    game_state
        .register_player_character(&defender_id, 6, 0, attrs, 0, None)
        .await;
    game_state
        .register_player_skills(&defender_id, Skills::default())
        .await;
    game_state.inventories.write().await.insert(
        defender_id,
        PlayerInventory {
            bag: vec![],
            equipped: [(EquipSlot::Chest, bag_item(35, "padded_battle_robe", 1))]
                .into_iter()
                .collect(),
        },
    );
    assert_eq!(game_state.effective_guard(&defender_id).await, 100);
    let mut rx = game_state.register_direct_channel(&defender_id).await;
    let mut expected_health = 1_000;

    let mut miss = make_monster("padded_miss", pos(0.0), 0);
    miss.owner_id = Some(owner_id);
    miss.level_override = Some(0);
    game_state
        .monsters
        .write()
        .await
        .insert(miss.id.clone(), miss);
    game_state
        .broadcast_monster_attack(&owner_id, "padded_miss", &defender_id)
        .await;
    assert!(drain(&mut rx).iter().any(|message| matches!(
        message,
        ServerMessage::MonsterAttackedPlayer { hit: false, .. }
    )));
    assert!(!game_state.player_skills.read().await[&defender_id]
        .map
        .contains_key(&SkillId::PaddedArmor));

    for (id, monster_type, expected_type, expected_mitigation) in [
        ("typed_slash", "goblin", PhysicalDamageType::Slash, 1),
        ("typed_pierce", "scp939", PhysicalDamageType::Pierce, 0),
        (
            "legacy_untyped",
            "test_monster",
            PhysicalDamageType::Untyped,
            0,
        ),
    ] {
        let mut monster = make_monster(id, pos(0.0), 0);
        monster.monster_type = monster_type.to_string();
        monster.owner_id = Some(owner_id);
        monster.level_override = Some(u8::MAX);
        game_state
            .monsters
            .write()
            .await
            .insert(id.to_string(), monster);

        game_state
            .broadcast_monster_attack(&owner_id, id, &defender_id)
            .await;
        let message = drain(&mut rx)
            .into_iter()
            .find(|message| {
                matches!(
                    message,
                    ServerMessage::MonsterAttackedPlayer { monster_id, .. }
                        if monster_id == id
                )
            })
            .expect("typed monster attack outcome");
        match message {
            ServerMessage::MonsterAttackedPlayer {
                hit,
                damage_type,
                raw_damage,
                mitigated_damage,
                damage,
                current_health,
                ..
            } => {
                assert!(hit);
                assert_eq!(damage_type, expected_type);
                assert_eq!(mitigated_damage, expected_mitigation);
                assert_eq!(damage + mitigated_damage, raw_damage);
                assert!(damage >= 1);
                expected_health -= damage;
                assert_eq!(current_health, expected_health);
            }
            _ => unreachable!(),
        }
    }

    let progress = game_state.player_skills.read().await[&defender_id].get(SkillId::PaddedArmor);
    assert_eq!(progress.xp, 15);
    let metrics = game_state.skill_balance_snapshot();
    assert_eq!(metrics.defense.defenses, 4);
    assert_eq!(metrics.defense.hits_taken, 3);
    assert_eq!(metrics.defense.avoids, 1);
    assert_eq!(metrics.defense.xp, 15);
    assert_eq!(metrics.defense_by_skill["padded_armor"].defenses, 4);
    assert_eq!(metrics.defense_by_skill["padded_armor"].xp, 15);
    assert_eq!(metrics.mitigation.hits, 3);
    assert_eq!(metrics.mitigation.mitigated_damage, 1);
    assert_eq!(metrics.mitigation_by_type["slash"].mitigated_damage, 1);
    assert_eq!(
        metrics.mitigation_by_construction["padded"].mitigated_damage,
        1
    );
}

#[tokio::test]
async fn leather_primary_armor_balances_typed_mitigation_and_skill_training() {
    let game_state = make_test_game_state("leather_typed_mitigation");
    let owner_id = pid("leather_monster_owner");
    let defender_id = pid("leather_defender");
    game_state
        .add_player(make_player("leather_monster_owner", 0.0, 0.0))
        .await;
    let mut defender = make_player("leather_defender", 0.0, 0.0);
    defender.health = 1_000;
    defender.max_health = 1_000;
    game_state.add_player(defender).await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 0;
    game_state
        .register_player_character(&defender_id, 6, 0, attrs, 0, None)
        .await;
    game_state
        .register_player_skills(&defender_id, Skills::default())
        .await;
    game_state.inventories.write().await.insert(
        defender_id,
        PlayerInventory {
            bag: vec![],
            equipped: [(EquipSlot::Chest, bag_item(36, "leather_armor", 1))]
                .into_iter()
                .collect(),
        },
    );
    assert_eq!(game_state.effective_guard(&defender_id).await, 2);
    let mut rx = game_state.register_direct_channel(&defender_id).await;
    let mut expected_health = 1_000;

    for (id, monster_type, expected_type, expected_mitigation) in [
        ("leather_slash", "goblin", PhysicalDamageType::Slash, 1),
        ("leather_pierce", "scp939", PhysicalDamageType::Pierce, 1),
        (
            "leather_untyped",
            "test_monster",
            PhysicalDamageType::Untyped,
            0,
        ),
    ] {
        let mut monster = make_monster(id, pos(0.0), 0);
        monster.monster_type = monster_type.to_string();
        monster.owner_id = Some(owner_id);
        monster.level_override = Some(u8::MAX);
        game_state
            .monsters
            .write()
            .await
            .insert(id.to_string(), monster);

        game_state
            .broadcast_monster_attack(&owner_id, id, &defender_id)
            .await;
        let message = drain(&mut rx)
            .into_iter()
            .find(|message| {
                matches!(
                    message,
                    ServerMessage::MonsterAttackedPlayer { monster_id, .. }
                        if monster_id == id
                )
            })
            .expect("typed monster attack outcome");
        match message {
            ServerMessage::MonsterAttackedPlayer {
                hit,
                damage_type,
                raw_damage,
                mitigated_damage,
                damage,
                current_health,
                ..
            } => {
                assert!(hit);
                assert_eq!(damage_type, expected_type);
                assert_eq!(mitigated_damage, expected_mitigation);
                assert_eq!(damage + mitigated_damage, raw_damage);
                assert!(damage >= 1);
                expected_health -= damage;
                assert_eq!(current_health, expected_health);
            }
            _ => unreachable!(),
        }
    }

    let progress = game_state.player_skills.read().await[&defender_id].get(SkillId::LeatherArmor);
    assert_eq!(progress.xp, 15);
    let metrics = game_state.skill_balance_snapshot();
    assert_eq!(metrics.defense_by_skill["leather_armor"].defenses, 3);
    assert_eq!(metrics.defense_by_skill["leather_armor"].hits_taken, 3);
    assert_eq!(metrics.defense_by_skill["leather_armor"].xp, 15);
    assert_eq!(metrics.mitigation.hits, 3);
    assert_eq!(metrics.mitigation.mitigated_damage, 2);
    assert_eq!(metrics.mitigation_by_type["slash"].mitigated_damage, 1);
    assert_eq!(metrics.mitigation_by_type["pierce"].mitigated_damage, 1);
    assert_eq!(
        metrics.mitigation_by_construction["leather"].mitigated_damage,
        2
    );
}

#[tokio::test]
async fn mail_primary_armor_combines_typed_mitigation_and_skill_training() {
    let game_state = make_test_game_state("mail_typed_mitigation");
    let owner_id = pid("mail_monster_owner");
    let defender_id = pid("mail_defender");
    game_state
        .add_player(make_player("mail_monster_owner", 0.0, 0.0))
        .await;
    let mut defender = make_player("mail_defender", 0.0, 0.0);
    defender.health = 1_000;
    defender.max_health = 1_000;
    game_state.add_player(defender).await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 0;
    game_state
        .register_player_character(&defender_id, 6, 0, attrs, 0, None)
        .await;
    game_state
        .register_player_skills(&defender_id, Skills::default())
        .await;
    game_state.inventories.write().await.insert(
        defender_id,
        PlayerInventory {
            bag: vec![],
            equipped: [(EquipSlot::Chest, bag_item(37, "chain_mail", 1))]
                .into_iter()
                .collect(),
        },
    );
    assert_eq!(game_state.effective_guard(&defender_id).await, 5);
    let mut rx = game_state.register_direct_channel(&defender_id).await;
    let mut expected_health = 1_000;

    for (id, monster_type, expected_type, expected_mitigation) in [
        ("mail_slash", "goblin", PhysicalDamageType::Slash, 2),
        ("mail_pierce", "scp939", PhysicalDamageType::Pierce, 1),
        (
            "mail_untyped",
            "test_monster",
            PhysicalDamageType::Untyped,
            0,
        ),
    ] {
        let mut monster = make_monster(id, pos(0.0), 0);
        monster.monster_type = monster_type.to_string();
        monster.owner_id = Some(owner_id);
        monster.level_override = Some(u8::MAX);
        game_state
            .monsters
            .write()
            .await
            .insert(id.to_string(), monster);

        game_state
            .broadcast_monster_attack(&owner_id, id, &defender_id)
            .await;
        let message = drain(&mut rx)
            .into_iter()
            .find(|message| {
                matches!(
                    message,
                    ServerMessage::MonsterAttackedPlayer { monster_id, .. }
                        if monster_id == id
                )
            })
            .expect("typed monster attack outcome");
        match message {
            ServerMessage::MonsterAttackedPlayer {
                hit,
                damage_type,
                raw_damage,
                mitigated_damage,
                damage,
                current_health,
                ..
            } => {
                assert!(hit);
                assert_eq!(damage_type, expected_type);
                assert_eq!(mitigated_damage, expected_mitigation);
                assert_eq!(damage + mitigated_damage, raw_damage);
                assert!(damage >= 1);
                expected_health -= damage;
                assert_eq!(current_health, expected_health);
            }
            _ => unreachable!(),
        }
    }

    let progress = game_state.player_skills.read().await[&defender_id].get(SkillId::MailArmor);
    assert_eq!(progress.xp, 15);
    let metrics = game_state.skill_balance_snapshot();
    assert_eq!(metrics.defense.defenses, 3);
    assert_eq!(metrics.defense.hits_taken, 3);
    assert_eq!(metrics.defense.xp, 15);
    assert_eq!(metrics.defense_by_skill["mail_armor"].defenses, 3);
    assert_eq!(metrics.defense_by_skill["mail_armor"].xp, 15);
    assert_eq!(metrics.mitigation.hits, 3);
    assert_eq!(metrics.mitigation.mitigated_damage, 3);
    assert_eq!(metrics.mitigation_by_type["slash"].mitigated_damage, 2);
    assert_eq!(metrics.mitigation_by_type["pierce"].mitigated_damage, 1);
    assert_eq!(
        metrics.mitigation_by_construction["mail"].mitigated_damage,
        3
    );
}

#[tokio::test]
async fn plate_primary_armor_combines_broad_mitigation_and_skill_training() {
    let game_state = make_test_game_state("plate_typed_mitigation");
    let owner_id = pid("plate_monster_owner");
    let defender_id = pid("plate_defender");
    game_state
        .add_player(make_player("plate_monster_owner", 0.0, 0.0))
        .await;
    let mut defender = make_player("plate_defender", 0.0, 0.0);
    defender.health = 1_000;
    defender.max_health = 1_000;
    game_state.add_player(defender).await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 0;
    game_state
        .register_player_character(&defender_id, 6, 0, attrs, 0, None)
        .await;
    game_state
        .register_player_skills(&defender_id, Skills::default())
        .await;
    game_state.inventories.write().await.insert(
        defender_id,
        PlayerInventory {
            bag: vec![],
            equipped: [(EquipSlot::Chest, bag_item(38, "breastplate", 1))]
                .into_iter()
                .collect(),
        },
    );
    assert_eq!(game_state.effective_guard(&defender_id).await, 7);
    let defense = game_state.player_defense_profile(&defender_id).await;
    assert_eq!(defense.armor_coverage_percent, 40);
    assert_eq!(
        defense.weighted_armor_protection,
        PhysicalProtection {
            slash: 2,
            pierce: 2,
            blunt: 1,
        }
    );
    let mut rx = game_state.register_direct_channel(&defender_id).await;
    let mut expected_health = 1_000;
    let mut observed_total_mitigation = 0;
    let mut observed_slash_mitigation = 0;
    let mut observed_pierce_mitigation = 0;

    for (id, monster_type, expected_type, profile_protection) in [
        ("plate_slash", "goblin", PhysicalDamageType::Slash, 2),
        ("plate_pierce", "scp939", PhysicalDamageType::Pierce, 2),
        (
            "plate_untyped",
            "test_monster",
            PhysicalDamageType::Untyped,
            0,
        ),
    ] {
        let mut monster = make_monster(id, pos(0.0), 0);
        monster.monster_type = monster_type.to_string();
        monster.owner_id = Some(owner_id);
        monster.level_override = Some(u8::MAX);
        game_state
            .monsters
            .write()
            .await
            .insert(id.to_string(), monster);

        game_state
            .broadcast_monster_attack(&owner_id, id, &defender_id)
            .await;
        let message = drain(&mut rx)
            .into_iter()
            .find(|message| {
                matches!(
                    message,
                    ServerMessage::MonsterAttackedPlayer { monster_id, .. }
                        if monster_id == id
                )
            })
            .expect("typed monster attack outcome");
        match message {
            ServerMessage::MonsterAttackedPlayer {
                hit,
                damage_type,
                raw_damage,
                mitigated_damage,
                damage,
                current_health,
                ..
            } => {
                assert!(hit);
                assert_eq!(damage_type, expected_type);
                assert_eq!(
                    mitigated_damage,
                    profile_protection.min(raw_damage.saturating_sub(1))
                );
                assert_eq!(damage + mitigated_damage, raw_damage);
                assert!(damage >= 1);
                observed_total_mitigation += mitigated_damage;
                match damage_type {
                    PhysicalDamageType::Slash => {
                        observed_slash_mitigation += mitigated_damage;
                    }
                    PhysicalDamageType::Pierce => {
                        observed_pierce_mitigation += mitigated_damage;
                    }
                    _ => {}
                }
                expected_health -= damage;
                assert_eq!(current_health, expected_health);
            }
            _ => unreachable!(),
        }
    }

    let progress = game_state.player_skills.read().await[&defender_id].get(SkillId::PlateArmor);
    assert_eq!(progress.xp, 15);
    let metrics = game_state.skill_balance_snapshot();
    assert_eq!(metrics.defense.defenses, 3);
    assert_eq!(metrics.defense.hits_taken, 3);
    assert_eq!(metrics.defense.xp, 15);
    assert_eq!(metrics.defense_by_skill["plate_armor"].defenses, 3);
    assert_eq!(metrics.defense_by_skill["plate_armor"].xp, 15);
    assert_eq!(metrics.mitigation.hits, 3);
    assert_eq!(
        metrics.mitigation.mitigated_damage,
        u64::from(observed_total_mitigation)
    );
    assert_eq!(
        metrics.mitigation_by_type["slash"].mitigated_damage,
        u64::from(observed_slash_mitigation)
    );
    assert_eq!(
        metrics.mitigation_by_type["pierce"].mitigated_damage,
        u64::from(observed_pierce_mitigation)
    );
    assert_eq!(
        metrics.mitigation_by_construction["plate"].mitigated_damage,
        u64::from(observed_total_mitigation)
    );
    assert_eq!(metrics.mitigation_by_coverage_band[1].hits, 3);
    assert_eq!(
        metrics.mitigation_by_coverage_band[1].mitigated_damage,
        u64::from(observed_total_mitigation)
    );
}

#[tokio::test]
async fn hybrid_primary_armor_combines_balanced_mitigation_and_skill_training() {
    let game_state = make_test_game_state("hybrid_typed_mitigation");
    let owner_id = pid("hybrid_monster_owner");
    let defender_id = pid("hybrid_defender");
    game_state
        .add_player(make_player("hybrid_monster_owner", 0.0, 0.0))
        .await;
    let mut defender = make_player("hybrid_defender", 0.0, 0.0);
    defender.health = 1_000;
    defender.max_health = 1_000;
    game_state.add_player(defender).await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 100;
    game_state
        .register_player_character(&defender_id, 6, 0, attrs, 0, None)
        .await;
    game_state
        .register_player_skills(&defender_id, Skills::default())
        .await;
    game_state.inventories.write().await.insert(
        defender_id,
        PlayerInventory {
            bag: vec![],
            equipped: [(EquipSlot::Chest, bag_item(39, "brigandine_coat", 1))]
                .into_iter()
                .collect(),
        },
    );
    assert_eq!(game_state.effective_guard(&defender_id).await, 102);
    let mut rx = game_state.register_direct_channel(&defender_id).await;
    let mut expected_health = 1_000;
    let mut observed_total_mitigation = 0;
    let mut observed_slash_mitigation = 0;
    let mut observed_pierce_mitigation = 0;

    let mut miss = make_monster("hybrid_miss", pos(0.0), 0);
    miss.owner_id = Some(owner_id);
    miss.level_override = Some(0);
    game_state
        .monsters
        .write()
        .await
        .insert(miss.id.clone(), miss);
    game_state
        .broadcast_monster_attack(&owner_id, "hybrid_miss", &defender_id)
        .await;
    assert!(drain(&mut rx).iter().any(|message| matches!(
        message,
        ServerMessage::MonsterAttackedPlayer { hit: false, .. }
    )));
    assert!(!game_state.player_skills.read().await[&defender_id]
        .map
        .contains_key(&SkillId::HybridArmor));

    for (id, monster_type, expected_type, profile_protection) in [
        ("hybrid_slash", "goblin", PhysicalDamageType::Slash, 2),
        ("hybrid_pierce", "scp939", PhysicalDamageType::Pierce, 2),
        (
            "hybrid_untyped",
            "test_monster",
            PhysicalDamageType::Untyped,
            0,
        ),
    ] {
        let mut monster = make_monster(id, pos(0.0), 0);
        monster.monster_type = monster_type.to_string();
        monster.owner_id = Some(owner_id);
        monster.level_override = Some(u8::MAX);
        game_state
            .monsters
            .write()
            .await
            .insert(id.to_string(), monster);

        game_state
            .broadcast_monster_attack(&owner_id, id, &defender_id)
            .await;
        let message = drain(&mut rx)
            .into_iter()
            .find(|message| {
                matches!(
                    message,
                    ServerMessage::MonsterAttackedPlayer { monster_id, .. }
                        if monster_id == id
                )
            })
            .expect("typed monster attack outcome");
        match message {
            ServerMessage::MonsterAttackedPlayer {
                hit,
                damage_type,
                raw_damage,
                mitigated_damage,
                damage,
                current_health,
                ..
            } => {
                assert!(hit);
                assert_eq!(damage_type, expected_type);
                assert_eq!(
                    mitigated_damage,
                    profile_protection.min(raw_damage.saturating_sub(1))
                );
                assert_eq!(damage + mitigated_damage, raw_damage);
                assert!(damage >= 1);
                observed_total_mitigation += mitigated_damage;
                match damage_type {
                    PhysicalDamageType::Slash => {
                        observed_slash_mitigation += mitigated_damage;
                    }
                    PhysicalDamageType::Pierce => {
                        observed_pierce_mitigation += mitigated_damage;
                    }
                    _ => {}
                }
                expected_health -= damage;
                assert_eq!(current_health, expected_health);
            }
            _ => unreachable!(),
        }
    }

    let progress = game_state.player_skills.read().await[&defender_id].get(SkillId::HybridArmor);
    assert_eq!(progress.xp, 15);
    let metrics = game_state.skill_balance_snapshot();
    assert_eq!(metrics.defense.defenses, 4);
    assert_eq!(metrics.defense.hits_taken, 3);
    assert_eq!(metrics.defense.avoids, 1);
    assert_eq!(metrics.defense.xp, 15);
    assert_eq!(metrics.defense_by_skill["hybrid_armor"].defenses, 4);
    assert_eq!(metrics.defense_by_skill["hybrid_armor"].xp, 15);
    assert_eq!(metrics.mitigation.hits, 3);
    assert_eq!(
        metrics.mitigation.mitigated_damage,
        u64::from(observed_total_mitigation)
    );
    assert_eq!(
        metrics.mitigation_by_type["slash"].mitigated_damage,
        u64::from(observed_slash_mitigation)
    );
    assert_eq!(
        metrics.mitigation_by_type["pierce"].mitigated_damage,
        u64::from(observed_pierce_mitigation)
    );
    assert_eq!(
        metrics.mitigation_by_construction["hybrid"].mitigated_damage,
        u64::from(observed_total_mitigation)
    );
}

#[tokio::test]
async fn landed_monster_hits_train_only_the_mapped_active_armor_skill() {
    let game_state = make_test_game_state("leather_armor_training");
    let owner_id = pid("armor_monster_owner");
    let defender_id = pid("armor_defender");
    game_state
        .add_player(make_player("armor_monster_owner", 0.0, 0.0))
        .await;
    let mut defender = make_player("armor_defender", 0.0, 0.0);
    defender.health = 100;
    defender.max_health = 100;
    game_state.add_player(defender).await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 20;
    game_state
        .register_player_character(&defender_id, 5, 0, attrs, 0, None)
        .await;
    game_state
        .register_player_skills(&defender_id, Skills::default())
        .await;
    let mut inventory = PlayerInventory::default();
    inventory
        .equipped
        .insert(EquipSlot::Chest, bag_item(40, "leather_armor", 1));
    game_state
        .inventories
        .write()
        .await
        .insert(defender_id, inventory);
    let mut defender_rx = game_state.register_direct_channel(&defender_id).await;

    {
        let mut monsters = game_state.monsters.write().await;
        for (id, level_override) in [
            ("armor_miss", None),
            ("armor_hit", Some(u8::MAX)),
            ("mail_armor_hit", Some(u8::MAX)),
            ("plate_armor_miss", None),
            ("plate_armor_hit", Some(u8::MAX)),
        ] {
            let mut monster = make_monster(id, pos(0.0), 0);
            monster.owner_id = Some(owner_id);
            monster.level_override = level_override;
            monsters.insert(id.to_string(), monster);
        }
    }

    game_state
        .broadcast_monster_attack(&owner_id, "armor_miss", &defender_id)
        .await;
    assert!(!game_state.player_skills.read().await[&defender_id]
        .map
        .contains_key(&SkillId::LeatherArmor));

    game_state
        .broadcast_monster_attack(&owner_id, "armor_hit", &defender_id)
        .await;
    let progress = game_state.player_skills.read().await[&defender_id].get(SkillId::LeatherArmor);
    assert_eq!(progress.xp, 5);
    assert_eq!(progress.level, 0);
    assert_eq!(game_state.effective_guard(&defender_id).await, 22);

    game_state
        .inventories
        .write()
        .await
        .get_mut(&defender_id)
        .unwrap()
        .equipped
        .insert(EquipSlot::Chest, bag_item(41, "chain_mail", 1));
    game_state
        .broadcast_monster_attack(&owner_id, "mail_armor_hit", &defender_id)
        .await;
    assert_eq!(
        game_state.player_skills.read().await[&defender_id]
            .get(SkillId::LeatherArmor)
            .xp,
        5
    );
    assert_eq!(
        game_state.player_skills.read().await[&defender_id]
            .get(SkillId::MailArmor)
            .xp,
        5
    );

    game_state
        .inventories
        .write()
        .await
        .get_mut(&defender_id)
        .unwrap()
        .equipped
        .insert(EquipSlot::Chest, bag_item(42, "breastplate", 1));
    game_state
        .broadcast_monster_attack(&owner_id, "plate_armor_miss", &defender_id)
        .await;
    assert!(!game_state.player_skills.read().await[&defender_id]
        .map
        .contains_key(&SkillId::PlateArmor));
    game_state
        .broadcast_monster_attack(&owner_id, "plate_armor_hit", &defender_id)
        .await;
    assert_eq!(
        game_state.player_skills.read().await[&defender_id]
            .get(SkillId::MailArmor)
            .xp,
        5
    );
    assert_eq!(
        game_state.player_skills.read().await[&defender_id]
            .get(SkillId::PlateArmor)
            .xp,
        5
    );

    let messages = drain(&mut defender_rx);
    assert!(messages.iter().any(|message| matches!(
        message,
        ServerMessage::SkillXpGained {
            skill: SkillId::LeatherArmor,
            xp_amount: 5,
            new_level: 0,
            ..
        }
    )));
    assert!(messages.iter().any(|message| matches!(
        message,
        ServerMessage::SkillXpGained {
            skill: SkillId::MailArmor,
            xp_amount: 5,
            new_level: 0,
            ..
        }
    )));
    assert!(messages.iter().any(|message| matches!(
        message,
        ServerMessage::SkillXpGained {
            skill: SkillId::PlateArmor,
            xp_amount: 5,
            new_level: 0,
            ..
        }
    )));
    assert!(!messages
        .iter()
        .any(|message| matches!(message, ServerMessage::GuardUpdated { .. })));

    let metrics = game_state.skill_balance_snapshot();
    assert_eq!(metrics.defense.defenses, 5);
    assert_eq!(metrics.defense.hits_taken, 3);
    assert_eq!(metrics.defense.avoids, 2);
    assert_eq!(metrics.defense.xp, 15);
    assert_eq!(metrics.defense_by_skill["leather_armor"].defenses, 2);
    assert_eq!(metrics.defense_by_skill["mail_armor"].defenses, 1);
    assert_eq!(metrics.defense_by_skill["plate_armor"].defenses, 2);
    assert_eq!(metrics.defense_xp_messages, 3);
    assert_eq!(metrics.defense_rows_created, 3);
}

#[tokio::test]
async fn leather_armor_bonus_threshold_pushes_one_combined_guard_update() {
    let game_state = make_test_game_state("leather_armor_guard_level_up");
    let owner_id = pid("armor_threshold_owner");
    let defender_id = pid("armor_threshold_defender");
    game_state
        .add_player(make_player("armor_threshold_owner", 0.0, 0.0))
        .await;
    let mut defender = make_player("armor_threshold_defender", 0.0, 0.0);
    defender.health = 100;
    defender.max_health = 100;
    game_state.add_player(defender).await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 20;
    game_state
        .register_player_character(&defender_id, 6, 0, attrs, 0, None)
        .await;
    let mut skills = Skills::default();
    skills.map.insert(
        SkillId::LeatherArmor,
        SkillProgress {
            level: 4,
            xp: skill_xp_for_level(5) - 5,
        },
    );
    skills.map.insert(
        SkillId::Shield,
        SkillProgress {
            level: 15,
            xp: skill_xp_for_level(15),
        },
    );
    game_state
        .register_player_skills(&defender_id, skills)
        .await;
    let mut inventory = PlayerInventory::default();
    inventory
        .equipped
        .insert(EquipSlot::Chest, bag_item(50, "leather_armor", 1));
    inventory
        .equipped
        .insert(EquipSlot::OffHand, bag_item(51, "wooden_shield", 1));
    game_state
        .inventories
        .write()
        .await
        .insert(defender_id, inventory);
    let mut rx = game_state.register_direct_channel(&defender_id).await;
    let mut monster = make_monster("armor_threshold_hit", pos(0.0), 0);
    monster.owner_id = Some(owner_id);
    monster.level_override = Some(u8::MAX);
    game_state
        .monsters
        .write()
        .await
        .insert(monster.id.clone(), monster);

    assert_eq!(game_state.effective_guard(&defender_id).await, 25);
    game_state
        .broadcast_monster_attack(&owner_id, "armor_threshold_hit", &defender_id)
        .await;

    let messages = drain(&mut rx);
    assert!(messages.iter().any(|message| matches!(
        message,
        ServerMessage::SkillXpGained {
            skill: SkillId::LeatherArmor,
            new_level: 5,
            leveled_up: true,
            ..
        }
    )));
    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(message, ServerMessage::GuardUpdated { guard: 26 }))
            .count(),
        1
    );
    assert_eq!(game_state.effective_guard(&defender_id).await, 26);
}

#[tokio::test]
async fn mail_armor_bonus_threshold_pushes_the_authoritative_guard_update() {
    let game_state = make_test_game_state("mail_armor_guard_level_up");
    let owner_id = pid("mail_threshold_owner");
    let defender_id = pid("mail_threshold_defender");
    game_state
        .add_player(make_player("mail_threshold_owner", 0.0, 0.0))
        .await;
    let mut defender = make_player("mail_threshold_defender", 0.0, 0.0);
    defender.health = 100;
    defender.max_health = 100;
    game_state.add_player(defender).await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 20;
    game_state
        .register_player_character(&defender_id, 7, 0, attrs, 0, None)
        .await;
    let mut skills = Skills::default();
    skills.map.insert(
        SkillId::MailArmor,
        SkillProgress {
            level: 4,
            xp: skill_xp_for_level(5) - 5,
        },
    );
    game_state
        .register_player_skills(&defender_id, skills)
        .await;
    game_state.inventories.write().await.insert(
        defender_id,
        PlayerInventory {
            bag: vec![],
            equipped: [(EquipSlot::Chest, bag_item(52, "chain_mail", 1))]
                .into_iter()
                .collect(),
        },
    );
    let mut rx = game_state.register_direct_channel(&defender_id).await;
    let mut monster = make_monster("mail_threshold_hit", pos(0.0), 0);
    monster.owner_id = Some(owner_id);
    monster.level_override = Some(u8::MAX);
    game_state
        .monsters
        .write()
        .await
        .insert(monster.id.clone(), monster);

    assert_eq!(game_state.effective_guard(&defender_id).await, 25);
    game_state
        .broadcast_monster_attack(&owner_id, "mail_threshold_hit", &defender_id)
        .await;

    let messages = drain(&mut rx);
    assert!(messages.iter().any(|message| matches!(
        message,
        ServerMessage::SkillXpGained {
            skill: SkillId::MailArmor,
            new_level: 5,
            leveled_up: true,
            ..
        }
    )));
    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(message, ServerMessage::GuardUpdated { guard: 26 }))
            .count(),
        1
    );
    assert_eq!(game_state.effective_guard(&defender_id).await, 26);
}

#[tokio::test]
async fn plate_armor_bonus_threshold_pushes_the_authoritative_guard_update() {
    let game_state = make_test_game_state("plate_armor_guard_level_up");
    let owner_id = pid("plate_threshold_owner");
    let defender_id = pid("plate_threshold_defender");
    game_state
        .add_player(make_player("plate_threshold_owner", 0.0, 0.0))
        .await;
    let mut defender = make_player("plate_threshold_defender", 0.0, 0.0);
    defender.health = 100;
    defender.max_health = 100;
    game_state.add_player(defender).await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 20;
    game_state
        .register_player_character(&defender_id, 8, 0, attrs, 0, None)
        .await;
    let mut skills = Skills::default();
    skills.map.insert(
        SkillId::PlateArmor,
        SkillProgress {
            level: 4,
            xp: skill_xp_for_level(5) - 5,
        },
    );
    game_state
        .register_player_skills(&defender_id, skills)
        .await;
    game_state.inventories.write().await.insert(
        defender_id,
        PlayerInventory {
            bag: vec![],
            equipped: [(EquipSlot::Chest, bag_item(53, "breastplate", 1))]
                .into_iter()
                .collect(),
        },
    );
    let mut rx = game_state.register_direct_channel(&defender_id).await;
    let mut monster = make_monster("plate_threshold_hit", pos(0.0), 0);
    monster.owner_id = Some(owner_id);
    monster.level_override = Some(u8::MAX);
    game_state
        .monsters
        .write()
        .await
        .insert(monster.id.clone(), monster);

    assert_eq!(game_state.effective_guard(&defender_id).await, 27);
    game_state
        .broadcast_monster_attack(&owner_id, "plate_threshold_hit", &defender_id)
        .await;

    let messages = drain(&mut rx);
    assert!(messages.iter().any(|message| matches!(
        message,
        ServerMessage::SkillXpGained {
            skill: SkillId::PlateArmor,
            new_level: 5,
            leveled_up: true,
            ..
        }
    )));
    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(message, ServerMessage::GuardUpdated { guard: 28 }))
            .count(),
        1
    );
    assert_eq!(game_state.effective_guard(&defender_id).await, 28);
}

#[tokio::test]
async fn padded_armor_bonus_threshold_adds_guard_to_mitigation_only_armor() {
    let game_state = make_test_game_state("padded_armor_guard_level_up");
    let owner_id = pid("padded_threshold_owner");
    let defender_id = pid("padded_threshold_defender");
    game_state
        .add_player(make_player("padded_threshold_owner", 0.0, 0.0))
        .await;
    let mut defender = make_player("padded_threshold_defender", 0.0, 0.0);
    defender.health = 100;
    defender.max_health = 100;
    game_state.add_player(defender).await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 20;
    game_state
        .register_player_character(&defender_id, 9, 0, attrs, 0, None)
        .await;
    let mut skills = Skills::default();
    skills.map.insert(
        SkillId::PaddedArmor,
        SkillProgress {
            level: 4,
            xp: skill_xp_for_level(5) - 5,
        },
    );
    game_state
        .register_player_skills(&defender_id, skills)
        .await;
    game_state.inventories.write().await.insert(
        defender_id,
        PlayerInventory {
            bag: vec![],
            equipped: [(EquipSlot::Chest, bag_item(54, "padded_battle_robe", 1))]
                .into_iter()
                .collect(),
        },
    );
    let mut rx = game_state.register_direct_channel(&defender_id).await;
    let mut monster = make_monster("padded_threshold_hit", pos(0.0), 0);
    monster.owner_id = Some(owner_id);
    monster.level_override = Some(u8::MAX);
    game_state
        .monsters
        .write()
        .await
        .insert(monster.id.clone(), monster);

    assert_eq!(game_state.effective_guard(&defender_id).await, 20);
    game_state
        .broadcast_monster_attack(&owner_id, "padded_threshold_hit", &defender_id)
        .await;

    let messages = drain(&mut rx);
    assert!(messages.iter().any(|message| matches!(
        message,
        ServerMessage::SkillXpGained {
            skill: SkillId::PaddedArmor,
            new_level: 5,
            leveled_up: true,
            ..
        }
    )));
    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(message, ServerMessage::GuardUpdated { guard: 21 }))
            .count(),
        1
    );
    assert_eq!(game_state.effective_guard(&defender_id).await, 21);
}

#[tokio::test]
async fn hybrid_armor_bonus_threshold_pushes_authoritative_guard_update() {
    let game_state = make_test_game_state("hybrid_armor_guard_level_up");
    let owner_id = pid("hybrid_threshold_owner");
    let defender_id = pid("hybrid_threshold_defender");
    game_state
        .add_player(make_player("hybrid_threshold_owner", 0.0, 0.0))
        .await;
    let mut defender = make_player("hybrid_threshold_defender", 0.0, 0.0);
    defender.health = 100;
    defender.max_health = 100;
    game_state.add_player(defender).await;
    let mut attrs = attrs_with_cha(10);
    attrs.guard = 20;
    game_state
        .register_player_character(&defender_id, 10, 0, attrs, 0, None)
        .await;
    let mut skills = Skills::default();
    skills.map.insert(
        SkillId::HybridArmor,
        SkillProgress {
            level: 4,
            xp: skill_xp_for_level(5) - 5,
        },
    );
    game_state
        .register_player_skills(&defender_id, skills)
        .await;
    game_state.inventories.write().await.insert(
        defender_id,
        PlayerInventory {
            bag: vec![],
            equipped: [(EquipSlot::Chest, bag_item(55, "brigandine_coat", 1))]
                .into_iter()
                .collect(),
        },
    );
    let mut rx = game_state.register_direct_channel(&defender_id).await;
    let mut monster = make_monster("hybrid_threshold_hit", pos(0.0), 0);
    monster.owner_id = Some(owner_id);
    monster.level_override = Some(u8::MAX);
    game_state
        .monsters
        .write()
        .await
        .insert(monster.id.clone(), monster);

    assert_eq!(game_state.effective_guard(&defender_id).await, 22);
    game_state
        .broadcast_monster_attack(&owner_id, "hybrid_threshold_hit", &defender_id)
        .await;

    let messages = drain(&mut rx);
    assert!(messages.iter().any(|message| matches!(
        message,
        ServerMessage::SkillXpGained {
            skill: SkillId::HybridArmor,
            new_level: 5,
            leveled_up: true,
            ..
        }
    )));
    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(message, ServerMessage::GuardUpdated { guard: 23 }))
            .count(),
        1
    );
    assert_eq!(game_state.effective_guard(&defender_id).await, 23);
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
        let mut monster = make_monster("distant_monster", pos(2.01), 0);
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

#[tokio::test]
async fn mapped_weapon_profiles_apply_shared_accuracy_range_and_cadence() {
    let game_state = make_test_game_state("sword_profiles");
    let player_id = pid("swordsman");
    let _rx = setup_weapon_attacker(&game_state, "swordsman", Some("iron_sword"), 3, 0).await;

    for (level, expected_bonus) in [(0, 0), (5, 1), (15, 2), (25, 3)] {
        let mut skills = Skills::default();
        if level > 0 {
            skills.map.insert(
                SkillId::OneHandedSword,
                SkillProgress {
                    level,
                    xp: skill_xp_for_level(level),
                },
            );
        }
        game_state.register_player_skills(&player_id, skills).await;
        let profile = game_state.player_weapon_attack_profile(&player_id).await;
        assert_eq!(profile.weapon_skill, Some(SkillId::OneHandedSword));
        assert_eq!(profile.weapon_skill_level, level);
        assert_eq!(profile.weapon_skill_attack_bonus, expected_bonus);
        assert_eq!(profile.enchant, 3);
        assert_eq!(profile.damage_dice, "1d8");
        assert_eq!(profile.damage_type, PhysicalDamageType::Slash);
        assert_eq!(profile.melee_range, DEFAULT_WEAPON_MELEE_RANGE_METERS);
        assert_eq!(
            profile.attack_cooldown,
            std::time::Duration::from_millis(u64::from(DEFAULT_WEAPON_ATTACK_COOLDOWN_MS))
        );
    }

    game_state
        .inventories
        .write()
        .await
        .get_mut(&player_id)
        .unwrap()
        .equipped
        .remove(&EquipSlot::MainHand);
    let profile = game_state.player_weapon_attack_profile(&player_id).await;
    assert_eq!(profile.weapon_skill, None);
    assert_eq!(profile.weapon_skill_attack_bonus, 0);
    assert_eq!(profile.damage_type, PhysicalDamageType::Untyped);

    game_state
        .register_player_skills(&player_id, {
            let mut skills = Skills::default();
            skills.map.insert(
                SkillId::Dagger,
                SkillProgress {
                    level: 15,
                    xp: skill_xp_for_level(15),
                },
            );
            skills
        })
        .await;
    game_state
        .inventories
        .write()
        .await
        .get_mut(&player_id)
        .unwrap()
        .equipped
        .insert(
            EquipSlot::MainHand,
            ItemInstance {
                instance_id: 3,
                item_def_id: "dagger".to_string(),
                quantity: 1,
                enchant: 0,
                durability: None,
            },
        );
    let profile = game_state.player_weapon_attack_profile(&player_id).await;
    assert_eq!(profile.weapon_skill, Some(SkillId::Dagger));
    assert_eq!(profile.weapon_skill_level, 15);
    assert_eq!(profile.weapon_skill_attack_bonus, 2);
    assert_eq!(profile.damage_dice, "1d4");
    assert_eq!(profile.damage_type, PhysicalDamageType::Slash);
    assert_eq!(profile.melee_range, DEFAULT_WEAPON_MELEE_RANGE_METERS);

    game_state
        .register_player_skills(&player_id, {
            let mut skills = Skills::default();
            skills.map.insert(
                SkillId::Spear,
                SkillProgress {
                    level: 25,
                    xp: skill_xp_for_level(25),
                },
            );
            skills
        })
        .await;
    game_state
        .inventories
        .write()
        .await
        .get_mut(&player_id)
        .unwrap()
        .equipped
        .insert(
            EquipSlot::MainHand,
            ItemInstance {
                instance_id: 4,
                item_def_id: "spear".to_string(),
                quantity: 1,
                enchant: 0,
                durability: None,
            },
        );
    let profile = game_state.player_weapon_attack_profile(&player_id).await;
    assert_eq!(profile.weapon_skill, Some(SkillId::Spear));
    assert_eq!(profile.weapon_skill_level, 25);
    assert_eq!(profile.weapon_skill_attack_bonus, 3);
    assert_eq!(profile.damage_dice, "1d6");
    assert_eq!(profile.damage_type, PhysicalDamageType::Pierce);
    assert_eq!(profile.melee_range, SPEAR_MELEE_RANGE_METERS);
    assert_eq!(
        profile.attack_cooldown,
        std::time::Duration::from_millis(u64::from(SPEAR_ATTACK_COOLDOWN_MS))
    );

    game_state
        .inventories
        .write()
        .await
        .get_mut(&player_id)
        .unwrap()
        .equipped
        .insert(EquipSlot::MainHand, bag_item(5, "torch", 1));
    let profile = game_state.player_weapon_attack_profile(&player_id).await;
    assert_eq!(profile.weapon_skill, None);
    assert_eq!(profile.damage_type, PhysicalDamageType::Blunt);
}

#[tokio::test]
async fn resolved_dagger_attack_grants_only_dagger_xp() {
    let game_state = make_test_game_state("dagger_xp");
    let player_id = pid("dagger_user");
    let mut rx = setup_trained_weapon_attacker(
        &game_state,
        "dagger_user",
        Some("dagger"),
        100,
        SkillId::Dagger,
        0,
    )
    .await;
    insert_combat_monster(&game_state, "dagger_target", pos(1.0), 0, 1_000).await;

    game_state
        .broadcast_player_attack(&player_id, "dagger_target".to_string())
        .await;

    let skills = &game_state.player_skills.read().await[&player_id];
    assert_eq!(skills.get(SkillId::Dagger).xp, 10);
    assert_eq!(
        skills.get(SkillId::OneHandedSword),
        SkillProgress::default()
    );
    assert!(drain(&mut rx).iter().any(|message| matches!(
        message,
        ServerMessage::SkillXpGained {
            skill: SkillId::Dagger,
            xp_amount: 10,
            ..
        }
    )));
    let metrics = game_state.skill_balance_snapshot();
    assert_eq!(metrics.weapon_by_skill["dagger"].attacks, 1);
    assert_eq!(metrics.weapon_by_skill["dagger"].xp, 10);
}

#[tokio::test]
async fn spear_attack_uses_its_range_cadence_and_skill_xp() {
    let game_state = make_test_game_state("spear_combat_profile");
    let player_id = pid("spear_user");
    let mut rx = setup_trained_weapon_attacker(
        &game_state,
        "spear_user",
        Some("spear"),
        100,
        SkillId::Spear,
        0,
    )
    .await;
    insert_combat_monster(&game_state, "spear_target", pos(2.9), 0, 1_000).await;

    game_state
        .broadcast_player_attack(&player_id, "spear_target".to_string())
        .await;
    assert_eq!(
        game_state.player_skills.read().await[&player_id]
            .get(SkillId::Spear)
            .xp,
        10
    );
    assert!(drain(&mut rx).iter().any(|message| matches!(
        message,
        ServerMessage::SkillXpGained {
            skill: SkillId::Spear,
            xp_amount: 10,
            ..
        }
    )));

    game_state
        .last_player_attacks
        .write()
        .await
        .insert(player_id, GameState::now_ms().saturating_sub(1_600));
    game_state
        .broadcast_player_attack(&player_id, "spear_target".to_string())
        .await;
    expect_attack_rejected(&mut rx, "spear_target", AttackRejectReason::Cooldown);
    assert_eq!(
        game_state.player_skills.read().await[&player_id]
            .get(SkillId::Spear)
            .xp,
        10
    );

    game_state
        .last_player_attacks
        .write()
        .await
        .insert(player_id, GameState::now_ms().saturating_sub(2_500));
    game_state
        .broadcast_player_attack(&player_id, "spear_target".to_string())
        .await;
    assert_eq!(
        game_state.player_skills.read().await[&player_id]
            .get(SkillId::Spear)
            .xp,
        20
    );
    let metrics = game_state.skill_balance_snapshot();
    assert_eq!(metrics.weapon_by_skill["spear"].attacks, 2);
    assert_eq!(metrics.weapon_by_skill["spear"].xp, 20);
}

#[tokio::test]
async fn resolved_sword_outcomes_grant_exact_phase_one_xp() {
    for (case, enchant, monster_health, expected_hit, expected_xp) in [
        ("miss", -100, 10, false, 5),
        ("hit", 100, 1_000, true, 10),
        ("kill", 100, 1, true, 20),
    ] {
        let game_state = make_test_game_state(&format!("sword_{case}"));
        let player_name = format!("sword_{case}_player");
        let player_id = pid(&player_name);
        let mut rx =
            setup_weapon_attacker(&game_state, &player_name, Some("iron_sword"), enchant, 0).await;
        let monster_id = format!("sword_{case}_monster");
        insert_combat_monster(&game_state, &monster_id, pos(1.0), 0, monster_health).await;

        game_state
            .broadcast_player_attack(&player_id, monster_id.clone())
            .await;

        let progress =
            game_state.player_skills.read().await[&player_id].get(SkillId::OneHandedSword);
        assert_eq!(progress.xp, expected_xp, "{case} XP");
        let messages = drain(&mut rx);
        assert!(messages.iter().any(|message| matches!(
            message,
            ServerMessage::PlayerAttacked {
                hit,
                damage_type: PhysicalDamageType::Slash,
                ..
            } if *hit == expected_hit
        )));
        assert!(messages.iter().any(|message| matches!(
            message,
            ServerMessage::SkillXpGained {
                skill: SkillId::OneHandedSword,
                xp_amount,
                ..
            } if *xp_amount == expected_xp
        )));
        if case == "kill" {
            assert_eq!(
                game_state.monsters.read().await[&monster_id].state,
                MonsterState::Dead
            );
            assert!(game_state.player_characters.read().await[&player_id].1 > 0);
        }
    }
}

#[tokio::test]
async fn rejected_sword_attacks_never_create_skill_progress() {
    let game_state = make_test_game_state("rejected_sword_xp");
    let player_id = pid("rejected_swordsman");
    let mut rx = setup_weapon_attacker(
        &game_state,
        "rejected_swordsman",
        Some("iron_sword"),
        100,
        0,
    )
    .await;
    insert_combat_monster(&game_state, "far", pos(2.01), 0, 10).await;
    insert_combat_monster(&game_state, "other_floor", pos(1.0), -1, 10).await;
    insert_combat_monster(&game_state, "dead", pos(1.0), 0, 10).await;
    game_state
        .monsters
        .write()
        .await
        .get_mut("dead")
        .unwrap()
        .state = MonsterState::Dead;

    for id in ["unknown", "far", "other_floor", "dead"] {
        game_state
            .broadcast_player_attack(&player_id, id.to_string())
            .await;
    }
    game_state
        .players
        .write()
        .await
        .get_mut(&player_id)
        .unwrap()
        .health = 0;
    insert_combat_monster(&game_state, "alive", pos(1.0), 0, 10).await;
    game_state
        .broadcast_player_attack(&player_id, "alive".to_string())
        .await;

    assert_eq!(
        game_state.player_skills.read().await[&player_id].get(SkillId::OneHandedSword),
        SkillProgress::default()
    );
    assert!(drain(&mut rx)
        .iter()
        .all(|message| !matches!(message, ServerMessage::SkillXpGained { .. })));
}

#[tokio::test]
async fn player_cooldown_is_atomic_across_targets_and_cleared_on_remove() {
    let game_state = make_test_game_state("player_attack_cooldown");
    let player_id = pid("cooldown_swordsman");
    let mut rx = setup_weapon_attacker(
        &game_state,
        "cooldown_swordsman",
        Some("iron_sword"),
        -100,
        0,
    )
    .await;
    insert_combat_monster(&game_state, "first", pos(1.0), 0, 10).await;
    insert_combat_monster(&game_state, "second", pos(1.0), 0, 10).await;

    game_state
        .broadcast_player_attack(&player_id, "first".to_string())
        .await;
    game_state
        .broadcast_player_attack(&player_id, "second".to_string())
        .await;

    assert_eq!(game_state.monsters.read().await["second"].health, 10);
    assert_eq!(
        game_state.player_skills.read().await[&player_id]
            .get(SkillId::OneHandedSword)
            .xp,
        5
    );
    let messages = drain(&mut rx);
    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(message, ServerMessage::PlayerAttacked { .. }))
            .count(),
        1
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        ServerMessage::PlayerAttackRejected {
            monster_id,
            reason: AttackRejectReason::Cooldown,
        } if monster_id == "second"
    )));
    assert!(game_state
        .last_player_attacks
        .read()
        .await
        .contains_key(&player_id));
    game_state.remove_player(&player_id).await;
    assert!(!game_state
        .last_player_attacks
        .read()
        .await
        .contains_key(&player_id));
}

#[tokio::test]
async fn phase_two_metrics_aggregate_combat_without_player_identity() {
    let game_state = make_test_game_state("phase_two_metrics");
    let player_id = pid("measured_swordsman");
    let _rx = setup_weapon_attacker(
        &game_state,
        "measured_swordsman",
        Some("iron_sword"),
        100,
        15,
    )
    .await;
    {
        let mut players = game_state.players.write().await;
        let player = players.get_mut(&player_id).unwrap();
        player.level = 10;
        player.client_kind = ClientKind::Cli;
    }
    insert_combat_monster(&game_state, "measured_first", pos(1.0), 0, 1_000).await;
    insert_combat_monster(&game_state, "measured_second", pos(1.0), 0, 1_000).await;

    game_state
        .broadcast_player_attack(&player_id, "measured_first".to_string())
        .await;
    game_state
        .broadcast_player_attack(&player_id, "measured_second".to_string())
        .await;
    game_state
        .last_player_attacks
        .write()
        .await
        .insert(player_id, GameState::now_ms().saturating_sub(1_600));
    game_state
        .broadcast_player_attack(&player_id, "measured_second".to_string())
        .await;

    let metrics = game_state.skill_balance_snapshot();
    assert_eq!(metrics.attack_requests, 3);
    assert_eq!(metrics.resolved_attacks, 2);
    assert_eq!(metrics.rejections.cooldown, 1);
    assert_eq!(metrics.weapon.attacks, 2);
    assert_eq!(metrics.weapon.hits, 2);
    assert_eq!(metrics.weapon.xp, 20);
    assert_eq!(metrics.weapon_by_skill["one_handed_sword"].attacks, 2);
    assert_eq!(metrics.weapon_by_skill_band[2].attacks, 2);
    assert_eq!(metrics.weapon_by_difficulty[0].attacks, 2);
    assert_eq!(metrics.weapon_by_client[1].attacks, 2);
    assert_eq!(metrics.weapon_by_level_pair[&(10, 2)].attacks, 2);
    assert_eq!(metrics.weapon_by_monster["kobold"].attacks, 2);
    assert_eq!(metrics.cadence_samples, 1);
    assert!(metrics.cadence_total_ms >= 1_600);
    assert_eq!(metrics.weapon_xp_messages, 2);
    assert_eq!(metrics.weapon_rows_created, 0);
    assert!(!game_state
        .skill_balance_report()
        .contains("measured_swordsman"));
}

#[tokio::test]
async fn duplicate_kill_requests_award_the_kill_only_once() {
    let game_state = make_test_game_state("duplicate_sword_kill");
    let player_id = pid("duplicate_swordsman");
    let mut rx = setup_weapon_attacker(
        &game_state,
        "duplicate_swordsman",
        Some("iron_sword"),
        100,
        0,
    )
    .await;
    insert_combat_monster(&game_state, "single_kill", pos(1.0), 0, 1).await;

    game_state
        .broadcast_player_attack(&player_id, "single_kill".to_string())
        .await;
    game_state
        .broadcast_player_attack(&player_id, "single_kill".to_string())
        .await;

    assert_eq!(
        game_state.player_skills.read().await[&player_id]
            .get(SkillId::OneHandedSword)
            .xp,
        20
    );
    assert_eq!(
        drain(&mut rx)
            .iter()
            .filter(|message| matches!(
                message,
                ServerMessage::SkillXpGained {
                    skill: SkillId::OneHandedSword,
                    ..
                }
            ))
            .count(),
        1
    );
}

#[tokio::test]
async fn unmapped_and_capped_weapon_use_stays_quiet() {
    let unmapped_state = make_test_game_state("unmapped_weapon_xp");
    let unmapped_id = pid("torch_user");
    let mut unmapped_rx =
        setup_weapon_attacker(&unmapped_state, "torch_user", Some("torch"), 100, 0).await;
    insert_combat_monster(&unmapped_state, "torch_target", pos(1.0), 0, 1_000).await;
    unmapped_state
        .broadcast_player_attack(&unmapped_id, "torch_target".to_string())
        .await;
    assert_eq!(
        unmapped_state.player_skills.read().await[&unmapped_id].get(SkillId::OneHandedSword),
        SkillProgress::default()
    );
    assert!(drain(&mut unmapped_rx)
        .iter()
        .all(|message| !matches!(message, ServerMessage::SkillXpGained { .. })));

    let capped_state = make_test_game_state("capped_sword_xp");
    let capped_id = pid("master_swordsman");
    let mut capped_rx = setup_weapon_attacker(
        &capped_state,
        "master_swordsman",
        Some("iron_sword"),
        -100,
        SKILL_LEVEL_CAP,
    )
    .await;
    insert_combat_monster(&capped_state, "cap_target", pos(1.0), 0, 10).await;
    capped_state
        .broadcast_player_attack(&capped_id, "cap_target".to_string())
        .await;
    assert!(drain(&mut capped_rx)
        .iter()
        .all(|message| !matches!(message, ServerMessage::SkillXpGained { .. })));
    assert!(capped_state.collect_dirty_skill_states().await.1.is_empty());
}
