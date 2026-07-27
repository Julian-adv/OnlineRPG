use super::*;
use crate::housing::HousingIO;
use crate::item_defs::ItemDefs;
use crate::monster_defs::MonsterDefs;
use crate::types::{
    CharacterClass, ClientKind, Gender, MonsterState, PlayerId, Position, ServerMessage,
};
use crate::world_config::world_config;
use onlinerpg_shared::inventory::{EquipSlot, GroundItem, ItemInstance, PlayerInventory};
use onlinerpg_shared::messages::DealKind;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::sync::mpsc::error::TryRecvError as MpscTryRecvError;

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

/// `GameState.players` is a list (numeric ids can't key a wasm-serialized
/// map), so snapshot assertions look their player up by id.
fn find_player(players: &[Player], id: PlayerId) -> &Player {
    players
        .iter()
        .find(|p| p.id == id)
        .expect("player missing from snapshot")
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
        object_id: None,
        last_combat_at: 0,
        client_kind: Default::default(),
    }
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
        last_attack_at: 0,
        last_move_at: 0,
        move_budget: 0.0,
    }
}

const NON_FINITE: [(f32, &str); 3] = [
    (f32::NAN, "NaN"),
    (f32::INFINITY, "+infinity"),
    (f32::NEG_INFINITY, "-infinity"),
];

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

/// Terrain for tests: negative-x tiles are 5 m under water, the rest 5 m
/// above. Lets fishing tests cast into real "water" without tile files.
struct SplitWorldTiles;

#[async_trait::async_trait]
impl onlinerpg_terrain::height::HeightTiles for SplitWorldTiles {
    async fn read_heightmap(&self, tx: i32, _tz: i32) -> std::io::Result<Vec<u8>> {
        // u16 encoding: round((meters + 500) / 0.05).
        let meters: f32 = if tx < 0 { -5.0 } else { 5.0 };
        let encoded = ((meters + 500.0) / 0.05) as u16;
        let verts = onlinerpg_terrain::defaults::VERTS_PER_SIDE;
        let mut buf = Vec::with_capacity(verts * verts * 2);
        for _ in 0..(verts * verts) {
            buf.extend_from_slice(&encoded.to_le_bytes());
        }
        Ok(buf)
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

/// A river world for the ocean-vs-river test: terrain is a 5 m-high plateau
/// everywhere (well above sea level), and the water field reports a river
/// surface at 5.4 m everywhere — the exact case the old `height < 0` check
/// wrongly rejected. Fishing must still work here.
struct RiverPlateauTiles;

#[async_trait::async_trait]
impl onlinerpg_terrain::height::HeightTiles for RiverPlateauTiles {
    async fn read_heightmap(&self, _tx: i32, _tz: i32) -> std::io::Result<Vec<u8>> {
        let encoded = ((5.0 + 500.0) / 0.05) as u16;
        let verts = onlinerpg_terrain::defaults::VERTS_PER_SIDE;
        let mut buf = Vec::with_capacity(verts * verts * 2);
        for _ in 0..(verts * verts) {
            buf.extend_from_slice(&encoded.to_le_bytes());
        }
        Ok(buf)
    }
}

struct RiverPlateauWater;

#[async_trait::async_trait]
impl onlinerpg_terrain::water::WaterTiles for RiverPlateauWater {
    async fn read_water_field(&self, _tx: i32, _tz: i32) -> std::io::Result<Option<Vec<u8>>> {
        // WFD1: 16-byte header + 65×65 pixels of 6 bytes; surfaceY 5.4 m.
        let verts = onlinerpg_terrain::defaults::VERTS_PER_SIDE;
        let mut out = Vec::new();
        out.extend_from_slice(b"WFD1");
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(verts as u16).to_le_bytes());
        out.extend_from_slice(&(verts as u16).to_le_bytes());
        out.extend_from_slice(&[0u8; 6]);
        let enc = ((5.4f32 + 500.0) / 0.05).round() as u16;
        for _ in 0..(verts * verts) {
            out.extend_from_slice(&enc.to_le_bytes());
            out.extend_from_slice(&[0, 0, 255, 0]); // flowX, flowZ, riverness, turbulence
        }
        Ok(Some(out))
    }
}

fn make_test_game_state(test_name: &str) -> GameState {
    let housing_dir = std::env::temp_dir().join(format!(
        "onlinerpg_{test_name}_housing_{}",
        uuid::Uuid::new_v4()
    ));
    let housing_io = Arc::new(HousingIO::new(housing_dir));
    let item_defs = ItemDefs::load();
    let world_drop_defs = crate::world_drop_defs::WorldDropDefs::load(&item_defs);
    let dungeon_defs = crate::dungeon_defs::DungeonDefs::load(&item_defs);
    GameState::new(
        MonsterDefs::load(),
        item_defs,
        world_drop_defs,
        GameState::default_start_datetime(),
        housing_io,
        vec![],
        dungeon_defs,
        Arc::new(onlinerpg_terrain::height::HeightSampler::new(
            SplitWorldTiles,
        )),
        Arc::new(onlinerpg_terrain::water::WaterSampler::new(SeaOnlyWater)),
    )
}

/// A game state whose terrain is a 5 m plateau with a river surface at 5.4 m
/// everywhere — for testing that fishing works in rivers whose beds are above
/// sea level (the ocean-vs-river regression).
fn make_river_game_state(test_name: &str) -> GameState {
    let housing_dir = std::env::temp_dir().join(format!(
        "onlinerpg_{test_name}_housing_{}",
        uuid::Uuid::new_v4()
    ));
    let housing_io = Arc::new(HousingIO::new(housing_dir));
    let item_defs = ItemDefs::load();
    let world_drop_defs = crate::world_drop_defs::WorldDropDefs::load(&item_defs);
    let dungeon_defs = crate::dungeon_defs::DungeonDefs::load(&item_defs);
    GameState::new(
        MonsterDefs::load(),
        item_defs,
        world_drop_defs,
        GameState::default_start_datetime(),
        housing_io,
        vec![],
        dungeon_defs,
        Arc::new(onlinerpg_terrain::height::HeightSampler::new(
            RiverPlateauTiles,
        )),
        Arc::new(onlinerpg_terrain::water::WaterSampler::new(
            RiverPlateauWater,
        )),
    )
}

/// Temp-file AuthService for tests whose paths touch the auth DB.
fn make_test_auth(test_name: &str) -> crate::auth::AuthService {
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

#[tokio::test]
async fn equipped_torch_syncs_live_and_late_join_player_state() {
    let game_state = make_test_game_state("late_join_torch_snapshot");
    let torch_holder_id = pid("torch_holder");

    game_state
        .add_player(make_player("torch_holder", 0.0, 0.0))
        .await;
    game_state.inventories.write().await.insert(
        torch_holder_id,
        PlayerInventory {
            bag: vec![bag_item(1, "torch", 1)],
            equipped: Default::default(),
        },
    );

    game_state.equip_item(&torch_holder_id, 1).await;
    assert!(game_state.get_all_players().await[&torch_holder_id].torch_on);

    let snapshot = game_state
        .add_player(make_player("late_joiner", 1.0, 0.0))
        .await
        .expect("nearby existing player should produce a GameState snapshot");
    match snapshot {
        ServerMessage::GameState { players, .. } => {
            assert!(find_player(&players, torch_holder_id).torch_on);
        }
        other => panic!("expected GameState, got {other:?}"),
    }

    game_state
        .unequip_item(&torch_holder_id, EquipSlot::OffHand)
        .await;

    assert!(!game_state.get_all_players().await[&torch_holder_id].torch_on);
}

#[tokio::test]
async fn respawn_player_revives_dead_player_only() {
    let game_state = make_test_game_state("respawn_dead");

    let player = Player {
        id: pid("player_dead"),
        name: "DeadPlayer".to_string(),
        position: Position {
            x: 12.0,
            y: 0.0,
            z: -4.0,
        },
        rotation: 1.25,
        level: 3,
        health: 0,
        max_health: 30,
        class: CharacterClass::Knight,
        gender: Gender::default(),
        is_official_npc: false,
        torch_on: false,
        floor_level: 0,
        object_type: None,
        object_id: None,
        last_combat_at: 0,
        client_kind: Default::default(),
    };
    let player_id = player.id;
    game_state.add_player(player).await;

    let mut direct_rx = game_state.register_direct_channel(&player_id).await;
    let mut broadcast_rx = game_state.subscribe();
    game_state.respawn_player(&player_id).await;

    let players = game_state.get_all_players().await;
    let revived = players
        .get(&player_id)
        .expect("Player should still exist after respawn");
    let spawn = &world_config().spawn_position;
    assert_eq!(revived.health, revived.max_health);
    assert_eq!(revived.position.x, spawn.x);
    assert_eq!(revived.position.y, spawn.y);
    assert_eq!(revived.position.z, spawn.z);
    assert_eq!(revived.rotation, spawn.rotation);

    match direct_rx.try_recv() {
        Ok(ServerMessage::PlayerRespawned { player }) => {
            assert_eq!(player.id, player_id);
            assert_eq!(player.health, player.max_health);
        }
        other => panic!("Expected direct PlayerRespawned, got {:?}", other),
    }

    match broadcast_rx.try_recv() {
        Err(TryRecvError::Empty) => {}
        Ok(msg) => {
            let server_msg: ServerMessage =
                rmp_serde::from_slice(&msg.bytes).expect("Failed to deserialize broadcast");
            panic!("Expected no respawn broadcast, got {:?}", server_msg);
        }
        Err(err) => panic!("Expected empty broadcast channel, got {:?}", err),
    }
}

#[tokio::test]
async fn respawn_player_ignores_alive_player() {
    let game_state = make_test_game_state("respawn_alive");

    let player = Player {
        id: pid("player_alive"),
        name: "AlivePlayer".to_string(),
        position: Position {
            x: 5.0,
            y: 0.0,
            z: 6.0,
        },
        rotation: 0.75,
        level: 2,
        health: 18,
        max_health: 20,
        class: CharacterClass::Knight,
        gender: Gender::default(),
        is_official_npc: false,
        torch_on: false,
        floor_level: 0,
        object_type: None,
        object_id: None,
        last_combat_at: 0,
        client_kind: Default::default(),
    };
    let player_id = player.id;
    game_state.add_player(player).await;

    let mut rx = game_state.subscribe();
    game_state.respawn_player(&player_id).await;

    let players = game_state.get_all_players().await;
    let unchanged = players
        .get(&player_id)
        .expect("Player should still exist after ignored respawn");
    assert_eq!(unchanged.health, 18);
    assert_eq!(unchanged.position.x, 5.0);
    assert_eq!(unchanged.position.y, 0.0);
    assert_eq!(unchanged.position.z, 6.0);
    assert_eq!(unchanged.rotation, 0.75);

    match rx.try_recv() {
        Err(TryRecvError::Empty) => {}
        Ok(msg) => {
            let server_msg: ServerMessage =
                rmp_serde::from_slice(&msg.bytes).expect("Failed to deserialize broadcast");
            panic!(
                "Expected no broadcast for alive respawn, got {:?}",
                server_msg
            );
        }
        Err(err) => panic!("Expected empty channel, got {:?}", err),
    }
}

#[tokio::test]
async fn chat_uses_direct_spatial_fanout_instead_of_global_broadcast() {
    let game_state = make_test_game_state("chat_spatial_fanout");
    let auth = make_test_auth("chat_spatial_fanout");
    let speaker_id = pid("speaker");
    let near_listener_id = pid("near_listener");
    let far_listener_id = pid("far_listener");

    game_state
        .add_player(make_player("speaker", 0.0, 0.0))
        .await;
    game_state
        .add_player(make_player("near_listener", 10.0, 0.0))
        .await;
    game_state
        .add_player(make_player("far_listener", 100.0, 0.0))
        .await;

    let mut speaker_rx = game_state.register_direct_channel(&speaker_id).await;
    let mut near_rx = game_state.register_direct_channel(&near_listener_id).await;
    let mut far_rx = game_state.register_direct_channel(&far_listener_id).await;

    let mut broadcast_rx = game_state.subscribe();
    game_state
        .send_chat_message(&speaker_id, "hello".to_string(), &auth)
        .await;

    match speaker_rx.try_recv() {
        Ok(ServerMessage::ChatMessage { player_id, message }) => {
            assert_eq!(player_id, pid("speaker"));
            assert_eq!(message, "hello");
        }
        other => panic!("Expected direct chat for speaker, got {:?}", other),
    }

    match near_rx.try_recv() {
        Ok(ServerMessage::ChatMessage { player_id, message }) => {
            assert_eq!(player_id, pid("speaker"));
            assert_eq!(message, "hello");
        }
        other => panic!("Expected direct chat for nearby listener, got {:?}", other),
    }

    match far_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Expected no direct chat for far listener, got {:?}", other),
    }

    match broadcast_rx.try_recv() {
        Err(TryRecvError::Empty) => {}
        Ok(msg) => {
            let server_msg: ServerMessage =
                rmp_serde::from_slice(&msg.bytes).expect("Failed to deserialize broadcast");
            panic!("Expected no chat broadcast, got {:?}", server_msg);
        }
        Err(err) => panic!("Expected empty broadcast channel, got {:?}", err),
    }
}

#[tokio::test]
async fn who_command_reports_online_counts_only_to_the_requester() {
    let game_state = make_test_game_state("who_command");
    let auth = make_test_auth("who_command");
    let asker_id = pid("asker");
    let bystander_id = pid("bystander");
    let mut asker = make_player("asker", 0.0, 0.0);
    asker.client_kind = ClientKind::Web;
    game_state.add_player(asker).await;
    let mut bystander = make_player("bystander", 5.0, 0.0);
    bystander.client_kind = ClientKind::Cli;
    game_state.add_player(bystander).await;
    let mut npc = make_player("npc_karl", 10.0, 0.0);
    npc.is_official_npc = true;
    npc.client_kind = ClientKind::Cli;
    game_state.add_player(npc).await;

    let mut asker_rx = game_state.register_direct_channel(&asker_id).await;
    let mut bystander_rx = game_state.register_direct_channel(&bystander_id).await;

    game_state
        .send_chat_message(&asker_id, "/who".to_string(), &auth)
        .await;

    match asker_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert_eq!(message, "Online: 3 (1 web, 1 cli, 1 npc)");
        }
        other => panic!("Expected online count reply, got {:?}", other),
    }

    match bystander_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("/who must not be relayed as chat, got {:?}", other),
    }
}

#[test]
fn whisper_command_parses_name_and_message() {
    use super::chat::parse_whisper_command;

    assert_eq!(
        parse_whisper_command("/w Rica hello there"),
        Some(("Rica", "hello there"))
    );
    assert_eq!(
        parse_whisper_command("/whisper Rica hi"),
        Some(("Rica", "hi"))
    );
    assert_eq!(
        parse_whisper_command(" /w  Rica   hi "),
        Some(("Rica", "hi"))
    );
    assert_eq!(parse_whisper_command("/w Rica"), Some(("Rica", "")));
    assert_eq!(parse_whisper_command("/w"), Some(("", "")));
    assert_eq!(parse_whisper_command("/who"), None);
    assert_eq!(parse_whisper_command("hello /w Rica"), None);
}

#[tokio::test]
async fn whisper_reaches_only_the_target_regardless_of_distance() {
    let game_state = make_test_game_state("whisper_delivery");
    let auth = make_test_auth("whisper_delivery");
    let sender_id = pid("sender");
    let target_id = pid("far_target");
    let bystander_id = pid("bystander");
    game_state.add_player(make_player("sender", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("far_target", 500.0, 0.0))
        .await;
    game_state
        .add_player(make_player("bystander", 5.0, 0.0))
        .await;

    let mut sender_rx = game_state.register_direct_channel(&sender_id).await;
    let mut target_rx = game_state.register_direct_channel(&target_id).await;
    let mut bystander_rx = game_state.register_direct_channel(&bystander_id).await;
    let mut broadcast_rx = game_state.subscribe();

    // Mixed case on purpose: the name must match case-insensitively and the
    // delivered `to` must carry the canonical spelling.
    game_state
        .send_chat_message(&sender_id, "/w Far_Target psst".to_string(), &auth)
        .await;

    for rx in [&mut target_rx, &mut sender_rx] {
        match rx.try_recv() {
            Ok(ServerMessage::WhisperMessage { from, to, message }) => {
                assert_eq!(from, "sender");
                assert_eq!(to, "far_target");
                assert_eq!(message, "psst");
            }
            other => panic!("Expected whisper delivery, got {:?}", other),
        }
    }

    match bystander_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Whisper must not reach bystanders, got {:?}", other),
    }

    match broadcast_rx.try_recv() {
        Err(TryRecvError::Empty) => {}
        Ok(msg) => {
            let server_msg: ServerMessage =
                rmp_serde::from_slice(&msg.bytes).expect("Failed to deserialize broadcast");
            panic!("Expected no whisper broadcast, got {:?}", server_msg);
        }
        Err(err) => panic!("Expected empty broadcast channel, got {:?}", err),
    }
}

#[tokio::test]
async fn whisper_never_falls_back_to_a_case_variant_of_another_player() {
    let game_state = make_test_game_state("whisper_case_variants");
    let auth = make_test_auth("whisper_case_variants");
    let sender_id = pid("sender");
    // Character names are UNIQUE only case-sensitively, so both can be online.
    let lower_id = pid("rica");
    let upper_id = pid("Rica");
    game_state.add_player(make_player("sender", 0.0, 0.0)).await;
    game_state.add_player(make_player("rica", 5.0, 0.0)).await;
    game_state.add_player(make_player("Rica", 10.0, 0.0)).await;

    let mut sender_rx = game_state.register_direct_channel(&sender_id).await;
    let mut lower_rx = game_state.register_direct_channel(&lower_id).await;
    let mut upper_rx = game_state.register_direct_channel(&upper_id).await;

    // Exact spelling goes to exactly that player, never the variant.
    game_state
        .send_chat_message(&sender_id, "/w rica psst".to_string(), &auth)
        .await;
    match lower_rx.try_recv() {
        Ok(ServerMessage::WhisperMessage { to, .. }) => assert_eq!(to, "rica"),
        other => panic!("Expected whisper for exact match, got {:?}", other),
    }
    match upper_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Case variant must not receive the whisper, got {:?}", other),
    }
    sender_rx.try_recv().expect("sender echo");

    // A spelling that matches neither exactly is ambiguous — refuse rather
    // than pick one.
    game_state
        .send_chat_message(&sender_id, "/w RICA psst".to_string(), &auth)
        .await;
    match sender_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => assert_eq!(
            message,
            "Whisper: several players match RICA; spell the name exactly."
        ),
        other => panic!("Expected ambiguity reply, got {:?}", other),
    }
    for rx in [&mut lower_rx, &mut upper_rx] {
        match rx.try_recv() {
            Err(MpscTryRecvError::Empty) => {}
            other => panic!("Ambiguous whisper must not be delivered, got {:?}", other),
        }
    }
}

