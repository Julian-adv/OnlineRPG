//! Server-owned house storeys: a storey changes only on its stairwell, and
//! the stored Y is the storey's own height, so a forged Y can neither pick
//! the storey a mover collides against nor lift it over that storey's walls.

use super::*;
use onlinerpg_shared::pathfinding::{RuntimeFloorGrid, RuntimePassability, StairwellInfo};

const N: u8 = 1;
const S: u8 = 4;

/// Two storeys over world cells x 10..13, z 10..14 (y_base 0 and 3.1), joined
/// by a stairwell up the x 10..11 column. The ground floor has a wall across
/// x 12..13 between z 12 and z 13; the upper storey is open. The x 12..13
/// column is beyond the stairwell's change margin, the x 11..12 one inside it.
fn two_storey_house() -> RuntimePassability {
    let grid = |floor_level, y_base, cells| RuntimeFloorGrid {
        floor_level,
        origin_x: 0,
        origin_z: 0,
        width: 3,
        depth: 4,
        y_base,
        wall_height: 3.0,
        cells,
    };
    let mut ground = vec![0u8; 12];
    ground[2 + 2 * 3] = S;
    ground[2 + 3 * 3] = N;
    RuntimePassability {
        house_origin_x: 10.0,
        house_origin_z: 10.0,
        min_x: 10.0,
        max_x: 13.0,
        min_z: 10.0,
        max_z: 14.0,
        floors: vec![grid(0, 0.0, ground), grid(1, 3.1, vec![0u8; 12])],
        stairwells: vec![StairwellInfo {
            local_min_x: 0,
            local_min_z: 0,
            local_max_x: 1,
            local_max_z: 4,
            lower_floor: 0,
            upper_floor: 1,
            along_z: true,
            reversed: false,
        }],
        yields_to_trapped_mover: false,
        is_ground: true,
    }
}

async fn house_with_player(
    tag: &str,
    name: &str,
    x: f32,
    z: f32,
    floor: i8,
) -> (GameState, PlayerId) {
    let game_state = make_test_game_state(tag);
    let player_id = pid(name);
    let mut player = make_player(name, x, z);
    player.floor_level = floor;
    game_state.add_player(player).await;
    game_state
        .passability_write()
        .insert("house:test".to_string(), two_storey_house());
    (game_state, player_id)
}

fn move_to(x: f32, y: f32, z: f32, floor_level: i8) -> MoveCommand {
    MoveCommand {
        floor_level,
        ..move_cmd(Position { x, y, z }, false)
    }
}

async fn queued_target(game_state: &GameState, player_id: &PlayerId) -> Option<(Position, i8)> {
    game_state
        .movement_intents
        .read()
        .await
        .get(player_id)
        .and_then(|q| q.back())
        .map(|w| (w.target, w.floor_level))
}

/// The exploit: standing on the ground floor, report the Y of the storey
/// above (or any height) and walk through the ground floor's wall. The stored
/// Y is the storey's own, so the wall still stands.
#[tokio::test]
async fn a_forged_height_cannot_pick_the_storey() {
    let (game_state, player_id) =
        house_with_player("storey_forged_y", "wallhack", 12.5, 11.5, 0).await;
    let mut rx = game_state.register_direct_channel(&player_id).await;

    for y in [3.1, 1000.0] {
        game_state
            .update_player_position(&player_id, move_to(12.5, y, 13.5, 0), false, false)
            .await;
        let (target, _) = queued_target(&game_state, &player_id)
            .await
            .expect("queued");
        assert_eq!(
            target.y, 0.0,
            "Y {y} is replaced by the ground floor's height"
        );
        for _ in 0..10 {
            game_state.tick_player_movement(0.1).await;
        }
        let (x, z) = player_xz(&game_state, &player_id).await;
        assert!(z < 13.0, "the ground floor wall still blocks: ({x}, {z})");
    }
    assert!(
        first_correction(&mut rx).is_some(),
        "the refused step is corrected"
    );
}

