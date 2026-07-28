use super::*;

mod buyback_tests;
mod merchant_tests;
mod resident_tests;

// --- Haggling (economy phase 2) ---

fn make_npc(id: &str, name: &str, x: f32, z: f32) -> Player {
    let mut p = make_player(id, x, z);
    p.name = name.to_string();
    p.is_official_npc = true;
    p
}

async fn set_floor(game_state: &GameState, name: &str, floor: i8) {
    game_state
        .players
        .write()
        .await
        .get_mut(&pid(name))
        .unwrap()
        .floor_level = floor;
}

/// Spawn a merchant NPC and a buyer with the given CHA/gold next to each
/// other, returning the buyer's direct-message receiver and the NPC's.
async fn setup_haggle(
    game_state: &GameState,
    cha: u8,
    gold: i64,
) -> (
    UnboundedReceiver<ServerMessage>,
    UnboundedReceiver<ServerMessage>,
) {
    game_state
        .add_player(make_npc("npc_rica", "Rica", 0.0, 0.0))
        .await;
    game_state.add_player(make_player("buyer", 1.0, 0.0)).await;
    game_state
        .register_player_character(&pid("buyer"), 1, 0, attrs_with_cha(cha), gold)
        .await;
    let buyer_rx = game_state.register_direct_channel(&pid("buyer")).await;
    let npc_rx = game_state.register_direct_channel(&pid("npc_rica")).await;
    (buyer_rx, npc_rx)
}