#[tokio::test]
async fn whisper_errors_go_only_to_the_sender() {
    let game_state = make_test_game_state("whisper_errors");
    let auth = make_test_auth("whisper_errors");
    let sender_id = pid("sender");
    let listener_id = pid("listener");
    game_state.add_player(make_player("sender", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("listener", 5.0, 0.0))
        .await;

    let mut sender_rx = game_state.register_direct_channel(&sender_id).await;
    let mut listener_rx = game_state.register_direct_channel(&listener_id).await;

    let expect_reply =
        |result: Result<ServerMessage, MpscTryRecvError>, expected: &str| match result {
            Ok(ServerMessage::SystemMessage { message }) => {
                assert_eq!(message, expected);
            }
            other => panic!("Expected whisper error reply, got {:?}", other),
        };

    game_state
        .send_chat_message(&sender_id, "/w Nobody hi".to_string(), &auth)
        .await;
    expect_reply(
        sender_rx.try_recv(),
        "Whisper: no one called Nobody is here.",
    );

    game_state
        .send_chat_message(&sender_id, "/w listener".to_string(), &auth)
        .await;
    expect_reply(sender_rx.try_recv(), "Whisper: /w <name> <message>");

    game_state
        .send_chat_message(&sender_id, "/w sender hi".to_string(), &auth)
        .await;
    expect_reply(sender_rx.try_recv(), "Whisper: that's you.");

    match listener_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!(
            "Whisper errors must not leak into local chat, got {:?}",
            other
        ),
    }
}

#[test]
fn block_command_parses_actions() {
    use super::chat::{parse_block_command, BlockCommand};

    assert_eq!(parse_block_command("/block"), Some(BlockCommand::List));
    assert_eq!(parse_block_command(" /block  "), Some(BlockCommand::List));
    assert_eq!(
        parse_block_command("/block Rica"),
        Some(BlockCommand::Block("Rica"))
    );
    assert_eq!(
        parse_block_command("/unblock Rica"),
        Some(BlockCommand::Unblock("Rica"))
    );
    assert_eq!(
        parse_block_command("/unblock"),
        Some(BlockCommand::Unblock(""))
    );
    assert_eq!(parse_block_command("/blocked"), None);
    assert_eq!(parse_block_command("hello /block Rica"), None);
}

#[tokio::test]
async fn blocked_players_chat_skips_the_blocker_only() {
    let game_state = make_test_game_state("block_chat_filter");
    let auth = make_test_auth("block_chat_filter");
    let speaker_id = pid("speaker");
    let blocker_id = pid("blocker");
    let listener_id = pid("listener");
    game_state
        .add_player(make_player("speaker", 0.0, 0.0))
        .await;
    game_state
        .add_player(make_player("blocker", 5.0, 0.0))
        .await;
    game_state
        .add_player(make_player("listener", 10.0, 0.0))
        .await;

    let mut blocker_rx = game_state.register_direct_channel(&blocker_id).await;
    let mut listener_rx = game_state.register_direct_channel(&listener_id).await;

    game_state
        .set_player_blocks(&blocker_id, vec!["speaker".to_string()])
        .await;
    game_state
        .send_chat_message(&speaker_id, "hello".to_string(), &auth)
        .await;

    match blocker_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Blocked chat must not reach the blocker, got {:?}", other),
    }
    match listener_rx.try_recv() {
        Ok(ServerMessage::ChatMessage { player_id, .. }) => assert_eq!(player_id, speaker_id),
        other => panic!("Bystander must still get the chat, got {:?}", other),
    }
}

#[tokio::test]
async fn blocked_senders_whisper_is_suppressed_but_still_echoed() {
    let game_state = make_test_game_state("block_whisper_filter");
    let auth = make_test_auth("block_whisper_filter");
    let sender_id = pid("sender");
    let blocker_id = pid("blocker");
    game_state.add_player(make_player("sender", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("blocker", 5.0, 0.0))
        .await;

    let mut sender_rx = game_state.register_direct_channel(&sender_id).await;
    let mut blocker_rx = game_state.register_direct_channel(&blocker_id).await;

    game_state
        .set_player_blocks(&blocker_id, vec!["sender".to_string()])
        .await;
    game_state
        .send_chat_message(&sender_id, "/w blocker psst".to_string(), &auth)
        .await;

    match blocker_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Whisper must not reach the blocker, got {:?}", other),
    }
    // The echo hides the block from the sender.
    match sender_rx.try_recv() {
        Ok(ServerMessage::WhisperMessage { from, to, message }) => {
            assert_eq!(from, "sender");
            assert_eq!(to, "blocker");
            assert_eq!(message, "psst");
        }
        other => panic!("Sender must still get the echo, got {:?}", other),
    }
}

#[tokio::test]
async fn block_command_persists_resolves_case_and_unblocks() {
    let auth = make_test_auth("block_command_flow");
    let account = auth.login_npc("npc_block_cmd_test").unwrap();
    let blocker_char = create_test_character(&auth, &account, "Blocker");
    let abuser_char = create_test_character(&auth, &account, "Abuser");

    let game_state = make_test_game_state("block_command_flow");
    let blocker_id = pid("Blocker");
    let abuser_id = pid("Abuser");
    game_state
        .add_player(make_player("Blocker", 0.0, 0.0))
        .await;
    game_state.add_player(make_player("Abuser", 5.0, 0.0)).await;
    game_state
        .register_player_character(&blocker_id, blocker_char.id, 0, attrs_with_cha(12), 0)
        .await;
    game_state
        .register_player_character(&abuser_id, abuser_char.id, 0, attrs_with_cha(12), 0)
        .await;

    let mut blocker_rx = game_state.register_direct_channel(&blocker_id).await;

    let expect_reply =
        |result: Result<ServerMessage, MpscTryRecvError>, expected: &str| match result {
            Ok(ServerMessage::SystemMessage { message }) => assert_eq!(message, expected),
            other => panic!("Expected block reply, got {:?}", other),
        };
    let command = |text: &str| game_state.send_chat_message(&blocker_id, text.to_string(), &auth);

    // Lowercase on purpose: resolution is case-insensitive when unambiguous.
    command("/block abuser").await;
    expect_reply(
        blocker_rx.try_recv(),
        "Block: Abuser is blocked; their chat and whispers are hidden. /unblock Abuser undoes this.",
    );
    assert_eq!(
        auth.load_blocked_names(blocker_char.id).unwrap(),
        vec!["Abuser".to_string()]
    );

    game_state
        .send_chat_message(&abuser_id, "spam".to_string(), &auth)
        .await;
    match blocker_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Blocked chat must not be delivered, got {:?}", other),
    }

    command("/block").await;
    expect_reply(blocker_rx.try_recv(), "Blocked: Abuser");

    command("/unblock ABUSER").await;
    expect_reply(
        blocker_rx.try_recv(),
        "Unblock: Abuser is no longer blocked.",
    );
    assert!(auth.load_blocked_names(blocker_char.id).unwrap().is_empty());

    game_state
        .send_chat_message(&abuser_id, "hello again".to_string(), &auth)
        .await;
    match blocker_rx.try_recv() {
        Ok(ServerMessage::ChatMessage { player_id, .. }) => assert_eq!(player_id, abuser_id),
        other => panic!("Chat must flow again after unblock, got {:?}", other),
    }

    // Guard rails: no self-block, no unknown names.
    command("/block Blocker").await;
    expect_reply(blocker_rx.try_recv(), "Block: that's you.");
    command("/block Nobody").await;
    expect_reply(
        blocker_rx.try_recv(),
        "Block: no character named Nobody; spell the name exactly.",
    );
}

#[test]
fn notice_command_sets_or_clears_message() {
    use super::parse_notice_command;

    assert_eq!(
        parse_notice_command("/notice Server maintenance"),
        Some(Some("Server maintenance"))
    );
    assert_eq!(parse_notice_command(" /notice "), Some(None));
    assert_eq!(parse_notice_command("/noticeboard"), None);
}

#[tokio::test]
async fn server_notice_is_stored_broadcast_and_clearable() {
    let game_state = make_test_game_state("server_notice");
    let auth = make_test_auth("server_notice");
    let player_id = pid("notice_admin");
    let mut receiver = game_state.subscribe();

    game_state
        .send_chat_message(&player_id, "/notice Server maintenance".to_string(), &auth)
        .await;
    assert_eq!(
        game_state.server_notice().await.as_deref(),
        Some("Server maintenance")
    );
    let message = receiver.recv().await.expect("notice broadcast");
    assert!(matches!(
        rmp_serde::from_slice::<ServerMessage>(&message.bytes),
        Ok(ServerMessage::ServerNotice {
            message: Some(message)
        }) if message == "Server maintenance"
    ));

    game_state
        .send_chat_message(&player_id, "/notice".to_string(), &auth)
        .await;
    assert_eq!(game_state.server_notice().await, None);
    let message = receiver.recv().await.expect("notice clear broadcast");
    assert!(matches!(
        rmp_serde::from_slice::<ServerMessage>(&message.bytes),
        Ok(ServerMessage::ServerNotice { message: None })
    ));
}

#[tokio::test]
async fn escape_command_returns_a_stuck_player_to_spawn() {
    let game_state = make_test_game_state("escape_to_spawn");
    let auth = make_test_auth("escape_to_spawn");
    let stuck_id = pid("stuck");
    let bystander_id = pid("bystander");
    game_state.add_player(make_player("stuck", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("bystander", 5.0, 0.0))
        .await;
    let mut bystander_rx = game_state.register_direct_channel(&bystander_id).await;

    game_state
        .send_chat_message(&stuck_id, "/escape".to_string(), &auth)
        .await;

    let spawn = &world_config().spawn_position;
    let (position, _, floor_level) = game_state
        .get_player_position(&stuck_id)
        .await
        .expect("player should still exist");
    // Approximate: the teleport runs the X through `wrap_world_x`, which is a
    // no-op away from the seam but not bit-exact.
    let expected = spawn.position();
    assert!(
        (position.x - expected.x).abs() < 0.01
            && (position.y - expected.y).abs() < 0.01
            && (position.z - expected.z).abs() < 0.01,
        "expected spawn {expected:?}, got {position:?}"
    );
    // Escaping a dungeon has to land on the surface, not carry its floor along.
    assert_eq!(floor_level, 0);

    // The command is consumed, never relayed as chat to anyone nearby. The
    // bystander does still get the movement traffic the teleport generates.
    while let Ok(msg) = bystander_rx.try_recv() {
        assert!(
            !matches!(
                msg,
                ServerMessage::ChatMessage { .. } | ServerMessage::SystemMessage { .. }
            ),
            "/escape must not be echoed as chat: {msg:?}"
        );
    }
}

#[tokio::test]
async fn escape_command_refused_while_in_combat() {
    let game_state = make_test_game_state("escape_in_combat");
    let auth = make_test_auth("escape_in_combat");
    let fighter_id = pid("fighter");
    game_state
        .add_player(make_player("fighter", 20.0, 30.0))
        .await;
    {
        let mut players = game_state.players.write().await;
        players.get_mut(&fighter_id).unwrap().last_combat_at = GameState::now_ms();
    }

    game_state
        .send_chat_message(&fighter_id, "/escape".to_string(), &auth)
        .await;

    let (position, _, _) = game_state
        .get_player_position(&fighter_id)
        .await
        .expect("player should still exist");
    assert_eq!(
        (position.x, position.z),
        (20.0, 30.0),
        "/escape must not double as a combat disengage"
    );
}

#[tokio::test]
async fn player_aoi_crosses_world_x_seam() {
    let game_state = make_test_game_state("player_aoi_x_wrap");
    let east_id = pid("east_player");
    let west_id = pid("west_player");

    game_state
        .add_player(make_player(
            "east_player",
            onlinerpg_shared::WORLD_MAX_X - 1.0,
            0.0,
        ))
        .await;
    game_state
        .add_player(make_player(
            "west_player",
            onlinerpg_shared::WORLD_MIN_X + 1.0,
            0.0,
        ))
        .await;

    let nearby = game_state
        .player_ids_within(&east_id, onlinerpg_shared::NPC_SIGHT_RADIUS)
        .await;
    assert!(nearby.contains(&east_id));
    assert!(nearby.contains(&west_id));
}

#[tokio::test]
async fn movement_into_aoi_sends_existing_monsters_and_ground_items() {
    let game_state = make_test_game_state("movement_world_entity_aoi");
    let player_id = pid("walker");
    let entity_position = Position {
        x: 50.0,
        y: 0.0,
        z: 0.0,
    };

    game_state.add_player(make_player("walker", 0.0, 0.0)).await;
    let mut direct_rx = game_state.register_direct_channel(&player_id).await;

    {
        let mut monsters = game_state.monsters.write().await;
        monsters.insert(
            "monster_a".to_string(),
            make_monster("monster_a", entity_position, 0),
        );
    }

    {
        let mut ground_items = game_state.ground_items.write().await;
        ground_items.insert(
            42,
            ServerGroundItem {
                item: GroundItem {
                    instance_id: 42,
                    item_def_id: "test_item".to_string(),
                    position: entity_position,
                    floor_level: 0,
                    enchant: 0,
                },
                dropped_at_ms: 0,
            },
        );
    }

    game_state
        .update_player_position(&player_id, move_cmd(entity_position, false), false, false)
        .await;
    game_state.tick_player_movement(60.0).await;

    match direct_rx.try_recv() {
        Ok(ServerMessage::MonsterSpawned { monster }) => {
            assert_eq!(monster.id, "monster_a");
        }
        other => panic!("Expected MonsterSpawned when entering AOI, got {:?}", other),
    }

    match direct_rx.try_recv() {
        Ok(ServerMessage::GroundItemAppeared { item }) => {
            assert_eq!(item.instance_id, 42);
        }
        other => panic!(
            "Expected GroundItemAppeared when entering AOI, got {:?}",
            other
        ),
    }

    match direct_rx.try_recv() {
        Ok(ServerMessage::PlayerMoved {
            player_id: moved_id,
            ..
        }) => {
            assert_eq!(moved_id, player_id);
        }
        other => panic!(
            "Expected self PlayerMoved after AOI snapshot, got {:?}",
            other
        ),
    }
}

#[tokio::test]
async fn player_movement_wraps_across_east_world_edge() {
    let game_state = make_test_game_state("movement_x_wrap");
    let player_id = pid("world_wrap_walker");
    game_state
        .add_player(make_player(
            "world_wrap_walker",
            onlinerpg_shared::WORLD_MAX_X - 0.25,
            0.0,
        ))
        .await;
    let mut direct_rx = game_state.register_direct_channel(&player_id).await;

    game_state
        .update_player_position(
            &player_id,
            MoveCommand {
                position: Position {
                    x: onlinerpg_shared::WORLD_MAX_X + 0.25,
                    y: 12.0,
                    z: 3.0,
                },
                rotation: 0.5,
                floor_level: 0,
                append: false,
            },
            false,
            false,
        )
        .await;
    game_state.tick_player_movement(60.0).await;

    let players = game_state.get_all_players().await;
    let wrapped = &players[&player_id];
    assert_eq!(wrapped.position.x, onlinerpg_shared::WORLD_MIN_X + 0.25);

    match direct_rx.try_recv() {
        Ok(ServerMessage::PlayerMoved { position, .. }) => {
            assert_eq!(position.x, onlinerpg_shared::WORLD_MIN_X + 0.25);
        }
        other => panic!("Expected wrapped self PlayerMoved, got {other:?}"),
    }
}

#[tokio::test]
async fn seam_crossing_movement_checks_destination_edge_collision() {
    let game_state = make_test_game_state("movement_seam_collision");
    let player_id = pid("seam_walker");
    game_state
        .add_player(make_player(
            "seam_walker",
            onlinerpg_shared::WORLD_MAX_X - 0.5,
            5.5,
        ))
        .await;
    game_state.sync_region_furniture(
        -16,
        0,
        &[table_placement(onlinerpg_shared::WORLD_MIN_X + 0.5, 5.5)],
    );

    game_state
        .update_player_position(
            &player_id,
            move_cmd(
                Position {
                    x: onlinerpg_shared::WORLD_MIN_X + 1.5,
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

    assert_eq!(
        player_xz(&game_state, &player_id).await,
        (onlinerpg_shared::WORLD_MAX_X - 0.5, 5.5)
    );
}

async fn player_x(game_state: &GameState, player_id: &PlayerId) -> f32 {
    game_state.get_all_players().await[player_id].position.x
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
    }
}

#[tokio::test]
async fn server_caps_player_movement_speed() {
    let game_state = make_test_game_state("movement_speed_cap");
    let player_id = pid("runner");
    game_state.add_player(make_player("runner", 0.0, 0.0)).await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(50.0), false), false, false)
        .await;

    assert_eq!(player_x(&game_state, &player_id).await, 0.0);

    game_state.tick_player_movement(1.0).await;
    let after_one_second = player_x(&game_state, &player_id).await;
    assert!(after_one_second > 2.0 && after_one_second < 4.0);

    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 50.0);
}

#[tokio::test]
async fn one_tick_budget_spans_queued_legs() {
    let game_state = make_test_game_state("movement_queue_budget");
    let player_id = pid("pathwalker");
    game_state
        .add_player(make_player("pathwalker", 0.0, 0.0))
        .await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(1.0), false), false, false)
        .await;
    for x in [2.0, 3.0] {
        game_state
            .update_player_position(&player_id, move_cmd(pos(x), true), false, false)
            .await;
    }

    // Budget ≈ 1.98m: leg 1 consumed whole, leg 2 partially — the cap holds
    // across legs, not per leg.
    game_state.tick_player_movement(0.5).await;
    let mid = player_x(&game_state, &player_id).await;
    assert!(mid > 1.5 && mid < 2.1, "mid was {mid}");

    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 3.0);
}

#[tokio::test]
async fn append_distance_guard_measures_from_queue_tail() {
    let game_state = make_test_game_state("movement_queue_tail_guard");
    let player_id = pid("longhauler");
    game_state
        .add_player(make_player("longhauler", 0.0, 0.0))
        .await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(50.0), false), false, false)
        .await;
    // 100 is >60m from the player but only 50m from the queue tail: accepted.
    game_state
        .update_player_position(&player_id, move_cmd(pos(100.0), true), false, false)
        .await;
    // 70m from the new tail: rejected.
    game_state
        .update_player_position(&player_id, move_cmd(pos(170.0), true), false, false)
        .await;

    game_state.tick_player_movement(600.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 100.0);
}

#[tokio::test]
async fn replace_drops_queued_waypoints() {
    let game_state = make_test_game_state("movement_queue_replace");
    let player_id = pid("rerouter");
    game_state
        .add_player(make_player("rerouter", 0.0, 0.0))
        .await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(10.0), false), false, false)
        .await;
    game_state
        .update_player_position(&player_id, move_cmd(pos(20.0), true), false, false)
        .await;
    game_state
        .update_player_position(&player_id, move_cmd(pos(5.0), false), false, false)
        .await;

    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 5.0);
}

