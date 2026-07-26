//! [`MonsterBrain`] — the per-monster behavior tree instance: its state, the
//! main tick entry point, and damage/death event handlers. Behavior tree
//! evaluation lives in [`super::behavior`]; movement and state transitions in
//! [`super::movement`].

use super::tree::BehaviorStatus;
use super::{
    AiCommand, AiState, BehaviorTree, NearbyPlayer, PathProvider, TickResult, DEFAULT_ATTACK_RANGE,
    DEFAULT_CHASE_RANGE, DEFAULT_HIT_STAGGER_MS, DEFAULT_MAX_MOVE_DIST, DEFAULT_MIN_MOVE_DIST,
    NETWORK_SYNC_INTERVAL_MS,
};
use crate::pathfinding::PathWaypoint;
use crate::{MonsterState, PlayerId, Position};
use rand::Rng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonsterBrain {
    pub monster_id: String,
    pub monster_type: String,
    pub behavior: String,
    pub position: Position,
    pub rotation: f32,
    pub health: u32,
    pub max_health: u32,
    pub(super) state: AiState,
    pub(super) state_timer_ms: f32,
    pub(super) target_player_id: Option<PlayerId>,
    pub(super) walk_speed: f32,
    pub(super) run_speed: f32,
    pub(super) attack_range: f32,
    pub(super) chase_range: f32,
    pub(super) attack_cooldown_ms: f32,
    pub(super) move_speed: f32,
    pub(super) target_position: Option<Position>,
    pub(super) waypoints: Vec<PathWaypoint>,
    pub(super) current_waypoint_idx: usize,
    pub(super) path_elapsed_ms: f32,
    pub(super) last_known_target_pos: Option<Position>,
    pub(super) spawn_position: Position,
    /// Passability floor for path queries. 0 = overworld/house ground;
    /// dungeon monsters use their depth's passability floor index.
    #[serde(default)]
    pub path_floor: u8,
    /// Time accumulated toward the next throttled network position sync while
    /// continuously moving. See [`Self::should_sync_move`].
    #[serde(default)]
    pub(super) sync_elapsed_ms: f32,
    /// Movement state at the last emitted sync, so entering a new one syncs at
    /// once instead of waiting out the interval.
    #[serde(default)]
    pub(super) last_synced_state: AiState,
    /// A path bend the next sync must not be allowed to cut across.
    #[serde(default)]
    pub(super) pending_bend_sync: bool,
}

