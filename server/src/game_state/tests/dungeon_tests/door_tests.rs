use super::*;

/// First registry dungeon plus its shallowest floor holding a plain
/// (unlocked) interior door.
fn first_dungeon_door(
    game_state: &GameState,
) -> (
    crate::dungeon_defs::DungeonEntranceDef,
    u8,
    onlinerpg_shared::dungeon::InteriorDoorSpec,
) {
    use onlinerpg_shared::dungeon::{generate_dungeon_for, interior_doors};
    let entrance = first_dungeon(game_state);
    let (depth, door) = generate_dungeon_for(&entrance.id)
        .iter()
        .find_map(|l| {
            interior_doors(l)
                .into_iter()
                .find(|d| !d.locked)
                .map(|d| (l.depth, d))
        })
        .expect("a floor with an interior door");
    (entrance, depth, door)
}

/// The Old Crypt's locked stair-room exit (its deepest floor) and the key.
fn crypt_locked_door(
    game_state: &GameState,
) -> (
    crate::dungeon_defs::DungeonEntranceDef,
    u8,
    onlinerpg_shared::dungeon::InteriorDoorSpec,
    String,
) {
    use onlinerpg_shared::dungeon::{generate_dungeon_for, interior_doors};
    let entrance = game_state.dungeon_defs.get("old_crypt").expect("old_crypt");
    let layouts = generate_dungeon_for(&entrance.id);
    let layout = layouts.last().unwrap();
    let door = interior_doors(layout)
        .into_iter()
        .find(|d| d.locked)
        .expect("the deepest crypt floor is locked");
    let key = entrance.key_item_id(layout.depth);
    (entrance.clone(), layout.depth, door, key)
}

/// Cell centers either side of `door`, at the low end of the opening or (with
/// `far_end`) the high one — the spots a player works it from.
fn door_side_positions(
    entrance: &crate::dungeon_defs::DungeonEntranceDef,
    depth: u8,
    door: &onlinerpg_shared::dungeon::InteriorDoorSpec,
    far_end: bool,
) -> (Position, Position) {
    let [ax, az, bx, bz] = door.seg();
    let (lat, line) = if door.spans_x() {
        (if far_end { bx - 1 } else { ax }, az)
    } else {
        (if far_end { bz - 1 } else { az }, ax)
    };
    let (outside, inside) = if door.spans_x() {
        ((lat, line - 1), (lat, line))
    } else {
        ((line - 1, lat), (line, lat))
    };
    let ep = entrance.position();
    (
        cell_center(&ep, depth, outside),
        cell_center(&ep, depth, inside),
    )
}

/// A player standing at `at` on the floor holding a door at `depth`.
async fn add_delver(game_state: &GameState, name: &str, at: Position, depth: u8) -> PlayerId {
    let mut player = make_player(name, at.x, at.z);
    player.position.y = at.y;
    player.floor_level = -(depth as i8);
    game_state.add_player(player).await;
    pid(name)
}

#[tokio::test]
async fn cross_floor_dungeon_door_toggle_is_rejected() {
    let game_state = make_test_game_state("dungeon_door_cross_floor");
    let (entrance, depth, door) = first_dungeon_door(&game_state);
    game_state
        .init_passability(&crate::terrain::io::TerrainIO::new(
            "nonexistent_terrain_dir".into(),
        ))
        .await
        .expect("init passability");
    let griefer_id = pid("surface_griefer");
    game_state
        .add_player(make_player("surface_griefer", entrance.x, entrance.z))
        .await;
    let mut observer = make_player("floor_observer", entrance.x, entrance.z);
    observer.floor_level = -(depth as i8);
    game_state.add_player(observer).await;
    let mut observer_rx = game_state
        .register_direct_channel(&pid("floor_observer"))
        .await;

    assert!(
        !game_state.dungeons.read().await.contains_key(&entrance.id),
        "the test requires an uninitialized dungeon runtime"
    );
    let before = game_state.dungeon_open_doors(&entrance.id).await;
    let result = game_state
        .toggle_dungeon_door(&griefer_id, &entrance.id, depth, door.door_id)
        .await;
    if let Some(is_open) = result {
        game_state
            .publish_dungeon_door_toggle(
                &griefer_id,
                entrance.id.clone(),
                depth,
                door.door_id,
                is_open,
            )
            .await;
    }

    assert_eq!(result, None, "a player on another floor must be rejected");
    assert_eq!(
        game_state.dungeon_open_doors(&entrance.id).await,
        before,
        "a rejected toggle must not change door state"
    );
    assert!(
        !game_state.dungeons.read().await.contains_key(&entrance.id),
        "a rejected toggle must not create dungeon runtime state"
    );
    assert!(matches!(
        observer_rx.try_recv(),
        Err(MpscTryRecvError::Empty)
    ));

    let (from, to) = door_side_positions(&entrance, depth, &door, false);
    let delver_id = add_delver(&game_state, "delver", from, depth).await;
    game_state
        .update_player_position(
            &delver_id,
            MoveCommand {
                floor_level: -(depth as i8),
                ..move_cmd(to, false)
            },
            false,
        )
        .await;
    game_state.tick_player_movement(60.0).await;
    assert_eq!(
        player_xz(&game_state, &delver_id).await,
        (from.x, from.z),
        "a rejected toggle must leave the door impassable"
    );
}

