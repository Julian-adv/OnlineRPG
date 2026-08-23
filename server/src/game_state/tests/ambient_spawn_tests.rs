//! Move-coupled ambient spawning (doc/REPEAT_FARMING.md, part 1): monsters are
//! granted by distance walked, placed just off the screen edge ahead, and only
//! where one could actually walk in from.
use super::*;

#[tokio::test]
async fn walking_spawns_monsters_and_standing_still_does_not() {
    let game_state = make_flat_world_game_state("ambient_walk_spawns");
    let player_id = pid("walker");
    game_state
        .add_player(make_player("walker", 200.0, 0.0))
        .await;
    game_state.enable_ambient_spawns();

    pace_player(&game_state, &player_id, 200.0, 0.0, 60).await;
    let after_walking = owned_monster_count(&game_state, &player_id).await;
    assert!(
        after_walking > 0,
        "600m of walking must draw at least one monster"
    );

    for _ in 0..100 {
        game_state.tick_player_movement(1.0).await;
    }
    assert_eq!(
        owned_monster_count(&game_state, &player_id).await,
        after_walking,
        "standing still must draw nothing — that is the whole point"
    );
}

#[tokio::test]
async fn walking_forever_still_stops_at_the_cap() {
    let game_state = make_flat_world_game_state("ambient_walk_cap");
    let player_id = pid("marathon");
    game_state
        .add_player(make_player("marathon", 200.0, 0.0))
        .await;
    game_state.enable_ambient_spawns();

    pace_player(&game_state, &player_id, 200.0, 0.0, 200).await;
    assert_eq!(
        owned_monster_count(&game_state, &player_id).await,
        world_config().max_monsters_per_player as usize,
        "2km of pacing fills the cap and stops there"
    );
}

#[tokio::test]
async fn spawns_land_on_the_screen_edge_inside_the_heading_cone() {
    let center = Position {
        x: 100.0,
        y: 0.0,
        z: 100.0,
    };
    // Screen axes: u runs right (world +x+z), v runs up (world +x−z).
    let screen = |p: &Position| {
        let inv = std::f32::consts::FRAC_1_SQRT_2;
        (
            (p.x - center.x + p.z - center.z) * inv,
            (p.x - center.x - (p.z - center.z)) * inv,
        )
    };
    for (dx, dz, label) in [
        (1.0, 1.0, "screen right"),
        (-1.0, -1.0, "screen left"),
        (1.0, -1.0, "screen up"),
        (-1.0, 1.0, "screen down"),
        (1.0, 0.2, "off-axis"),
    ] {
        for _ in 0..200 {
            let point = GameState::screen_cone_point(&center, dx, dz);
            let (u, v) = screen(&point);
            assert!(
                (u.abs().max(v.abs()) - 20.0).abs() < 0.01,
                "{label}: the point must sit on the square's edge, got ({u},{v})"
            );
            let off = (dz.atan2(dx) - (point.z - center.z).atan2(point.x - center.x))
                .abs()
                .to_degrees();
            assert!(
                off <= 30.01,
                "{label}: {off}° off the heading — the cone is 30°"
            );
        }
    }
}

#[tokio::test]
async fn nothing_spawns_in_open_water() {
    // SplitWorldTiles puts negative-x tiles 5m under the sea surface.
    let game_state = make_test_game_state("ambient_water");
    let player_id = pid("swimmer");
    game_state
        .add_player(make_player("swimmer", -200.0, 0.0))
        .await;
    game_state.enable_ambient_spawns();
    for leg in 1..=10 {
        walk_player_to(&game_state, &player_id, -200.0, 100.0 * leg as f32).await;
    }
    assert_eq!(
        owned_monster_count(&game_state, &player_id).await,
        0,
        "the sea is not a hunting ground"
    );
}

#[tokio::test]
async fn nothing_spawns_around_a_no_spawn_zone() {
    let zone = onlinerpg_shared::NoSpawnZone {
        min_x: 100.0,
        max_x: 300.0,
        min_z: -100.0,
        max_z: 1100.0,
    };
    let game_state =
        make_game_state_with_zones("ambient_no_spawn_zone", FlatLand, SeaOnlyWater, vec![zone]);
    let player_id = pid("townie");
    game_state
        .add_player(make_player("townie", 200.0, 0.0))
        .await;
    game_state.enable_ambient_spawns();

    pace_player(&game_state, &player_id, 200.0, 0.0, 100).await;
    assert_eq!(
        owned_monster_count(&game_state, &player_id).await,
        0,
        "a town and its margin stay clear however far one walks through it"
    );
}

