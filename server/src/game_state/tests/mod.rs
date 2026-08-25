use super::*;
use crate::housing::HousingIO;
use crate::item_defs::ItemDefs;
use crate::monster_defs::MonsterDefs;
use crate::types::{
    AttackRejectReason, CharacterClass, ClientKind, Gender, MonsterLifecycle, MonsterState,
    PlayerId, Position, ServerMessage,
};
use crate::world_config::world_config;
use onlinerpg_shared::inventory::{EquipSlot, GroundItem, ItemInstance, PlayerInventory};
use onlinerpg_shared::messages::DealKind;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::sync::mpsc::error::TryRecvError as MpscTryRecvError;

mod ambient_spawn_tests;
mod cape_dye_tests;
mod cape_texture_tests;
mod chat_tests;
mod collision_tests;
mod combat_tests;
mod dungeon_tests;
mod enchant_tests;
mod fishing_tests;
mod friend_tests;
mod house_floor_tests;
mod hunger_tests;
mod inventory_tests;
mod monster_ai_tests;
mod movement_tests;
mod party_tests;
mod persistence_tests;
mod pickup_tests;
mod player_tests;
mod player_trade_tests;
mod skills_tests;
mod spawn_scale_tests;
mod spawn_soak_tests;
mod stall_tests;
mod tip_hat_tests;
mod trading_tests;

/// Stable numeric id derived from a fixture's name, so tests keep naming
/// players ("owner", "buyer") instead of carrying opaque integers. Only needs
/// to be consistent within one process; a collision between two names in the
/// same test would surface as an immediate failure, never as a silent pass.
fn pid(name: &str) -> PlayerId {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    // Mask to 32 bits and avoid 0, which is the "no player" sentinel.
    PlayerId::from((hasher.finish() & 0xFFFF_FFFF).max(1))
}

fn make_player(id: &str, x: f32, z: f32) -> Player {
    Player {
        id: pid(id),
        name: id.to_string(),
        position: Position { x, y: 0.0, z },
        rotation: 0.0,
        level: 1,
        health: 10,
        max_health: 10,
        class: CharacterClass::Knight,
        gender: Gender::default(),
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
    }
}

fn attrs_with_cha(cha: u8) -> CharacterAttributes {
    CharacterAttributes {
        r#str: 10,
        dex: 10,
        con: 10,
        int: 10,
        wis: 10,
        cha,
        guard: 0,
    }
}

fn bag_item(instance_id: u64, item_def_id: &str, quantity: u32) -> ItemInstance {
    ItemInstance {
        instance_id,
        item_def_id: item_def_id.to_string(),
        quantity,
        enchant: 0,
        cape_color: None,
        cape_texture: None,
    }
}

fn pos(x: f32) -> Position {
    Position { x, y: 0.0, z: 0.0 }
}

fn move_cmd(position: Position, append: bool) -> MoveCommand {
    MoveCommand {
        position,
        rotation: 0.0,
        floor_level: 0,
        append,
        sprinting: false,
    }
}

fn table_placement(x: f32, z: f32) -> onlinerpg_shared::furniture::FurniturePlacement {
    onlinerpg_shared::furniture::FurniturePlacement {
        type_id: "table".to_string(),
        x,
        y: 0.0,
        z,
        rotation_deg: 0.0,
        floor_level: 0,
    }
}

async fn player_xz(game_state: &GameState, player_id: &PlayerId) -> (f32, f32) {
    let player = &game_state.get_all_players().await[player_id];
    (player.position.x, player.position.z)
}

/// Decodes `Shared` payloads back into `ServerMessage`, mirroring the
/// receiver API so tests assert on exactly what a client would decode.
struct DirectRx(tokio::sync::mpsc::UnboundedReceiver<DirectMessage>);

impl DirectRx {
    fn try_recv(&mut self) -> Result<ServerMessage, MpscTryRecvError> {
        self.0.try_recv().map(|payload| match payload {
            DirectMessage::Typed(msg) => msg,
            DirectMessage::Shared(bytes) => onlinerpg_shared::deserialize_server_msg(&bytes)
                .expect("shared direct payload decodes"),
        })
    }
}

/// Test-side name for the connection channel, wrapped in the decoder.
trait RegisterDirectChannel {
    async fn register_direct_channel(&self, player_id: &PlayerId) -> DirectRx;
}

impl RegisterDirectChannel for GameState {
    async fn register_direct_channel(&self, player_id: &PlayerId) -> DirectRx {
        DirectRx(self.register_connection_channel(player_id).await)
    }
}

fn drain(rx: &mut DirectRx) -> Vec<ServerMessage> {
    let mut msgs = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        msgs.push(msg);
    }
    msgs
}