#[tokio::test]
async fn same_floor_dungeon_door_toggle_still_updates_state() {
    let game_state = make_test_game_state("dungeon_door_same_floor");
    let (entrance, depth, door) = first_dungeon_door(&game_state);
    let (outside, _) = door_side_positions(&entrance, depth, &door, false);
    let player_id = add_delver(&game_state, "delver", outside, depth).await;

    assert_eq!(
        game_state
            .toggle_dungeon_door(&player_id, &entrance.id, depth, door.door_id)
            .await,
        Some(true)
    );
    assert!(game_state
        .dungeon_open_doors(&entrance.id)
        .await
        .contains(&(depth, door.door_id)));
    assert_eq!(
        game_state
            .toggle_dungeon_door(&player_id, &entrance.id, depth, door.door_id)
            .await,
        Some(false)
    );
}

/// Reach is also what pins a toggle to the dungeon the player stands in — a
/// door is only in reach from its own grid. One registry dungeon, so the
/// cross-dungeon case has nothing to exercise directly.
#[tokio::test]
async fn out_of_reach_dungeon_door_toggle_is_rejected() {
    let game_state = make_test_game_state("dungeon_door_out_of_reach");
    let (entrance, depth, door) = first_dungeon_door(&game_state);
    let (outside, _) = door_side_positions(&entrance, depth, &door, false);
    // Half the 80-cell floor: past the reach of even the widest opening, but
    // still a place a player on this floor can legitimately stand.
    let far = Position {
        x: outside.x + 40.0,
        ..outside
    };
    let player_id = add_delver(&game_state, "sniper", far, depth).await;

    assert_eq!(
        game_state
            .toggle_dungeon_door(&player_id, &entrance.id, depth, door.door_id)
            .await,
        None,
        "the door must be out of reach from across the floor"
    );
    assert!(game_state.dungeon_open_doors(&entrance.id).await.is_empty());
}

/// Reach is measured to the whole doorway, not its middle — openings run up
/// to 17 cells wide, so a midpoint would put a player standing at one jamb
/// half an opening away from their own door.
#[test]
fn door_line_distance_spans_the_whole_doorway() {
    use super::dungeon::door_line_dist_sq;
    use onlinerpg_shared::dungeon::dungeon_origin;

    let entrance = Position {
        x: 100.0,
        y: 0.0,
        z: 200.0,
    };
    let (ox, oz) = dungeon_origin(entrance.x, entrance.z);
    // A 17-cell opening spanning X at grid line z = 40.
    let seg = [20, 40, 37, 40];
    let at = |x: f32, z: f32| Position { x, y: 0.0, z };

    // Anywhere along the opening: only the perpendicular offset counts.
    for cell_x in [20.0, 28.5, 37.0] {
        assert_eq!(
            door_line_dist_sq(&entrance, seg, &at(ox + cell_x, oz + 41.0)),
            1.0
        );
    }
    // Past an end, the offset to that end is added back.
    assert_eq!(
        door_line_dist_sq(&entrance, seg, &at(ox + 40.0, oz + 40.0)),
        9.0
    );

    // X wraps with the world, so a dungeon straddling the seam still measures
    // across it rather than the long way round.
    let seam = Position {
        x: onlinerpg_shared::WORLD_MAX_X - 1.0,
        ..entrance
    };
    let (sx, _) = dungeon_origin(seam.x, seam.z);
    let wrapped = sx + 20.0 - onlinerpg_shared::WORLD_WIDTH_X;
    assert_eq!(door_line_dist_sq(&seam, seg, &at(wrapped, oz + 41.0)), 1.0);
}

