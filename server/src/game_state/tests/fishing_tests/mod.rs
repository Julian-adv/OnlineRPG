// ---- Fishing (doc/FISHING.md) ----------------------------------------------
// Paused-time tests: tokio's clock is frozen, `time::advance` moves it, and
// `tick_fishing` is driven by hand — the state machine runs deterministically.

use super::*;
use onlinerpg_shared::fishing::{
    auto_stance, FishState, FishingAction, FishingOutcome, BITE_WINDOW_MS, CAST_MS, ESCAPE_XP,
    FIGHT_TIMEOUT_MS, LATENCY_GRACE_MS, WAIT_MAX_MS, WAIT_MIN_MS,
};
use tokio::time::{advance, Duration};

mod economy_tests;
mod flow_tests;
mod interruption_tests;
mod inventory_tests;
mod session_tests;

/// Player on the shore of the test world's western sea (negative x is
/// 5 m underwater in `SplitWorldTiles`), rod equipped, ready to cast.
async fn make_angler(game_state: &GameState, name: &str) -> (PlayerId, DirectRx) {
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
        .register_player_character(&id, 1, 0, attrs_with_cha(10), 0, None)
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

/// A river world for the ocean-vs-river test: terrain is a 5 m-high plateau
/// everywhere (well above sea level), and the water field reports a river
/// surface at 5.4 m everywhere — the exact case the old `height < 0` check
/// wrongly rejected. Fishing must still work here.
struct RiverPlateauTiles;

#[async_trait::async_trait]
impl onlinerpg_terrain::height::HeightTiles for RiverPlateauTiles {
    async fn read_heightmap(&self, _tx: i32, _tz: i32) -> std::io::Result<Vec<u8>> {
        Ok(uniform_heightmap(5.0))
    }
}

struct RiverPlateauWater;

#[async_trait::async_trait]
impl onlinerpg_terrain::water::WaterTiles for RiverPlateauWater {
    async fn read_water_field(&self, _tx: i32, _tz: i32) -> std::io::Result<Option<Vec<u8>>> {
        // 16-byte header + 65×65 pixels of 6 bytes; surfaceY 5.4 m.
        let verts = onlinerpg_terrain::defaults::VERTS_PER_SIDE;
        let mut out = Vec::new();
        out.extend_from_slice(onlinerpg_shared::worldgen::tile_bake::WATER_FIELD_BIN_MAGIC);
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

/// A game state whose terrain is a 5 m plateau with a river surface at 5.4 m
/// everywhere — for testing that fishing works in rivers whose beds are above
/// sea level (the ocean-vs-river regression).
fn make_river_game_state(test_name: &str) -> GameState {
    make_game_state_with(test_name, RiverPlateauTiles, RiverPlateauWater)
}

/// Advance paused time in tick-sized steps, running the fishing tick at
/// each step — the paused-clock equivalent of the 250 ms `run_ticks` task.
async fn advance_with_ticks(game_state: &GameState, total_ms: u64) {
    let mut remaining = total_ms;
    while remaining > 0 {
        let step = remaining.min(250);
        advance(Duration::from_millis(step)).await;
        game_state.tick_fishing(None).await;
        remaining -= step;
    }
}

/// Tick forward only until the bite fires (the wait is a random roll —
/// blindly advancing the full range would blow through the bite window
/// whenever the roll came up short). Panics if no bite arrives within
/// the cast plus the maximum wait.
async fn advance_until_bite(game_state: &GameState, rx: &mut DirectRx) {
    let budget_ms = u64::from(CAST_MS) + u64::from(WAIT_MAX_MS) + 500;
    let mut elapsed = 0;
    while elapsed < budget_ms {
        advance(Duration::from_millis(250)).await;
        game_state.tick_fishing(None).await;
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
