use super::*;
use onlinerpg_shared::dungeon::{cell_center, ENTRANCE_DOOR_ID};

mod chest_tests;
mod discovery_tests;
mod door_tests;
mod monster_tests;

/// Give `player_id` an empty bag, plus one `item_def_id` when given.
async fn give_bag(game_state: &GameState, player_id: &PlayerId, item_def_id: Option<&str>) {
    game_state
        .inventories
        .write()
        .await
        .insert(*player_id, Default::default());
    if let Some(id) = item_def_id {
        assert!(game_state.give_item(player_id, id).await);
    }
}
