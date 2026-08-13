use super::*;

/// The floor we declare must follow our height: the server derives the
/// floor it collides against from the Y we send and validates the
/// declaration against it, so the two have to resolve identically.
#[test]
fn declared_floor_tracks_height() {
    let (s, dungeon, _rx) = dungeon_state();
    let (x, z) = (dungeon.entrance.x, dungeon.entrance.z);

    assert_eq!(s.wire_floor_at(x, z, dungeon.entrance.y), 0);
    assert_eq!(s.wire_floor_at(x, z, dungeon.floor_y(1)), -1);
    assert_eq!(s.wire_floor_at(x, z, dungeon.floor_y(3)), -3);
    // Mid-ramp resolves to whichever floor is nearer, never past the last.
    assert_eq!(s.wire_floor_at(x, z, dungeon.entrance.y - 1.0), 0);
    assert_eq!(s.wire_floor_at(x, z, dungeon.entrance.y - 3.0), -1);
    let deepest = dungeon.max_depth();
    assert_eq!(
        s.wire_floor_at(x, z, dungeon.floor_y(deepest) - 50.0),
        -(deepest as i8)
    );
}

/// Chest sightings run off the live passability, so the cell they tell the
/// mover to stand on must be one A* can actually route to — a clutter prop
/// is a sealed pillar, and aiming at it strands the agent every time.
#[test]
fn a_sighted_chest_is_approached_from_a_cell_a_path_can_reach() {
    let (mut s, dungeon, _rx) = dungeon_state();
    let depth = in_the_chest_room(&mut s, &dungeon);
    let floor = dungeon.passability_floor(depth);

    let chests = s.chests_in_sight();
    assert!(
        chests
            .iter()
            .any(|c| c.kind == crate::dungeon::ChestKind::Treasure),
        "the chest room should show its treasure chest"
    );
    assert!(
        chests.len() > 1,
        "old_crypt's chest room also holds a clutter chest"
    );
    for chest in chests {
        let a = chest.approach;
        assert!(
            s.world_cache.read().unwrap().is_walkable(&a, floor),
            "{:?} is approached from a sealed cell",
            chest.kind
        );
        assert!(
            s.find_path_to(a.x, a.z, floor).found,
            "{:?} has no route to its approach cell",
            chest.kind
        );
    }
}

/// Every coordinate the underground state line hands the LLM has to be a
/// cell it can actually stand on. A shaft is walkable on this floor only
/// along one row — its min corner and the cell half a metre over are both
/// rock — so a wrong end or a rounded centre reads as a wall.
#[test]
fn the_floor_map_only_names_cells_the_agent_can_stand_on() {
    let (mut s, _crypt, _rx) = dungeon_state();
    let mut orientations = std::collections::HashSet::new();

    for def in onlinerpg_shared::dungeon::entrances() {
        let dungeon = s
            .world_cache
            .read()
            .unwrap()
            .dungeon_by_id(&def.id)
            .expect("registered dungeon");

        // Every door open: a shut one is a detour the mover handles, so it
        // must not be confused with a cell walled off for good.
        let doors: Vec<(u8, u32)> = (1..=dungeon.max_depth())
            .flat_map(|d| {
                dungeon
                    .closed_doors(d, &HashSet::new())
                    .into_iter()
                    .map(move |door| (d, door.door_id))
            })
            .collect();
        s.world_cache
            .write()
            .unwrap()
            .set_dungeon_doors(&dungeon.id, &doors);

        for depth in 1..=dungeon.max_depth() {
            let layout = &dungeon.layouts()[depth as usize - 1];
            orientations.insert(layout.up_shaft.reversed);
            let floor = dungeon.passability_floor(depth);
            stand_at(&mut s, &dungeon, depth, layout.rooms[0].center());

            let line = s.format_dungeon_state().expect("underground state line");
            let where_ = format!("{} floor {depth}", dungeon.id);
            let named = coordinates_in(&line);
            assert!(
                named.len() > layout.rooms.len(),
                "{where_} should name every room plus the stairs, got {named:?}"
            );
            for (x, z) in named {
                let p = Position { x, y: 0.0, z };
                // Printed coordinates must survive the round trip back to
                // the cell they name. Cell centres sit on .5, so rounding
                // them to whole metres silently names the cell next door.
                let cell = world_to_cell(&dungeon.entrance, x, z);
                let centre = cell_center(&dungeon.entrance, depth, cell);
                assert_eq!(
                    (centre.x, centre.z),
                    (x, z),
                    "{where_} prints ({x}, {z}), which reads back as the cell \
                         centred on ({}, {})\n{line}",
                    centre.x,
                    centre.z
                );
                assert!(
                    s.world_cache.read().unwrap().is_walkable(&p, floor),
                    "{where_} points the agent at ({x}, {z}), which is solid rock\n{line}"
                );
                // A shaft's interior is carved but walled off from this
                // floor, so walkable is not enough — the goal has to be
                // routable too.
                assert!(
                    s.find_path_to(x, z, floor).found,
                    "{where_} points the agent at ({x}, {z}), which no route \
                         reaches\n{line}"
                );
            }
        }
    }

    assert_eq!(
        orientations.len(),
        2,
        "sample covers only one shaft orientation, so it cannot catch an \
             entry/exit mix-up"
    );
}

