//! State-transition and path-following helpers for [`MonsterBrain`]: entering
//! idle/move/flee states, computing and following waypoint paths, and facing
//! the next waypoint.

use super::{AiCommand, AiState, MonsterBrain, PathProvider};
use crate::pathfinding::PathWaypoint;
use crate::world::{bearing_xz, shortest_world_delta_x, wrap_world_x};
use crate::Position;
use rand::Rng;

impl MonsterBrain {
    // =========================================================================
    // Transition helpers
    // =========================================================================

    pub(super) fn transition_to_idle(&mut self, commands: &mut Vec<AiCommand>) {
        self.state = AiState::Idle;
        self.state_timer_ms = 0.0;
        self.target_position = None;
        self.waypoints.clear();
        self.current_waypoint_idx = 0;
        self.clear_path_bend();
        commands.push(self.make_move_cmd());
    }

    pub(super) fn transition_to_move(
        &mut self,
        commands: &mut Vec<AiCommand>,
        min_move_dist: f32,
        max_move_dist: f32,
        path_provider: &dyn PathProvider,
        rng: &mut impl Rng,
    ) {
        let angle: f32 = rng.gen_range(0.0..std::f32::consts::TAU);
        let dist: f32 = rng.gen_range(min_move_dist..max_move_dist);

        let target_x = wrap_world_x(self.position.x + angle.cos() * dist);
        let target_z = self.position.z + angle.sin() * dist;

        // Walk vs run probability based on distance
        let walk_prob = (-0.075 * dist + 0.95).clamp(0.0, 1.0);
        let is_walk = rng.gen::<f32>() < walk_prob;

        if is_walk {
            self.state = AiState::Walk;
            self.move_speed = self.walk_speed;
        } else {
            self.state = AiState::Run;
            self.move_speed = self.run_speed;
        }

        self.state_timer_ms = 0.0;
        self.target_position = Some(Position {
            x: target_x,
            y: self.position.y,
            z: target_z,
        });

        self.compute_path(target_x, target_z, path_provider);

        if self.waypoints.is_empty() {
            self.state = AiState::Idle;
            self.target_position = None;
            return;
        }

        self.face_first_waypoint();
        self.clear_path_bend();

        // target_position was set above, safe to unwrap
        commands.push(AiCommand::Move {
            monster_id: self.monster_id.clone(),
            position: self.position,
            rotation: self.rotation,
            state: self.state.to_monster_state(),
            target_position: self.target_position.unwrap(),
        });
    }

    pub(super) fn transition_to_flee(
        &mut self,
        safe_dist: f32,
        commands: &mut Vec<AiCommand>,
        path_provider: &dyn PathProvider,
    ) {
        self.state = AiState::Flee;
        self.state_timer_ms = 0.0;
        self.move_speed = self.run_speed;

        self.start_flee_path(safe_dist, path_provider);

        if self.waypoints.is_empty() {
            self.state = AiState::Idle;
            self.state_timer_ms = 0.0;
            self.target_position = None;
            return;
        }

        self.clear_path_bend();
        commands.push(self.make_move_cmd());
    }

    /// Pick a flee leg pointing directly away from the last known threat
    /// position, long enough to end up outside `safe_dist`. Falls back to the
    /// spawn point when the threat position is unknown or the away path is
    /// blocked. Leaves `waypoints` empty when no path is available.
    pub(super) fn start_flee_path(&mut self, safe_dist: f32, path_provider: &dyn PathProvider) {
        if let Some(threat) = self.last_known_target_pos {
            // Away from the threat, so the delta runs threat -> self.
            let dx = shortest_world_delta_x(threat.x, self.position.x);
            let dz = self.position.z - threat.z;
            let dist = (dx * dx + dz * dz).sqrt();
            if dist > f32::EPSILON {
                let dest_x = wrap_world_x(self.position.x + dx / dist * safe_dist);
                let dest_z = self.position.z + dz / dist * safe_dist;
                if self.try_path_to(dest_x, dest_z, path_provider) {
                    return;
                }
            }
        }

        if !self.try_path_to(self.spawn_position.x, self.spawn_position.z, path_provider) {
            self.target_position = None;
        }
    }

    /// Set `target_position`, path to it, and face the first waypoint.
    /// Returns false (leaving `waypoints` empty) when no path is available.
    fn try_path_to(&mut self, x: f32, z: f32, path_provider: &dyn PathProvider) -> bool {
        self.target_position = Some(Position {
            x,
            y: self.position.y,
            z,
        });
        self.compute_path(x, z, path_provider);
        if self.waypoints.is_empty() {
            return false;
        }
        self.face_first_waypoint();
        true
    }

    // =========================================================================
    // Movement helpers
    // =========================================================================

    /// Waypoints are canonical like `position`, so facing one takes the
    /// periodic delta. `PathWaypoint` isn't a `Position`, hence not
    /// `Position::bearing_xz_to`.
    fn face_toward(&mut self, x: f32, z: f32) {
        self.rotation = bearing_xz(
            shortest_world_delta_x(self.position.x, x),
            z - self.position.z,
        )
        .unwrap_or(self.rotation);
    }