impl MonsterBrain {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        monster_id: String,
        monster_type: String,
        behavior: String,
        position: Position,
        health: u32,
        max_health: u32,
        walk_speed: f32,
        run_speed: f32,
        attack_range: f32,
        chase_range: f32,
        attack_cooldown_ms: f32,
    ) -> Self {
        Self {
            monster_id,
            monster_type,
            behavior,
            rotation: 0.0,
            health,
            max_health,
            state: AiState::Idle,
            state_timer_ms: 0.0,
            target_player_id: None,
            walk_speed,
            run_speed,
            attack_range: if attack_range > 0.0 {
                attack_range
            } else {
                DEFAULT_ATTACK_RANGE
            },
            chase_range: if chase_range > 0.0 {
                chase_range
            } else {
                DEFAULT_CHASE_RANGE
            },
            attack_cooldown_ms,
            move_speed: walk_speed,
            target_position: None,
            waypoints: Vec::new(),
            current_waypoint_idx: 0,
            path_elapsed_ms: 0.0,
            last_known_target_pos: None,
            spawn_position: position,
            position,
            path_floor: 0,
            sync_elapsed_ms: 0.0,
            last_synced_state: AiState::Idle,
            pending_bend_sync: false,
        }
    }

    /// The path just changed direction here — force the next tick to sync.
    ///
    /// The server only sees the straight line between two reported positions,
    /// and a bend swallowed inside one sync interval makes that line a chord off
    /// the walkable path, which its collision sweep rightly refuses. Reporting
    /// bends keeps every segment inside one smoothed leg, which
    /// `pathfinding::is_line_passable` cleared with `y: None` and a
    /// `PLAYER_RADIUS` sweep — stricter than the server's. Keep that inequality:
    /// it is what makes an accepted path un-refusable.
    pub(super) fn mark_path_bend(&mut self) {
        self.pending_bend_sync = true;
    }

    /// Consume a pending bend without disturbing the sync timer, for states
    /// that report only at bends rather than on an interval.
    pub(super) fn take_path_bend(&mut self) -> bool {
        std::mem::take(&mut self.pending_bend_sync)
    }

    /// Drop a pending bend for a caller that emits this pose itself, so the
    /// bend isn't reported twice.
    pub(super) fn clear_path_bend(&mut self) {
        self.pending_bend_sync = false;
    }

    /// Gate for the per-tick position emits of continuously-moving states
    /// (chase/return/flee): true on entering a new movement state, on a path
    /// bend, or once `NETWORK_SYNC_INTERVAL_MS` has elapsed. Resets the timer
    /// when it fires so the brain simulates every frame but only syncs a couple
    /// of times a second. Remote clients interpolate toward `target_position`
    /// between syncs.
    pub(super) fn should_sync_move(&mut self) -> bool {
        if self.take_path_bend()
            || self.state != self.last_synced_state
            || self.sync_elapsed_ms >= NETWORK_SYNC_INTERVAL_MS
        {
            self.sync_elapsed_ms = 0.0;
            self.last_synced_state = self.state;
            true
        } else {
            false
        }
    }

    /// Snap to the position the server settled on and drop the current path, so
    /// the next tick re-plans from there. Otherwise the brain keeps walking from
    /// a position the server refused, and every later move is swept from the one
    /// it kept — the same refusal forever.
    pub fn apply_authoritative_position(&mut self, position: Position) {
        if self.state == AiState::Dead {
            return;
        }
        self.position = position;
        self.waypoints.clear();
        self.current_waypoint_idx = 0;
        self.clear_path_bend();
    }

    pub fn state(&self) -> AiState {
        self.state
    }

    pub fn network_state(&self) -> MonsterState {
        self.state.to_monster_state()
    }

    pub fn is_dead(&self) -> bool {
        self.state == AiState::Dead
    }

    // =========================================================================
    // Main tick
    // =========================================================================

    /// Build a `TickResult` snapshot of the brain's current pose plus `commands`.
    fn tick_result(&self, commands: Vec<AiCommand>) -> TickResult {
        TickResult {
            commands,
            position: self.position,
            rotation: self.rotation,
            state: self.state.to_monster_state(),
        }
    }

    pub fn tick_with_behavior_tree(
        &mut self,
        delta_ms: f32,
        nearby_players: &[NearbyPlayer],
        behavior_tree: &BehaviorTree,
        path_provider: &dyn PathProvider,
        rng: &mut impl Rng,
    ) -> TickResult {
        if self.state == AiState::Dead || self.health == 0 {
            return self.tick_result(vec![]);
        }

        self.state_timer_ms += delta_ms;
        self.path_elapsed_ms += delta_ms;
        self.sync_elapsed_ms += delta_ms;
        let mut commands = Vec::new();

        if self.state == AiState::Hit {
            if self.state_timer_ms < DEFAULT_HIT_STAGGER_MS {
                return self.tick_result(commands);
            }
            self.state = AiState::Idle;
            self.state_timer_ms = 0.0;
        }

        if matches!(self.state, AiState::Walk | AiState::Run) {
            self.tick_patrol(delta_ms, &mut commands, path_provider, rng);
        } else {
            let status = self.eval_behavior_node(
                &behavior_tree.root,
                delta_ms,
                nearby_players,
                &mut commands,
                path_provider,
                rng,
            );

            if status == BehaviorStatus::Failure && self.state != AiState::Idle {
                self.transition_to_idle(&mut commands);
            }
        }

        self.tick_result(commands)
    }

    // =========================================================================
    // Event handlers
    // =========================================================================

    /// Apply incoming damage and acquire `attacker_id` as the target. Returns
    /// `false` if the brain is already dead or the hit was lethal (in which
    /// case no further reaction should be produced).
    fn apply_hit(&mut self, attacker_id: &PlayerId, hit: bool, damage: u32) -> bool {
        if self.state == AiState::Dead {
            return false;
        }

        self.health = self.health.saturating_sub(if hit { damage } else { 0 });
        self.target_player_id = Some(*attacker_id);
        self.move_speed = self.run_speed;

        if self.health == 0 {
            self.state = AiState::Dead;
            return false;
        }

        true
    }

    pub fn handle_hit_with_behavior_tree(
        &mut self,
        attacker_id: &PlayerId,
        hit: bool,
        damage: u32,
    ) -> Vec<AiCommand> {
        if !self.apply_hit(attacker_id, hit, damage) {
            return vec![];
        }

        if hit {
            self.state = AiState::Hit;
            self.state_timer_ms = 0.0;
            self.clear_path_bend();
            vec![self.make_move_cmd()]
        } else {
            // A miss (and the server's out-of-range provoke event) still
            // acquires the attacker. Cancel any in-progress wander so the
            // next AI tick evaluates the combat branches immediately instead
            // of finishing the old patrol path first.
            let mut commands = Vec::new();
            self.transition_to_idle(&mut commands);
            commands
        }
    }

    pub fn handle_death(&mut self) {
        self.state = AiState::Dead;
        self.health = 0;
    }

    // =========================================================================

    fn tick_patrol(
        &mut self,
        delta_ms: f32,
        commands: &mut Vec<AiCommand>,
        path_provider: &dyn PathProvider,
        rng: &mut impl Rng,
    ) {
        if self.target_position.is_none() {
            self.transition_to_idle(commands);
            return;
        }

        let reached = self.follow_path(delta_ms);
        if reached {
            if rng.gen::<f32>() < 0.5 {
                self.transition_to_idle(commands);
            } else {
                self.transition_to_move(
                    commands,
                    DEFAULT_MIN_MOVE_DIST,
                    DEFAULT_MAX_MOVE_DIST,
                    path_provider,
                    rng,
                );
            }
        } else if self.take_path_bend() {
            // A wander leg is otherwise reported only at its ends, so the
            // server would see one chord across every corner of it. Bends alone
            // are enough — no interval sync, since a straight run between two
            // bends is a sub-segment of a leg the path check already cleared.
            commands.push(self.make_move_cmd());
        }
    }

    /// Build a `Move` command from the brain's current pose, defaulting the
    /// target position to the current position when none is set.
    pub(super) fn make_move_cmd(&self) -> AiCommand {
        AiCommand::Move {
            monster_id: self.monster_id.clone(),
            position: self.position,
            rotation: self.rotation,
            state: self.state.to_monster_state(),
            target_position: self.target_position.unwrap_or(self.position),
        }
    }
}