/// Standing at the far jamb of a wide door is still standing at it.
#[tokio::test]
async fn wide_dungeon_door_toggles_from_its_far_end() {
    use onlinerpg_shared::dungeon::{generate_dungeon_for, interior_doors};
    let game_state = make_test_game_state("dungeon_door_wide");
    let entrance = first_dungeon(&game_state);
    let (depth, door) = generate_dungeon_for(&entrance.id)
        .iter()
        .flat_map(|l| interior_doors(l).into_iter().map(|d| (l.depth, d)))
        .filter(|(_, d)| !d.locked)
        .max_by_key(|(_, d)| d.len)
        .expect("a widest interior door");

    let (at, _) = door_side_positions(&entrance, depth, &door, true);
    let player_id = add_delver(&game_state, "delver", at, depth).await;

    assert_eq!(
        game_state
            .toggle_dungeon_door(&player_id, &entrance.id, depth, door.door_id)
            .await,
        Some(true),
        "a {}-cell opening must be workable from its far end",
        door.len
    );
}

#[tokio::test]
async fn unknown_dungeon_door_id_is_rejected() {
    let game_state = make_test_game_state("dungeon_door_unknown_id");
    let (entrance, depth, door) = first_dungeon_door(&game_state);
    let (outside, _) = door_side_positions(&entrance, depth, &door, false);
    let player_id = add_delver(&game_state, "delver", outside, depth).await;

    for door_id in [door.door_id ^ 0xABCD, u32::MAX] {
        assert_eq!(
            game_state
                .toggle_dungeon_door(&player_id, &entrance.id, depth, door_id)
                .await,
            None
        );
    }
    assert!(game_state.dungeon_open_doors(&entrance.id).await.is_empty());
}

#[tokio::test]
async fn surface_entrance_door_toggle_gates_on_the_entrance() {
    use onlinerpg_shared::dungeon::generate_dungeon_for;
    let game_state = make_test_game_state("dungeon_door_entrance");
    let entrance = first_dungeon(&game_state);
    // Standing on the shaft's surface landing, where the entrance door is. The
    // gate circles the marker rather than the door, so it only holds while the
    // generator keeps the two together.
    let at_door = cell_center(
        &entrance.position(),
        0,
        generate_dungeon_for(&entrance.id)[0].up_shaft.entry_cell(),
    );
    let offset = at_door.dist_xz_sq(&entrance.position()).sqrt();
    assert!(
        offset < EVENT_DELIVERY_RADIUS * 0.5,
        "the entrance door sits {offset:.1}m from the marker its gate centers on"
    );

    let near_id = add_delver(&game_state, "visitor", at_door, 0).await;
    let away = Position {
        x: entrance.x + EVENT_DELIVERY_RADIUS + 10.0,
        ..entrance.position()
    };
    let away_id = add_delver(&game_state, "passerby", away, 0).await;

    assert_eq!(
        game_state
            .toggle_dungeon_door(&away_id, &entrance.id, 0, ENTRANCE_DOOR_ID)
            .await,
        None,
        "the entrance door must not be reachable from across the world"
    );
    assert_eq!(
        game_state
            .toggle_dungeon_door(&near_id, &entrance.id, 0, 7)
            .await,
        None,
        "depth 0 has exactly one door id"
    );
    assert!(game_state.dungeon_open_doors(&entrance.id).await.is_empty());

    assert_eq!(
        game_state
            .toggle_dungeon_door(&near_id, &entrance.id, 0, ENTRANCE_DOOR_ID)
            .await,
        Some(true)
    );
    assert_eq!(
        game_state.dungeon_open_doors(&entrance.id).await,
        vec![(0, ENTRANCE_DOOR_ID)]
    );
}

