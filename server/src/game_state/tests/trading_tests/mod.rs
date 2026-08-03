use super::*;

mod batch_tests;
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
async fn setup_haggle(game_state: &GameState, cha: u8, gold: i64) -> (DirectRx, DirectRx) {
    game_state
        .add_player(make_npc("npc_rica", "Rica", 0.0, 0.0))
        .await;
    game_state.add_player(make_player("buyer", 1.0, 0.0)).await;
    game_state
        .register_player_character(&pid("buyer"), 1, 0, attrs_with_cha(cha), gold, None)
        .await;
    let buyer_rx = game_state.register_direct_channel(&pid("buyer")).await;
    let npc_rx = game_state.register_direct_channel(&pid("npc_rica")).await;
    (buyer_rx, npc_rx)
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
        .add_player(make_npc("npc_karl", "Karl", 0.0, 0.0))
        .await;
    game_state.add_player(make_player("seller", 1.0, 0.0)).await;
    game_state
        .register_player_character(&pid("seller"), 1, 0, attrs_with_cha(10), 0, None)
        .await;
    game_state
        .register_player_character(&pid("npc_karl"), 2, 0, attrs_with_cha(10), npc_gold, None)
        .await;
    let mut inventories = game_state.inventories.write().await;
    inventories.insert(
        pid("npc_karl"),
        PlayerInventory {
            bag: npc_bag,
            ..Default::default()
        },
    );
    inventories.insert(
        pid("seller"),
        PlayerInventory {
            bag: seller_bag,
            ..Default::default()
        },
    );
}

fn enchanted_bag_item(
    instance_id: u64,
    item_def_id: &str,
    quantity: u32,
    enchant: i32,
) -> ItemInstance {
    ItemInstance {
        instance_id,
        item_def_id: item_def_id.to_string(),
        quantity,
        enchant,
    }
}