#[tokio::test]
async fn full_waypoint_queue_drops_oldest_leg() {
    let game_state = make_test_game_state("movement_queue_cap");
    let player_id = pid("spammer");
    game_state
        .add_player(make_player("spammer", 0.0, 0.0))
        .await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(1.0), false), false, false)
        .await;
    for i in 2..=40 {
        game_state
            .update_player_position(&player_id, move_cmd(pos(i as f32), true), false, false)
            .await;
    }

    // Overflow evicts from the front, so the tail survives and the sim still
    // reaches the client's final position (a reject-newest policy would strand
    // the player at 32).
    game_state.tick_player_movement(600.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 40.0);
}

#[tokio::test]
async fn non_finite_move_is_rejected() {
    let game_state = make_test_game_state("movement_nan_reject");
    let player_id = pid("glitcher");
    game_state
        .add_player(make_player("glitcher", 1.0, 1.0))
        .await;

    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        game_state
            .update_player_position(&player_id, move_cmd(pos(bad), false), false, false)
            .await;
    }
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 1.0);
}

#[tokio::test]
async fn positive_floor_above_limit_is_rejected() {
    let game_state = make_test_game_state("positive_floor_limit");
    let player_id = pid("ghost");
    let observer_id = pid("observer");
    let max_floor = onlinerpg_shared::housing::MAX_FLOOR_LEVEL as i8;
    game_state.add_player(make_player("ghost", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("observer", 1.0, 0.0))
        .await;
    let mut observer_rx = game_state.register_direct_channel(&observer_id).await;

    for invalid_floor in [max_floor + 1, 120] {
        game_state
            .update_player_floor(&player_id, invalid_floor)
            .await;
        assert_eq!(
            game_state.players.read().await[&player_id].floor_level,
            0,
            "floor {invalid_floor} must be rejected"
        );
        assert!(matches!(
            observer_rx.try_recv(),
            Err(MpscTryRecvError::Empty)
        ));
    }

    game_state.update_player_floor(&player_id, max_floor).await;
    assert_eq!(
        game_state.players.read().await[&player_id].floor_level,
        max_floor
    );
}

#[tokio::test]
async fn player_move_cannot_bypass_positive_floor_limit() {
    let game_state = make_test_game_state("positive_floor_move_limit");
    let player_id = pid("ghost");
    let observer_id = pid("observer");
    let max_floor = onlinerpg_shared::housing::MAX_FLOOR_LEVEL as i8;
    game_state.add_player(make_player("ghost", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("observer", 1.0, 0.0))
        .await;
    let mut observer_rx = game_state.register_direct_channel(&observer_id).await;

    game_state
        .update_player_position(
            &player_id,
            MoveCommand {
                position: pos(1.0),
                rotation: 0.0,
                floor_level: max_floor + 1,
                append: false,
            },
            false,
            false,
        )
        .await;

    assert!(
        !game_state
            .movement_intents
            .read()
            .await
            .contains_key(&player_id),
        "an invalid floor move must not enqueue an intent"
    );
    let player = game_state.players.read().await[&player_id].clone();
    assert_eq!(player.position.x, 0.0);
    assert_eq!(player.floor_level, 0);
    assert!(matches!(
        observer_rx.try_recv(),
        Err(MpscTryRecvError::Empty)
    ));

    game_state
        .update_player_position(
            &player_id,
            MoveCommand {
                position: pos(1.0),
                rotation: 0.0,
                floor_level: max_floor,
                append: false,
            },
            false,
            false,
        )
        .await;
    game_state.tick_player_movement(1.0).await;
    let player = game_state.players.read().await[&player_id].clone();
    assert_eq!(player.position.x, 1.0);
    assert_eq!(player.floor_level, max_floor);
}

#[tokio::test]
async fn a_rejected_floor_change_snaps_the_client_back() {
    let game_state = make_test_game_state("positive_floor_correction");
    let player_id = pid("ghost");
    let max_floor = onlinerpg_shared::housing::MAX_FLOOR_LEVEL as i8;
    game_state.add_player(make_player("ghost", 2.0, 3.0)).await;
    let mut rx = game_state.register_direct_channel(&player_id).await;

    game_state
        .update_player_floor(&player_id, max_floor + 1)
        .await;
    let (position, _, floor_level) =
        first_correction(&mut rx).expect("a rejected floor must snap the client back");
    assert_eq!(floor_level, 0);
    assert_eq!((position.x, position.z), (2.0, 3.0));

    // The snap rides the refused-move throttle: a client that keeps reporting
    // the bad floor is not yanked once per packet.
    game_state
        .update_player_position(
            &player_id,
            MoveCommand {
                floor_level: max_floor + 1,
                ..move_cmd(pos(1.0), false)
            },
            false,
            false,
        )
        .await;
    assert!(first_correction(&mut rx).is_none());
}

#[tokio::test]
async fn a_rejected_move_floor_snaps_the_client_back() {
    let game_state = make_test_game_state("positive_floor_move_correction");
    let player_id = pid("ghost");
    let max_floor = onlinerpg_shared::housing::MAX_FLOOR_LEVEL as i8;
    game_state.add_player(make_player("ghost", 2.0, 3.0)).await;
    let mut rx = game_state.register_direct_channel(&player_id).await;

    game_state
        .update_player_position(
            &player_id,
            MoveCommand {
                floor_level: max_floor + 1,
                ..move_cmd(pos(1.0), false)
            },
            false,
            false,
        )
        .await;
    let (position, _, floor_level) =
        first_correction(&mut rx).expect("a rejected move floor must snap the client back");
    assert_eq!(floor_level, 0);
    assert_eq!((position.x, position.z), (2.0, 3.0));
}

#[test]
fn restored_floor_falls_back_to_surface() {
    let max_floor = onlinerpg_shared::housing::MAX_FLOOR_LEVEL as i8;

    for saved in [max_floor + 1, 120, i8::MAX] {
        assert_eq!(
            super::restored_floor_level(saved),
            0,
            "floor {saved} must fall back to the surface"
        );
    }

    // Negative floors use the existing dungeon rehydration path.
    for saved in [i8::MIN, -1, 0, max_floor] {
        assert_eq!(
            super::restored_floor_level(saved),
            saved,
            "floor {saved} must be restored as-is"
        );
    }
}

#[tokio::test]
async fn far_move_target_is_rejected() {
    let game_state = make_test_game_state("movement_far_reject");
    let player_id = pid("warper");
    game_state.add_player(make_player("warper", 0.0, 0.0)).await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(100.0), false), false, false)
        .await;
    game_state.tick_player_movement(600.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 0.0);
}

#[tokio::test]
async fn admin_move_applies_immediately() {
    let game_state = make_test_game_state("movement_admin_bypass");
    let player_id = pid("gm");
    game_state.add_player(make_player("gm", 0.0, 0.0)).await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(100.0), false), true, false)
        .await;
    assert_eq!(player_x(&game_state, &player_id).await, 100.0);
}

#[tokio::test]
async fn teleport_clears_pending_move_intent() {
    let game_state = make_test_game_state("movement_teleport_clears");
    let player_id = pid("traveler");
    game_state
        .add_player(make_player("traveler", 0.0, 0.0))
        .await;

    game_state
        .update_player_position(&player_id, move_cmd(pos(50.0), false), false, false)
        .await;
    game_state
        .teleport_player(
            &player_id,
            Position {
                x: 5.0,
                y: 0.0,
                z: 5.0,
            },
            0.0,
            0,
        )
        .await;
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_x(&game_state, &player_id).await, 5.0);
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
    assert!(
        game_state
            .validate_spawn_request(&player_id, "goblin", &valid, 1.5)
            .await
    );

    for (label, position) in non_finite_positions(valid) {
        assert!(
            !game_state
                .validate_spawn_request(&player_id, "goblin", &position, 1.5)
                .await,
            "position {label}"
        );
    }
    for (rotation, value_name) in NON_FINITE {
        assert!(
            !game_state
                .validate_spawn_request(&player_id, "goblin", &valid, rotation)
                .await,
            "rotation {value_name}"
        );
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
    // attacker gets no PlayerAttacked echo.
    let health = game_state
        .monsters
        .read()
        .await
        .get("dungeon_monster")
        .map(|m| m.health)
        .unwrap();
    assert_eq!(health, 10, "cross-floor attack must not damage the monster");
    match guard_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Expected no attack echo across floors, got {:?}", other),
    }
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
    match attacker_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Expected no rejected attack echo, got {other:?}"),
    }
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
    match attacker_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("Expected no attack event beyond 10m, got {other:?}"),
    }
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

    match attacker_rx.try_recv() {
        Err(MpscTryRecvError::Empty) => {}
        other => panic!("a dead player's attack must be dropped, got {other:?}"),
    }
    assert_eq!(
        game_state.monsters.read().await["nearby_monster"].health,
        10,
        "a dead player's attack must deal no damage"
    );
}

#[tokio::test]
async fn pickup_broadcasts_the_pickup_animation() {
    let game_state = make_test_game_state("pickup_anim_broadcast");
    game_state.add_player(make_player("picker", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("watcher", 2.0, 0.0))
        .await;
    {
        let mut inventories = game_state.inventories.write().await;
        inventories.insert(pid("picker"), Default::default());
    }
    {
        let mut ground_items = game_state.ground_items.write().await;
        ground_items.insert(
            42,
            ServerGroundItem {
                item: GroundItem {
                    instance_id: 42,
                    item_def_id: "test_item".to_string(),
                    position: Position {
                        x: 0.5,
                        y: 0.0,
                        z: 0.0,
                    },
                    floor_level: 0,
                    enchant: 0,
                },
                dropped_at_ms: 0,
            },
        );
    }
    let mut watcher_rx = game_state.register_direct_channel(&pid("watcher")).await;
    let mut picker_rx = game_state.register_direct_channel(&pid("picker")).await;

    // Driven by PickupStarted at the clip's first frame, not by the pickup
    // itself — which lands a third of a clip later, at the grab moment.
    game_state.broadcast_pickup_animation(&pid("picker")).await;

    let mut saw_animation = false;
    while let Ok(msg) = watcher_rx.try_recv() {
        if let ServerMessage::PlayerInteractionChanged {
            player_id,
            object_type,
        } = msg
        {
            assert_eq!(player_id, pid("picker"));
            assert_eq!(object_type.as_deref(), Some("pickup"));
            saw_animation = true;
        }
    }
    assert!(
        saw_animation,
        "nearby players must see the pickup animation"
    );

    // The picker already plays it locally, so it is excluded from the fan-out.
    while let Ok(msg) = picker_rx.try_recv() {
        assert!(
            !matches!(msg, ServerMessage::PlayerInteractionChanged { .. }),
            "the picker must not receive its own pickup broadcast"
        );
    }

    // The pickup itself no longer carries the animation.
    game_state.pickup_item(&pid("picker"), 42).await;
    while let Ok(msg) = watcher_rx.try_recv() {
        assert!(
            !matches!(msg, ServerMessage::PlayerInteractionChanged { .. }),
            "pickup_item must not broadcast the animation a second time"
        );
    }
}

#[tokio::test]
async fn pickup_animation_is_not_sent_beyond_the_delivery_radius() {
    let game_state = make_test_game_state("pickup_anim_radius");
    game_state.add_player(make_player("picker", 0.0, 0.0)).await;
    let far = super::EVENT_DELIVERY_RADIUS + 10.0;
    game_state
        .add_player(make_player("distant", far, 0.0))
        .await;
    let mut distant_rx = game_state.register_direct_channel(&pid("distant")).await;

    game_state.broadcast_pickup_animation(&pid("picker")).await;

    while let Ok(msg) = distant_rx.try_recv() {
        assert!(
            !matches!(msg, ServerMessage::PlayerInteractionChanged { .. }),
            "the crouch must not reach players outside the delivery radius"
        );
    }
}

// --- Haggling (economy phase 2) ---

fn make_merchant_npc(id: &str, x: f32, z: f32) -> Player {
    let mut p = make_player(id, x, z);
    p.name = "Rica".to_string();
    p.is_official_npc = true;
    p
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

/// Spawn a merchant NPC and a buyer with the given CHA/gold next to each
/// other, returning the buyer's direct-message receiver and the NPC's.
async fn set_floor(game_state: &GameState, name: &str, floor: i8) {
    game_state
        .players
        .write()
        .await
        .get_mut(&pid(name))
        .unwrap()
        .floor_level = floor;
}

async fn setup_haggle(
    game_state: &GameState,
    cha: u8,
    gold: i64,
) -> (
    tokio::sync::mpsc::UnboundedReceiver<ServerMessage>,
    tokio::sync::mpsc::UnboundedReceiver<ServerMessage>,
) {
    game_state
        .add_player(make_merchant_npc("npc_rica", 0.0, 0.0))
        .await;
    game_state.add_player(make_player("buyer", 1.0, 0.0)).await;
    game_state
        .register_player_character(&pid("buyer"), 1, 0, attrs_with_cha(cha), gold)
        .await;
    let buyer_rx = game_state.register_direct_channel(&pid("buyer")).await;
    let npc_rx = game_state.register_direct_channel(&pid("npc_rica")).await;
    (buyer_rx, npc_rx)
}

#[test]
fn haggling_band_invariant_boundary() {
    // Rica's actual rate must satisfy the invariant; 60% is the first rate
    // where max haggled sell (60% * 1.25) meets min haggled buy (75%).
    assert!(deals::band_invariant_holds(40));
    assert!(deals::band_invariant_holds(59));
    assert!(!deals::band_invariant_holds(60));
}

#[test]
fn haggling_band_widens_with_cha_within_limits() {
    assert_eq!(deals::deal_half_band_pct(10), 10);
    assert_eq!(deals::deal_half_band_pct(3), 5);
    assert_eq!(deals::deal_half_band_pct(13), 16);
    assert_eq!(deals::deal_half_band_pct(18), 25);
    assert_eq!(deals::deal_half_band_pct(255), 25);
}

#[tokio::test]
async fn offer_deal_clamps_modifier_to_cha_band() {
    let game_state = make_test_game_state("offer_clamp");
    let (mut buyer_rx, mut npc_rx) = setup_haggle(&game_state, 10, 0).await;

    game_state
        .offer_deal(
            &pid("npc_rica"),
            &pid("buyer"),
            "iron_sword",
            DealKind::Buy,
            -50,
            "loyal customer",
        )
        .await;

    match buyer_rx.try_recv() {
        Ok(ServerMessage::DealUpdated {
            item_def_id,
            kind,
            modifier_pct,
            ..
        }) => {
            assert_eq!(item_def_id, "iron_sword");
            assert_eq!(kind, DealKind::Buy);
            assert_eq!(modifier_pct, -10, "CHA 10 band is ±10");
        }
        other => panic!("Expected DealUpdated for buyer, got {:?}", other),
    }
    match npc_rx.try_recv() {
        Ok(ServerMessage::DealResult {
            accepted,
            applied_modifier_pct,
            ..
        }) => {
            assert!(accepted);
            assert_eq!(applied_modifier_pct, -10);
        }
        other => panic!("Expected DealResult for NPC, got {:?}", other),
    }
}

#[tokio::test]
async fn cross_floor_offer_deal_is_rejected() {
    let game_state = make_test_game_state("cross_floor_offer");
    let (mut buyer_rx, mut npc_rx) = setup_haggle(&game_state, 10, 0).await;
    set_floor(&game_state, "buyer", 1).await;

    game_state
        .offer_deal(
            &pid("npc_rica"),
            &pid("buyer"),
            "iron_sword",
            DealKind::Buy,
            -50,
            "loyal customer",
        )
        .await;

    assert!(matches!(buyer_rx.try_recv(), Err(MpscTryRecvError::Empty)));
    match npc_rx.try_recv() {
        Ok(ServerMessage::DealResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("another floor"), "got: {message}");
        }
        other => panic!("Expected DealResult rejection for NPC, got {other:?}"),
    }
}

#[tokio::test]
async fn offer_deal_enforces_cooldown_and_player_budget() {
    let game_state = make_test_game_state("offer_limits");
    let (_buyer_rx, mut npc_rx) = setup_haggle(&game_state, 18, 0).await;

    // First offer: accepted (CHA 18 → band ±25, cost 2500 on iron_sword).
    game_state
        .offer_deal(
            &pid("npc_rica"),
            &pid("buyer"),
            "iron_sword",
            DealKind::Buy,
            -25,
            "first",
        )
        .await;
    match npc_rx.try_recv() {
        Ok(ServerMessage::DealResult { accepted, .. }) => assert!(accepted),
        other => panic!("Expected accepted DealResult, got {:?}", other),
    }

    // Immediate second offer: rejected by the cooldown.
    game_state
        .offer_deal(
            &pid("npc_rica"),
            &pid("buyer"),
            "dagger",
            DealKind::Buy,
            -5,
            "second",
        )
        .await;
    match npc_rx.try_recv() {
        Ok(ServerMessage::DealResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("cooldown"), "got: {message}");
        }
        other => panic!("Expected cooldown rejection, got {:?}", other),
    }

    // Cooldown lifted: the player's daily discount cap (4000) now rejects a
    // second 2500-cost discount.
    game_state.clear_deal_cooldowns_for_test().await;
    game_state
        .offer_deal(
            &pid("npc_rica"),
            &pid("buyer"),
            "iron_sword",
            DealKind::Buy,
            -25,
            "third",
        )
        .await;
    match npc_rx.try_recv() {
        Ok(ServerMessage::DealResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("discount limit"), "got: {message}");
        }
        other => panic!("Expected budget rejection, got {:?}", other),
    }
}

#[tokio::test]
async fn buy_item_applies_deal_once() {
    let game_state = make_test_game_state("buy_with_deal");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 30_000).await;
    {
        let mut inventories = game_state.inventories.write().await;
        inventories.insert(pid("buyer"), Default::default());
    }

    game_state
        .offer_deal(
            &pid("npc_rica"),
            &pid("buyer"),
            "iron_sword",
            DealKind::Buy,
            -10,
            "deal",
        )
        .await;

    // First buy uses the -10% deal: 10000 → 9000.
    game_state
        .buy_item(&pid("buyer"), &pid("npc_rica"), "iron_sword")
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 21_000);

    // The deal is single-use: the second buy pays full price.
    game_state
        .buy_item(&pid("buyer"), &pid("npc_rica"), "iron_sword")
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 11_000);
}