#[tokio::test]
async fn dungeon_door_toggle_delivery_gates_radius_and_floor() {
    let game_state = make_test_game_state("dungeon_door_delivery");
    let entrance = first_dungeon(&game_state);
    let ep = entrance.position();
    let toggler = pid("door_toggler");
    let near_surface = pid("near_surface");
    let far_surface = pid("far_surface");
    let near_underground = pid("near_underground");
    game_state
        .add_player(make_player("door_toggler", ep.x, ep.z))
        .await;
    game_state
        .add_player(make_player("near_surface", ep.x + 30.0, ep.z))
        .await;
    game_state
        .add_player(make_player("far_surface", ep.x + 100.0, ep.z))
        .await;
    let mut delver = make_player("near_underground", ep.x + 10.0, ep.z);
    delver.floor_level = -1;
    game_state.add_player(delver).await;

    let mut toggler_rx = game_state.register_direct_channel(&toggler).await;
    let mut near_rx = game_state.register_direct_channel(&near_surface).await;
    let mut far_rx = game_state.register_direct_channel(&far_surface).await;
    let mut under_rx = game_state.register_direct_channel(&near_underground).await;
    let mut broadcast_rx = game_state.subscribe();

    game_state
        .publish_dungeon_door_toggle(&toggler, entrance.id.clone(), 0, 0, true)
        .await;

    // Surface door: never global; surface players within EVENT_DELIVERY_RADIUS
    // only. Underground players wait for the floor-entry snapshot; players
    // farther out re-pull the snapshot when they cross into range.
    assert!(matches!(broadcast_rx.try_recv(), Err(TryRecvError::Empty)));
    for rx in [&mut toggler_rx, &mut near_rx] {
        assert!(matches!(
            rx.try_recv(),
            Ok(ServerMessage::DungeonDoorToggled {
                entrance_id,
                depth: 0,
                door_id: 0,
                is_open: true,
            }) if entrance_id == entrance.id
        ));
    }
    assert!(matches!(far_rx.try_recv(), Err(MpscTryRecvError::Empty)));
    assert!(matches!(under_rx.try_recv(), Err(MpscTryRecvError::Empty)));

    game_state
        .publish_dungeon_door_toggle(&near_underground, entrance.id.clone(), 1, 123, false)
        .await;

    // Interior door: gated to the door's floor, so nearby surface players
    // hear nothing.
    assert!(matches!(broadcast_rx.try_recv(), Err(TryRecvError::Empty)));
    assert!(matches!(
        under_rx.try_recv(),
        Ok(ServerMessage::DungeonDoorToggled {
            entrance_id,
            depth: 1,
            door_id: 123,
            is_open: false,
        }) if entrance_id == entrance.id
    ));
    assert!(matches!(
        toggler_rx.try_recv(),
        Err(MpscTryRecvError::Empty)
    ));
    assert!(matches!(near_rx.try_recv(), Err(MpscTryRecvError::Empty)));

    // Delivery never depends on where the toggler's own floor tracking sits:
    // they hear their toggle regardless, and the surface circle is unaffected.
    // (`toggle_dungeon_door` only lets a floor-0 player reach depth 0, so this
    // is a property of the delivery, not a reachable production case.)
    game_state
        .publish_dungeon_door_toggle(&near_underground, entrance.id.clone(), 0, 0, false)
        .await;
    for rx in [&mut under_rx, &mut toggler_rx, &mut near_rx] {
        assert!(matches!(
            rx.try_recv(),
            Ok(ServerMessage::DungeonDoorToggled {
                depth: 0,
                is_open: false,
                ..
            })
        ));
    }
    assert!(matches!(far_rx.try_recv(), Err(MpscTryRecvError::Empty)));
}

