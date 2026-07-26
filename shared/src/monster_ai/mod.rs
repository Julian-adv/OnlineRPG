//! Shared monster AI behavior tree runtime — used by both WASM (client) and native Rust (agent-client).
//!
//! The runtime is stateful per-monster via [`MonsterBrain`]. Each tick receives
//! external inputs (delta time, nearby players) and returns a list of
//! [`AiCommand`]s that the caller translates into network messages.
//!
//! The module is split into:
//! - [`tree`] — behavior tree data model and JSON loading
//! - [`command`] — AI state and behavior input/output types
//! - [`path`] — pathfinding abstraction ([`PathProvider`])
//! - [`brain`] — the per-monster [`MonsterBrain`] (struct, tick, event handlers)
//! - [`behavior`] — behavior tree execution (condition/action evaluation)
//! - [`movement`] — state transitions and path-following helpers

use std::collections::HashMap;

mod behavior;
mod brain;
mod command;
mod movement;
mod path;
mod tree;

#[cfg(test)]
mod tests;

pub use brain::MonsterBrain;
pub use command::{AiCommand, AiState, NearbyPlayer, TickResult};
pub use path::{CachePathProvider, PathProvider};
pub use tree::{behavior_tree_for, load_behavior_trees, BehaviorNode, BehaviorTree};

/// Attack clip length per monster type, in ms, measured from the models by
/// `tools/measure-monster-attack-clips.mjs`.
static ATTACK_CLIPS_JSON: &str = include_str!("../../../data/monster_attack_clips.json");

/// How long `monster_type`'s swing runs, or 0 for a type with no measured clip
/// (a test type, or one whose model has no attack animation).
pub fn attack_clip_ms(monster_type: &str) -> f32 {
    use std::sync::OnceLock;
    static CLIPS: OnceLock<HashMap<String, f32>> = OnceLock::new();
    CLIPS
        .get_or_init(|| {
            serde_json::from_str(ATTACK_CLIPS_JSON).expect("monster_attack_clips.json is malformed")
        })
        .get(monster_type)
        .copied()
        .unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// Shared tuning constants. Public ones are part of the module's API; private
// ones are visible to all submodules as descendants of `monster_ai`.
// ---------------------------------------------------------------------------

const DEFAULT_IDLE_CHECK_MS: f32 = 1000.0;
const DEFAULT_MIN_MOVE_DIST: f32 = 2.0;
const DEFAULT_MAX_MOVE_DIST: f32 = 10.0;
pub const DEFAULT_WALK_SPEED: f32 = 1.0;
pub const DEFAULT_RUN_SPEED: f32 = 8.0;
pub const DEFAULT_ATTACK_RANGE: f32 = 2.0;
pub const DEFAULT_CHASE_RANGE: f32 = 25.0;
pub const DEFAULT_ATTACK_COOLDOWN_MS: f32 = 1500.0;
const DEFAULT_LEASH_RANGE: f32 = 50.0;
const DEFAULT_HIT_STAGGER_MS: f32 = 800.0;
const DEFAULT_FLEE_HEALTH_RATIO: f32 = 0.0;
const DEFAULT_FLEE_MAX_DURATION_MS: f32 = 15000.0;
const FLEE_SAFE_DIST_MARGIN: f32 = 5.0;
const DEFAULT_RETURN_ARRIVE_DIST: f32 = 5.0;
const DEFAULT_PATH_RECALC_MS: f32 = 500.0;
const DEFAULT_TARGET_MOVE_THRESHOLD: f32 = 3.0;
/// How far past its attack range a monster holds the attack before falling back
/// to the chase. Releasing at the radius it engages at makes a target that walks
/// away flip the state every frame — 28 times a second at a 60Hz tick, which
/// thrashes the animation and drags the model along in an attack pose. Absolute
/// rather than scaled: the margin has to beat how far a target moves between
/// decisions, which has nothing to do with the monster's reach. Well inside the
/// server's own (also absolute) reach slack.
const ATTACK_RELEASE_MARGIN_METERS: f32 = 0.5;
/// Least time between network position syncs while a monster is continuously
/// moving (chase/return/flee). The brain simulates every frame but only emits a
/// `Move` this often, cutting ~60/s of packets to ~2/s; remote clients
/// interpolate toward `target_position` in between, and state changes still sync
/// immediately. Server-authoritative movement (F-006) absorbs the coarser rate.
const NETWORK_SYNC_INTERVAL_MS: f32 = 500.0;
pub const DEFAULT_BEHAVIOR: &str = "brave";
/// Behavior tree used by proactive (선공형) monsters that acquire and attack
/// targets on sight. Selected when `Monster::aggressive` is set, overriding the
/// monster type's configured behavior.
pub const AGGRESSIVE_BEHAVIOR: &str = "aggressive";

fn param(params: &HashMap<String, f32>, name: &str, default: f32) -> f32 {
    params.get(name).copied().unwrap_or(default)
}