#[tokio::test]
async fn sell_item_applies_deal_bonus() {
    let game_state = make_test_game_state("sell_with_deal");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 18, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(onlinerpg_shared::inventory::ItemInstance {
            instance_id: 7,
            item_def_id: "iron_sword".to_string(),
            quantity: 1,
            enchant: 0,
        });
        inventories.insert(pid("buyer"), inv);
    }

    game_state
        .offer_deal(
            &pid("npc_rica"),
            &pid("buyer"),
            "iron_sword",
            DealKind::Sell,
            25,
            "today's wanted item",
        )
        .await;

    // Sell rate 40% with a +25% bonus: 10000 * 0.4 * 1.25 = 5000.
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 5_000);
}

#[tokio::test]
async fn cross_floor_shop_actions_leave_economy_state_unchanged() {
    let game_state = make_test_game_state("cross_floor_shop");
    let (mut buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    game_state.inventories.write().await.insert(
        pid("buyer"),
        PlayerInventory {
            bag: vec![bag_item(7, "iron_sword", 1), bag_item(8, "dagger", 1)],
            ..Default::default()
        },
    );

    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    let entry_id = {
        let buybacks = game_state.buybacks.read().await;
        buybacks[&(1, "Rica".to_string())][0].entry.entry_id
    };
    set_floor(&game_state, "buyer", 1).await;

    while buyer_rx.try_recv().is_ok() {}
    let gold_before = game_state.get_player_gold(&pid("buyer")).await;
    let bag_before = game_state.inventories.read().await[&pid("buyer")]
        .bag
        .clone();

    game_state
        .open_shop(&pid("buyer"), &pid("npc_rica"), true)
        .await;
    game_state
        .buy_item(&pid("buyer"), &pid("npc_rica"), "iron_sword")
        .await;
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 8)
        .await;
    game_state
        .buyback_item(&pid("buyer"), &pid("npc_rica"), entry_id)
        .await;

    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, gold_before);
    let bag_after = game_state.inventories.read().await[&pid("buyer")]
        .bag
        .clone();
    assert_eq!(bag_after, bag_before);
    assert!(
        game_state.open_shops.read().await.is_empty(),
        "a rejected shop open must not register a hold"
    );
    assert_eq!(
        game_state.buybacks.read().await[&(1, "Rica".to_string())].len(),
        1,
        "a rejected buyback must remain available"
    );
    for _ in 0..4 {
        match buyer_rx.try_recv() {
            Ok(ServerMessage::TradeError { message }) => {
                assert!(message.contains("another floor"), "got: {message}")
            }
            other => panic!("Expected cross-floor TradeError, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn sell_to_merchant_records_buyback_and_restores_item() {
    let game_state = make_test_game_state("buyback_roundtrip");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(ItemInstance {
            instance_id: 7,
            item_def_id: "iron_sword".to_string(),
            quantity: 1,
            enchant: 2,
        });
        inventories.insert(pid("buyer"), inv);
    }

    // Sell rate 40%: 10000 → 4000 payout, recorded for buyback.
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 4_000);
    let entry = {
        let buybacks = game_state.buybacks.read().await;
        let list = &buybacks[&(1, "Rica".to_string())];
        assert_eq!(list.len(), 1);
        list[0].entry.clone()
    };
    assert_eq!(entry.price, 4_000);
    assert_eq!(entry.enchant, 2);

    // Buying back costs the exact payout and restores the enchanted unit.
    game_state
        .buyback_item(&pid("buyer"), &pid("npc_rica"), entry.entry_id)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 0);
    {
        let inventories = game_state.inventories.read().await;
        let bag = &inventories[&pid("buyer")].bag;
        assert_eq!(bag.len(), 1);
        assert_eq!(bag[0].item_def_id, "iron_sword");
        assert_eq!(bag[0].enchant, 2);
    }
    let buybacks = game_state.buybacks.read().await;
    assert!(buybacks
        .get(&(1, "Rica".to_string()))
        .is_none_or(|list| list.is_empty()));
}

#[tokio::test]
async fn buyback_rejects_without_enough_gold() {
    let game_state = make_test_game_state("buyback_no_gold");
    let (mut buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 1));
        inventories.insert(pid("buyer"), inv);
    }

    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    let entry_id = {
        let buybacks = game_state.buybacks.read().await;
        buybacks[&(1, "Rica".to_string())][0].entry.entry_id
    };
    {
        let mut gold = game_state.player_gold.write().await;
        *gold.get_mut(&pid("buyer")).unwrap() = 3_999;
    }

    game_state
        .buyback_item(&pid("buyer"), &pid("npc_rica"), entry_id)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 3_999);
    {
        let inventories = game_state.inventories.read().await;
        assert!(inventories[&pid("buyer")].bag.is_empty());
    }
    // The entry survives a failed buyback.
    let buybacks = game_state.buybacks.read().await;
    assert_eq!(buybacks[&(1, "Rica".to_string())].len(), 1);
    drop(buybacks);
    while let Ok(msg) = buyer_rx.try_recv() {
        if let ServerMessage::TradeError { message } = msg {
            assert_eq!(message, "Not enough gold");
            return;
        }
    }
    panic!("expected a TradeError");
}

#[tokio::test]
async fn buyback_list_keeps_only_the_newest_entries() {
    let game_state = make_test_game_state("buyback_cap");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 12));
        inventories.insert(pid("buyer"), inv);
    }

    for _ in 0..12 {
        game_state
            .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
            .await;
    }
    let buybacks = game_state.buybacks.read().await;
    assert_eq!(buybacks[&(1, "Rica".to_string())].len(), 10);
}

/// Backdate every stored entry so it has already expired.
async fn expire_all_buybacks(game_state: &GameState) {
    let mut buybacks = game_state.buybacks.write().await;
    for list in buybacks.values_mut() {
        for stored in list.iter_mut() {
            stored.expires_at_ms = 0;
        }
    }
}

#[tokio::test]
async fn expired_buyback_entries_are_swept_with_their_pair() {
    let game_state = make_test_game_state("buyback_expiry_sweep");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 1));
        inventories.insert(pid("buyer"), inv);
    }
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    assert_eq!(game_state.buybacks.read().await.len(), 1);

    expire_all_buybacks(&game_state).await;
    // Opening the shop sweeps globally, so the whole pair goes — otherwise an
    // offline character's entries would never be reached again.
    game_state
        .open_shop(&pid("buyer"), &pid("npc_rica"), true)
        .await;
    assert!(game_state.buybacks.read().await.is_empty());
}

#[tokio::test]
async fn expired_buyback_cannot_be_repurchased() {
    let game_state = make_test_game_state("buyback_expiry_reject");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 1));
        inventories.insert(pid("buyer"), inv);
    }
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    let entry_id = {
        let buybacks = game_state.buybacks.read().await;
        buybacks[&(1, "Rica".to_string())][0].entry.entry_id
    };
    let gold_after_sell = game_state.get_player_gold(&pid("buyer")).await;

    expire_all_buybacks(&game_state).await;
    game_state
        .buyback_item(&pid("buyer"), &pid("npc_rica"), entry_id)
        .await;

    // Rejected: no item restored and no gold taken.
    assert_eq!(
        game_state.get_player_gold(&pid("buyer")).await,
        gold_after_sell
    );
    assert!(game_state.inventories.read().await[&pid("buyer")]
        .bag
        .is_empty());
}

#[tokio::test]
async fn buyback_survives_a_reconnect() {
    let game_state = make_test_game_state("buyback_reconnect");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 1));
        inventories.insert(pid("buyer"), inv);
    }
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    let entry_id = {
        let buybacks = game_state.buybacks.read().await;
        buybacks[&(1, "Rica".to_string())][0].entry.entry_id
    };

    // Disconnect: the session's player id and registration go away.
    game_state.remove_player(&pid("buyer")).await;
    game_state.unregister_player_character(&pid("buyer")).await;

    // Reconnect under a fresh player id but the same character (id 1).
    game_state.add_player(make_player("buyer2", 1.0, 0.0)).await;
    game_state
        .register_player_character(&pid("buyer2"), 1, 0, attrs_with_cha(10), 4_000)
        .await;
    {
        let mut inventories = game_state.inventories.write().await;
        inventories.insert(pid("buyer2"), Default::default());
    }

    game_state
        .buyback_item(&pid("buyer2"), &pid("npc_rica"), entry_id)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer2")).await, 0);
    let inventories = game_state.inventories.read().await;
    assert_eq!(inventories[&pid("buyer2")].bag[0].item_def_id, "iron_sword");
}

/// The entry is consumed inside the gold/inventory critical section, so two
/// requests racing for one entry_id must restore one unit, not duplicate it.
#[tokio::test]
async fn concurrent_buybacks_of_one_entry_restore_a_single_unit() {
    let game_state = make_test_game_state("buyback_race");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 1));
        inventories.insert(pid("buyer"), inv);
    }
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    let entry_id = {
        let buybacks = game_state.buybacks.read().await;
        buybacks[&(1, "Rica".to_string())][0].entry.entry_id
    };
    // Fund a second purchase, so only the entry consumption — not the gold
    // check — can stop a duplicate.
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 4_000);
    game_state
        .player_gold
        .write()
        .await
        .insert(pid("buyer"), 8_000);

    let (buyer, npc) = (pid("buyer"), pid("npc_rica"));
    tokio::join!(
        game_state.buyback_item(&buyer, &npc, entry_id),
        game_state.buyback_item(&buyer, &npc, entry_id),
    );

    // One unit back, paid for once — the loser is rejected, not served.
    let inventories = game_state.inventories.read().await;
    assert_eq!(inventories[&pid("buyer")].bag.len(), 1);
    drop(inventories);
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 4_000);
    // The loser's sweep drops the pair once its last entry is consumed.
    assert!(game_state
        .buybacks
        .read()
        .await
        .get(&(1, "Rica".to_string()))
        .is_none_or(|list| list.is_empty()));
}

#[tokio::test]
async fn buyback_after_haggled_sell_is_gold_neutral() {
    let game_state = make_test_game_state("buyback_deal_neutral");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 18, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 1));
        inventories.insert(pid("buyer"), inv);
    }
    game_state
        .offer_deal(
            &pid("npc_rica"),
            &pid("buyer"),
            "iron_sword",
            DealKind::Sell,
            25,
            "today's wanted item",
        )
        .await;

    // Boosted payout: 10000 * 0.4 * 1.25 = 5000. Buying back costs the
    // same 5000, so the deal cannot be turned into a money pump.
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 5_000);
    let entry_id = {
        let buybacks = game_state.buybacks.read().await;
        let entry = &buybacks[&(1, "Rica".to_string())][0].entry;
        assert_eq!(entry.price, 5_000);
        entry.entry_id
    };
    game_state
        .buyback_item(&pid("buyer"), &pid("npc_rica"), entry_id)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 0);
    let inventories = game_state.inventories.read().await;
    assert_eq!(inventories[&pid("buyer")].bag.len(), 1);
}

#[tokio::test]
async fn buyback_rejects_when_too_heavy_and_keeps_the_entry() {
    let game_state = make_test_game_state("buyback_overweight");
    let (mut buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 100_000).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 1));
        inventories.insert(pid("buyer"), inv);
    }
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    let entry_id = {
        let buybacks = game_state.buybacks.read().await;
        buybacks[&(1, "Rica".to_string())][0].entry.entry_id
    };

    // Fill the bag right up to the carry limit so the returning sword
    // cannot fit.
    {
        let max_weight = game_state.max_carry_weight(&pid("buyer")).await;
        let sword_weight = game_state.item_defs.weight("iron_sword");
        let fill = (max_weight / sword_weight) as u32;
        let mut inventories = game_state.inventories.write().await;
        inventories.get_mut(&pid("buyer")).unwrap().bag = vec![bag_item(8, "iron_sword", fill)];
    }

    let gold_before = game_state.get_player_gold(&pid("buyer")).await;
    game_state
        .buyback_item(&pid("buyer"), &pid("npc_rica"), entry_id)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, gold_before);
    let buybacks = game_state.buybacks.read().await;
    assert_eq!(buybacks[&(1, "Rica".to_string())].len(), 1);
    drop(buybacks);
    while let Ok(msg) = buyer_rx.try_recv() {
        if let ServerMessage::TradeError { message } = msg {
            assert_eq!(message, "Too heavy to carry");
            return;
        }
    }
    panic!("expected a TradeError");
}

#[tokio::test]
async fn buyback_is_scoped_to_the_selling_character() {
    let game_state = make_test_game_state("buyback_scoped");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 1));
        inventories.insert(pid("buyer"), inv);
    }
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    let entry_id = {
        let buybacks = game_state.buybacks.read().await;
        buybacks[&(1, "Rica".to_string())][0].entry.entry_id
    };

    // A different character (id 2) cannot take the seller's entry, even
    // with the exact entry id and enough gold.
    game_state.add_player(make_player("other", 1.0, 0.5)).await;
    game_state
        .register_player_character(&pid("other"), 2, 0, attrs_with_cha(10), 100_000)
        .await;
    {
        let mut inventories = game_state.inventories.write().await;
        inventories.insert(pid("other"), Default::default());
    }
    let mut other_rx = game_state.register_direct_channel(&pid("other")).await;
    game_state
        .buyback_item(&pid("other"), &pid("npc_rica"), entry_id)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("other")).await, 100_000);
    let buybacks = game_state.buybacks.read().await;
    assert_eq!(buybacks[&(1, "Rica".to_string())].len(), 1);
    drop(buybacks);
    while let Ok(msg) = other_rx.try_recv() {
        if let ServerMessage::TradeError { message } = msg {
            assert_eq!(message, "That item is no longer available");
            return;
        }
    }
    panic!("expected a TradeError");
}

// --- Resident (non-merchant) trading (economy phase 3) ---

fn make_resident_npc(id: &str, x: f32, z: f32) -> Player {
    let mut p = make_player(id, x, z);
    p.name = "Karl".to_string();
    p.is_official_npc = true;
    p
}

fn bag_item(instance_id: u64, item_def_id: &str, quantity: u32) -> ItemInstance {
    ItemInstance {
        instance_id,
        item_def_id: item_def_id.to_string(),
        quantity,
        enchant: 0,
    }
}

/// Spawn the resident trader Karl (wishlist: torch, dagger @120%) next to a
/// seller. Karl's wallet and bag are set explicitly; the seller starts with
/// the given bag and no gold.
async fn setup_resident_trade(
    game_state: &GameState,
    npc_gold: i64,
    npc_bag: Vec<ItemInstance>,
    seller_bag: Vec<ItemInstance>,
) {
    game_state
        .add_player(make_resident_npc("npc_karl", 0.0, 0.0))
        .await;
    game_state.add_player(make_player("seller", 1.0, 0.0)).await;
    game_state
        .register_player_character(&pid("seller"), 1, 0, attrs_with_cha(10), 0)
        .await;
    game_state
        .register_player_character(&pid("npc_karl"), 2, 0, attrs_with_cha(10), npc_gold)
        .await;
    let mut inventories = game_state.inventories.write().await;
    inventories.insert(
        pid("npc_karl"),
        onlinerpg_shared::inventory::PlayerInventory {
            bag: npc_bag,
            ..Default::default()
        },
    );
    inventories.insert(
        pid("seller"),
        onlinerpg_shared::inventory::PlayerInventory {
            bag: seller_bag,
            ..Default::default()
        },
    );
}

#[tokio::test]
async fn resident_buys_wishlist_item_at_premium_from_wallet() {
    let game_state = make_test_game_state("resident_sell");
    setup_resident_trade(&game_state, 10_000, vec![], vec![bag_item(7, "torch", 1)]).await;

    // Torch base 50 at Karl's 120% wishlist rate → 60.
    game_state
        .sell_item(&pid("seller"), &pid("npc_karl"), 7)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("seller")).await, 60);
    assert_eq!(
        game_state.get_player_gold(&pid("npc_karl")).await,
        10_000 - 60
    );

    // The torch landed in Karl's real inventory; the seller's bag is empty.
    let inventories = game_state.inventories.read().await;
    assert_eq!(inventories[&pid("npc_karl")].bag.len(), 1);
    assert_eq!(inventories[&pid("npc_karl")].bag[0].item_def_id, "torch");
    assert!(inventories[&pid("seller")].bag.is_empty());
}

#[tokio::test]
async fn resident_rejects_items_off_the_wishlist() {
    let game_state = make_test_game_state("resident_off_wishlist");
    setup_resident_trade(
        &game_state,
        10_000,
        vec![],
        vec![bag_item(7, "iron_sword", 1)],
    )
    .await;
    let mut seller_rx = game_state.register_direct_channel(&pid("seller")).await;

    game_state
        .sell_item(&pid("seller"), &pid("npc_karl"), 7)
        .await;

    assert_eq!(game_state.get_player_gold(&pid("seller")).await, 0);
    match seller_rx.try_recv() {
        Ok(ServerMessage::TradeError { message }) => {
            assert!(message.contains("no use"), "got: {message}")
        }
        other => panic!("Expected TradeError, got {:?}", other),
    }
    let inventories = game_state.inventories.read().await;
    assert_eq!(
        inventories[&pid("seller")].bag.len(),
        1,
        "item must be retained"
    );
}

#[tokio::test]
async fn resident_wallet_caps_purchases() {
    let game_state = make_test_game_state("resident_wallet_cap");
    // Karl has 59 gold units; the torch costs him 60.
    setup_resident_trade(&game_state, 59, vec![], vec![bag_item(7, "torch", 1)]).await;
    let mut seller_rx = game_state.register_direct_channel(&pid("seller")).await;

    game_state
        .sell_item(&pid("seller"), &pid("npc_karl"), 7)
        .await;

    assert_eq!(game_state.get_player_gold(&pid("seller")).await, 0);
    assert_eq!(game_state.get_player_gold(&pid("npc_karl")).await, 59);
    match seller_rx.try_recv() {
        Ok(ServerMessage::TradeError { message }) => {
            assert!(message.contains("afford"), "got: {message}")
        }
        other => panic!("Expected TradeError, got {:?}", other),
    }
}

#[tokio::test]
async fn resident_sells_stock_but_keeps_wishlist_items() {
    let game_state = make_test_game_state("resident_stock");
    // Karl carries a spear (sellable stock) and a torch (wishlist: kept).
    setup_resident_trade(
        &game_state,
        0,
        vec![bag_item(11, "spear", 1), bag_item(12, "torch", 1)],
        vec![],
    )
    .await;
    {
        let mut gold = game_state.player_gold.write().await;
        gold.insert(pid("seller"), 10_000);
    }
    let mut seller_rx = game_state.register_direct_channel(&pid("seller")).await;

    // Spear base 3500 — instance moves to the buyer, gold to Karl.
    game_state
        .buy_item(&pid("seller"), &pid("npc_karl"), "spear")
        .await;
    assert_eq!(game_state.get_player_gold(&pid("seller")).await, 6_500);
    assert_eq!(game_state.get_player_gold(&pid("npc_karl")).await, 3_500);
    {
        let inventories = game_state.inventories.read().await;
        assert_eq!(inventories[&pid("seller")].bag.len(), 1);
        assert_eq!(inventories[&pid("seller")].bag[0].item_def_id, "spear");
        assert_eq!(inventories[&pid("npc_karl")].bag.len(), 1);
        assert_eq!(inventories[&pid("npc_karl")].bag[0].item_def_id, "torch");
    }
    while seller_rx.try_recv().is_ok() {}

    // The torch is on Karl's wishlist: he keeps it (no buy-back pump).
    game_state
        .buy_item(&pid("seller"), &pid("npc_karl"), "torch")
        .await;
    assert_eq!(game_state.get_player_gold(&pid("seller")).await, 6_500);
    match seller_rx.try_recv() {
        Ok(ServerMessage::TradeError { message }) => {
            assert!(message.contains("part with"), "got: {message}")
        }
        other => panic!("Expected TradeError, got {:?}", other),
    }
}