/// A shut interior dungeon door must block server-simulated movement across
/// its corridor mouth from boot (doors default shut); toggling it open lets
/// the move through, toggling again reseals it.
#[tokio::test]
async fn dungeon_door_blocks_movement_until_opened() {
    let game_state = make_test_game_state("dungeon_door_block");
    let (entrance, depth, door) = first_dungeon_door(&game_state);
    game_state
        .init_passability(&crate::terrain::io::TerrainIO::new(
            "nonexistent_terrain_dir".into(),
        ))
        .await
        .expect("init passability");

    let (from, to) = door_side_positions(&entrance, depth, &door, false);
    let player_id = add_delver(&game_state, "delver", from, depth).await;
    let go = |p: Position| MoveCommand {
        floor_level: -(depth as i8),
        ..move_cmd(p, false)
    };

    // Shut (boot default): the crossing is sealed.
    game_state
        .update_player_position(&player_id, go(to), false)
        .await;
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_xz(&game_state, &player_id).await, (from.x, from.z));

    // Open: same move goes through.
    assert_eq!(
        game_state
            .toggle_dungeon_door(&player_id, &entrance.id, depth, door.door_id)
            .await,
        Some(true)
    );
    game_state
        .update_player_position(&player_id, go(to), false)
        .await;
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_xz(&game_state, &player_id).await, (to.x, to.z));

    // Shut again: the way back is sealed.
    assert_eq!(
        game_state
            .toggle_dungeon_door(&player_id, &entrance.id, depth, door.door_id)
            .await,
        Some(false)
    );
    game_state
        .update_player_position(&player_id, go(from), false)
        .await;
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_xz(&game_state, &player_id).await, (to.x, to.z));
}

/// Arriving on a dungeon floor must push the full open-door snapshot: the
/// live DungeonDoorToggled broadcast is floor- and radius-gated, so a player
/// who registered the dungeon before someone else toggled a door (or who was
/// on another floor at the time) would otherwise render it stale.
#[tokio::test]
async fn floor_entry_pushes_open_door_snapshot() {
    let game_state = make_test_game_state("dungeon_door_entry_snapshot");
    let (entrance, depth, door) = first_dungeon_door(&game_state);
    let (at_door, _) = door_side_positions(&entrance, depth, &door, false);
    let opener_id = add_delver(&game_state, "opener", at_door, depth).await;

    // Player A opens a door before B has ever seen the dungeon.
    assert_eq!(
        game_state
            .toggle_dungeon_door(&opener_id, &entrance.id, depth, door.door_id)
            .await,
        Some(true)
    );

    let player_id = pid("latecomer");
    game_state
        .add_player(make_player("latecomer", entrance.x, entrance.z))
        .await;
    let mut direct_rx = game_state.register_direct_channel(&player_id).await;

    let inside = Position {
        x: entrance.x,
        y: entrance.y - 4.0,
        z: entrance.z,
    };
    game_state
        .handle_player_floor_change(&player_id, 0, -(depth as i8), &inside, &inside)
        .await;

    let mut doors_state = None;
    for msg in drain(&mut direct_rx) {
        if let ServerMessage::DungeonDoorsState { entrance_id, doors } = msg {
            assert_eq!(entrance_id, entrance.id);
            doors_state = Some(doors);
        }
    }
    let doors = doors_state.expect("floor entry should push DungeonDoorsState");
    assert!(
        doors.contains(&(depth, door.door_id)),
        "snapshot should list the door A opened, got {doors:?}"
    );
}

