use super::*;

#[tokio::test]
async fn simulated_movement_is_blocked_by_solid_furniture() {
    let game_state = make_test_game_state("movement_furniture_block");
    let player_id = pid("wallwalker");
    game_state
        .add_player(make_player("wallwalker", 0.5, 4.5))
        .await;
    // A table centred on cell (0, 5) seals it (EDGE_ALL).
    game_state.sync_region_furniture(0, 0, &[table_placement(0.5, 5.5)]);

    // Straight through the sealed cell: the sim must stop at the wall.
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
            false,
        )
        .await;
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_xz(&game_state, &player_id).await, (0.5, 4.5));

    // A move that never touches the sealed cell still goes through.
    game_state
        .update_player_position(
            &player_id,
            move_cmd(
                Position {
                    x: 3.5,
                    y: 0.0,
                    z: 4.5,
                },
                false,
            ),
            false,
            false,
        )
        .await;
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_xz(&game_state, &player_id).await, (3.5, 4.5));
}

/// A client picks its own Y and the server only ever infers the storey back
/// from it, so a Y over the table tops used to clear every piece of furniture
/// on the floor. The forged height has to be banked by a legal step first —
/// collision reads the Y the mover is standing at, not the one it reports.
#[tokio::test]
async fn a_forged_height_cannot_clear_solid_furniture() {
    let game_state = make_test_game_state("movement_forged_height");
    let player_id = pid("flyer");
    game_state.add_player(make_player("flyer", 0.5, 4.5)).await;
    game_state.sync_region_furniture(0, 0, &[table_placement(0.5, 5.5)]);

    // Away from the table, so the step itself is legal. Nothing validates Y,
    // so the server stores the 1.5m the client claims (tables are 1m tall).
    let perch = Position {
        x: 0.5,
        y: 1.5,
        z: 3.5,
    };
    game_state
        .update_player_position(&player_id, move_cmd(perch, false), false, false)
        .await;
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_xz(&game_state, &player_id).await, (0.5, 3.5));

    // Straight through the sealed cell from the perch.
    game_state
        .update_player_position(
            &player_id,
            move_cmd(
                Position {
                    x: 0.5,
                    y: 1.5,
                    z: 6.5,
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
        (0.5, 3.5),
        "a forged height must not walk the player over the table"
    );
}

#[tokio::test]
async fn queued_waypoints_route_around_furniture() {
    let game_state = make_test_game_state("movement_queue_around");
    let player_id = pid("detourist");
    game_state
        .add_player(make_player("detourist", 0.5, 4.5))
        .await;
    game_state.sync_region_furniture(0, 0, &[table_placement(0.5, 5.5)]);

    // The client's path around the sealed cell, leg by leg. A single replace
    // to the final target would beeline through the table and block
    // (`simulated_movement_is_blocked_by_solid_furniture`).
    let legs = [(1.5, 4.5, false), (1.5, 6.5, true), (0.5, 6.5, true)];
    for (x, z, append) in legs {
        game_state
            .update_player_position(
                &player_id,
                move_cmd(Position { x, y: 0.0, z }, append),
                false,
                false,
            )
            .await;
    }

    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_xz(&game_state, &player_id).await, (0.5, 6.5));
}

#[tokio::test]
async fn blocked_leg_drops_remaining_queue() {
    let game_state = make_test_game_state("movement_queue_blocked_drop");
    let player_id = pid("stopper");
    game_state
        .add_player(make_player("stopper", 0.5, 4.5))
        .await;
    game_state.sync_region_furniture(0, 0, &[table_placement(0.5, 5.5)]);

    // Second leg dives into the sealed cell; the third would be clear again,
    // but a blocked queue must not keep walking later legs.
    let legs = [(1.5, 4.5, false), (0.5, 5.5, true), (0.5, 6.5, true)];
    for (x, z, append) in legs {
        game_state
            .update_player_position(
                &player_id,
                move_cmd(Position { x, y: 0.0, z }, append),
                false,
                false,
            )
            .await;
    }

    // The diagonal into the table grazes it, so the leg slides along X the way
    // the client does instead of stalling — but it stays unfinished.
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_xz(&game_state, &player_id).await, (0.5, 4.5));
    // Resuming it walks straight at the sealed cell, which neither axis can
    // save: the queue drops and the clear third leg is never walked.
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_xz(&game_state, &player_id).await, (0.5, 4.5));
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_xz(&game_state, &player_id).await, (0.5, 4.5));
}

