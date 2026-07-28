pub use onlinerpg_shared::entity::ClientKind;
pub use onlinerpg_shared::{
    AttackRejectReason, Character, CharacterAttributes, CharacterClass, ClientMessage,
    GameDateTime, Gender, Monster, MonsterState, Player, PlayerId, Position, ServerMessage,
};
use std::sync::atomic::{AtomicU64, Ordering};

/// Session handles are minted from a counter rather than randomly. They are
/// never persisted and never authorize anything on their own — the acting
/// player always comes from the authenticated connection — so they only need
/// to be unique for the lifetime of the process. A small integer also costs
/// 1-3 wire bytes against a UUID string's 38, and every broadcast carries one.
///
/// Starts at 1; see `PlayerId` for why 0 is reserved.
static NEXT_PLAYER_ID: AtomicU64 = AtomicU64::new(1);

fn next_player_id() -> PlayerId {
    PlayerId::from(NEXT_PLAYER_ID.fetch_add(1, Ordering::Relaxed))
}

#[allow(clippy::too_many_arguments)]
pub fn new_player(
    name: String,
    level: u32,
    max_health: u32,
    class: CharacterClass,
    gender: Gender,
    position: Position,
    rotation: f32,
    is_official_npc: bool,
    client_kind: ClientKind,
) -> Player {
    Player {
        id: next_player_id(),
        name,
        position,
        rotation,
        level,
        health: max_health,
        max_health,
        class,
        gender,
        is_official_npc,
        torch_on: false,
        floor_level: 0,
        object_type: None,
        main_hand: None,
        object_id: None,
        last_combat_at: 0,
        client_kind,
    }
}