/// Walk a player to (x, z) through the real move path — queued waypoint plus
/// movement tick — so distance-coupled ambient spawning sees the displacement.
/// Long walks are split into legs the move guard accepts.
async fn walk_player_to(game_state: &GameState, player_id: &PlayerId, x: f32, z: f32) {
    // Comfortably inside the move guard, so no leg is refused.
    const LEG: f32 = onlinerpg_shared::MAX_MOVE_TARGET_DISTANCE * 0.8;
    loop {
        let from = game_state.players.read().await[player_id].position;
        let dx = onlinerpg_shared::shortest_world_delta_x(from.x, x);
        let dz = z - from.z;
        let remaining = (dx * dx + dz * dz).sqrt();
        if remaining < 0.001 {
            return;
        }
        let t = (LEG / remaining).min(1.0);
        let target = Position {
            x: onlinerpg_shared::wrap_world_x(from.x + dx * t),
            y: from.y,
            z: from.z + dz * t,
        };
        game_state
            .update_player_position(
                player_id,
                crate::game_state::MoveCommand {
                    position: target,
                    rotation: 0.0,
                    floor_level: 0,
                    append: false,
                    sprinting: false,
                },
                false,
                false,
            )
            .await;
        game_state.tick_player_movement(60.0).await;
    }
}

/// Pace `legs` × 10m back and forth from (x, z). Pacing rather than striding
/// off: a monster left outside the AOI is released and despawned, so the tail
/// of a straight run says nothing about what it drew.
async fn pace_player(game_state: &GameState, player_id: &PlayerId, x: f32, z: f32, legs: usize) {
    for leg in 0..legs {
        let to = if leg.is_multiple_of(2) { z + 10.0 } else { z };
        walk_player_to(game_state, player_id, x, to).await;
    }
}

/// Monsters this player owns and has not killed.
async fn owned_monster_count(game_state: &GameState, player_id: &PlayerId) -> usize {
    game_state.monsters.read().await.owned_alive_by(player_id)
}

fn first_dungeon(game_state: &GameState) -> crate::dungeon_defs::DungeonEntranceDef {
    game_state
        .dungeon_defs
        .all()
        .next()
        .expect("a dungeon def")
        .clone()
}

/// Which player's client is simulating a monster.
async fn owner_of(game_state: &GameState, monster_id: &str) -> Option<PlayerId> {
    game_state
        .monsters
        .read()
        .await
        .get(monster_id)
        .and_then(|m| m.owner_id)
}

fn first_correction(rx: &mut DirectRx) -> Option<(Position, f32, i8)> {
    std::iter::from_fn(|| rx.try_recv().ok()).find_map(|msg| match msg {
        ServerMessage::PositionCorrected {
            position,
            rotation,
            floor_level,
        } => Some((position, rotation, floor_level)),
        _ => None,
    })
}

fn make_monster(id: &str, position: Position, floor_level: i8) -> crate::types::Monster {
    crate::types::Monster {
        id: id.to_string(),
        monster_type: "test_monster".to_string(),
        position,
        rotation: 0.0,
        state: MonsterState::Idle,
        owner_id: None,
        health: 10,
        max_health: 10,
        floor_level,
        level_override: None,
        aggressive: false,
        lifecycle: MonsterLifecycle::Ambient,
        last_attack_at: 0,
        last_move_at: 0,
        move_budget: 0.0,
        owner_since: 0,
    }
}

const NON_FINITE: [(f32, &str); 3] = [
    (f32::NAN, "NaN"),
    (f32::INFINITY, "+infinity"),
    (f32::NEG_INFINITY, "-infinity"),
];

/// Raw u16 heightmap buffer whose every vertex sits at `meters`.
fn uniform_heightmap(meters: f32) -> Vec<u8> {
    let encoded = ((meters + 500.0) / 0.05) as u16;
    let verts = onlinerpg_terrain::defaults::VERTS_PER_SIDE;
    let mut buf = Vec::with_capacity(verts * verts * 2);
    for _ in 0..(verts * verts) {
        buf.extend_from_slice(&encoded.to_le_bytes());
    }
    buf
}

/// Terrain for tests: negative-x tiles are 5 m under water, the rest 5 m
/// above. Lets fishing tests cast into real "water" without tile files.
struct SplitWorldTiles;

#[async_trait::async_trait]
impl onlinerpg_terrain::height::HeightTiles for SplitWorldTiles {
    async fn read_heightmap(&self, tx: i32, _tz: i32) -> std::io::Result<Vec<u8>> {
        Ok(uniform_heightmap(if tx < 0 { -5.0 } else { 5.0 }))
    }
}