    pub(super) fn face_first_waypoint(&mut self) {
        if let Some(wp) = self.waypoints.first() {
            let (x, z) = (wp.x, wp.z);
            self.face_toward(x, z);
        }
    }

    /// Path query in the periodic frame nearest the monster: a canonical goal
    /// on the far side of the seam would otherwise ask for a path a whole
    /// world width long, and the waypoints come back re-wrapped. A house
    /// straddling the seam is invisible to the query — the gap
    /// `passability::wrapped_block_info` covers on the server.
    pub(super) fn query_path(
        &self,
        goal_x: f32,
        goal_z: f32,
        path_provider: &dyn PathProvider,
    ) -> crate::pathfinding::PathResult {
        self.query_path_from(
            self.position.x,
            self.position.z,
            goal_x,
            goal_z,
            path_provider,
        )
    }

    /// The same seam-aware query from an arbitrary start on our floor (used
    /// to measure a route from a sidestep candidate).
    pub(super) fn query_path_from(
        &self,
        start_x: f32,
        start_z: f32,
        goal_x: f32,
        goal_z: f32,
        path_provider: &dyn PathProvider,
    ) -> crate::pathfinding::PathResult {
        let local_goal_x = start_x + shortest_world_delta_x(start_x, goal_x);
        let mut result = path_provider.find_path(
            start_x,
            start_z,
            self.path_floor,
            local_goal_x,
            goal_z,
            self.path_floor,
        );
        for wp in &mut result.waypoints {
            wp.x = wrap_world_x(wp.x);
        }
        result
    }

    /// Path toward the goal, accepting a partial leg: A* answers an
    /// unreachable goal with the path to the closest reachable cell
    /// (`found: false`). Callers that must not settle for that check
    /// `query_path().found` themselves.
    pub(super) fn compute_path(
        &mut self,
        goal_x: f32,
        goal_z: f32,
        path_provider: &dyn PathProvider,
    ) {
        let waypoints = self.query_path(goal_x, goal_z, path_provider).waypoints;
        self.install_path(waypoints);
    }

    /// Adopt a freshly queried leg: reset progress and the repath timer, and
    /// mark a bend when this cuts a leg short.
    pub(super) fn install_path(&mut self, waypoints: Vec<PathWaypoint>) {
        let turned_mid_leg = self.current_waypoint_idx < self.waypoints.len();
        self.waypoints = waypoints;
        self.current_waypoint_idx = 0;
        self.path_elapsed_ms = 0.0;
        // Only a repath that cuts a leg short is a bend. Replacing a finished
        // path starts where the last report already left off — and combat
        // repaths every tick while flapping in and out of attack range.
        if turned_mid_leg {
            self.mark_path_bend();
        }
    }

    /// Follow waypoints. Returns true if path is exhausted.
    pub(super) fn follow_path(&mut self, delta_ms: f32) -> bool {
        self.follow_path_gated(delta_ms, false).0
    }

    /// Like `follow_path`, but when `gated` refuses to step into a cell in
    /// `occupied_cells` — the mover holds in place instead (NetHack-style
    /// queueing, doc/MONSTER_SEPARATION.md). Returns (reached, held).
    pub(super) fn follow_path_gated(&mut self, delta_ms: f32, gated: bool) -> (bool, bool) {
        if self.current_waypoint_idx >= self.waypoints.len() {
            return (true, false);
        }

        let wp = &self.waypoints[self.current_waypoint_idx];
        let (wp_x, wp_z) = (wp.x, wp.z);
        let dx = shortest_world_delta_x(self.position.x, wp_x);
        let dz = wp_z - self.position.z;
        let dist = (dx * dx + dz * dz).sqrt();
        let step = self.move_speed * delta_ms / 1000.0;
        let snap = dist <= step;
        let (nx, nz) = if snap {
            (wp_x, wp_z)
        } else {
            (
                wrap_world_x(self.position.x + dx / dist * step),
                self.position.z + dz / dist * step,
            )
        };

        if gated {
            let to = super::cell_of(nx, nz);
            if to != super::cell_of(self.position.x, self.position.z)
                && self.occupied_cells.contains(&to)
            {
                return (false, true);
            }
        }

        self.position.x = nx;
        self.position.z = nz;
        if snap {
            self.current_waypoint_idx += 1;
            self.mark_path_bend();

            if self.current_waypoint_idx >= self.waypoints.len() {
                return (true, false);
            }

            let next = &self.waypoints[self.current_waypoint_idx];
            let (x, z) = (next.x, next.z);
            self.face_toward(x, z);
        } else {
            self.rotation = bearing_xz(dx, dz).unwrap_or(self.rotation);
        }

        (false, false)
    }

    pub(super) fn target_moved_significantly_by(
        &self,
        target_pos: &Position,
        threshold: f32,
    ) -> bool {
        match &self.last_known_target_pos {
            None => true,
            Some(last) => last.dist_xz_sq(target_pos) > threshold * threshold,
        }
    }
}