#[tokio::test]
async fn resident_shop_state_reports_wishlist_and_stock() {
    let game_state = make_test_game_state("resident_shop_state");
    setup_resident_trade(
        &game_state,
        4_321,
        vec![
            bag_item(11, "spear", 1),
            bag_item(12, "torch", 1),
            bag_item(13, "worn_iron_sword", 1),
        ],
        vec![],
    )
    .await;
    let mut seller_rx = game_state.register_direct_channel(&pid("seller")).await;

    game_state
        .open_shop(&pid("seller"), &pid("npc_karl"), true)
        .await;

    match seller_rx.try_recv() {
        Ok(ServerMessage::ShopState {
            merchant_name,
            catalog,
            sell_rate_percent,
            wishlist,
            stock,
            ..
        }) => {
            assert_eq!(merchant_name, "Karl");
            assert!(catalog.is_empty());
            assert_eq!(sell_rate_percent, 120);
            assert_eq!(wishlist, vec!["torch".to_string(), "dagger".to_string()]);
            // Stock excludes the wishlist torch and the unpriced worn sword.
            assert_eq!(stock.len(), 1);
            assert_eq!(stock[0].item_def_id, "spear");
            assert_eq!(stock[0].quantity, 1);
        }
        other => panic!("Expected ShopState, got {:?}", other),
    }
}

#[tokio::test]
async fn resident_deal_band_is_wider_and_wishlist_scoped() {
    let game_state = make_test_game_state("resident_deal_band");
    setup_resident_trade(&game_state, 10_000, vec![], vec![]).await;
    let mut npc_rx = game_state.register_direct_channel(&pid("npc_karl")).await;

    // CHA 10 resident band is ±20 (twice the merchant ±10).
    game_state
        .offer_deal(
            &pid("npc_karl"),
            &pid("seller"),
            "torch",
            DealKind::Sell,
            40,
            "really need torches tonight",
        )
        .await;
    match npc_rx.try_recv() {
        Ok(ServerMessage::DealResult {
            accepted,
            applied_modifier_pct,
            ..
        }) => {
            assert!(accepted);
            assert_eq!(applied_modifier_pct, 20);
        }
        other => panic!("Expected DealResult, got {:?}", other),
    }

    // Sell offers outside the wishlist are rejected.
    game_state.clear_deal_cooldowns_for_test().await;
    game_state
        .offer_deal(
            &pid("npc_karl"),
            &pid("seller"),
            "iron_sword",
            DealKind::Sell,
            10,
            "nice sword",
        )
        .await;
    match npc_rx.try_recv() {
        Ok(ServerMessage::DealResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("wishlist"), "got: {message}");
        }
        other => panic!("Expected rejection, got {:?}", other),
    }
}

#[tokio::test]
async fn open_trade_pushes_shop_state_to_the_player() {
    let game_state = make_test_game_state("open_trade");
    setup_resident_trade(&game_state, 1_000, vec![], vec![]).await;
    let mut seller_rx = game_state.register_direct_channel(&pid("seller")).await;
    let npc_rx = game_state.register_direct_channel(&pid("npc_karl")).await;

    game_state
        .open_trade(&pid("npc_karl"), &pid("seller"))
        .await;
    match seller_rx.try_recv() {
        Ok(ServerMessage::ShopState { merchant_name, .. }) => assert_eq!(merchant_name, "Karl"),
        other => panic!("Expected ShopState, got {:?}", other),
    }

    // A non-trading NPC cannot push a window; the seller hears nothing.
    game_state
        .add_player({
            let mut p = make_player("npc_nobody", 0.5, 0.0);
            p.name = "Nobody".to_string();
            p.is_official_npc = true;
            p
        })
        .await;
    let mut nobody_rx = game_state.register_direct_channel(&pid("npc_nobody")).await;
    game_state
        .open_trade(&pid("npc_nobody"), &pid("seller"))
        .await;
    match nobody_rx.try_recv() {
        Ok(ServerMessage::TradeError { message }) => {
            assert!(message.contains("nothing to trade"), "got: {message}")
        }
        other => panic!("Expected TradeError, got {:?}", other),
    }
    drop(npc_rx);
}

#[tokio::test]
async fn cross_floor_open_trade_is_rejected_before_reaching_the_player() {
    let game_state = make_test_game_state("cross_floor_open_trade");
    setup_resident_trade(&game_state, 1_000, vec![], vec![]).await;
    set_floor(&game_state, "seller", 1).await;
    let mut seller_rx = game_state.register_direct_channel(&pid("seller")).await;
    let mut npc_rx = game_state.register_direct_channel(&pid("npc_karl")).await;

    game_state
        .open_trade(&pid("npc_karl"), &pid("seller"))
        .await;

    assert!(matches!(seller_rx.try_recv(), Err(MpscTryRecvError::Empty)));
    match npc_rx.try_recv() {
        Ok(ServerMessage::TradeError { message }) => {
            assert!(message.contains("another floor"), "got: {message}")
        }
        other => panic!("Expected cross-floor TradeError, got {other:?}"),
    }
}

#[tokio::test]
async fn salary_pays_once_per_day_rollover_up_to_cap() {
    let game_state = make_test_game_state("salary");
    setup_resident_trade(&game_state, 27_000, vec![], vec![]).await;

    // First tick after boot only records the day.
    game_state.tick_npc_salaries().await;
    assert_eq!(game_state.get_player_gold(&pid("npc_karl")).await, 27_000);

    // Roll the ledger back a day: the next tick pays one salary, capped at
    // the 30_000 wallet cap (27_000 + 5_000 → 30_000).
    {
        let mut last = game_state.npc_salary_last_day.write().await;
        *last = last.map(|d| d - 1);
    }
    game_state.tick_npc_salaries().await;
    assert_eq!(game_state.get_player_gold(&pid("npc_karl")).await, 30_000);

    // Same day again: no double payment.
    game_state.tick_npc_salaries().await;
    assert_eq!(game_state.get_player_gold(&pid("npc_karl")).await, 30_000);
}

// --- Enchant weapon scrolls ---

/// Spawn a live player wielding `weapon` at the given enchant level with one
/// enchant scroll (instance 2) in the bag, and return their direct channel.
async fn setup_enchant_reader(
    game_state: &GameState,
    weapon: Option<(&str, i32)>,
    scrolls: u32,
) -> tokio::sync::mpsc::UnboundedReceiver<ServerMessage> {
    game_state.add_player(make_player("reader", 0.0, 0.0)).await;
    let rx = game_state.register_direct_channel(&pid("reader")).await;

    let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
    if let Some((weapon_def_id, enchant)) = weapon {
        inv.equipped.insert(
            EquipSlot::MainHand,
            ItemInstance {
                instance_id: 1,
                item_def_id: weapon_def_id.to_string(),
                quantity: 1,
                enchant,
            },
        );
    }
    inv.bag
        .push(bag_item(2, "scroll_of_enchant_weapon", scrolls));
    game_state
        .inventories
        .write()
        .await
        .insert(pid("reader"), inv);
    rx
}

#[tokio::test]
async fn enchant_scroll_enchants_wielded_weapon() {
    let game_state = make_test_game_state("enchant_ok");
    let _rx = setup_enchant_reader(&game_state, Some(("iron_sword", 0)), 1).await;

    game_state.use_item(&pid("reader"), 2).await;

    let inv = game_state
        .get_player_inventory(&pid("reader"))
        .await
        .unwrap();
    let weapon = inv.equipped.get(&EquipSlot::MainHand).unwrap();
    assert_eq!(weapon.enchant, 1);
    assert!(inv.bag.is_empty(), "the scroll should be consumed");
}

#[tokio::test]
async fn enchant_scroll_requires_wielded_weapon() {
    let game_state = make_test_game_state("enchant_no_weapon");
    let mut rx = setup_enchant_reader(&game_state, None, 1).await;

    game_state.use_item(&pid("reader"), 2).await;

    let inv = game_state
        .get_player_inventory(&pid("reader"))
        .await
        .unwrap();
    assert_eq!(inv.bag.len(), 1, "the scroll should be kept");
    match rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(
                message.contains("no weapon"),
                "unexpected message: {message}"
            );
        }
        other => panic!("Expected a system reply, got {:?}", other),
    }
}

#[tokio::test]
async fn enchant_scroll_destroys_over_enchanted_weapon() {
    let game_state = make_test_game_state("enchant_boom");
    // At +12 the success floor is 1%, so each read is a 99% destruction
    // roll. 100 scrolls make survival odds ~1e-200: the loop below is
    // deterministic for all practical purposes.
    let _rx = setup_enchant_reader(&game_state, Some(("iron_sword", 12)), 100).await;

    let reader = pid("reader");
    for _ in 0..100 {
        game_state.use_item(&reader, 2).await;
        let inv = game_state.get_player_inventory(&reader).await.unwrap();
        if !inv.equipped.contains_key(&EquipSlot::MainHand) {
            return; // evaporated, as expected
        }
    }
    panic!("the weapon should have evaporated within 100 reads at 99% odds");
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
    let p = &game_state.get_all_players().await[player_id];
    (p.position.x, p.position.z)
}

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

/// Drain a direct channel and return the first `PositionCorrected` in it.
fn first_correction(
    rx: &mut mpsc::UnboundedReceiver<ServerMessage>,
) -> Option<(Position, f32, i8)> {
    std::iter::from_fn(|| rx.try_recv().ok()).find_map(|msg| match msg {
        ServerMessage::PositionCorrected {
            position,
            rotation,
            floor_level,
        } => Some((position, rotation, floor_level)),
        _ => None,
    })
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

use onlinerpg_shared::dungeon::{cell_center, ENTRANCE_DOOR_ID};

fn first_dungeon(game_state: &GameState) -> crate::dungeon_defs::DungeonEntranceDef {
    game_state
        .dungeon_defs
        .all()
        .next()
        .expect("a dungeon def")
        .clone()
}

/// First registry dungeon plus its shallowest floor holding an interior door.
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
        .find_map(|l| interior_doors(l).first().copied().map(|d| (l.depth, d)))
        .expect("a floor with an interior door");
    (entrance, depth, door)
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
    game_state.init_passability("nonexistent_terrain_dir").await;
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
    game_state.init_passability("nonexistent_terrain_dir").await;

    let (from, to) = door_side_positions(&entrance, depth, &door, false);
    let player_id = add_delver(&game_state, "delver", from, depth).await;
    let go = |p: Position| MoveCommand {
        floor_level: -(depth as i8),
        ..move_cmd(p, false)
    };

    // Shut (boot default): the crossing is sealed.
    game_state
        .update_player_position(&player_id, go(to), false, false)
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
        .update_player_position(&player_id, go(to), false, false)
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
        .update_player_position(&player_id, go(from), false, false)
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
    while let Ok(msg) = direct_rx.try_recv() {
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
            false,
        )
        .await;
    game_state.tick_player_movement(60.0).await;
    assert_eq!(player_xz(&game_state, &player_id).await, (0.5, 6.5));
}

#[tokio::test]
async fn dirty_save_is_retried_after_failure() {
    let (auth, db_path) = make_test_auth_with_path("dirty_save_retry");
    let account = auth.login_npc("npc_dirty_save_retry").unwrap();
    let attributes = attrs_with_cha(12);
    let record = create_test_character(&auth, &account, "Retryknight");
    let initial_inventory = auth.load_inventory(record.id).unwrap();

    let game_state = make_test_game_state("dirty_save_retry");
    let player_id = pid("dirty_save_retry");
    let mut player = make_player("dirty_save_retry", 42.0, 0.0);
    player.name = record.name.clone();
    game_state.add_player(player).await;
    game_state
        .register_player_character(&player_id, record.id, record.xp, attributes, record.gold)
        .await;
    game_state.inventories.write().await.insert(
        player_id,
        PlayerInventory {
            bag: vec![bag_item(1, "torch", 1)],
            equipped: Default::default(),
        },
    );
    game_state.mark_dirty(&player_id).await;
    game_state.mark_inventory_dirty(&player_id).await;

    let failure_connection = rusqlite::Connection::open(&db_path).unwrap();
    failure_connection
        .execute_batch(
            "CREATE TRIGGER fail_character_update
             BEFORE UPDATE ON characters
             BEGIN
                 SELECT RAISE(FAIL, 'test save failure');
             END;",
        )
        .unwrap();

    game_state.flush_dirty_saves(&auth).await;

    assert!(game_state.dirty_players.read().await.contains(&player_id));
    assert!(game_state
        .dirty_inventories
        .read()
        .await
        .contains(&player_id));
    assert_ne!(
        auth.get_character_for_account(&account, record.id)
            .unwrap()
            .last_x,
        42.0
    );
    assert_eq!(auth.load_inventory(record.id).unwrap(), initial_inventory);

    failure_connection
        .execute("DROP TRIGGER fail_character_update", [])
        .unwrap();
    game_state.flush_dirty_saves(&auth).await;

    assert!(!game_state.dirty_players.read().await.contains(&player_id));
    assert!(!game_state
        .dirty_inventories
        .read()
        .await
        .contains(&player_id));
    assert_eq!(
        auth.get_character_for_account(&account, record.id)
            .unwrap()
            .last_x,
        42.0
    );
    assert_eq!(
        auth.load_inventory(record.id).unwrap(),
        vec![crate::auth::ItemRow {
            item_def_id: "torch".to_string(),
            quantity: 1,
            equip_slot: None,
            enchant: 0,
        }]
    );
}

// F-015: session replacement (kick) must flush the departing session's
// inventory to the DB before a replacement login can load it. Otherwise the
// old async disconnect-save races the new session's load, letting a dropped
// item survive in the DB snapshot and be picked up again (duplication).
#[tokio::test]
async fn kick_flushes_dropped_inventory_before_replacement_load() {
    let auth = make_test_auth("f015_kick_flush");
    let account = auth.login_npc("npc_f015_account").unwrap();
    let attributes = attrs_with_cha(12);
    let record = create_test_character(&auth, &account, "Dupeknight");
    let char_id = record.id;

    // DB snapshot before the drop: one sword in the bag.
    auth.save_batch(
        &[],
        &[(
            char_id,
            vec![crate::auth::ItemRow {
                item_def_id: "worn_iron_sword".to_string(),
                quantity: 1,
                equip_slot: None,
                enchant: 0,
            }],
        )],
        &[],
        None,
    )
    .unwrap();

    let game_state = make_test_game_state("f015_kick_flush");

    // Session A enters: player registered and inventory loaded from the DB.
    let a = pid("session_a");
    let mut player = make_player("session_a", 0.0, 0.0);
    player.name = record.name.clone();
    game_state.add_player(player).await;
    game_state
        .register_player_character(&a, char_id, record.xp, attributes, record.gold)
        .await;
    game_state.load_player_inventory(&a, char_id, &auth).await;

    // A drops the sword (in-memory only; not yet persisted).
    let instance_id = game_state.get_player_inventory(&a).await.unwrap().bag[0].instance_id;
    game_state.drop_item(&a, instance_id).await;
    assert!(game_state
        .get_player_inventory(&a)
        .await
        .unwrap()
        .bag
        .is_empty());
    // The window F-015 raced: the DB still holds the pre-drop snapshot.
    assert_eq!(auth.load_inventory(char_id).unwrap().len(), 1);

    // A replacement login kicks A by (unique) character name.
    game_state.kick_player_by_name(&record.name, &auth).await;

    // The kick flushed A's post-drop inventory and detached it, so a
    // replacement load reads zero swords (no dupe) instead of a stale one.
    assert_eq!(auth.load_inventory(char_id).unwrap().len(), 0);
    assert!(game_state.get_player_inventory(&a).await.is_none());
}

fn make_door_test_house() -> onlinerpg_shared::housing::HouseData {
    use crate::housing::test_fixtures::{house_at, room_at};
    let mut house = house_at(10.0, 10.0, vec![room_at(0, 0)]);
    house.rooms[0].wall_north[0].variant = onlinerpg_shared::housing::WallVariant::WithDoor;
    house
}

#[tokio::test]
async fn open_door_state_is_stamped_onto_served_house_data() {
    let game_state = make_test_game_state("door_state_stamp");
    let house = make_door_test_house();
    game_state.housing_io.write_house(&house).await.unwrap();

    // Door world position: origin + (segment 0 center, north edge) = (10.5, 10)
    let toggler = pid("toggler");
    game_state
        .add_player(make_player("toggler", 10.5, 10.5))
        .await;

    let toggled = game_state
        .toggle_door(&toggler, &house.id, 0, WallDirection::North, 0)
        .await;
    assert_eq!(toggled, Some(true));

    // A reconnecting client re-fetches the house from disk (is_open false there);
    // the stamp overlays the in-memory open state.
    let mut served = vec![house.clone()];
    game_state.apply_open_door_state(&mut served).await;
    assert!(served[0].rooms[0].wall_north[0].is_open);

    // Closing the door clears the stamp.
    game_state
        .toggle_door(&toggler, &house.id, 0, WallDirection::North, 0)
        .await;
    let mut served = vec![house.clone()];
    game_state.apply_open_door_state(&mut served).await;
    assert!(!served[0].rooms[0].wall_north[0].is_open);

    // A house edit (passability reinstall) forgets its open doors.
    game_state
        .toggle_door(&toggler, &house.id, 0, WallDirection::North, 0)
        .await;
    game_state.passability_add_house(&house).await;
    let mut served = vec![house.clone()];
    game_state.apply_open_door_state(&mut served).await;
    assert!(!served[0].rooms[0].wall_north[0].is_open);
}

const CRYPT_ID: &str = "old_crypt";

