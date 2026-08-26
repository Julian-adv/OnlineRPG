use super::*;

fn discovery_snapshots(msgs: Vec<ServerMessage>) -> Vec<Vec<String>> {
    msgs.into_iter()
        .filter_map(|msg| match msg {
            ServerMessage::DungeonDiscoveries { entrance_ids } => Some(entrance_ids),
            _ => None,
        })
        .collect()
}

async fn stage_wanderer(
    game_state: &GameState,
    name: &str,
    character_id: i64,
    x: f32,
    z: f32,
    known: Vec<String>,
) -> (PlayerId, DirectRx) {
    let player_id = pid(name);
    game_state.add_player(make_player(name, x, z)).await;
    game_state
        .register_player_character(&player_id, character_id, 0, attrs_with_cha(12), 0, None)
        .await;
    game_state.set_dungeon_discoveries(&player_id, known).await;
    let rx = game_state.register_direct_channel(&player_id).await;
    (player_id, rx)
}

async fn walk_to(game_state: &GameState, player_id: &PlayerId, x: f32, z: f32) {
    game_state
        .update_player_position(player_id, move_cmd(Position { x, y: 0.0, z }, false), false)
        .await;
    game_state.tick_player_movement(30.0).await;
}

/// Walking into event-delivery range of an entrance announces it exactly
/// once, queues the DB write, and a relog seeded from the DB stays quiet.
#[tokio::test]
async fn entrance_discovery_fires_once_and_survives_a_relog() {
    let auth = make_test_auth("dungeon_discovery");
    let account = auth.login_npc("npc_dungeon_discovery").unwrap();
    let character = create_test_character(&auth, &account, "Wayfarer");

    let game_state = make_test_game_state("dungeon_discovery");
    let entrance = first_dungeon(&game_state);
    let (player_id, mut rx) = stage_wanderer(
        &game_state,
        "Wayfarer",
        character.id,
        entrance.x - 60.0,
        entrance.z,
        Vec::new(),
    )
    .await;

    walk_to(&game_state, &player_id, entrance.x - 50.0, entrance.z).await;
    assert!(
        discovery_snapshots(drain(&mut rx)).is_empty(),
        "outside the delivery radius nothing is discovered"
    );

    walk_to(&game_state, &player_id, entrance.x - 30.0, entrance.z).await;
    assert_eq!(
        discovery_snapshots(drain(&mut rx)),
        vec![vec![entrance.id.clone()]],
        "entering the radius announces the entrance"
    );

    walk_to(&game_state, &player_id, entrance.x - 20.0, entrance.z).await;
    assert!(
        discovery_snapshots(drain(&mut rx)).is_empty(),
        "a known entrance is never re-announced"
    );

    game_state.flush_dirty_saves(&auth).await;
    let stored = auth.load_dungeon_history(character.id).unwrap().1;
    assert_eq!(stored, vec![entrance.id.clone()]);

    game_state.unregister_player_character(&player_id).await;
    game_state.remove_player(&player_id).await;

    let (rejoined_id, mut rejoined_rx) = stage_wanderer(
        &game_state,
        "Wayfarer Rejoined",
        character.id,
        entrance.x - 60.0,
        entrance.z,
        stored,
    )
    .await;
    walk_to(&game_state, &rejoined_id, entrance.x - 30.0, entrance.z).await;
    assert!(
        discovery_snapshots(drain(&mut rejoined_rx)).is_empty(),
        "a DB-seeded relog already knows the entrance"
    );
}

/// Every point that satisfies the discovery predicate must map to a cell
/// listing that entrance — a gap would permanently mute discovery there.
/// The ±45 m scan bound is deliberately independent of the builder's own
/// region math (sight radius 43, footprint offsets within ±41), so a wrong
/// bounding box in `discovery_cells` cannot hide from this test.
#[tokio::test]
async fn discovery_cell_prefilter_covers_the_discovery_region() {
    let game_state = make_test_game_state("discovery_prefilter");
    let radius_sq = EVENT_DELIVERY_RADIUS * EVENT_DELIVERY_RADIUS;
    for entrance in game_state.dungeon_defs.all() {
        for xi in -45..=45 {
            for zi in -45..=45 {
                let pos = Position {
                    x: entrance.x + xi as f32,
                    y: 0.0,
                    z: entrance.z + zi as f32,
                };
                let discoverable = pos.dist_xz_sq(&entrance.position()) <= radius_sq
                    || entrance.footprint_contains(pos.x, pos.z);
                if !discoverable {
                    continue;
                }
                let listed = game_state
                    .dungeon_discovery_cells
                    .get(&SpatialCell::from_position(&pos))
                    .is_some_and(|entrances| entrances.iter().any(|e| e.id == entrance.id));
                assert!(
                    listed,
                    "({}, {}) near '{}' is discoverable but not indexed",
                    pos.x, pos.z, entrance.id
                );
            }
        }
    }
}

/// A session whose discovery load failed (never seeded) still discovers:
/// a missing entry means "nothing known yet", and the INSERT OR IGNORE
/// persistence absorbs any re-announcement of an already-stored entrance.
#[tokio::test]
async fn unseeded_player_still_discovers() {
    let auth = make_test_auth("dungeon_discovery_unseeded");
    let account = auth.login_npc("npc_discovery_unseeded").unwrap();
    let character = create_test_character(&auth, &account, "Unseeded");

    let game_state = make_test_game_state("dungeon_discovery_unseeded");
    let entrance = first_dungeon(&game_state);
    let player_id = pid("Unseeded");
    game_state
        .add_player(make_player("Unseeded", entrance.x - 60.0, entrance.z))
        .await;
    game_state
        .register_player_character(&player_id, character.id, 0, attrs_with_cha(12), 0, None)
        .await;
    let mut rx = game_state.register_direct_channel(&player_id).await;

    walk_to(&game_state, &player_id, entrance.x - 10.0, entrance.z).await;
    assert_eq!(
        discovery_snapshots(drain(&mut rx)),
        vec![vec![entrance.id.clone()]]
    );
    game_state.flush_dirty_saves(&auth).await;
    assert_eq!(
        auth.load_dungeon_history(character.id).unwrap().1,
        vec![entrance.id.clone()]
    );
}