/// Blows obey the same seal movement does: neither side of a shut door can
/// reach the other, and opening it restores both directions. One monster per
/// half — a monster's swing consumes its cooldown whether or not it lands.
#[tokio::test]
async fn dungeon_door_blocks_attacks_until_opened() {
    let game_state = make_test_game_state("dungeon_door_block_attacks");
    let (entrance, depth, door) = first_dungeon_door(&game_state);
    game_state
        .init_passability(&crate::terrain::io::TerrainIO::new(
            "nonexistent_terrain_dir".into(),
        ))
        .await
        .expect("init passability");

    let (from, to) = door_side_positions(&entrance, depth, &door, false);
    let player_id = add_delver(&game_state, "delver", from, depth).await;
    let mut delver_rx = game_state.register_direct_channel(&player_id).await;
    {
        let mut monsters = game_state.monsters.write().await;
        for id in ["shut_door_monster", "open_door_monster"] {
            let mut monster = make_monster(id, to, -(depth as i8));
            monster.owner_id = Some(player_id);
            monsters.insert(id.to_string(), monster);
        }
    }

    game_state
        .broadcast_player_attack(&player_id, "shut_door_monster".to_string())
        .await;
    assert_eq!(
        game_state.monsters.read().await["shut_door_monster"].health,
        10,
        "a swing through a shut door must not damage the monster"
    );
    expect_attack_rejected(
        &mut delver_rx,
        "shut_door_monster",
        AttackRejectReason::OutOfRange,
    );

    game_state
        .broadcast_monster_attack(&player_id, "shut_door_monster", &player_id)
        .await;
    assert_eq!(
        game_state.players.read().await[&player_id].last_combat_at,
        0,
        "a monster behind a shut door must not reach the player"
    );

    assert_eq!(
        game_state
            .toggle_dungeon_door(&player_id, &entrance.id, depth, door.door_id)
            .await,
        Some(true)
    );

    game_state
        .broadcast_monster_attack(&player_id, "open_door_monster", &player_id)
        .await;
    assert_ne!(
        game_state.players.read().await[&player_id].last_combat_at,
        0,
        "an open doorway must let the monster strike back"
    );

    // Both attack paths stamp `last_combat_at`; clear it so the player's own
    // swing is what the next assertion reads.
    game_state
        .players
        .write()
        .await
        .get_mut(&player_id)
        .expect("the delver is on the floor")
        .last_combat_at = 0;
    game_state
        .broadcast_player_attack(&player_id, "open_door_monster".to_string())
        .await;
    assert_ne!(
        game_state.players.read().await[&player_id].last_combat_at,
        0,
        "an open doorway must let the swing through"
    );
}

#[tokio::test]
async fn furniture_removal_reopens_blocked_cells() {
    let game_state = make_test_game_state("movement_furniture_removed");
    let player_id = pid("returner");
    game_state
        .add_player(make_player("returner", 0.5, 4.5))
        .await;
    game_state.sync_region_furniture(0, 0, &[table_placement(0.5, 5.5)]);
    // The map editor clearing the region must unblock movement again.
    game_state.sync_region_furniture(0, 0, &[]);

    game_state
        .update_player_position(
            &player_id,
            move_cmd(
                Position {
                    x: 0.5,
                    y: 0.0,
                    z: 6.5,
                },
                false,
            ),
            false,
        )
        .await;
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_xz(&game_state, &player_id).await, (0.5, 6.5));
}

/// A locked door wants the floor's key from either side, and does not spend
/// it; without one the toggle is refused with a reason the player sees.
#[tokio::test]
async fn locked_door_needs_the_floor_key_and_keeps_it() {
    let game_state = make_test_game_state("locked_door_key");
    let (entrance, depth, door, key) = crypt_locked_door(&game_state);
    let (outside, inside) = door_side_positions(&entrance, depth, &door, false);
    let keyless = add_delver(&game_state, "keyless", inside, depth).await;
    give_bag(&game_state, &keyless, None).await;
    let mut keyless_rx = game_state.register_direct_channel(&keyless).await;

    assert_eq!(
        game_state
            .toggle_dungeon_door(&keyless, &entrance.id, depth, door.door_id)
            .await,
        None
    );
    match keyless_rx.try_recv() {
        Ok(ServerMessage::InteractionRejected { reason }) => {
            assert!(reason.contains("locked"), "got {reason}")
        }
        other => panic!("expected a rejection, got {other:?}"),
    }
    assert!(game_state.dungeon_open_doors(&entrance.id).await.is_empty());

    let holder = add_delver(&game_state, "holder", outside, depth).await;
    give_bag(&game_state, &holder, Some(&key)).await;
    assert_eq!(
        game_state
            .toggle_dungeon_door(&holder, &entrance.id, depth, door.door_id)
            .await,
        Some(true)
    );
    assert!(
        game_state.inventories.read().await[&holder]
            .bag
            .iter()
            .any(|i| i.item_def_id == key),
        "the door does not eat the key"
    );
    // Shutting it early wants the key too.
    assert_eq!(
        game_state
            .toggle_dungeon_door(&keyless, &entrance.id, depth, door.door_id)
            .await,
        None
    );
    assert_eq!(
        game_state
            .toggle_dungeon_door(&holder, &entrance.id, depth, door.door_id)
            .await,
        Some(false)
    );
}

