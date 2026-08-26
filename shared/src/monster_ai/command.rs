//! Behavior input/output types: the internal [`AiState`], the [`NearbyPlayer`]
//! projection fed into each tick, and the [`AiCommand`]/[`TickResult`] outputs.

use crate::{MonsterState, PlayerId, Position};
use serde::{Deserialize, Serialize};

/// Internal behavior state (superset of network [`MonsterState`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AiState {
    #[default]
    Idle,
    Walk,
    Run,
    Chase,
    Attack,
    Hit,
    Dead,
    Flee,
    Return,
    /// Chase on hold: queued behind a stander, or waiting on an unreachable
    /// target (doc/MONSTER_SEPARATION.md). Reports as Idle.
    Hold,
}

impl AiState {
    pub fn to_monster_state(self) -> MonsterState {
        match self {
            AiState::Idle => MonsterState::Idle,
            AiState::Walk => MonsterState::Walk,
            AiState::Run => MonsterState::Run,
            AiState::Chase => MonsterState::Run,
            AiState::Attack => MonsterState::Attack,
            AiState::Hit => MonsterState::Hit,
            AiState::Dead => MonsterState::Dead,
            AiState::Flee => MonsterState::Run,
            AiState::Return => MonsterState::Walk,
            AiState::Hold => MonsterState::Idle,
        }
    }

    /// Chasing, queued behind a stander, or swinging at a target.
    pub fn is_engaged(self) -> bool {
        matches!(self, AiState::Chase | AiState::Hold | AiState::Attack)
    }

    /// Walks a path between ticks.
    pub fn is_on_the_move(self) -> bool {
        matches!(
            self,
            AiState::Chase | AiState::Walk | AiState::Run | AiState::Flee | AiState::Return
        )
    }
}

/// Minimal player projection for behavior input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearbyPlayer {
    pub id: PlayerId,
    pub position: Position,
    pub health: u32,
}

/// Other monsters' last-synced poses, for cell-occupancy separation
/// (doc/MONSTER_SEPARATION.md). Caller filters out dead monsters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NearbyMonster {
    pub id: String,
    pub position: Position,
    /// Last-synced network state. Only stationary states
    /// ([`MonsterState::is_stationary`]) occupy cells — a ~500ms-stale
    /// position is wrong for a mover.
    pub state: MonsterState,
    #[serde(default)]
    pub path_floor: u8,
}

/// Behavior output — translated by the caller into network messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AiCommand {
    Move {
        monster_id: String,
        position: Position,
        rotation: f32,
        state: MonsterState,
        /// Where a remote view walks the model until the next sync — a point on
        /// the mover's own path, not its destination. Aiming a viewer's straight
        /// line at the destination walks the model through the walls the path
        /// goes around. See `MonsterBrain::current_leg_target`.
        target_position: Position,
    },
    Attack {
        monster_id: String,
        target_player_id: PlayerId,
    },
}

/// Result of a single brain tick — always includes current position/rotation
/// so the caller can update the visual even when no commands are emitted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickResult {
    pub commands: Vec<AiCommand>,
    pub position: Position,
    pub rotation: f32,
    pub state: MonsterState,
}