/// Put a character next to the Old Crypt's chest on the deepest floor with
/// the guardian already dead — every check `open_dungeon_chest` makes before
/// the nightly refill gate.
async fn stage_chest_opener(game_state: &GameState, name: &str, character_id: i64) -> PlayerId {
    let player_id = pid(name);
    game_state.ensure_dungeon_runtime(CRYPT_ID).await;
    let entrance = game_state
        .dungeon_defs
        .get(CRYPT_ID)
        .expect("old_crypt def");

    let (deepest, chest_pos) = {
        let dungeons = game_state.dungeons.read().await;
        let rt = dungeons.get(CRYPT_ID).expect("crypt runtime");
        let deepest = rt.layouts.len() as u8;
        let chest = rt
            .layouts
            .last()
            .and_then(|l| l.chest)
            .expect("crypt's last floor has a chest");
        (
            deepest,
            onlinerpg_shared::dungeon::cell_center(&entrance.position(), deepest, chest),
        )
    };

    let mut player = make_player(name, chest_pos.x, chest_pos.z);
    player.floor_level = -(deepest as i8);
    game_state.add_player(player).await;
    game_state
        .register_player_character(&player_id, character_id, 0, attrs_with_cha(12), 0)
        .await;
    {
        let mut dungeons = game_state.dungeons.write().await;
        let rt = dungeons.get_mut(CRYPT_ID).expect("crypt runtime");
        let floor = rt
            .floors
            .entry(deepest)
            .or_insert_with(|| super::dungeon::FloorRuntime {
                slots: Vec::new(),
                players: HashSet::new(),
                boss_defeated: false,
            });
        floor.players.insert(player_id);
        floor.boss_defeated = true;
    }
    player_id
}

/// Assert the next direct message refuses a chest interaction, with a reason
/// containing `expected`.
fn assert_chest_rejected(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<ServerMessage>,
    expected: &str,
) {
    match rx.try_recv() {
        Ok(ServerMessage::InteractionRejected { reason }) => {
            assert!(
                reason.contains(expected),
                "expected {expected:?}, got {reason}"
            );
        }
        other => panic!(
            "Expected a rejection containing {expected:?}, got {:?}",
            other
        ),
    }
}

/// A relog mints a fresh PlayerId, so the refill gate has to be keyed by
/// character and reloaded from the DB — otherwise the chest is farmable by
/// logging out and back in.
#[tokio::test]
async fn dungeon_chest_stays_empty_across_a_relog() {
    let auth = make_test_auth("chest_relog");
    let account = auth.login_npc("npc_chest_relog").unwrap();
    let character = create_test_character(&auth, &account, "Delver");

    let game_state = make_test_game_state("chest_relog");
    let player_id = stage_chest_opener(&game_state, "Delver", character.id).await;

    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    assert!(
        game_state.get_player_gold(&player_id).await > 0,
        "first open should pay out"
    );

    // Log out, then back in as a new session for the same character.
    game_state.unregister_player_character(&player_id).await;
    game_state.remove_player(&player_id).await;

    let opens = auth.load_dungeon_chest_opens(character.id).unwrap();
    assert_eq!(opens.len(), 1, "the open should have been persisted");
    let rejoined_id = stage_chest_opener(&game_state, "Delver Rejoined", character.id).await;
    game_state.set_chest_opens(character.id, opens).await;

    let mut rejoined_rx = game_state.register_direct_channel(&rejoined_id).await;
    game_state
        .open_dungeon_chest(&rejoined_id, CRYPT_ID, &auth)
        .await;

    assert_chest_rejected(&mut rejoined_rx, "nightfall");
    assert_eq!(
        game_state.get_player_gold(&rejoined_id).await,
        0,
        "second open must not pay out"
    );
}

#[tokio::test]
async fn dungeon_chest_refills_once_per_night() {
    let auth = make_test_auth("chest_nightfall");
    let account = auth.login_npc("npc_chest_nightfall").unwrap();
    let character = create_test_character(&auth, &account, "Nightcaller");

    let game_state = make_test_game_state("chest_nightfall");
    let player_id = stage_chest_opener(&game_state, "Nightcaller", character.id).await;
    let mut direct_rx = game_state.register_direct_channel(&player_id).await;

    // Opened at midnight, before the day's sunset.
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    let after_first = game_state.get_player_gold(&player_id).await;
    assert!(after_first > 0, "first open should pay out");
    while direct_rx.try_recv().is_ok() {}

    // Still the same night: no refill.
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    assert_eq!(
        game_state.get_player_gold(&player_id).await,
        after_first,
        "the chest must not refill before nightfall"
    );

    // Past sunset — a new night, so one more open.
    game_state.debug_set_time(23, 0);
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    let after_second = game_state.get_player_gold(&player_id).await;
    assert!(
        after_second > after_first,
        "the chest should refill at nightfall"
    );

    // Same night again: still one open only.
    game_state.debug_set_time(23, 30);
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;
    assert_eq!(
        game_state.get_player_gold(&player_id).await,
        after_second,
        "one open per night, not one per visit"
    );
}

/// An unspawned boss slot (spawn pending, or refused by the global monster
/// cap) used to read as "no guardian alive" and open the gate.
#[tokio::test]
async fn dungeon_chest_stays_shut_until_the_guardian_dies() {
    let auth = make_test_auth("chest_guardian");
    let account = auth.login_npc("npc_chest_guardian").unwrap();
    let character = create_test_character(&auth, &account, "Challenger");

    let game_state = make_test_game_state("chest_guardian");
    let player_id = stage_chest_opener(&game_state, "Challenger", character.id).await;
    {
        let mut dungeons = game_state.dungeons.write().await;
        let rt = dungeons.get_mut(CRYPT_ID).expect("crypt runtime");
        let deepest = rt.layouts.len() as u8;
        rt.floors
            .get_mut(&deepest)
            .expect("staged floor")
            .boss_defeated = false;
    }

    let mut direct_rx = game_state.register_direct_channel(&player_id).await;
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;

    assert_chest_rejected(&mut direct_rx, "The guardian still lives");
    assert_eq!(game_state.get_player_gold(&player_id).await, 0);
    assert!(
        auth.load_dungeon_chest_opens(character.id)
            .unwrap()
            .is_empty(),
        "a refused open must not consume the night's chest"
    );
}

/// The nightly claim is the durable anti-duplication boundary. A failed DB
/// write must reject the open without rewards and release only this attempt's
/// in-memory claim so a repaired storage layer can retry.
#[tokio::test]
async fn dungeon_chest_persistence_failure_rejects_without_reward_and_can_retry() {
    let (auth, db_path) = make_test_auth_with_path("chest_persist_fail");
    let account = auth.login_npc("npc_chest_persist_fail").unwrap();
    let character = create_test_character(&auth, &account, "CarefulDelver");

    let game_state = make_test_game_state("chest_persist_fail");
    let player_id = stage_chest_opener(&game_state, "CarefulDelver", character.id).await;
    game_state
        .inventories
        .write()
        .await
        .insert(player_id, PlayerInventory::default());
    let mut direct_rx = game_state.register_direct_channel(&player_id).await;

    rusqlite::Connection::open(&db_path)
        .unwrap()
        .execute("DROP TABLE character_dungeon_chests", [])
        .unwrap();

    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &auth)
        .await;

    assert_chest_rejected(&mut direct_rx, "saved");
    assert_eq!(
        game_state.get_player_gold(&player_id).await,
        0,
        "failed persistence must not pay gold"
    );
    assert!(
        game_state
            .get_player_inventory(&player_id)
            .await
            .expect("staged inventory")
            .bag
            .is_empty(),
        "failed persistence must not grant chest items"
    );

    let repaired_auth = crate::auth::AuthService::new(db_path).unwrap();

    while direct_rx.try_recv().is_ok() {}
    game_state
        .open_dungeon_chest(&player_id, CRYPT_ID, &repaired_auth)
        .await;

    assert!(
        game_state.get_player_gold(&player_id).await > 0,
        "retry after schema repair should pay out"
    );
    assert!(
        !game_state
            .get_player_inventory(&player_id)
            .await
            .expect("staged inventory")
            .bag
            .is_empty(),
        "retry after schema repair should grant chest items"
    );
    assert_eq!(
        repaired_auth
            .load_dungeon_chest_opens(character.id)
            .unwrap()
            .len(),
        1,
        "successful retry should persist exactly one chest claim"
    );
}

/// The nightly refill gate compares two of these, so the index has to move
/// forward exactly once a day and never backwards — including across the
/// seasonal swing in sunset time.
#[test]
fn night_epoch_advances_once_per_game_day() {
    use crate::game_state::time::{
        GAME_DAYS_PER_MONTH, GAME_DAYS_PER_YEAR, GAME_HOURS_PER_DAY, GAME_START_YEAR,
    };

    let start = GameState::night_epoch_at(GAME_START_YEAR as u32, 1, 1, 0, 0);
    let mut previous = start;
    let mut advances = 0;

    // A full game year, sampled every 10 minutes.
    for day in 0..GAME_DAYS_PER_YEAR {
        let month = (day / GAME_DAYS_PER_MONTH + 1) as u8;
        let day_of_month = (day % GAME_DAYS_PER_MONTH + 1) as u8;
        for step in 0..(GAME_HOURS_PER_DAY * 6) {
            let hour = (step / 6) as u8;
            let minute = ((step % 6) * 10) as u8;
            let epoch = GameState::night_epoch_at(
                GAME_START_YEAR as u32,
                month,
                day_of_month,
                hour,
                minute,
            );
            assert!(
                epoch == previous || epoch == previous + 1,
                "night epoch jumped from {previous} to {epoch} at {month}/{day_of_month} {hour}:{minute}"
            );
            if epoch != previous {
                advances += 1;
                previous = epoch;
            }
        }
    }

    assert_eq!(
        advances, GAME_DAYS_PER_YEAR,
        "one nightfall per game day over a year"
    );
}

/// Logging in twice kicks the first session, whose cleanup then runs *after*
/// the replacement already loaded its chest history. Dropping character-keyed
/// state on that late cleanup would hand the live session a fresh chest.
#[tokio::test]
async fn session_replacement_keeps_the_live_session_chest_claim() {
    let auth = make_test_auth("chest_kick");
    let account = auth.login_npc("npc_chest_kick").unwrap();
    let character = create_test_character(&auth, &account, "Doubler");

    let game_state = make_test_game_state("chest_kick");
    let first_id = stage_chest_opener(&game_state, "Doubler", character.id).await;
    game_state
        .open_dungeon_chest(&first_id, CRYPT_ID, &auth)
        .await;
    assert!(game_state.get_player_gold(&first_id).await > 0);

    // Replacement session logs in and loads the claim from the DB...
    let second_id = stage_chest_opener(&game_state, "Doubler Again", character.id).await;
    let opens = auth.load_dungeon_chest_opens(character.id).unwrap();
    game_state.set_chest_opens(character.id, opens).await;
    // ...then the kicked connection finally tears itself down.
    game_state.unregister_player_character(&first_id).await;
    game_state.remove_player(&first_id).await;

    let mut second_rx = game_state.register_direct_channel(&second_id).await;
    game_state
        .open_dungeon_chest(&second_id, CRYPT_ID, &auth)
        .await;

    assert_chest_rejected(&mut second_rx, "nightfall");
    assert_eq!(game_state.get_player_gold(&second_id).await, 0);
}

// Trained skills: XP grants update the map, notify the owner directly, and
// mark the player for the next dirty flush; a capped skill goes quiet.
#[tokio::test]
async fn skill_xp_grant_notifies_owner_and_marks_dirty() {
    use onlinerpg_shared::skills::{skill_xp_for_level, SkillId, SKILL_LEVEL_CAP};

    let game_state = make_test_game_state("skill_xp_grant");
    let player = pid("angler");
    game_state.add_player(make_player("angler", 0.0, 0.0)).await;
    game_state
        .register_player_character(&player, 42, 0, attrs_with_cha(10), 0)
        .await;
    game_state
        .register_player_skills(&player, Default::default())
        .await;
    let mut rx = game_state.register_direct_channel(&player).await;

    // 150 XP: crosses the level-1 threshold (100).
    let result = game_state
        .add_skill_xp(&player, SkillId::Fishing, 150)
        .await
        .expect("grant should apply");
    assert_eq!(result.new_level, 1);
    assert!(result.leveled_up);

    let msg = rx.try_recv().expect("owner should be notified");
    match msg {
        ServerMessage::SkillXpGained {
            skill,
            xp_amount,
            total_xp,
            new_level,
            leveled_up,
        } => {
            assert_eq!(skill, SkillId::Fishing);
            assert_eq!(xp_amount, 150);
            assert_eq!(total_xp, 150);
            assert_eq!(new_level, 1);
            assert!(leveled_up);
        }
        other => panic!("expected SkillXpGained, got {other:?}"),
    }

    // The dirty flush picks the player up exactly once, with the saved rows
    // and the drained id for the failed-save retry path.
    let (dirty_ids, dirty) = game_state.collect_dirty_skill_states().await;
    assert_eq!(dirty_ids, vec![player]);
    assert_eq!(dirty.len(), 1);
    assert_eq!(dirty[0].0, 42);
    assert_eq!(dirty[0].1.len(), 1);
    assert_eq!(dirty[0].1[0].skill_id, "fishing");
    assert_eq!(dirty[0].1[0].xp, 150);
    assert!(game_state.collect_dirty_skill_states().await.1.is_empty());
    // A failed save puts the dirtiness back for the next flush.
    game_state.restore_dirty_skills(vec![player]).await;
    assert_eq!(game_state.collect_dirty_skill_states().await.1.len(), 1);

    // Cap out: the clamp reports what was banked, then further grants no-op
    // (no message, no dirty flag).
    game_state
        .add_skill_xp(&player, SkillId::Fishing, u64::MAX)
        .await
        .expect("clamped grant still applies");
    assert!(game_state
        .add_skill_xp(&player, SkillId::Fishing, 10)
        .await
        .is_none());
    let _ = rx.try_recv().expect("cap-out grant notifies");
    assert!(matches!(rx.try_recv(), Err(MpscTryRecvError::Empty)));
    let (_, dirty) = game_state.collect_dirty_skill_states().await;
    assert_eq!(dirty.len(), 1);
    assert_eq!(dirty[0].1[0].xp, skill_xp_for_level(SKILL_LEVEL_CAP));
    assert_eq!(dirty[0].1[0].level, SKILL_LEVEL_CAP);
}

// Logout detach must snapshot skills for the save and drop the in-memory
// entry, exactly like inventories (F-015 shape).
#[tokio::test]
async fn take_player_skills_snapshots_and_detaches() {
    use onlinerpg_shared::skills::SkillId;

    let game_state = make_test_game_state("skill_detach");
    let player = pid("angler2");
    game_state
        .add_player(make_player("angler2", 0.0, 0.0))
        .await;
    game_state
        .register_player_character(&player, 7, 0, attrs_with_cha(10), 0)
        .await;
    game_state
        .register_player_skills(&player, Default::default())
        .await;
    game_state
        .add_skill_xp(&player, SkillId::Fishing, 600)
        .await
        .unwrap();

    let (character_id, rows) = game_state
        .take_player_skills(&player)
        .await
        .expect("skills should detach");
    assert_eq!(character_id, 7);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].skill_id, "fishing");
    assert_eq!(rows[0].xp, 600);
    assert_eq!(rows[0].level, 2);

    // Detached: nothing left to take or flush.
    assert!(game_state.take_player_skills(&player).await.is_none());
    assert!(game_state.collect_dirty_skill_states().await.1.is_empty());
}

// ---- Fishing (doc/FISHING.md) ----------------------------------------------
// Paused-time tests: tokio's clock is frozen, `time::advance` moves it, and
// `tick_fishing()` is driven by hand — the state machine runs deterministically.

mod fishing_tests {
    use super::*;
    use onlinerpg_shared::fishing::{
        FishingAction, FishingOutcome, BITE_WINDOW_MS, CAST_MS, ESCAPE_XP, LATENCY_GRACE_MS,
        WAIT_MAX_MS, WAIT_MIN_MS,
    };
    use onlinerpg_shared::inventory::EquipSlot;
    use tokio::time::{advance, Duration};

    /// Player on the shore of the test world's western sea (negative x is
    /// 5 m underwater in `SplitWorldTiles`), rod equipped, ready to cast.
    async fn make_angler(
        game_state: &GameState,
        name: &str,
    ) -> (
        PlayerId,
        tokio::sync::mpsc::UnboundedReceiver<ServerMessage>,
    ) {
        let id = pid(name);
        game_state.add_player(make_player(name, -100.0, 50.0)).await;
        let mut equipped = std::collections::HashMap::new();
        equipped.insert(EquipSlot::MainHand, bag_item(999, "fishing_rod", 1));
        game_state.inventories.write().await.insert(
            id,
            PlayerInventory {
                bag: vec![],
                equipped,
            },
        );
        game_state
            .register_player_character(&id, 1, 0, attrs_with_cha(10), 0)
            .await;
        game_state
            .register_player_skills(&id, Default::default())
            .await;
        let rx = game_state.register_direct_channel(&id).await;
        (id, rx)
    }

    fn water_target() -> Position {
        Position {
            x: -103.0,
            y: 0.0,
            z: 50.0,
        }
    }

