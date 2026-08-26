//! [`MonsterBrain`] — the per-monster behavior tree instance: its state, the
//! main tick entry point, and damage/death event handlers. Behavior tree
//! evaluation lives in [`super::behavior`]; movement and state transitions in
//! [`super::movement`].

use super::tree::BehaviorStatus;
use super::{
    cell_of, AiCommand, AiState, BehaviorTree, NearbyMonster, NearbyPlayer, PathProvider,
    TickResult, DEFAULT_ATTACK_RANGE, DEFAULT_CHASE_RANGE, DEFAULT_HIT_STAGGER_MS,
    DEFAULT_MAX_MOVE_DIST, DEFAULT_MIN_MOVE_DIST, NETWORK_SYNC_INTERVAL_MS,
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
    /// Time left before the next swing, kept apart from `state_timer_ms` so
    /// that leaving and re-entering Attack cannot re-arm the cooldown. Zero is
    /// ready: the first swing on contact does not wait one out.
    #[serde(default)]
    pub(super) attack_cooldown_left_ms: f32,
    /// How long this type's swing animation runs. The attack holds the state
    /// this long so a swing it started finishes; 0 releases as soon as the
    /// target steps out.
    #[serde(default)]
    pub(super) swing_commit_ms: f32,
    /// Time left in the swing currently being delivered.
    #[serde(default)]
    pub(super) swing_left_ms: f32,
    /// The free cell near the target the chase is heading to; kept while it
    /// stays free so re-slotting doesn't oscillate.
    #[serde(default)]
    pub(super) chase_goal_cell: Option<(i32, i32)>,
    /// Cells occupied by standing nearby monsters, rebuilt each tick.
    #[serde(skip)]
    pub(super) occupied_cells: Vec<(i32, i32)>,
    /// A standing monster with a smaller id shares our cell this tick — we
    /// are the one who moves aside. Rebuilt each tick.
    #[serde(skip)]
    pub(super) cell_yield: bool,
    /// Walking off a shared cell to a slot of our own after a yield. Attack
    /// entry stays refused until arrival, or the yielder would stop at the
    /// first cell boundary, still overlapped.
    #[serde(default)]
    pub(super) reslotting: bool,
    /// Detour goal; repaths keep avoiding standers until arrival.
    #[serde(default)]
    pub(super) detour_goal: Option<(f32, f32)>,
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
        let swing_commit_ms = super::attack_clip_ms(&monster_type);
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
            attack_cooldown_left_ms: 0.0,
            swing_commit_ms,
            swing_left_ms: 0.0,
            chase_goal_cell: None,
            occupied_cells: Vec::new(),
            cell_yield: false,
            reslotting: false,
            detour_goal: None,
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

    /// A sync the step must not precede: a new movement state or a path bend,
    /// where a post-step pose would put the server's chord across the corner.
    pub(super) fn sync_due_before_step(&self) -> bool {
        self.pending_bend_sync || self.state != self.last_synced_state
    }

    /// Gate for the per-tick position emits of continuously-moving states
    /// (chase/return/flee): a state change, a bend, or `NETWORK_SYNC_INTERVAL_MS`
    /// elapsed. Remote clients interpolate toward the command's
    /// `target_position` between syncs — see [`Self::current_leg_target`].
    pub(super) fn should_sync_move(&self) -> bool {
        self.sync_due_before_step() || self.sync_elapsed_ms >= NETWORK_SYNC_INTERVAL_MS
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
        self.pending_bend_sync = false;
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

    pub(super) fn advance_timers(&mut self, delta_ms: f32) {
        self.state_timer_ms += delta_ms;
        self.path_elapsed_ms += delta_ms;
        self.sync_elapsed_ms += delta_ms;
        self.attack_cooldown_left_ms = (self.attack_cooldown_left_ms - delta_ms).max(0.0);
        self.swing_left_ms = (self.swing_left_ms - delta_ms).max(0.0);
    }

    /// Build a `TickResult` snapshot of the brain's current pose plus `commands`.
    fn tick_result(&self, commands: Vec<AiCommand>) -> TickResult {
        TickResult {
            commands,
            position: self.position,
            rotation: self.rotation,
            state: self.network_state(),
        }
    }

    pub fn tick_with_behavior_tree(
        &mut self,
        delta_ms: f32,
        nearby_players: &[NearbyPlayer],
        nearby_monsters: &[NearbyMonster],
        behavior_tree: &BehaviorTree,
        path_provider: &dyn PathProvider,
        rng: &mut impl Rng,
    ) -> TickResult {
        if self.state == AiState::Dead || self.health == 0 {
            return self.tick_result(vec![]);
        }

        self.occupied_cells.clear();
        self.cell_yield = false;
        let my_cell = cell_of(self.position.x, self.position.z);
        for m in nearby_monsters {
            if !m.state.is_stationary()
                || m.path_floor != self.path_floor
                || m.id == self.monster_id
            {
                continue;
            }
            let cell = cell_of(m.position.x, m.position.z);
            self.occupied_cells.push(cell);
            // Sharing a cell with a smaller-id stander: we are the one who
            // yields (see bt_attack_target / the door-wait spread).
            if cell == my_cell && m.id.as_str() < self.monster_id.as_str() {
                self.cell_yield = true;
            }
        }
        if !self.state.is_engaged() {
            self.reslotting = false;
            self.detour_goal = None;
        }

        self.advance_timers(delta_ms);
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
        let previous_target = self.target_player_id;
        if !self.apply_hit(attacker_id, hit, damage) {
            return vec![];
        }

        if hit {
            self.state = AiState::Hit;
            self.state_timer_ms = 0.0;
            vec![self.make_move_cmd()]
        } else if self.state.is_engaged() {
            // Already engaged: idling here stopped a charging monster and
            // slid it back to the last tick's pose on every missed swing.
            // A new attacker only needs the next repath aimed at them.
            if previous_target != self.target_player_id {
                self.last_known_target_pos = None;
            }
            vec![]
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
        } else if self.pending_bend_sync {
            // A wander leg is otherwise reported only at its ends, so the
            // server would see one chord across every corner of it. Bends alone
            // are enough — no interval sync, since a straight run between two
            // bends is a sub-segment of a leg the path check already cleared.
            commands.push(self.make_move_cmd());
        }
    }

    /// The one `Move` literal, and the one place a sync is recorded: a later
    /// state change syncs at once even when it returns to the state synced
    /// before this one, and the bend this pose reports is consumed.
    pub(super) fn move_cmd_to(&mut self, target_position: Position) -> AiCommand {
        self.sync_elapsed_ms = 0.0;
        self.last_synced_state = self.state;
        self.pending_bend_sync = false;
        AiCommand::Move {
            monster_id: self.monster_id.clone(),
            position: self.position,
            rotation: self.rotation,
            state: self.state.to_monster_state(),
            target_position,
        }
    }

    pub(super) fn make_move_cmd(&mut self) -> AiCommand {
        let aim = self.current_leg_target();
        self.move_cmd_to(aim)
    }
}