/// Breakables are offered off the live passability the same way chests
/// are, and a smashed one drops out of the listing.
#[tokio::test]
async fn a_smashed_prop_stops_being_offered() {
    let (mut s, dungeon, _rx) = dungeon_state();
    let depth = in_the_chest_room(&mut s, &dungeon);
    let floor = dungeon.passability_floor(depth);
    let prop = s
        .breakables_in_sight()
        .first()
        .copied()
        .expect("the chest room holds breakable clutter");
    assert!(
        s.find_path_to(prop.approach.x, prop.approach.z, floor)
            .found,
        "a prop is offered with a cell we can route to"
    );

    s.push_event(ServerMessage::DungeonPropBroken {
        entrance_id: dungeon.id.clone(),
        depth,
        prop_id: prop.prop_id,
    });
    assert!(!s
        .breakables_in_sight()
        .iter()
        .any(|b| b.prop_id == prop.prop_id));
}

/// A clutter prop is marked opened before the server answers, because an
/// already-claimed one answers with silence. A rejection says it never
/// opened, so the mark has to come back off — otherwise a chest the agent
/// merely stood too far from is invisible for the rest of the floor.
#[tokio::test]
async fn a_rejected_prop_open_becomes_visible_again() {
    let (mut s, dungeon, _rx) = dungeon_state();
    let depth = in_the_chest_room(&mut s, &dungeon);
    let prop = match s
        .chests_in_sight()
        .into_iter()
        .find(|c| matches!(c.kind, crate::dungeon::ChestKind::Prop(_)))
        .expect("the chest room holds a clutter chest")
        .kind
    {
        crate::dungeon::ChestKind::Prop(id) => id,
        _ => unreachable!(),
    };

    s.chest_open_sent(&dungeon.id, depth, crate::dungeon::ChestKind::Prop(prop));
    assert!(
        !s.chests_in_sight()
            .iter()
            .any(|c| c.kind == crate::dungeon::ChestKind::Prop(prop)),
        "a sent open hides the chest so we stop targeting it"
    );

    s.push_event(ServerMessage::InteractionRejected {
        reason: "Too far from the chest".to_string(),
    });
    assert!(
        s.chests_in_sight()
            .iter()
            .any(|c| c.kind == crate::dungeon::ChestKind::Prop(prop)),
        "a refused open leaves the chest there to try again"
    );
}

/// An emptied treasure chest still stands there, so it keeps showing — but
/// the line says it has nothing left, or the agent walks back to it all
/// night for the same refusal.
#[tokio::test]
async fn an_emptied_treasure_chest_says_so() {
    let (mut s, dungeon, _rx) = dungeon_state();
    let depth = in_the_chest_room(&mut s, &dungeon);
    assert!(!s.format_world_state().contains("refills at nightfall"));

    s.chest_open_sent(&dungeon.id, depth, crate::dungeon::ChestKind::Treasure);
    s.push_event(ServerMessage::InteractionRejected {
        reason: "The chest is empty (it refills at nightfall)".to_string(),
    });

    let world = s.format_world_state();
    assert!(
        world.contains("a great chest standing alone")
            && world.contains("you emptied it; it refills at nightfall"),
        "{world}"
    );
}