#[tokio::test]
async fn what_spawns_still_follows_the_distance_from_town() {
    let game_state = make_flat_world_game_state("ambient_town_distance");
    let player_id = pid("novice");
    let town = world_config().spawn_position.position();
    game_state
        .add_player(make_player("novice", town.x, town.z))
        .await;
    game_state.enable_ambient_spawns();
    // Pace by the town gates: never far from them, so only the lowest types
    // are eligible however long the walk goes on.
    pace_player(&game_state, &player_id, town.x, town.z, 60).await;

    let monsters = game_state.monsters.read().await;
    let spawned: Vec<&str> = monsters.values().map(|m| m.monster_type.as_str()).collect();
    assert!(!spawned.is_empty(), "walking near town still spawns");
    for monster_type in spawned {
        assert!(
            game_state.min_ambient_town_distance(monster_type) <= 100.0,
            "{monster_type} belongs further out than this walk ever went"
        );
    }
}

/// The types on offer follow the distance from town, as they did when the
/// client asked for them: nothing deep near the gates, everything far out.
#[tokio::test]
async fn eligible_types_follow_the_distance_from_town() {
    let game_state = make_flat_world_game_state("ambient_type_gate");
    let town = world_config().spawn_position.position();
    let far = Position {
        x: town.x,
        z: town.z
            + world_config()
                .ambient_spawns
                .iter()
                .map(|rule| game_state.min_ambient_town_distance(&rule.monster_type))
                .fold(0.0_f32, f32::max)
            + 1.0,
        y: 0.0,
    };

    let mut near_town = std::collections::HashSet::new();
    let mut deep = std::collections::HashSet::new();
    for _ in 0..500 {
        near_town.extend(game_state.pick_ambient_type(&town));
        deep.extend(game_state.pick_ambient_type(&far));
    }
    for monster_type in &near_town {
        assert_eq!(
            game_state.min_ambient_town_distance(monster_type),
            0.0,
            "{monster_type} belongs further from town than the gates"
        );
    }
    assert_eq!(
        deep.len(),
        world_config().ambient_spawns.len(),
        "past the deepest gate every ambient type is on offer"
    );
}

/// Admin (trusted) moves apply straight away and never enter the movement
/// queue, so they need their own roll — this is how a live admin run from town
/// to the dungeon met nothing at all.
#[tokio::test]
async fn a_trusted_admin_walk_still_draws_monsters() {
    let game_state = make_flat_world_game_state("ambient_trusted_walk");
    let player_id = pid("gm");
    game_state.add_player(make_player("gm", 200.0, 0.0)).await;
    game_state.enable_ambient_spawns();

    // Count every arrival: walking on releases the monsters left behind, so
    // the tail of a straight run says nothing about what it drew.
    let mut arrivals = 0usize;
    let mut owned = 0usize;
    let mut z = 0.0;
    for _ in 0..100 {
        z += 5.0;
        game_state
            .update_player_position(
                &player_id,
                crate::game_state::MoveCommand {
                    position: Position {
                        x: 200.0,
                        y: 5.0,
                        z,
                    },
                    rotation: 0.0,
                    floor_level: 0,
                    append: false,
                    sprinting: false,
                },
                true,
                false,
            )
            .await;
        let now = owned_monster_count(&game_state, &player_id).await;
        arrivals += now.saturating_sub(owned);
        owned = now;
    }
    assert!(
        arrivals > 0,
        "500m of admin walking must draw monsters like anyone else's"
    );
}

/// A teleport is not a walk: the jump must not cash in as a near-certain spawn.
#[tokio::test]
async fn a_teleport_is_not_a_walk() {
    let game_state = make_flat_world_game_state("ambient_teleport");
    let player_id = pid("blinker");
    game_state
        .add_player(make_player("blinker", 200.0, 0.0))
        .await;
    game_state.enable_ambient_spawns();

    for leg in 1..=50 {
        game_state
            .teleport_player(
                &player_id,
                Position {
                    x: 200.0,
                    y: 5.0,
                    z: 1_000.0 * leg as f32,
                },
                0.0,
                0,
            )
            .await;
    }
    assert_eq!(
        owned_monster_count(&game_state, &player_id).await,
        0,
        "50 teleports of a kilometre each must earn nothing"
    );
}
