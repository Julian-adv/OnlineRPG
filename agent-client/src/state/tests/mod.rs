mod dungeon_tests;
mod events_tests;
mod inventory_tests;
mod movement_tests;
mod music_tests;
mod social_tests;
mod world_tests;

use super::*;

struct NoTiles;

#[async_trait::async_trait]
impl onlinerpg_terrain::height::HeightTiles for NoTiles {
    async fn read_heightmap(&self, _tx: i32, _tz: i32) -> std::io::Result<Vec<u8>> {
        Err(std::io::Error::other("no terrain in tests"))
    }
}

#[async_trait::async_trait]
impl crate::splat::SplatTiles for NoTiles {
    async fn read_splat(&self, _tx: i32, _tz: i32) -> std::io::Result<Vec<u8>> {
        Err(std::io::Error::other("no terrain in tests"))
    }
}

pub(crate) fn test_state() -> (SharedState, mpsc::Receiver<ClientMessage>) {
    let (tx, rx) = mpsc::channel(8);
    let state = SharedState::new(
        Vec::new(),
        tx,
        Arc::new(HeightSampler::new(NoTiles)),
        Arc::new(crate::splat::SplatSampler::new(NoTiles)),
        Arc::new(std::sync::RwLock::new(WorldCache::new())),
        None,
    );
    (state, rx)
}

pub(crate) fn p(x: f32, y: f32, z: f32) -> Position {
    Position { x, y, z }
}

pub(crate) fn monster(id: &str) -> Monster {
    Monster {
        id: id.to_string(),
        monster_type: "slime".to_string(),
        position: p(0.0, 0.0, 0.0),
        rotation: 0.0,
        state: MonsterState::Idle,
        owner_id: None,
        health: 10,
        max_health: 10,
        floor_level: 0,
        level_override: None,
        aggressive: false,
        lifecycle: Default::default(),
        last_attack_at: 0,
        last_move_at: 0,
        move_budget: 0.0,
        owner_since: 0,
    }
}

pub(crate) fn ground_item(id: u64, def: &str, x: f32, z: f32, floor: i8) -> GroundItem {
    GroundItem {
        instance_id: id,
        item_def_id: def.to_string(),
        position: p(x, 0.0, z),
        floor_level: floor,
        quantity: 1,
        enchant: 0,
        dropped_by: None,
        cape_color: None,
        cape_texture: None,
    }
}

/// A `ground_item` a player put down, as a tip test sees it.
fn dropped_item(id: u64, def: &str, x: f32, z: f32, by: PlayerId) -> GroundItem {
    GroundItem {
        dropped_by: Some(by),
        ..ground_item(id, def, x, z, 0)
    }
}

pub(crate) fn test_player(x: f32, z: f32) -> Player {
    Player {
        id: PlayerId::from(1),
        name: "Me".to_string(),
        position: p(x, 0.0, z),
        rotation: 0.0,
        level: 1,
        health: 10,
        max_health: 10,
        class: onlinerpg_shared::CharacterClass::Rogue,
        gender: Default::default(),
        is_official_npc: false,
        torch_on: false,
        floor_level: 0,
        object_type: None,
        main_hand: None,
        back: None,
        object_id: None,
        last_combat_at: 0,
        client_kind: Default::default(),
        back_color: None,
        back_texture: None,
        wet: false,
    }
}

fn dungeon_state() -> (
    SharedState,
    Arc<crate::dungeon::Dungeon>,
    mpsc::Receiver<ClientMessage>,
) {
    let mut cache = WorldCache::new();
    cache.register_dungeons();
    let world = Arc::new(std::sync::RwLock::new(cache));
    let dungeon = world.read().unwrap().dungeon_at(-1450.0, 4720.0).unwrap();
    let (tx, rx) = mpsc::channel(64);
    let mut state = SharedState::new(
        Vec::new(),
        tx,
        Arc::new(HeightSampler::new(NoTiles)),
        Arc::new(crate::splat::SplatSampler::new(NoTiles)),
        world,
        None,
    );
    state.self_player = Some(Player {
        position: dungeon.entrance,
        ..test_player(0.0, 0.0)
    });
    (state, dungeon, rx)
}

/// Pull every "(x, z)" pair out of a state line.
fn coordinates_in(line: &str) -> Vec<(f32, f32)> {
    line.split('(')
        .skip(1)
        .filter_map(|rest| rest.split_once(')'))
        .filter_map(|(inner, _)| inner.split_once(','))
        .filter_map(|(x, z)| Some((x.trim().parse().ok()?, z.trim().parse().ok()?)))
        .collect()
}

/// Put the agent on `depth`, standing on `cell`.
fn stand_at(
    s: &mut SharedState,
    dungeon: &crate::dungeon::Dungeon,
    depth: u8,
    cell: (i32, i32),
) -> Position {
    let stand = onlinerpg_shared::dungeon::cell_center(&dungeon.entrance, depth, cell);
    s.self_floor_level = -(depth as i8);
    s.self_player = Some(Player {
        position: stand,
        floor_level: -(depth as i8),
        ..test_player(stand.x, stand.z)
    });
    stand
}

/// Standing where the chest room is, on the deepest floor.
fn in_the_chest_room(s: &mut SharedState, dungeon: &crate::dungeon::Dungeon) -> u8 {
    let depth = dungeon.max_depth();
    let layout = dungeon.layouts().last().unwrap();
    let cell = layout.chest.unwrap();
    let room = layout.room_at(cell.0, cell.1).unwrap();
    stand_at(s, dungeon, depth, layout.stand_cell(room.center()));
    depth
}

/// A point partway down the entrance ramp, low enough to read as floor 1.
fn mid_shaft_point(dungeon: &crate::dungeon::Dungeon) -> (f32, f32, f32) {
    let e = dungeon.entrance;
    // Past the ramp's midpoint (so the nearest floor is the one below) but
    // short of the bottom landing.
    let low = dungeon.floor_y(1) + 0.5;
    let high = (e.y + dungeon.floor_y(1)) / 2.0 - 0.2;
    let mut step = 0;
    while step < 80 * 80 {
        let x = e.x - 20.0 + (step % 80) as f32 * 0.5;
        let z = e.z - 20.0 + (step / 80) as f32 * 0.5;
        step += 1;
        if let Some(y) = dungeon.ground_y(0, x, z) {
            if y > low && y < high {
                return (x, z, y);
            }
        }
    }
    panic!("no mid-ramp point found on the entrance shaft");
}

/// The server overwrites Y; this only guards our own pose model.
const FLOOR_Y_SANITY: f32 = 2.5;