/// A door opened with a key shuts itself after `LOCKED_DOOR_OPEN_DURATION`,
/// telling the floor; a reopen restarts the clock rather than being cut
/// short by the first opening's timer.
#[tokio::test(start_paused = true)]
async fn locked_door_shuts_itself_again() {
    use crate::game_state::dungeon::LOCKED_DOOR_OPEN_DURATION;
    let game_state = make_test_game_state("locked_door_timer");
    game_state
        .init_passability(&crate::terrain::io::TerrainIO::new(
            "nonexistent_terrain_dir".into(),
        ))
        .await
        .expect("init passability");
    let (entrance, depth, door, key) = crypt_locked_door(&game_state);
    let (outside, _) = door_side_positions(&entrance, depth, &door, false);
    let holder = add_delver(&game_state, "holder", outside, depth).await;
    give_bag(&game_state, &holder, Some(&key)).await;
    game_state
        .handle_player_floor_change(&holder, 0, -(depth as i8), &outside, &outside)
        .await;
    let mut rx = game_state.register_direct_channel(&holder).await;

    let is_open = |gs: &GameState| {
        let entrance_id = entrance.id.clone();
        let gs = gs.clone();
        async move {
            gs.dungeon_open_doors(&entrance_id)
                .await
                .contains(&(depth, door.door_id))
        }
    };

    assert_eq!(
        game_state
            .toggle_dungeon_door(&holder, &entrance.id, depth, door.door_id)
            .await,
        Some(true)
    );
    // Let the close task arm its timer before the clock moves.
    tokio::task::yield_now().await;
    tokio::time::advance(LOCKED_DOOR_OPEN_DURATION / 2).await;
    assert!(is_open(&game_state).await, "half way: still open");

    // Shut and reopen: the first timer must not close this second opening.
    game_state
        .toggle_dungeon_door(&holder, &entrance.id, depth, door.door_id)
        .await;
    game_state
        .toggle_dungeon_door(&holder, &entrance.id, depth, door.door_id)
        .await;
    tokio::task::yield_now().await;
    tokio::time::advance(LOCKED_DOOR_OPEN_DURATION / 2 + std::time::Duration::from_millis(1)).await;
    assert!(is_open(&game_state).await, "the reopen has its own clock");

    tokio::time::advance(LOCKED_DOOR_OPEN_DURATION / 2).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert!(!is_open(&game_state).await, "it locks itself again");
    assert!(
        drain(&mut rx).iter().any(|m| matches!(
            m,
            ServerMessage::DungeonDoorToggled { is_open: false, door_id: id, .. } if *id == door.door_id
        )),
        "the floor hears it shut"
    );
}

/// A monster on the far side of a shut door cannot see the player, so it
/// neither acquires nor keeps them as a target; the open doorway lets it.
#[tokio::test]
async fn monster_brains_do_not_see_through_a_shut_door() {
    let game_state = make_test_game_state("dungeon_door_blocks_sight");
    game_state.enable_server_monster_ai();
    let (entrance, depth, door) = first_dungeon_door(&game_state);
    game_state
        .init_passability(&crate::terrain::io::TerrainIO::new(
            "nonexistent_terrain_dir".into(),
        ))
        .await
        .expect("init passability");

    let (inside, outside) = door_side_positions(&entrance, depth, &door, false);
    let player_id = add_delver(&game_state, "delver", inside, depth).await;
    let monster = game_state
        .spawn_monster(
            "goblin".to_string(),
            outside,
            0.0,
            Some(player_id),
            -(depth as i8),
            MonsterLifecycle::Ambient,
            None,
            true,
        )
        .await
        .expect("goblin spawns")
        .id;

    for _ in 0..10 {
        game_state.tick_monster_ai_by(200.0).await;
    }
    assert_eq!(
        game_state.brain_target(&monster).await,
        None,
        "a shut door hides the player"
    );

    assert_eq!(
        game_state
            .toggle_dungeon_door(&player_id, &entrance.id, depth, door.door_id)
            .await,
        Some(true)
    );
    for _ in 0..10 {
        game_state.tick_monster_ai_by(200.0).await;
    }
    assert_eq!(
        game_state.brain_target(&monster).await,
        Some(player_id),
        "the open doorway shows the player"
    );
}