#[tokio::test]
async fn grazing_corner_slides_instead_of_stalling() {
    let game_state = make_test_game_state("movement_corner_slide");
    let player_id = pid("slider");
    game_state.add_player(make_player("slider", 1.5, 4.5)).await;
    game_state.sync_region_furniture(0, 0, &[table_placement(0.5, 5.5)]);

    // A diagonal clipping the sealed cell's corner. The client slides around
    // it, so a stalling server would strand its shadow a cell behind and refuse
    // every later leg the client sent from the far side.
    game_state
        .update_player_position(
            &player_id,
            move_cmd(
                Position {
                    x: 0.5,
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
    assert_eq!(player_xz(&game_state, &player_id).await, (0.5, 4.5));
}

#[tokio::test]
async fn repeated_beeline_past_a_wall_does_not_pin_the_shadow() {
    let game_state = make_test_game_state("movement_no_pin");
    let player_id = pid("chaser");
    game_state.add_player(make_player("chaser", 2.5, 4.5)).await;
    // A wall of sealed cells along z=5, open past x=3.
    let wall = [
        table_placement(0.5, 5.5),
        table_placement(1.5, 5.5),
        table_placement(2.5, 5.5),
    ];
    game_state.sync_region_furniture(0, 0, &wall);

    // Combat chase sends the monster's raw position, not a pathfound leg, so
    // every packet is a fresh beeline through the wall. A shadow that stalls on
    // those never reaches the far side and refuses each later packet forever —
    // the production symptom this guards.
    for _ in 0..4 {
        game_state
            .update_player_position(
                &player_id,
                move_cmd(
                    Position {
                        x: 3.5,
                        y: 0.0,
                        z: 6.5,
                    },
                    false,
                ),
                false,
                false,
            )
            .await;
        game_state.tick_player_movement(1.0).await;
    }

    assert_eq!(player_xz(&game_state, &player_id).await, (3.5, 6.5));
}

#[tokio::test]
async fn a_refused_step_snaps_the_client_back_to_the_server() {
    let game_state = make_test_game_state("movement_correction");
    let player_id = pid("desynced");
    game_state
        .add_player(make_player("desynced", 0.5, 4.5))
        .await;
    let mut rx = game_state.register_direct_channel(&player_id).await;
    game_state.sync_region_furniture(0, 0, &[table_placement(0.5, 5.5)]);

    // Straight into the sealed cell: neither axis can save it, so the server
    // stops and must say where it actually has the player.
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
            false,
        )
        .await;
    game_state.tick_player_movement(60.0).await;

    let (position, _, _) = first_correction(&mut rx).expect("a refused step is corrected");
    assert_eq!((position.x, position.z), (0.5, 4.5));

    // Throttled: a client that cannot act on the snap must not be yanked every
    // tick, so an immediate second refusal stays silent.
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
            false,
        )
        .await;
    game_state.tick_player_movement(60.0).await;
    assert!(first_correction(&mut rx).is_none());
}

#[tokio::test]
async fn a_slid_step_is_not_corrected() {
    let game_state = make_test_game_state("movement_slide_no_correction");
    let player_id = pid("grazer");
    game_state.add_player(make_player("grazer", 1.5, 4.5)).await;
    let mut rx = game_state.register_direct_channel(&player_id).await;
    game_state.sync_region_furniture(0, 0, &[table_placement(0.5, 5.5)]);

    // Grazing the corner slides; the client did the same, so there is nothing
    // to reconcile and yanking it here would be a visible false positive.
    game_state
        .update_player_position(
            &player_id,
            move_cmd(
                Position {
                    x: 0.5,
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

    assert_eq!(player_xz(&game_state, &player_id).await, (0.5, 4.5));
    assert!(first_correction(&mut rx).is_none());
}

#[tokio::test]
async fn npc_movement_is_exempt_from_collision() {
    let game_state = make_test_game_state("movement_npc_exempt");
    let player_id = pid("npc_bot");
    game_state
        .add_player(make_player("npc_bot", 0.5, 4.5))
        .await;
    game_state.sync_region_furniture(0, 0, &[table_placement(0.5, 5.5)]);

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
            true,
        )
        .await;
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_xz(&game_state, &player_id).await, (0.5, 6.5));
}

/// One dungeon-shaped entry (never yields to a trapped mover) whose cell
/// (1, 1) is walled in on all four sides, the way a prop pillar seals its own.
fn sealed_dungeon_entry() -> onlinerpg_shared::pathfinding::RuntimePassability {
    use onlinerpg_shared::pathfinding::{RuntimeFloorGrid, RuntimePassability};
    const N: u8 = 1;
    const E: u8 = 2;
    const S: u8 = 4;
    const W: u8 = 8;
    let mut cells = vec![0u8; 16];
    let idx = |x: usize, z: usize| x + z * 4;
    cells[idx(1, 1)] = N | E | S | W;
    cells[idx(1, 0)] = S;
    cells[idx(1, 2)] = N;
    cells[idx(0, 1)] = E;
    cells[idx(2, 1)] = W;
    RuntimePassability {
        house_origin_x: 0.0,
        house_origin_z: 0.0,
        min_x: 0.0,
        max_x: 4.0,
        min_z: 0.0,
        max_z: 4.0,
        floors: vec![RuntimeFloorGrid {
            floor_level: 0,
            origin_x: 0,
            origin_z: 0,
            width: 4,
            depth: 4,
            y_base: 0.0,
            wall_height: 3.0,
            cells,
        }],
        stairwells: vec![],
        yields_to_trapped_mover: false,
    }
}

/// A restart makes a broken prop solid again under whoever logged out standing
/// on it, and a dungeon never yields to a trapped mover — so the tick has to
/// walk the player out instead of pinning them back inside the pillar. The
/// refusal that found the seal must not also be corrected: that snap carries
/// the cell they were refused at and would put them straight back in it.
#[tokio::test]
async fn a_player_sealed_in_is_walked_out_instead_of_pinned() {
    let game_state = make_test_game_state("movement_sealed_escape");
    let player_id = pid("boxed");
    game_state.add_player(make_player("boxed", 1.5, 1.5)).await;
    let mut rx = game_state.register_direct_channel(&player_id).await;
    game_state
        .passability_write()
        .insert("dungeon:test".to_string(), sealed_dungeon_entry());

    game_state
        .update_player_position(
            &player_id,
            move_cmd(
                Position {
                    x: 1.5,
                    y: 0.0,
                    z: 3.5,
                },
                false,
            ),
            false,
            false,
        )
        .await;
    game_state.tick_player_movement(60.0).await;

    let (x, z) = player_xz(&game_state, &player_id).await;
    assert!(
        (x - 1.5).abs() <= 1.0 && (z - 1.5).abs() <= 1.0,
        "moved to an adjoining cell, not across the floor: ({x}, {z})"
    );
    assert!(!onlinerpg_shared::pathfinding::is_cell_sealed(
        &game_state.passability_read(),
        x,
        z,
        0,
        Some(0.0)
    ));
    assert_eq!(
        first_correction(&mut rx),
        None,
        "a rescued player must not be snapped back to the cell they were sealed in"
    );
}

/// Furniture parked on a standing player seals their cell in the mask, but
/// `blocking_entry_for_mover` lets them walk out of it — so pressing into the
/// one side that is a wall must not read as sealed in and teleport them.
#[tokio::test]
async fn furniture_dropped_on_a_player_is_not_a_seal() {
    let game_state = make_test_game_state("movement_furniture_not_sealed");
    let player_id = pid("buried");
    game_state.add_player(make_player("buried", 0.5, 4.5)).await;
    // Tables on three sides, the room's own wall entry on the fourth.
    game_state.sync_region_furniture(
        0,
        0,
        &[
            table_placement(0.5, 5.5),
            table_placement(1.5, 4.5),
            table_placement(0.5, 3.5),
        ],
    );
    game_state
        .passability_write()
        .insert("dungeon:test".to_string(), sealed_dungeon_entry());

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
            false,
        )
        .await;
    game_state.tick_player_movement(60.0).await;

    assert_eq!(
        player_xz(&game_state, &player_id).await,
        (0.5, 4.5),
        "the player can still walk out from under furniture; nothing should move them"
    );
}

/// The escape is only for a mover with no legal step at all: an ordinary bump
/// into a wall must leave the player where they are.
#[test]
fn a_mover_with_a_way_out_is_left_alone() {
    let mut cache = onlinerpg_shared::pathfinding::PassabilityCache::new();
    cache.insert("dungeon:test".to_string(), sealed_dungeon_entry());
    let beside = Position {
        x: 2.5,
        y: 0.0,
        z: 1.5,
    };
    assert!(
        crate::game_state::passability::escape_from_sealed_cell(&cache, &beside, 0, |_, _| true)
            .is_none()
    );
}

const WARRENS_ID: &str = "orc_warrens";

/// The orc warrens with its passability installed, as `init_passability`
/// does at boot — the runtime alone has no cells.
async fn warrens_with_passability(tag: &str) -> (GameState, Position) {
    let game_state = make_test_game_state(tag);
    game_state.ensure_dungeon_runtime(WARRENS_ID).await;
    let entrance = game_state
        .dungeon_defs
        .get(WARRENS_ID)
        .expect("orc_warrens def")
        .position();
    {
        let dungeons = game_state.dungeons.read().await;
        let rt = dungeons.get(WARRENS_ID).expect("warrens runtime");
        game_state.passability_write().insert(
            onlinerpg_shared::dungeon::dungeon_cache_key(WARRENS_ID),
            onlinerpg_shared::dungeon::dungeon_passability(&entrance, &rt.layouts),
        );
    }
    (game_state, entrance)
}

/// Props cluster in room corners, so the neighbour behind the wall is usually
/// rock. Every prop of a real dungeon floor, through the same call the
/// movement tick makes: nothing may land off the carved floor.
#[tokio::test]
async fn no_prop_cell_escapes_onto_the_rock_outside_the_maze() {
    use onlinerpg_shared::dungeon::{cell_center, passability_floor_for_depth, world_to_cell};

    let (game_state, entrance) = warrens_with_passability("movement_escape_stays_carved").await;
    let depth = 9u8;
    let props = {
        let dungeons = game_state.dungeons.read().await;
        dungeons.get(WARRENS_ID).expect("warrens runtime").layouts[depth as usize - 1]
            .props
            .iter()
            .map(|p| (p.x, p.z))
            .collect::<Vec<_>>()
    };

    let mut freed = 0;
    for (px, pz) in props {
        let inside = cell_center(&entrance, depth, (px, pz));
        let Some(out) = game_state
            .sealed_player_escape(&inside, passability_floor_for_depth(depth))
            .await
        else {
            continue;
        };
        freed += 1;
        let (cx, cz) = world_to_cell(&entrance, out.x, out.z);
        let dungeons = game_state.dungeons.read().await;
        let layout =
            &dungeons.get(WARRENS_ID).expect("warrens runtime").layouts[depth as usize - 1];
        assert!(
            layout.is_carved(cx, cz),
            "prop ({px}, {pz}) escaped onto rock at cell ({cx}, {cz})"
        );
    }
    assert!(freed > 0, "no sealed prop cell on depth {depth} to check");
}

mod init_passability_boot {
    use super::*;
    use crate::terrain::io::TerrainIO;
    use crate::test_util::unique_temp_dir;

    fn write_objects_file(terrain_dir: &std::path::Path, contents: &str) {
        let path = onlinerpg_terrain::coords::object_path(terrain_dir, 0, 0);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[tokio::test]
    async fn missing_terrain_dir_boots_with_empty_furniture() {
        let game_state = make_test_game_state("passability_missing_dir");
        game_state
            .init_passability(&TerrainIO::new(unique_temp_dir("passability_missing")))
            .await
            .expect("missing terrain dir is a verified empty world");
    }

    #[tokio::test]
    async fn loads_solid_furniture_from_region_objects() {
        let game_state = make_test_game_state("passability_load_objects");
        let terrain_dir = unique_temp_dir("passability_objects");
        write_objects_file(
            &terrain_dir,
            r#"{"placements":[{"type":"table","x":0.5,"y":0.0,"z":5.5}]}"#,
        );
        let result = game_state
            .init_passability(&TerrainIO::new(terrain_dir.clone()))
            .await;
        std::fs::remove_dir_all(&terrain_dir).unwrap();
        result.expect("valid region objects load");
        let key = onlinerpg_shared::furniture::region_cache_key(0, 0);
        assert!(game_state.passability_read().contains_key(&key));
    }

    #[tokio::test]
    async fn corrupt_region_objects_file_refuses_boot() {
        let game_state = make_test_game_state("passability_corrupt_objects");
        let terrain_dir = unique_temp_dir("passability_corrupt");
        write_objects_file(&terrain_dir, "{ not json");
        let result = game_state
            .init_passability(&TerrainIO::new(terrain_dir.clone()))
            .await;
        std::fs::remove_dir_all(&terrain_dir).unwrap();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn unlistable_objects_dir_refuses_boot() {
        let game_state = make_test_game_state("passability_unlistable_objects");
        let terrain_dir = unique_temp_dir("passability_unlistable");
        std::fs::create_dir_all(&terrain_dir).unwrap();
        std::fs::write(terrain_dir.join("objects"), "not a directory").unwrap();
        let result = game_state
            .init_passability(&TerrainIO::new(terrain_dir.clone()))
            .await;
        std::fs::remove_dir_all(&terrain_dir).unwrap();
        assert!(result.is_err());
    }
}

/// Rock is sealed, not blank: a mover who ends up in it (a floor claimed at a
/// shaft's margin) cannot roam it, and is not "rescued" into the carved cell
/// next door — that would be a lift through the wrong floor's walls.
#[tokio::test]
async fn rock_is_a_dead_end_with_no_rescue() {
    use onlinerpg_shared::dungeon::{cell_center, passability_floor_for_depth, GRID};

    let (game_state, entrance) = warrens_with_passability("movement_rock_dead_end").await;
    let depth = 3u8;
    let floor = passability_floor_for_depth(depth);
    let (rock, room, deeper_rock) = {
        let dungeons = game_state.dungeons.read().await;
        let layout =
            &dungeons.get(WARRENS_ID).expect("warrens runtime").layouts[depth as usize - 1];
        (1..GRID - 1)
            .flat_map(|z| (1..GRID - 1).map(move |x| (x, z)))
            .find_map(|(x, z)| {
                (!layout.is_carved(x, z)
                    && layout.is_carved(x + 1, z)
                    && !layout.is_carved(x - 1, z))
                .then(|| {
                    (
                        cell_center(&entrance, depth, (x, z)),
                        cell_center(&entrance, depth, (x + 1, z)),
                        cell_center(&entrance, depth, (x - 1, z)),
                    )
                })
            })
            .expect("rock beside a carved cell")
    };

    assert!(
        game_state
            .sealed_player_escape(&rock, floor)
            .await
            .is_none(),
        "a mover in rock must not be nudged onto the floor"
    );
    let cache = game_state.passability_read();
    for (from, to) in [(rock, room), (room, rock), (rock, deeper_rock)] {
        assert!(
            matches!(
                super::super::passability::resolve_step(
                    &cache, from.x, from.z, to.x, to.z, floor, from.y
                ),
                super::super::passability::StepOutcome::Blocked(_)
            ),
            "step ({:.1},{:.1}) -> ({:.1},{:.1}) must be blocked",
            from.x,
            from.z,
            to.x,
            to.z
        );
    }
}