    fn drain(rx: &mut tokio::sync::mpsc::UnboundedReceiver<ServerMessage>) -> Vec<ServerMessage> {
        let mut msgs = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            msgs.push(msg);
        }
        msgs
    }

    /// Advance paused time in tick-sized steps, running the fishing tick at
    /// each step — the paused-clock equivalent of the 250 ms `run_ticks` task.
    async fn advance_with_ticks(game_state: &GameState, total_ms: u64) {
        let mut remaining = total_ms;
        while remaining > 0 {
            let step = remaining.min(250);
            advance(Duration::from_millis(step)).await;
            game_state.tick_fishing().await;
            remaining -= step;
        }
    }

    /// Tick forward only until the bite fires (the wait is a random roll —
    /// blindly advancing the full range would blow through the bite window
    /// whenever the roll came up short). Panics if no bite arrives within
    /// the cast plus the maximum wait.
    async fn advance_until_bite(
        game_state: &GameState,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<ServerMessage>,
    ) {
        let budget_ms = u64::from(CAST_MS) + u64::from(WAIT_MAX_MS) + 500;
        let mut elapsed = 0;
        while elapsed < budget_ms {
            advance(Duration::from_millis(250)).await;
            game_state.tick_fishing().await;
            elapsed += 250;
            if drain(rx)
                .iter()
                .any(|m| matches!(m, ServerMessage::FishingBite { .. }))
            {
                return;
            }
        }
        panic!("no bite within the cast + maximum wait");
    }

    /// Play the struggle with a fixed strategy until the session ends,
    /// returning the outcome (plus every message seen on the way). The
    /// strategy sees each round's announced `fish_state` — exactly what a
    /// real client (human UI or agent reflex) gets.
    async fn fight_to_the_end(
        game_state: &GameState,
        id: &PlayerId,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<ServerMessage>,
        strategy: impl Fn(onlinerpg_shared::fishing::FishState) -> FishingAction,
    ) -> (FishingOutcome, Vec<ServerMessage>) {
        let mut seen = Vec::new();
        for _ in 0..200 {
            for msg in drain(rx) {
                match &msg {
                    ServerMessage::FishingStruggleRound {
                        player_id,
                        fish_state,
                        ..
                    } if player_id == id => {
                        let action = strategy(*fish_state);
                        seen.push(msg.clone());
                        game_state.respond_fishing(id, action).await;
                    }
                    ServerMessage::FishingEnded { outcome, .. } => {
                        seen.push(msg.clone());
                        return (outcome.clone(), seen);
                    }
                    _ => seen.push(msg.clone()),
                }
            }
            advance(Duration::from_millis(250)).await;
            game_state.tick_fishing().await;
        }
        panic!("struggle never ended");
    }

    #[tokio::test(start_paused = true)]
    async fn cast_requires_rod_water_and_range() {
        let game_state = make_test_game_state("fishing_cast_validation");
        let (id, mut rx) = make_angler(&game_state, "angler_val").await;

        // Land (positive x in the split world) is refused.
        game_state
            .start_fishing(
                &id,
                Position {
                    x: 200.0,
                    y: 0.0,
                    z: 50.0,
                },
            )
            .await;
        // Out of range (>8m) is refused even over water.
        game_state
            .start_fishing(
                &id,
                Position {
                    x: -150.0,
                    y: 0.0,
                    z: 50.0,
                },
            )
            .await;
        let errors = drain(&mut rx)
            .into_iter()
            .filter(|m| matches!(m, ServerMessage::FishingError { .. }))
            .count();
        assert_eq!(errors, 2);

        // No rod → refused.
        let bare = pid("angler_bare");
        game_state
            .add_player(make_player("angler_bare", -100.0, 50.0))
            .await;
        game_state.inventories.write().await.insert(
            bare,
            PlayerInventory {
                bag: vec![],
                equipped: Default::default(),
            },
        );
        let mut bare_rx = game_state.register_direct_channel(&bare).await;
        game_state.start_fishing(&bare, water_target()).await;
        assert!(drain(&mut bare_rx)
            .iter()
            .any(|m| matches!(m, ServerMessage::FishingError { .. })));

        // Rod + water + in range → the cast broadcast reaches the angler.
        game_state.start_fishing(&id, water_target()).await;
        assert!(drain(&mut rx)
            .iter()
            .any(|m| matches!(m, ServerMessage::FishingCasted { .. })));
    }

    // The regression this PR fixes: a river's bed sits ABOVE sea level (its
    // carved channel bottoms out at sea level and climbs into the hills), so
    // the old `terrain height < 0` water test rejected every inland river.
    // With the unified water field, a river surface above its bed reads as
    // water and the cast lands — end to end, all the way to a caught fish.
    #[tokio::test(start_paused = true)]
    async fn fishing_works_in_a_river_above_sea_level() {
        let game_state = make_river_game_state("fishing_river");
        // Plateau terrain is +5 m; the player and the water are both up there.
        let (id, mut rx) = make_angler(&game_state, "angler_river").await;

        game_state.start_fishing(&id, water_target()).await;
        let casted = drain(&mut rx);
        let bobber = casted.iter().find_map(|m| match m {
            ServerMessage::FishingCasted { position, .. } => Some(*position),
            _ => None,
        });
        let bobber = bobber.expect("river cast should be accepted, not refused as land");
        // The bobber floats on the river surface (~5.4 m), not at sea level.
        assert!(
            bobber.y > 5.0,
            "bobber should sit on the river surface, got y={}",
            bobber.y
        );

        // And the whole loop still resolves to a catch over the river.
        advance_until_bite(&game_state, &mut rx).await;
        game_state.respond_fishing(&id, FishingAction::Hook).await;
        let (outcome, _) =
            fight_to_the_end(&game_state, &id, &mut rx, |state| state.correct_action()).await;
        assert!(
            matches!(outcome, FishingOutcome::Caught { .. }),
            "perfect play should land a river fish, got {outcome:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn full_catch_flow_awards_fish_and_skill_xp() {
        let game_state = make_test_game_state("fishing_catch_flow");
        let (id, mut rx) = make_angler(&game_state, "angler_catch").await;

        game_state.start_fishing(&id, water_target()).await;
        advance_until_bite(&game_state, &mut rx).await;

        game_state.respond_fishing(&id, FishingAction::Hook).await;
        // Answer every struggle round correctly: the catch must land.
        let (outcome, msgs) =
            fight_to_the_end(&game_state, &id, &mut rx, |state| state.correct_action()).await;
        let FishingOutcome::Caught {
            item_def_id: fish_id,
            size_cm,
            ..
        } = outcome
        else {
            panic!("perfect struggle play must catch, got {outcome:?}");
        };
        // The fight announced at least the first round and its results.
        assert!(msgs
            .iter()
            .any(|m| matches!(m, ServerMessage::FishingStruggleRound { .. })));
        assert!(msgs
            .iter()
            .any(|m| matches!(m, ServerMessage::FishingRoundResult { correct: true, .. })));
        assert!(size_cm > 0);
        // What lands depends on the species: coin catches pay copper, items
        // (fish and junk) land in the bag; only rarity ≥ 1 (fish) grants XP.
        let defs = ItemDefs::load();
        let def = defs.get(&fish_id).expect("caught def");
        if def.is_coin_catch() {
            assert!(
                msgs.iter().any(|m| matches!(
                    m,
                    ServerMessage::GoldGained { amount } if *amount > 0
                )),
                "coin catch must pay copper"
            );
        } else {
            let inv = game_state.get_player_inventory(&id).await.unwrap();
            assert!(inv
                .bag
                .iter()
                .any(|item| item.item_def_id == fish_id && item.quantity == 1));
            assert!(msgs
                .iter()
                .any(|m| matches!(m, ServerMessage::InventoryUpdated { .. })));
        }
        let rarity = def.rarity_tier.unwrap_or(1);
        let got_xp = msgs.iter().any(|m| {
            matches!(
                m,
                ServerMessage::SkillXpGained { xp_amount, .. } if *xp_amount >= 10
            )
        });
        assert_eq!(
            got_xp,
            rarity >= 1,
            "fish grant XP, junk and coins do not (caught {fish_id}, rarity {rarity})"
        );
        // Session is gone: a second hook is an error, not a double catch.
        game_state.respond_fishing(&id, FishingAction::Hook).await;
        assert!(drain(&mut rx)
            .iter()
            .any(|m| matches!(m, ServerMessage::FishingError { .. })));
    }

    #[tokio::test(start_paused = true)]
    async fn ignored_bite_escapes_with_consolation_xp() {
        let game_state = make_test_game_state("fishing_bite_timeout");
        let (id, mut rx) = make_angler(&game_state, "angler_afk").await;

        game_state.start_fishing(&id, water_target()).await;
        advance_until_bite(&game_state, &mut rx).await;

        // Sleep through the bite window plus both grace budgets.
        advance_with_ticks(
            &game_state,
            u64::from(BITE_WINDOW_MS + 2 * LATENCY_GRACE_MS + 500),
        )
        .await;
        let msgs = drain(&mut rx);
        assert!(msgs.iter().any(|m| matches!(
            m,
            ServerMessage::FishingEnded {
                outcome: FishingOutcome::Escaped,
                ..
            }
        )));
        assert!(msgs.iter().any(|m| matches!(
            m,
            ServerMessage::SkillXpGained { xp_amount, .. } if *xp_amount == 2
        )));
    }

    #[tokio::test(start_paused = true)]
    async fn hooking_early_scares_the_fish_off_without_xp() {
        let game_state = make_test_game_state("fishing_early_hook");
        let (id, mut rx) = make_angler(&game_state, "angler_eager").await;

        game_state.start_fishing(&id, water_target()).await;
        advance_with_ticks(&game_state, u64::from(CAST_MS) + 500).await;
        drain(&mut rx);

        game_state.respond_fishing(&id, FishingAction::Hook).await;
        let msgs = drain(&mut rx);
        assert!(msgs.iter().any(|m| matches!(
            m,
            ServerMessage::FishingEnded {
                outcome: FishingOutcome::Escaped,
                ..
            }
        )));
        assert!(!msgs
            .iter()
            .any(|m| matches!(m, ServerMessage::SkillXpGained { .. })));
    }

    #[tokio::test(start_paused = true)]
    async fn moving_aborts_the_session() {
        let game_state = make_test_game_state("fishing_move_cancels");
        let (id, mut rx) = make_angler(&game_state, "angler_wanderer").await;

        game_state.start_fishing(&id, water_target()).await;
        advance_with_ticks(&game_state, u64::from(CAST_MS) + 500).await;
        drain(&mut rx);

        // The connection layer cancels on any PlayerMove; this is that hook.
        game_state.cancel_fishing_if_active(&id).await;
        assert!(drain(&mut rx).iter().any(|m| matches!(
            m,
            ServerMessage::FishingEnded {
                outcome: FishingOutcome::Aborted,
                ..
            }
        )));
        // Idempotent: cancelling again stays quiet.
        game_state.cancel_fishing_if_active(&id).await;
        assert!(drain(&mut rx).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn caught_fish_stack_in_the_bag() {
        let game_state = make_test_game_state("fishing_stacks");
        let (id, mut rx) = make_angler(&game_state, "angler_stacker").await;

        let defs = ItemDefs::load();
        let mut bagged_catches = 0u32;
        for _ in 0..3 {
            game_state.start_fishing(&id, water_target()).await;
            advance_until_bite(&game_state, &mut rx).await;
            game_state.respond_fishing(&id, FishingAction::Hook).await;
            let (outcome, _) =
                fight_to_the_end(&game_state, &id, &mut rx, |state| state.correct_action()).await;
            let FishingOutcome::Caught { item_def_id, .. } = outcome else {
                panic!("perfect play must catch");
            };
            // Coin catches pay copper instead of taking a bag slot.
            if !defs.get(&item_def_id).unwrap().is_coin_catch() {
                bagged_catches += 1;
            }
        }
        let inv = game_state.get_player_inventory(&id).await.unwrap();
        let total_fish: u32 = inv.bag.iter().map(|item| item.quantity).sum();
        assert_eq!(total_fish, bagged_catches);
        // Stacking means fewer entries than catches unless every species
        // differed; either way no entry may duplicate another's def id.
        let mut ids: Vec<_> = inv.bag.iter().map(|i| i.item_def_id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), inv.bag.len(), "one bag entry per species");
    }

    // The struggle: wrong answers pump tension until the fish escapes;
    // silence is reaped by the tick with the same penalty.
    #[tokio::test(start_paused = true)]
    async fn struggle_wrong_answers_escape_the_fish() {
        use onlinerpg_shared::fishing::FishState;

        let game_state = make_test_game_state("fishing_struggle_wrong");
        let (id, mut rx) = make_angler(&game_state, "angler_clumsy").await;

        game_state.start_fishing(&id, water_target()).await;
        advance_until_bite(&game_state, &mut rx).await;
        game_state.respond_fishing(&id, FishingAction::Hook).await;

        // Always answer with the wrong action: minimum penalty 35/round means
        // escape by the third miss at the latest.
        let (outcome, msgs) = fight_to_the_end(&game_state, &id, &mut rx, |state| match state {
            FishState::Pulling => FishingAction::Reel,
            FishState::Tiring => FishingAction::GiveLine,
        })
        .await;
        assert_eq!(outcome, FishingOutcome::Escaped);
        let wrongs = msgs
            .iter()
            .filter(|m| matches!(m, ServerMessage::FishingRoundResult { correct: false, .. }))
            .count();
        assert!(wrongs >= 2, "tension needs at least two misses to top out");
    }

    #[tokio::test(start_paused = true)]
    async fn struggle_silence_is_reaped_round_by_round() {
        let game_state = make_test_game_state("fishing_struggle_afk");
        let (id, mut rx) = make_angler(&game_state, "angler_frozen").await;

        game_state.start_fishing(&id, water_target()).await;
        advance_until_bite(&game_state, &mut rx).await;
        game_state.respond_fishing(&id, FishingAction::Hook).await;

        // Never respond; just let the tick reap every round.
        let mut msgs = Vec::new();
        let mut outcome = None;
        'outer: for _ in 0..400 {
            for msg in {
                let mut v = Vec::new();
                while let Ok(m) = rx.try_recv() {
                    v.push(m);
                }
                v
            } {
                if let ServerMessage::FishingEnded { outcome: o, .. } = &msg {
                    outcome = Some(o.clone());
                    msgs.push(msg);
                    break 'outer;
                }
                msgs.push(msg);
            }
            advance(Duration::from_millis(250)).await;
            game_state.tick_fishing().await;
        }
        assert_eq!(outcome, Some(FishingOutcome::Escaped));
        assert!(msgs
            .iter()
            .any(|m| matches!(m, ServerMessage::FishingRoundResult { correct: false, .. })));
    }

    // ---- PR9 hardening: concurrency, radius, aborts, overflow, consumption ----

    /// Two sessions live in the same map at once; each angler's bites and rounds
    /// are answered only by their owner, and both land their own catch with
    /// their own XP. Locks the per-player session isolation.
    #[tokio::test(start_paused = true)]
    async fn two_anglers_fish_independently() {
        let game_state = make_test_game_state("fishing_two_anglers");
        let (a, mut rx_a) = make_angler(&game_state, "angler_left").await;
        let (b, mut rx_b) = make_angler(&game_state, "angler_right").await;

        game_state.start_fishing(&a, water_target()).await;
        game_state.start_fishing(&b, water_target()).await;

        let mut ended: std::collections::HashMap<PlayerId, FishingOutcome> = Default::default();
        let mut xp: std::collections::HashMap<PlayerId, u64> = Default::default();
        let mut caught: std::collections::HashMap<PlayerId, String> = Default::default();
        for _ in 0..400 {
            if ended.len() == 2 {
                break;
            }
            for (me, rx) in [(a, &mut rx_a), (b, &mut rx_b)] {
                for msg in drain(rx) {
                    match msg {
                        ServerMessage::FishingBite { player_id } if player_id == me => {
                            game_state.respond_fishing(&me, FishingAction::Hook).await;
                        }
                        ServerMessage::FishingStruggleRound {
                            player_id,
                            fish_state,
                            ..
                        } if player_id == me => {
                            game_state
                                .respond_fishing(&me, fish_state.correct_action())
                                .await;
                        }
                        ServerMessage::SkillXpGained { xp_amount, .. } => {
                            *xp.entry(me).or_default() += xp_amount;
                        }
                        ServerMessage::FishingEnded { player_id, outcome } if player_id == me => {
                            if let FishingOutcome::Caught {
                                ref item_def_id, ..
                            } = outcome
                            {
                                caught.insert(me, item_def_id.clone());
                            }
                            ended.insert(me, outcome);
                        }
                        _ => {}
                    }
                }
            }
            advance(Duration::from_millis(250)).await;
            game_state.tick_fishing().await;
        }

        assert_eq!(ended.len(), 2, "both anglers must finish their sessions");
        for (me, outcome) in &ended {
            assert!(
                matches!(outcome, FishingOutcome::Caught { .. }),
                "correct play must land the catch for {me}: {outcome:?}"
            );
            // XP mirrors the species each angler independently drew: fish grant
            // it, rarity-0 flotsam does not.
            let is_fish = game_state
                .item_defs
                .get(caught.get(me).expect("caught id"))
                .expect("caught def")
                .is_fish();
            assert_eq!(
                xp.get(me).copied().unwrap_or(0) > 0,
                is_fish,
                "skill XP must match what {me} caught ({:?})",
                caught.get(me)
            );
        }
    }

    /// Responding with no session gets a direct error and must not touch
    /// anyone else's line.
    #[tokio::test(start_paused = true)]
    async fn responding_without_a_session_is_an_error_and_touches_nobody() {
        let game_state = make_test_game_state("fishing_kibitzer");
        let (a, mut rx_a) = make_angler(&game_state, "angler_focused").await;
        let b = pid("kibitzer");
        game_state
            .add_player(make_player("kibitzer", -100.0, 50.0))
            .await;
        let mut rx_b = game_state.register_direct_channel(&b).await;

        game_state.start_fishing(&a, water_target()).await;
        game_state.respond_fishing(&b, FishingAction::Hook).await;
        assert!(
            drain(&mut rx_b)
                .iter()
                .any(|m| matches!(m, ServerMessage::FishingError { .. })),
            "the kibitzer is told they are not fishing"
        );

        // The angler's session is unaffected: bite, fight, catch as normal.
        advance_until_bite(&game_state, &mut rx_a).await;
        game_state.respond_fishing(&a, FishingAction::Hook).await;
        let (outcome, _) =
            fight_to_the_end(&game_state, &a, &mut rx_a, |state| state.correct_action()).await;
        assert!(matches!(outcome, FishingOutcome::Caught { .. }));
    }

    /// Fishing broadcasts reach a bystander on the shore but not someone beyond
    /// the delivery radius — bobbers render for neighbors, not the whole server.
    #[tokio::test(start_paused = true)]
    async fn fishing_broadcasts_are_radius_gated() {
        let game_state = make_test_game_state("fishing_radius");
        let (a, mut rx_a) = make_angler(&game_state, "angler_star").await;
        // Bobber lands at (-103, 50); 43 m delivery radius (shared::world).
        let near = pid("bystander_near");
        game_state
            .add_player(make_player("bystander_near", -90.0, 50.0))
            .await;
        let mut rx_near = game_state.register_direct_channel(&near).await;
        let far = pid("bystander_far");
        game_state
            .add_player(make_player("bystander_far", -40.0, 50.0))
            .await;
        let mut rx_far = game_state.register_direct_channel(&far).await;

        game_state.start_fishing(&a, water_target()).await;
        advance_until_bite(&game_state, &mut rx_a).await;
        game_state.respond_fishing(&a, FishingAction::Hook).await;
        let (outcome, _) =
            fight_to_the_end(&game_state, &a, &mut rx_a, |state| state.correct_action()).await;
        assert!(matches!(outcome, FishingOutcome::Caught { .. }));

        fn fishing_msg_count(msgs: &[ServerMessage]) -> usize {
            msgs.iter()
                .filter(|m| {
                    matches!(
                        m,
                        ServerMessage::FishingCasted { .. }
                            | ServerMessage::FishingBite { .. }
                            | ServerMessage::FishingStruggleRound { .. }
                            | ServerMessage::FishingRoundResult { .. }
                            | ServerMessage::FishingEnded { .. }
                    )
                })
                .count()
        }
        // The near bystander sees every phase — not just the cast. Asserting
        // each kind separately keeps a partial regression (e.g. the fight going
        // direct-only) from hiding behind the bobber broadcast.
        let near_msgs = drain(&mut rx_near);
        for (name, seen) in [
            (
                "FishingCasted",
                near_msgs
                    .iter()
                    .any(|m| matches!(m, ServerMessage::FishingCasted { .. })),
            ),
            (
                "FishingBite",
                near_msgs
                    .iter()
                    .any(|m| matches!(m, ServerMessage::FishingBite { .. })),
            ),
            (
                "FishingStruggleRound",
                near_msgs
                    .iter()
                    .any(|m| matches!(m, ServerMessage::FishingStruggleRound { .. })),
            ),
            (
                "FishingEnded",
                near_msgs
                    .iter()
                    .any(|m| matches!(m, ServerMessage::FishingEnded { .. })),
            ),
        ] {
            assert!(seen, "a 13 m bystander must see {name}");
        }
        assert_eq!(
            fishing_msg_count(&drain(&mut rx_far)),
            0,
            "a 63 m player hears nothing"
        );
    }

    /// The real kill chain: a lethal monster blow must run the death chokepoint
    /// and abort the victim's fishing session. This drives `did_die` through
    /// `broadcast_monster_attack`, so rewiring the combat path away from
    /// `on_player_died` fails here (the direct-call test below only locks the
    /// chokepoint's contents, not this wiring — a mutation test proved that).
    #[tokio::test(start_paused = true)]
    async fn a_lethal_blow_reels_in_the_line() {
        let game_state = make_test_game_state("fishing_combat_death");
        let (id, mut rx) = make_angler(&game_state, "angler_mauled").await;
        let handler = pid("beast_handler");
        game_state
            .add_player(make_player("beast_handler", -99.0, 50.0))
            .await;
        // One hit kills; misses are re-rolled on fresh monsters (each has its
        // own attack cooldown), so the kill is effectively certain.
        game_state
            .players
            .write()
            .await
            .get_mut(&id)
            .unwrap()
            .health = 1;
        {
            let mut monsters = game_state.monsters.write().await;
            for i in 0..20 {
                let mid = format!("killer_{i}");
                let mut monster = make_monster(
                    &mid,
                    Position {
                        x: -99.0,
                        y: 0.0,
                        z: 50.0,
                    },
                    0,
                );
                monster.owner_id = Some(handler);
                monsters.insert(mid, monster);
            }
        }

        game_state.start_fishing(&id, water_target()).await;
        advance_with_ticks(&game_state, u64::from(CAST_MS) + 250).await;

        for i in 0..20 {
            game_state
                .broadcast_monster_attack(&handler, &format!("killer_{i}"), &id)
                .await;
            if game_state.players.read().await.get(&id).unwrap().health == 0 {
                break;
            }
        }
        assert_eq!(
            game_state.players.read().await.get(&id).unwrap().health,
            0,
            "twenty adjacent swings at 1 HP must land a kill"
        );
        assert!(
            drain(&mut rx).iter().any(|m| matches!(
                m,
                ServerMessage::FishingEnded {
                    outcome: FishingOutcome::Aborted,
                    ..
                }
            )),
            "a combat death must abort the fishing session through the real kill chain"
        );
    }

    /// Dying reels the line in: the death chokepoint aborts the session and no
    /// bite ever reaches the corpse.
    #[tokio::test(start_paused = true)]
    async fn death_reels_in_the_line() {
        let game_state = make_test_game_state("fishing_death");
        let (id, mut rx) = make_angler(&game_state, "angler_doomed").await;

        game_state.start_fishing(&id, water_target()).await;
        advance_with_ticks(&game_state, u64::from(CAST_MS) + 250).await;
        game_state.on_player_died(&id).await;

        assert!(
            drain(&mut rx).iter().any(|m| matches!(
                m,
                ServerMessage::FishingEnded {
                    outcome: FishingOutcome::Aborted,
                    ..
                }
            )),
            "death must abort the session"
        );
        // Session is gone for real: even the longest wait produces no bite.
        advance_with_ticks(&game_state, u64::from(WAIT_MAX_MS) + 1000).await;
        assert!(
            drain(&mut rx)
                .iter()
                .all(|m| !matches!(m, ServerMessage::FishingBite { .. })),
            "no bite may reach a dead angler"
        );
    }

    /// Putting the rod away reels the line in.
    #[tokio::test(start_paused = true)]
    async fn unequipping_the_rod_aborts_the_session() {
        let game_state = make_test_game_state("fishing_unequip");
        let (id, mut rx) = make_angler(&game_state, "angler_fickle").await;

        game_state.start_fishing(&id, water_target()).await;
        advance_with_ticks(&game_state, u64::from(CAST_MS) + 250).await;
        game_state.unequip_item(&id, EquipSlot::MainHand).await;

        assert!(
            drain(&mut rx).iter().any(|m| matches!(
                m,
                ServerMessage::FishingEnded {
                    outcome: FishingOutcome::Aborted,
                    ..
                }
            )),
            "stowing the rod must abort the session"
        );
    }

    /// Swapping a weapon into the main hand displaces the rod — same abort.
    #[tokio::test(start_paused = true)]
    async fn swapping_a_weapon_into_the_main_hand_aborts_the_session() {
        let game_state = make_test_game_state("fishing_swap");
        let (id, mut rx) = make_angler(&game_state, "angler_armed").await;
        game_state
            .inventories
            .write()
            .await
            .get_mut(&id)
            .unwrap()
            .bag
            .push(bag_item(1000, "iron_sword", 1));

        game_state.start_fishing(&id, water_target()).await;
        advance_with_ticks(&game_state, u64::from(CAST_MS) + 250).await;
        game_state.equip_item(&id, 1000).await;

        let main_hand = game_state
            .inventories
            .read()
            .await
            .get(&id)
            .unwrap()
            .equipped
            .get(&EquipSlot::MainHand)
            .unwrap()
            .item_def_id
            .clone();
        assert_eq!(main_hand, "iron_sword", "the sword displaced the rod");
        assert!(
            drain(&mut rx).iter().any(|m| matches!(
                m,
                ServerMessage::FishingEnded {
                    outcome: FishingOutcome::Aborted,
                    ..
                }
            )),
            "losing the rod to a swap must abort the session"
        );
    }

    /// Gear changes that leave the rod alone don't break concentration — putting
    /// on a cap mid-wait keeps the line wet and the bite still arrives.
    #[tokio::test(start_paused = true)]
    async fn equipping_armor_does_not_break_concentration() {
        let game_state = make_test_game_state("fishing_armor");
        let (id, mut rx) = make_angler(&game_state, "angler_dapper").await;
        game_state
            .inventories
            .write()
            .await
            .get_mut(&id)
            .unwrap()
            .bag
            .push(bag_item(1001, "leather_cap", 1));

        game_state.start_fishing(&id, water_target()).await;
        advance_with_ticks(&game_state, u64::from(CAST_MS) + 250).await;
        game_state.equip_item(&id, 1001).await;

        assert!(
            drain(&mut rx)
                .iter()
                .all(|m| !matches!(m, ServerMessage::FishingEnded { .. })),
            "a hat must not abort the session"
        );
        // Still fishing: the bite arrives (advance_until_bite panics otherwise).
        advance_until_bite(&game_state, &mut rx).await;
    }

    /// An overweight bag spills the catch as a ground item — never silently lost.
    #[tokio::test]
    async fn a_full_bag_spills_the_catch_on_the_ground() {
        let game_state = make_test_game_state("fishing_spill");
        let (id, mut rx) = make_angler(&game_state, "angler_hoarder").await;
        // 150 torches = the full STR-10 carry allowance before the catch.
        game_state
            .inventories
            .write()
            .await
            .get_mut(&id)
            .unwrap()
            .bag
            .push(bag_item(700, "torch", 150));

        game_state.award_stackable_item(&id, "raw_minnow").await;

        assert!(
            drain(&mut rx).iter().any(|m| matches!(
                m,
                ServerMessage::GroundItemSpawned { item, .. } if item.item_def_id == "raw_minnow"
            )),
            "the overflow catch must spill to the ground"
        );
        assert!(
            game_state
                .inventories
                .read()
                .await
                .get(&id)
                .unwrap()
                .bag
                .iter()
                .all(|i| i.item_def_id != "raw_minnow"),
            "nothing was bagged"
        );
        assert!(
            game_state
                .ground_items
                .read()
                .await
                .values()
                .any(|g| g.item.item_def_id == "raw_minnow"),
            "the fish is on the ground, pickable"
        );
    }

    /// `category "fish"` rides the same heal path as potions: eating a trout
    /// restores its dice and consumes the fish.
    #[tokio::test]
    async fn eating_a_fish_heals_like_a_potion() {
        let game_state = make_test_game_state("fishing_eat");
        let (id, mut rx) = make_angler(&game_state, "angler_hungry").await;
        game_state
            .inventories
            .write()
            .await
            .get_mut(&id)
            .unwrap()
            .bag
            .push(bag_item(800, "raw_trout", 1));
        game_state
            .players
            .write()
            .await
            .get_mut(&id)
            .unwrap()
            .health = 2;

        game_state.use_item(&id, 800).await;

        let health = game_state.players.read().await.get(&id).unwrap().health;
        assert!(
            (4..=10).contains(&health),
            "2d4 heal from 2 HP lands in 4..=10 (capped), got {health}"
        );
        assert!(
            game_state
                .inventories
                .read()
                .await
                .get(&id)
                .unwrap()
                .bag
                .iter()
                .all(|i| i.item_def_id != "raw_trout"),
            "the fish was eaten"
        );
        let _ = drain(&mut rx);
    }

    /// The pure catch-table math: every roll in range maps to a candidate, the
    /// boundaries are exact, and skill shifts weight toward rare fish while
    /// never boosting rarity-0 junk.
    #[test]
    fn pick_catch_maps_every_roll_and_skill_shifts_toward_rare_fish() {
        use crate::game_state::fishing::{effective_weight, pick_catch, CatchCandidate};

        let candidates = vec![
            CatchCandidate {
                item_def_id: "common".into(),
                rarity: 1,
                catch_weight: 50,
            },
            CatchCandidate {
                item_def_id: "rare".into(),
                rarity: 5,
                catch_weight: 2,
            },
            CatchCandidate {
                item_def_id: "junk".into(),
                rarity: 0,
                catch_weight: 8,
            },
        ];

        let total: u64 = candidates.iter().map(|c| effective_weight(c, 0)).sum();
        assert_eq!(total, 60);
        for roll in 0..total {
            let idx = pick_catch(&candidates, 0, roll).expect("every in-range roll lands");
            assert!(idx < candidates.len());
        }
        assert!(
            pick_catch(&candidates, 0, total).is_none(),
            "an out-of-range roll picks nothing"
        );
        // Exact boundaries of the cumulative walk.
        assert_eq!(pick_catch(&candidates, 0, 0), Some(0));
        assert_eq!(pick_catch(&candidates, 0, 49), Some(0));
        assert_eq!(pick_catch(&candidates, 0, 50), Some(1));
        assert_eq!(pick_catch(&candidates, 0, 51), Some(1));
        assert_eq!(pick_catch(&candidates, 0, 52), Some(2));
        assert_eq!(pick_catch(&candidates, 0, 59), Some(2));
        // Skill adds level × rarity: the rare fish gains the most relative
        // weight, junk (rarity 0) gains none.
        assert_eq!(effective_weight(&candidates[0], 10), 60);
        assert_eq!(effective_weight(&candidates[1], 10), 52);
        assert_eq!(effective_weight(&candidates[2], 10), 8);
    }

    /// Dropping the equipped rod on the ground mid-session is "putting it away"
    /// too — the drop path must run the same abort as unequip (this was a real
    /// hole: only equip/unequip called the hook).
    #[tokio::test(start_paused = true)]
    async fn dropping_the_equipped_rod_aborts_the_session() {
        let game_state = make_test_game_state("fishing_drop_rod");
        let (id, mut rx) = make_angler(&game_state, "angler_butterfingers").await;

        game_state.start_fishing(&id, water_target()).await;
        advance_with_ticks(&game_state, u64::from(CAST_MS) + 250).await;
        // 999 is the rod's instance id from make_angler.
        game_state.drop_item(&id, 999).await;

        assert!(
            drain(&mut rx).iter().any(|m| matches!(
                m,
                ServerMessage::FishingEnded {
                    outcome: FishingOutcome::Aborted,
                    ..
                }
            )),
            "dropping the rod must abort the session"
        );
        assert!(
            game_state
                .ground_items
                .read()
                .await
                .values()
                .any(|g| g.item.item_def_id == "fishing_rod"),
            "the rod itself lands on the ground"
        );
    }

    /// A deliberate reel-in mid-struggle ends Aborted and forfeits everything:
    /// no consolation XP, no item, no coins — abandoning a hooked fish is the
    /// one outcome that pays nothing.
    #[tokio::test(start_paused = true)]
    async fn stopping_mid_struggle_forfeits_everything() {
        let game_state = make_test_game_state("fishing_stop");
        let (id, mut rx) = make_angler(&game_state, "angler_quitter").await;

        game_state.start_fishing(&id, water_target()).await;
        advance_until_bite(&game_state, &mut rx).await;
        game_state.respond_fishing(&id, FishingAction::Hook).await;
        let _ = drain(&mut rx); // clear the hook confirmation / first round
        game_state.stop_fishing(&id).await;

        let msgs = drain(&mut rx);
        assert!(
            msgs.iter().any(|m| matches!(
                m,
                ServerMessage::FishingEnded {
                    outcome: FishingOutcome::Aborted,
                    ..
                }
            )),
            "stop mid-struggle ends Aborted"
        );
        assert!(
            msgs.iter()
                .all(|m| !matches!(m, ServerMessage::SkillXpGained { .. })),
            "abandoning a hooked fish pays no consolation XP"
        );
        assert!(
            msgs.iter()
                .all(|m| !matches!(m, ServerMessage::InventoryUpdated { .. })
                    && !matches!(m, ServerMessage::GoldGained { .. })),
            "no item or coins are awarded on a deliberate stop"
        );
    }

    /// Hooking after the window plus grace has passed is TooLate: the fish
    /// escapes with the consolation XP. Locks the three-tier lag contract
    /// (in time → hooked; past grace → escaped; the tick reaper only at 2×grace).
    #[tokio::test(start_paused = true)]
    async fn a_late_hook_escapes_with_consolation_xp() {
        let game_state = make_test_game_state("fishing_late_hook");
        let (id, mut rx) = make_angler(&game_state, "angler_asleep").await;

        game_state.start_fishing(&id, water_target()).await;
        advance_until_bite(&game_state, &mut rx).await;
        // Sleep through the window and the grace — but respond before the tick
        // reaper (which allows 2× grace), so the handler judges the verdict.
        advance(Duration::from_millis(
            u64::from(BITE_WINDOW_MS) + u64::from(LATENCY_GRACE_MS) + 100,
        ))
        .await;
        game_state.respond_fishing(&id, FishingAction::Hook).await;

        let msgs = drain(&mut rx);
        assert!(
            msgs.iter().any(|m| matches!(
                m,
                ServerMessage::FishingEnded {
                    outcome: FishingOutcome::Escaped,
                    ..
                }
            )),
            "a hook past the grace window is TooLate → Escaped"
        );
        assert!(
        msgs.iter().any(
            |m| matches!(m, ServerMessage::SkillXpGained { xp_amount, .. } if *xp_amount == ESCAPE_XP)
        ),
        "the late angler still gets the {ESCAPE_XP} XP consolation"
    );
    }

    /// One line per angler: casting again while a session is live is refused
    /// and leaves the original session running.
    #[tokio::test(start_paused = true)]
    async fn casting_twice_is_refused() {
        let game_state = make_test_game_state("fishing_double_cast");
        let (id, mut rx) = make_angler(&game_state, "angler_greedy").await;

        game_state.start_fishing(&id, water_target()).await;
        game_state.start_fishing(&id, water_target()).await;
        assert!(
            drain(&mut rx)
                .iter()
                .any(|m| matches!(m, ServerMessage::FishingError { .. })),
            "the second cast is refused"
        );
        // The original session is intact: its bite still arrives.
        advance_until_bite(&game_state, &mut rx).await;
    }

    /// The remaining cast validations: a defeated player and a dungeon floor
    /// are both refused with a direct error and no session.
    #[tokio::test(start_paused = true)]
    async fn cast_refused_while_defeated_or_indoors() {
        let game_state = make_test_game_state("fishing_cast_guards");
        let (id, mut rx) = make_angler(&game_state, "angler_unfit").await;

        game_state
            .players
            .write()
            .await
            .get_mut(&id)
            .unwrap()
            .health = 0;
        game_state.start_fishing(&id, water_target()).await;
        assert!(
            drain(&mut rx)
                .iter()
                .any(|m| matches!(m, ServerMessage::FishingError { .. })),
            "a defeated player cannot cast"
        );

        {
            let mut players = game_state.players.write().await;
            let p = players.get_mut(&id).unwrap();
            p.health = 10;
            p.floor_level = 1;
        }
        game_state.start_fishing(&id, water_target()).await;
        assert!(
            drain(&mut rx)
                .iter()
                .any(|m| matches!(m, ServerMessage::FishingError { .. })),
            "no fishing in the dungeon"
        );
        assert!(
            game_state.fishing_sessions.read().await.is_empty(),
            "neither refused cast may leave a session behind"
        );
    }

    /// The shared wallet path behind coin catches and coin piles: credits
    /// accumulate and each award notifies with both GoldUpdate and GoldGained.
    #[tokio::test]
    async fn award_copper_updates_the_wallet_and_notifies() {
        let game_state = make_test_game_state("fishing_wallet");
        let (id, mut rx) = make_angler(&game_state, "angler_paid").await;

        game_state.award_copper(&id, 25).await;
        game_state.award_copper(&id, 10).await;

        let msgs = drain(&mut rx);
        assert!(
            msgs.iter()
                .any(|m| matches!(m, ServerMessage::GoldGained { amount } if *amount == 25)),
            "first credit announced"
        );
        assert!(
            msgs.iter()
                .any(|m| matches!(m, ServerMessage::GoldUpdate { gold } if *gold == 35)),
            "the wallet total accumulates to 35"
        );
    }

    /// The wait roll is pure given an RNG: level 0 spans the shared range,
    /// skill shortens it 2% per level, and the floor holds at half the minimum.
    #[test]
    fn wait_roll_shortens_with_skill_and_respects_the_floor() {
        use crate::game_state::fishing::roll_wait_ms;
        use rand::rngs::StdRng;
        use rand::SeedableRng;

        for seed in 0..50u64 {
            let mut rng = StdRng::seed_from_u64(seed);
            let base = roll_wait_ms(0, &mut rng);
            assert!(
                (u64::from(WAIT_MIN_MS)..=u64::from(WAIT_MAX_MS)).contains(&base),
                "level 0 wait {base} outside the shared range"
            );

            let mut rng = StdRng::seed_from_u64(seed);
            let skilled = roll_wait_ms(20, &mut rng);
            // Same seed → same base draw, shortened to exactly 60%.
            assert_eq!(skilled, base * 60 / 100, "level 20 = 40% shorter");

            let mut rng = StdRng::seed_from_u64(seed);
            assert_eq!(
                roll_wait_ms(50, &mut rng),
                u64::from(WAIT_MIN_MS) / 2,
                "an absurd level bottoms out at half the minimum wait"
            );
        }
    }
}