/// No baked water field anywhere: every tile samples as flat sea level. With
/// `SplitWorldTiles` this makes negative-x the ocean (bed −5, surface 0 →
/// deep water) and positive-x dry land (bed +5, surface 0 → no water),
/// matching the original ocean/land fishing tests.
struct SeaOnlyWater;

#[async_trait::async_trait]
impl onlinerpg_terrain::water::WaterTiles for SeaOnlyWater {
    async fn read_water_field(&self, _tx: i32, _tz: i32) -> std::io::Result<Option<Vec<u8>>> {
        Ok(None)
    }
}

/// Land everywhere at 5m: a walk in any direction stays on dry flat ground,
/// so ambient spawn placement is never refused by the terrain.
struct FlatLand;

#[async_trait::async_trait]
impl onlinerpg_terrain::height::HeightTiles for FlatLand {
    async fn read_heightmap(&self, _tx: i32, _tz: i32) -> std::io::Result<Vec<u8>> {
        Ok(uniform_heightmap(5.0))
    }
}

/// A flat, dry, grassy world — the fixture for ambient spawn tests.
fn make_flat_world_game_state(test_name: &str) -> GameState {
    make_game_state_with(test_name, FlatLand, SeaOnlyWater)
}

/// Splat for tests: every cell is the vegetation base, so a spawn is never
/// rejected for the ground it lands on.
struct GrassSplat;

#[async_trait::async_trait]
impl onlinerpg_terrain::splat::SplatTiles for GrassSplat {
    async fn read_splat(&self, _tx: i32, _tz: i32) -> std::io::Result<Vec<u8>> {
        Ok(vec![0u8; onlinerpg_terrain::defaults::SPLATMAP_SIZE])
    }
}

fn make_game_state_with(
    test_name: &str,
    height: impl onlinerpg_terrain::height::HeightTiles + 'static,
    water: impl onlinerpg_terrain::water::WaterTiles + 'static,
) -> GameState {
    make_game_state_with_zones(test_name, height, water, vec![])
}

fn make_game_state_with_zones(
    test_name: &str,
    height: impl onlinerpg_terrain::height::HeightTiles + 'static,
    water: impl onlinerpg_terrain::water::WaterTiles + 'static,
    no_spawn_zones: Vec<onlinerpg_shared::NoSpawnZone>,
) -> GameState {
    let housing_dir = std::env::temp_dir().join(format!(
        "onlinerpg_{test_name}_housing_{}",
        uuid::Uuid::new_v4()
    ));
    let housing_io = Arc::new(HousingIO::new(housing_dir));
    let item_defs = crate::item_defs::item_defs().clone();
    let world_drop_defs = crate::world_drop_defs::WorldDropDefs::load(&item_defs);
    let monster_defs = MonsterDefs::load();
    let dungeon_defs = crate::dungeon_defs::DungeonDefs::load(&item_defs, &monster_defs);
    GameState::new(
        monster_defs,
        item_defs,
        world_drop_defs,
        GameState::default_start_datetime(),
        housing_io,
        no_spawn_zones,
        dungeon_defs,
        Arc::new(onlinerpg_terrain::height::HeightSampler::new(height)),
        Arc::new(onlinerpg_terrain::water::WaterSampler::new(water)),
        Arc::new(onlinerpg_terrain::splat::SplatSampler::new(GrassSplat)),
        Arc::new(
            crate::cape_texture::CapeTextureStore::new(std::env::temp_dir().join(format!(
                "onlinerpg_{test_name}_capes_{}",
                uuid::Uuid::new_v4()
            )))
            .expect("cape texture store"),
        ),
    )
}

pub(crate) fn make_test_game_state(test_name: &str) -> GameState {
    make_game_state_with(test_name, SplitWorldTiles, SeaOnlyWater)
}

/// Temp-file AuthService for tests whose paths touch the auth DB.
pub(crate) fn make_test_auth(test_name: &str) -> crate::auth::AuthService {
    make_test_auth_with_path(test_name).0
}

/// Variant exposing the DB path for tests that open a second connection.
fn make_test_auth_with_path(test_name: &str) -> (crate::auth::AuthService, std::path::PathBuf) {
    let db_path = std::env::temp_dir().join(format!(
        "onlinerpg_{test_name}_auth_{}.db",
        uuid::Uuid::new_v4()
    ));
    (
        crate::auth::AuthService::new(db_path.clone()).unwrap(),
        db_path,
    )
}

fn create_test_character(
    auth: &crate::auth::AuthService,
    account: &str,
    name: &str,
) -> crate::auth::CharacterRecord {
    auth.create_character(
        account,
        name,
        &attrs_with_cha(12),
        16,
        CharacterClass::Knight,
        Gender::Male,
    )
    .unwrap()
}
