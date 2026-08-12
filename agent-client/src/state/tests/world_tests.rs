use super::*;

/// Our own performances land in the recent-song list, oldest first and
/// capped; the world state shows the list only to an agent that busks,
/// and never counts someone else's tune as ours.
#[test]
fn recent_songs_render_for_the_busker_only() {
    let (mut s, _rx) = test_state();
    let me = test_player(0.0, 0.0);
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);
    s.plays_music = true;

    s.push_event(ServerMessage::PlayerMusicStarted {
        player_id: PlayerId::from(2),
        track: "Someone Else's Tune".to_string(),
        elapsed_secs: 0.0,
    });
    for i in 0..10 {
        s.push_event(ServerMessage::PlayerMusicStarted {
            player_id: PlayerId::from(1),
            track: format!("Song {i}"),
            elapsed_secs: 0.0,
        });
    }

    let world = s.format_world_state();
    assert!(
        world.contains("Songs you played recently, oldest first: Song 2,"),
        "capped at MAX_RECENT_SONGS, oldest dropped: {world}"
    );
    assert!(world.contains("Song 9"), "{world}");
    assert!(!world.contains("Someone Else's Tune"), "{world}");

    s.plays_music = false;
    assert!(!s.format_world_state().contains("Songs you played recently"));
}

/// The world state lists reachable ground items closest first, and
/// leaves out other floors and anything out of sight.
#[test]
fn world_state_lists_nearby_ground_items() {
    let (mut s, _rx) = test_state();
    s.self_player = Some(test_player(0.0, 0.0));
    for item in [
        ground_item(1, "small_sword", 5.0, 0.0, 0),
        ground_item(2, "wooden_shield", 2.0, 0.0, 0),
        ground_item(3, "coin_pile", 1.0, 0.0, 0),
        ground_item(4, "iron_sword", 0.0, NPC_SIGHT_RADIUS + 5.0, 0),
        ground_item(5, "healing_potion", 3.0, 0.0, 1),
    ] {
        s.remember_ground_item(item);
    }

    let lines: Vec<String> = s
        .format_world_state()
        .lines()
        .filter(|l| l.starts_with("Item on ground:"))
        .map(str::to_string)
        .collect();

    assert_eq!(
        lines,
        vec![
            "Item on ground: coin_pile (1.0m away) [id 3]",
            "Item on ground: wooden_shield (2.0m away) [id 2]",
            "Item on ground: small_sword (5.0m away) [id 1]",
        ]
    );
}

/// An announced item is loot the agent may go for right away — the
/// server does any withholding.
#[test]
fn an_announced_drop_is_actionable_at_once() {
    let (mut s, _rx) = test_state();
    s.self_player = Some(test_player(0.0, 0.0));

    s.push_event(ServerMessage::GroundItemSpawned {
        item: ground_item(1, "goblin_sword", 2.0, 0.0, 0),
    });
    s.push_event(ServerMessage::GroundItemAppeared {
        item: ground_item(2, "small_sword", 3.0, 0.0, 0),
    });

    let ids: Vec<u64> = s
        .ground_items_in_sight()
        .iter()
        .map(|(_, i)| i.instance_id)
        .collect();
    assert_eq!(ids, vec![1, 2]);
    assert!(s.ground_item(1).is_some());
    assert!(s.format_world_state().contains("goblin_sword"));
}

/// A field strewn with drops is summarised, not listed line by line.
#[test]
fn world_state_caps_the_ground_item_list() {
    let (mut s, _rx) = test_state();
    s.self_player = Some(test_player(0.0, 0.0));
    for id in 1..=(MAX_LISTED_GROUND_ITEMS as u64 + 3) {
        let item = ground_item(id, "small_sword", id as f32 * 0.5, 0.0, 0);
        s.remember_ground_item(item);
    }

    let world = s.format_world_state();
    let listed = world
        .lines()
        .filter(|l| l.starts_with("Item on ground:"))
        .count();

    assert_eq!(listed, MAX_LISTED_GROUND_ITEMS);
    assert!(world.contains("(and 3 more items further away)"), "{world}");
}