/// Registering the dungeon is all the shared A* needs to walk the entrance
/// stairwell: a path from the surface to floor 1 must exist and end there.
#[test]
fn a_path_leads_from_the_entrance_down_to_the_first_floor() {
    let (s, dungeon, _rx) = dungeon_state();
    let landing = dungeon.arrival_position(1).unwrap();
    let floor = dungeon.passability_floor(1);

    let path = s.find_path_to(landing.x, landing.z, floor);

    assert!(path.found, "no route from the entrance down to floor 1");
    assert_eq!(path.waypoints.last().map(|w| w.floor), Some(floor));
}

/// Every step of that descent must declare a floor the server accepts and
/// collides against identically — it derives the floor from the Y we send,
/// so a step whose declaration and height disagree gets snapped back.
#[test]
fn descending_steps_declare_a_floor_the_server_accepts() {
    let (mut s, dungeon, _rx) = dungeon_state();
    let landing = dungeon.arrival_position(1).unwrap();
    let path = s.find_path_to(landing.x, landing.z, dungeon.passability_floor(1));
    assert!(path.found);

    let mut seen_underground = false;
    for wp in &path.waypoints {
        // Mirror the mover: subdivide the leg and pose each step.
        loop {
            let position = s.self_player.as_ref().unwrap().position;
            let to_wp = crate::geom::PlanarDelta::to_xz(&position, wp.x, wp.z);
            if to_wp.dist < 0.1 {
                break;
            }
            let (sx, sz) = if to_wp.dist <= 3.0 {
                (wp.x, wp.z)
            } else {
                let r = 3.0 / to_wp.dist;
                (position.x + to_wp.dx * r, position.z + to_wp.dz * r)
            };
            let (pose, floor_level) = s.step_pose(sx, sz, wp.floor, position.y);
            if floor_level < 0 {
                seen_underground = true;
                let expected = dungeon.floor_y(floor_level.unsigned_abs());
                assert!(
                    (pose.y - expected).abs() <= SERVER_FLOOR_Y_TOLERANCE,
                    "floor {floor_level} declared at y={} (floor sits at {expected})",
                    pose.y
                );
            }
            s.self_player.as_mut().unwrap().position = pose;
            s.self_floor_level = floor_level;
        }
    }

    assert!(seen_underground, "the walk never went underground");
    assert_eq!(s.self_floor_level, -1);
}

/// Re-pathing from halfway down the stairs (after a fight or a correction)
/// must still work: those cells are keyed to the floor above, so searching
/// under the floor we are nearest would strand the agent on the steps.
#[test]
fn a_path_still_leads_on_from_halfway_down_the_stairs() {
    let (mut s, dungeon, _rx) = dungeon_state();
    let (x, z, y) = mid_shaft_point(&dungeon);
    s.self_player.as_mut().unwrap().position = Position { x, y, z };
    s.self_floor_level = s.wire_floor_at(x, z, y);

    assert_eq!(s.self_floor_level, -1, "mid-ramp should read as floor 1");
    assert_eq!(
        s.passability_floor(),
        0,
        "stair cells are keyed one floor up"
    );

    let landing = dungeon.arrival_position(1).unwrap();
    let path = s.find_path_to(landing.x, landing.z, dungeon.passability_floor(1));
    assert!(path.found, "no route on from the middle of the stairs");
}

/// The stairs down sit behind shut doors on most floors, so opening one has
/// to reopen the cells A* walks — otherwise the agent never gets past
/// floor 1 no matter how many doors it toggles.
#[test]
fn opening_a_door_reopens_the_route_behind_it() {
    let (mut s, dungeon, _rx) = dungeon_state();
    let landing = dungeon.arrival_position(1).unwrap();
    s.self_player.as_mut().unwrap().position = landing;
    s.self_floor_level = -1;

    let below = dungeon.arrival_position(2).unwrap();
    let goal_floor = dungeon.passability_floor(2);
    assert!(
        !s.find_path_to(below.x, below.z, goal_floor).found,
        "floor 1's stairs down are supposed to start sealed"
    );

    let doors: Vec<(u8, u32)> = dungeon
        .closed_doors(1, &HashSet::new())
        .iter()
        .map(|d| (1u8, d.door_id))
        .collect();
    assert!(!doors.is_empty());
    s.world_cache
        .write()
        .unwrap()
        .set_dungeon_doors(&dungeon.id, &doors);

    assert!(
        s.find_path_to(below.x, below.z, goal_floor).found,
        "the way down stayed sealed after opening floor 1's doors"
    );
}
