use super::*;
use onlinerpg_shared::dungeon::{cell_center, ENTRANCE_DOOR_ID};

mod chest_tests;
mod discovery_tests;
mod door_tests;

fn first_dungeon(game_state: &GameState) -> crate::dungeon_defs::DungeonEntranceDef {
    game_state
        .dungeon_defs
        .all()
        .next()
        .expect("a dungeon def")
        .clone()
}