/// A `DoorToggled` must land on both faces of the door: the passability
/// edge A* walks and the `HouseData` wall the door hunt reads. With only
/// the edge updated, `closed_doors_on_our_floor` kept re-listing a door
/// that was already open and the agent toggled it shut again.
#[test]
fn door_toggle_keeps_house_walls_in_step_with_the_edges() {
    use onlinerpg_shared::housing::{
        HouseData, PassabilityGrid, RoomData, WallConfig, WallDirection, WallVariant,
    };

    let wall = |variant| WallConfig {
        variant,
        texture: 0,
        is_open: false,
    };
    let room = RoomData {
        room_type: Default::default(),
        roof_type: Default::default(),
        roof_ridge_dir: Default::default(),
        stair_reversed: false,
        local_x: 0,
        local_z: 0,
        size_x: 1,
        size_z: 1,
        floor_level: 0,
        floor_texture: 0,
        roof_texture: 0,
        wall_height: 3.0,
        wall_north: vec![wall(WallVariant::WithDoor)],
        wall_south: vec![wall(WallVariant::Solid)],
        wall_east: vec![wall(WallVariant::Solid)],
        wall_west: vec![wall(WallVariant::Solid)],
    };

    let house = HouseData {
        id: "h".to_string(),
        owner_id: "test".to_string(),
        origin: onlinerpg_shared::Position {
            x: 10.0,
            y: 0.0,
            z: 10.0,
        },
        rooms: vec![room],
        passability: vec![PassabilityGrid {
            floor_level: 0,
            origin_x: 0,
            origin_z: 0,
            width: 1,
            depth: 1,
            // All four edges walled (N=1, E=2, S=4, W=8), door shut.
            cells: vec![1 | 2 | 4 | 8],
        }],
    };

    let mut world = WorldCache::new();
    world.add_house(house);

    let door_blocked = |world: &WorldCache| {
        pathfinding::is_movement_blocked(world.passability_cache(), 10.5, 10.5, 10.5, 9.5, 0, None)
    };
    assert!(door_blocked(&world), "the north door starts shut");

    world.update_door("h", 0, WallDirection::North, 0, true);
    assert!(
        world.houses()["h"].rooms[0].wall_north[0].is_open,
        "HouseData must track the open"
    );
    assert!(!door_blocked(&world), "the edge must open with the door");

    world.update_door("h", 0, WallDirection::North, 0, false);
    assert!(!world.houses()["h"].rooms[0].wall_north[0].is_open);
    assert!(door_blocked(&world), "the edge must seal again");
}

/// Splat tiles that paint one road cell and one river cell near origin,
/// so the grid test can assert glyph placement against known world
/// coordinates.
struct PaintedSplat;

#[async_trait::async_trait]
impl crate::splat::SplatTiles for PaintedSplat {
    async fn read_splat(&self, tx: i32, tz: i32) -> std::io::Result<Vec<u8>> {
        let mut data = vec![0u8; onlinerpg_terrain::defaults::SPLATMAP_SIZE];
        if (tx, tz) == (0, 0) {
            let mut paint = |wx: f32, wz: f32, pal: u8| {
                let cx = (wx + 32.0).floor() as usize;
                let cz = (wz + 32.0).floor() as usize;
                data[(cz * 64 + cx) * 4] = pal << 4;
            };
            paint(6.0, 0.0, crate::splat::PAL_ROAD);
            paint(-6.0, -6.0, crate::splat::PAL_RIVER_BED);
        }
        Ok(data)
    }
}

#[tokio::test]
async fn terrain_grid_labels_world_coordinates_and_paints_surfaces() {
    let (mut s, _rx) = test_state();
    s.splat_sampler = Arc::new(crate::splat::SplatSampler::new(PaintedSplat));
    s.self_player = Some(test_player(0.0, 0.0));
    let grid = s.terrain_grid_job().expect("on the surface").render().await;

    assert!(
        grid.contains("x=-27 to x=27"),
        "header must carry the exact west/east span:\n{grid}"
    );
    assert!(grid.contains("Map: surface, you at (0, 0)"));

    let cells_of = |prefix: &str| -> Vec<String> {
        grid.lines()
            .find(|l| l.starts_with(prefix))
            .unwrap_or_else(|| panic!("no row {prefix} in:\n{grid}"))
            .split_whitespace()
            .skip(1)
            .map(str::to_string)
            .collect()
    };
    // Row z=0: self at column 9, the road cell (6, 0) at column 11.
    let mid = cells_of("z=0 ");
    assert_eq!(mid[9], "@");
    assert_eq!(mid[11], "R");
    // Row z=-6: the river cell (-6, -6) at column 7.
    let north = cells_of("z=-6 ");
    assert_eq!(north[7], "~");
}

#[test]
fn terrain_grid_is_absent_underground() {
    let (mut s, _rx) = test_state();
    s.self_player = Some(test_player(0.0, 0.0));
    s.self_floor_level = -1;
    assert!(s.terrain_grid_job().is_none());
}