#[tokio::test]
async fn a_storey_change_off_the_stairs_is_refused_and_snapped() {
    let (game_state, player_id) =
        house_with_player("storey_off_stairs", "climber", 12.5, 11.5, 0).await;
    let mut rx = game_state.register_direct_channel(&player_id).await;

    game_state
        .update_player_position(&player_id, move_to(12.5, 3.1, 13.5, 1), false, false)
        .await;
    assert!(queued_target(&game_state, &player_id).await.is_none());
    let (position, _, floor_level) = first_correction(&mut rx).expect("snapped back");
    assert_eq!((position.x, position.z, floor_level), (12.5, 11.5, 0));
    assert_eq!(game_state.players.read().await[&player_id].floor_level, 0);

    // A standalone change is ignored the same way.
    game_state.update_player_floor(&player_id, 1).await;
    assert_eq!(game_state.players.read().await[&player_id].floor_level, 0);
}

#[tokio::test]
async fn a_storey_change_on_the_stairs_is_accepted_both_ways() {
    let (game_state, player_id) =
        house_with_player("storey_on_stairs", "climber", 10.5, 12.5, 0).await;
    let mut rx = game_state.register_direct_channel(&player_id).await;

    game_state
        .update_player_position(&player_id, move_to(10.5, 3.1, 13.5, 1), false, false)
        .await;
    let (target, floor) = queued_target(&game_state, &player_id)
        .await
        .expect("queued");
    assert_eq!(
        (floor, target.y),
        (1, 3.1),
        "the exit landing is at the upper storey's height"
    );
    game_state.tick_player_movement(1.0).await;
    assert_eq!(game_state.players.read().await[&player_id].floor_level, 1);
    assert!(first_correction(&mut rx).is_none());

    // Back down, as a standalone change from the landing.
    game_state.update_player_floor(&player_id, 0).await;
    assert_eq!(game_state.players.read().await[&player_id].floor_level, 0);

    // Two storeys at once is not a climb.
    game_state.update_player_floor(&player_id, 2).await;
    assert_eq!(game_state.players.read().await[&player_id].floor_level, 0);
}

/// A storey held where nothing of it stands under the mover (the room was
/// edited away) releases to the ground; otherwise the client, which no longer
/// believes in the storey, has every move refused.
#[tokio::test]
async fn a_storey_without_floor_under_the_mover_releases_to_the_ground() {
    let (game_state, player_id) =
        house_with_player("storey_released", "stranded", 50.5, 50.5, 1).await;
    game_state.update_player_floor(&player_id, 0).await;
    assert_eq!(game_state.players.read().await[&player_id].floor_level, 0);
}

#[tokio::test]
async fn a_storey_still_under_the_mover_is_kept() {
    let (game_state, player_id) = house_with_player("storey_kept", "upstairs", 12.5, 11.5, 1).await;
    game_state.update_player_floor(&player_id, 0).await;
    assert_eq!(game_state.players.read().await[&player_id].floor_level, 1);
}

/// The stored Y comes from the cache (ramp profile pinned by the shared
/// `storey_ground_y` tests), except on open terrain where nothing stands.
#[tokio::test]
async fn stairwell_height_is_derived_not_reported() {
    let (game_state, player_id) =
        house_with_player("storey_derived_y", "walker", 10.5, 10.5, 0).await;
    game_state
        .update_player_position(&player_id, move_to(10.5, 1000.0, 12.0, 0), false, false)
        .await;
    let (target, _) = queued_target(&game_state, &player_id)
        .await
        .expect("queued");
    assert!(
        (target.y - 1.55).abs() < 1e-4,
        "mid-flight ramp height, got {}",
        target.y
    );
}

#[tokio::test]
async fn open_terrain_keeps_the_reported_height() {
    let (game_state, player_id) =
        house_with_player("terrain_reported_y", "rover", 50.5, 50.5, 0).await;
    game_state
        .update_player_position(&player_id, move_to(51.5, 7.0, 50.5, 0), false, false)
        .await;
    let (target, _) = queued_target(&game_state, &player_id)
        .await
        .expect("queued");
    assert_eq!(target.y, 7.0);
}

/// Official NPCs walk their schedules across storeys (a force-move through a
/// shut door included) unvalidated, as they already do through walls.
#[tokio::test]
async fn an_official_npc_changes_storey_off_the_stairs() {
    let (game_state, player_id) = house_with_player("storey_npc", "rica", 12.5, 11.5, 0).await;
    game_state
        .update_player_position(&player_id, move_to(12.5, 3.1, 13.5, 1), false, true)
        .await;
    let (target, floor) = queued_target(&game_state, &player_id)
        .await
        .expect("queued");
    assert_eq!((floor, target.y), (1, 3.1));
    game_state.tick_player_movement(5.0).await;
    assert_eq!(game_state.players.read().await[&player_id].floor_level, 1);
}
