//! Behavior tree execution for [`MonsterBrain`]: node traversal plus the
//! condition and action handlers (`bt_*`) that drive the brain's state.

use super::tree::BehaviorStatus;
use super::{
    cell_center, cell_of, leg_crosses_occupied, param, path_len, AiCommand, AiState, BehaviorNode,
    MonsterBrain, NearbyPlayer, PathProvider, ATTACK_RELEASE_MARGIN_METERS,
    CHASE_CELL_RANGE_MARGIN, DEFAULT_FLEE_HEALTH_RATIO, DEFAULT_FLEE_MAX_DURATION_MS,
    DEFAULT_IDLE_CHECK_MS, DEFAULT_LEASH_RANGE, DEFAULT_MAX_MOVE_DIST, DEFAULT_MIN_MOVE_DIST,
    DEFAULT_PATH_RECALC_MS, DEFAULT_RETURN_ARRIVE_DIST, DEFAULT_TARGET_MOVE_THRESHOLD,
    DETOUR_MAX_NODES, DETOUR_MAX_PATH_METERS, ENGAGE_FRACTION, ENGAGE_INSET_METERS,
    FLEE_SAFE_DIST_MARGIN, MAX_SLOT_PATH_TRIES, MIN_PARTIAL_PROGRESS_METERS,
    NETWORK_SYNC_INTERVAL_MS, SIDESTEP_MAX_PATH_METERS,
};
use crate::world::{shortest_world_delta_x, wrap_world_x};
use rand::Rng;
use std::collections::HashMap;

/// How close a chase closes before it swings: well inside the reach, so a
/// monster doesn't stop and flail at the edge of it.
fn engage_limit(range: f32) -> f32 {
    range * ENGAGE_FRACTION
}

impl MonsterBrain {
    pub(super) fn eval_behavior_node(
        &mut self,
        node: &BehaviorNode,
        delta_ms: f32,
        nearby_players: &[NearbyPlayer],
        commands: &mut Vec<AiCommand>,
        path_provider: &dyn PathProvider,
        rng: &mut impl Rng,
    ) -> BehaviorStatus {
        match node {
            BehaviorNode::Selector { children } => {
                for child in children {
                    match self.eval_behavior_node(
                        child,
                        delta_ms,
                        nearby_players,
                        commands,
                        path_provider,
                        rng,
                    ) {
                        BehaviorStatus::Failure => {}
                        status => return status,
                    }
                }
                BehaviorStatus::Failure
            }
            BehaviorNode::Sequence { children } => {
                for child in children {
                    match self.eval_behavior_node(
                        child,
                        delta_ms,
                        nearby_players,
                        commands,
                        path_provider,
                        rng,
                    ) {
                        BehaviorStatus::Success => {}
                        status => return status,
                    }
                }
                BehaviorStatus::Success
            }
            BehaviorNode::Condition { name, params } => {
                self.eval_condition(name, params, nearby_players, rng)
            }
            BehaviorNode::Action { name, params } => self.eval_action(
                name,
                params,
                delta_ms,
                nearby_players,
                commands,
                path_provider,
                rng,
            ),
        }
    }

    fn eval_condition(
        &mut self,
        name: &str,
        params: &HashMap<String, f32>,
        nearby_players: &[NearbyPlayer],
        rng: &mut impl Rng,
    ) -> BehaviorStatus {
        match name {
            "has_target" => self.current_target(nearby_players).is_some().into(),
            "target_in_range" => {
                let range = param(params, "range", self.chase_range);
                self.select_target_in_range(nearby_players, range).into()
            }
            "is_beyond_leash" => {
                let range = param(params, "range", DEFAULT_LEASH_RANGE);
                (self.state == AiState::Return
                    || self.position.dist_xz_sq(&self.spawn_position) > range * range)
                    .into()
            }
            "health_below_ratio" => {
                let ratio = param(params, "ratio", DEFAULT_FLEE_HEALTH_RATIO);
                let health_ratio = if self.max_health == 0 {
                    0.0
                } else {
                    self.health as f32 / self.max_health as f32
                };
                (self.state == AiState::Flee || health_ratio <= ratio).into()
            }
            "chance" => {
                let probability = param(params, "probability", 0.0).clamp(0.0, 1.0);
                (matches!(self.state, AiState::Flee | AiState::Return)
                    || rng.gen::<f32>() < probability)
                    .into()
            }
            _ => BehaviorStatus::Failure,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn eval_action(
        &mut self,
        name: &str,
        params: &HashMap<String, f32>,
        delta_ms: f32,
        nearby_players: &[NearbyPlayer],
        commands: &mut Vec<AiCommand>,
        path_provider: &dyn PathProvider,
        rng: &mut impl Rng,
    ) -> BehaviorStatus {
        match name {
            "idle" => {
                if self.state != AiState::Idle {
                    self.transition_to_idle(commands);
                }
                BehaviorStatus::Success
            }
            "wander" => self.bt_wander(params, commands, path_provider, rng),
            "return_to_spawn" => self.bt_return_to_spawn(params, delta_ms, commands, path_provider),
            "flee_from_target" => {
                self.bt_flee_from_target(params, delta_ms, nearby_players, commands, path_provider)
            }
            "attack_target" => {
                self.bt_attack_target(params, nearby_players, commands, path_provider)
            }
            "chase_target" => {
                self.bt_chase_target(params, delta_ms, nearby_players, commands, path_provider)
            }
            _ => BehaviorStatus::Failure,
        }
    }

    fn bt_wander(
        &mut self,
        params: &HashMap<String, f32>,
        commands: &mut Vec<AiCommand>,
        path_provider: &dyn PathProvider,
        rng: &mut impl Rng,
    ) -> BehaviorStatus {
        let check_ms = param(params, "checkMs", DEFAULT_IDLE_CHECK_MS);
        if self.state_timer_ms < check_ms {
            return BehaviorStatus::Failure;
        }

        let min_move_dist = param(params, "minMoveDist", DEFAULT_MIN_MOVE_DIST);
        let max_move_dist = param(params, "maxMoveDist", DEFAULT_MAX_MOVE_DIST);

        self.state_timer_ms = 0.0;
        self.transition_to_move(commands, min_move_dist, max_move_dist, path_provider, rng);
        if matches!(self.state, AiState::Walk | AiState::Run) {
            BehaviorStatus::Running
        } else {
            BehaviorStatus::Failure
        }
    }

    fn bt_return_to_spawn(
        &mut self,
        params: &HashMap<String, f32>,
        delta_ms: f32,
        commands: &mut Vec<AiCommand>,
        path_provider: &dyn PathProvider,
    ) -> BehaviorStatus {
        let arrive_dist = param(params, "arriveDist", DEFAULT_RETURN_ARRIVE_DIST);
        if self.position.dist_xz_sq(&self.spawn_position) <= arrive_dist * arrive_dist {
            self.transition_to_idle(commands);
            return BehaviorStatus::Success;
        }

        if self.state != AiState::Return {
            self.state = AiState::Return;
            self.state_timer_ms = 0.0;
            self.move_speed = self.walk_speed;
            self.target_position = Some(self.spawn_position);
            self.compute_path(self.spawn_position.x, self.spawn_position.z, path_provider);
            if self.waypoints.is_empty() {
                self.transition_to_idle(commands);
                return BehaviorStatus::Failure;
            }
            self.face_first_waypoint();
        }

        self.follow_path(delta_ms);
        if self.should_sync_move() {
            commands.push(self.make_move_cmd());
        }
        BehaviorStatus::Running
    }

    fn bt_flee_from_target(
        &mut self,
        params: &HashMap<String, f32>,
        delta_ms: f32,
        nearby_players: &[NearbyPlayer],
        commands: &mut Vec<AiCommand>,
        path_provider: &dyn PathProvider,
    ) -> BehaviorStatus {
        if self.target_player_id.is_none() && self.state != AiState::Flee {
            return BehaviorStatus::Failure;
        }

        // Flee until the threat is outside its sight range, not for a fixed time.
        // maxDurationMs is only a failsafe against fleeing forever from a chaser.
        let safe_dist = param(params, "safeDist", self.chase_range + FLEE_SAFE_DIST_MARGIN);
        let max_duration_ms = param(params, "maxDurationMs", DEFAULT_FLEE_MAX_DURATION_MS);

        if let Some(target) = self.current_target(nearby_players) {
            self.last_known_target_pos = Some(target.position);
        }

        if self.beyond_safe_dist(safe_dist) {
            self.finish_flee(commands);
            return BehaviorStatus::Success;
        }

        if self.state != AiState::Flee {
            self.transition_to_flee(safe_dist, commands, path_provider);
            if self.state != AiState::Flee {
                return BehaviorStatus::Failure;
            }
            return BehaviorStatus::Running;
        }

        if self.state_timer_ms >= max_duration_ms {
            self.finish_flee(commands);
            return BehaviorStatus::Success;
        }

        let reached = self.follow_path(delta_ms);
        if reached {
            if self.last_known_target_pos.is_none() || self.beyond_safe_dist(safe_dist) {
                self.finish_flee(commands);
                return BehaviorStatus::Success;
            }
            // Path ran out while the threat can still see us — start a new leg.
            self.start_flee_path(safe_dist, path_provider);
            if self.waypoints.is_empty() {
                self.finish_flee(commands);
                return BehaviorStatus::Success;
            }
        }

        if self.should_sync_move() {
            commands.push(self.make_move_cmd());
        }
        BehaviorStatus::Running
    }

    /// True when the last known threat position is far enough away to stop
    /// fleeing. Returns false while the threat position is unknown.
    fn beyond_safe_dist(&self, safe_dist: f32) -> bool {
        match &self.last_known_target_pos {
            Some(threat) => self.position.dist_xz_sq(threat) >= safe_dist * safe_dist,
            None => false,
        }
    }

    fn finish_flee(&mut self, commands: &mut Vec<AiCommand>) {
        self.target_player_id = None;
        self.last_known_target_pos = None;
        self.transition_to_idle(commands);
    }

    fn bt_attack_target(
        &mut self,
        params: &HashMap<String, f32>,
        nearby_players: &[NearbyPlayer],
        commands: &mut Vec<AiCommand>,
        path_provider: &dyn PathProvider,
    ) -> BehaviorStatus {
        let target = match self.current_target(nearby_players) {
            Some(target) => target,
            None => return BehaviorStatus::Failure,
        };

        // Stacked attackers in one cell: the smallest id keeps it, the rest
        // yield back to chase for slots of their own. Entry stays refused
        // while the re-slot walk is under way so a yielder doesn't stop at
        // the first cell boundary, still overlapped. A committed swing lands
        // first, as with the range release below.
        if self.swing_left_ms <= 0.0 {
            if self.cell_yield {
                self.reslotting = true;
                return BehaviorStatus::Failure;
            }
            if self.reslotting && self.state != AiState::Attack {
                return BehaviorStatus::Failure;
            }
        }

        let range = param(params, "range", self.attack_range);
        // Engage well inside `range`, release further out (see
        // doc/MONSTER_SEPARATION.md 접근 거리, ATTACK_RELEASE_MARGIN_METERS).
        let limit = if self.state == AiState::Attack {
            range + ATTACK_RELEASE_MARGIN_METERS
        } else if self.as_close_as_we_get() {
            range
        } else {
            engage_limit(range)
        };
        // A swing already under way finishes; see `swing_left_ms`. A wall
        // between the two refuses the blow the way distance does, so the chase
        // is the only way left to reach the target.
        if self.swing_left_ms <= 0.0
            && (self.position.dist_xz_sq(&target.position) > limit * limit
                || path_provider.attack_line_blocked(
                    self.position.x,
                    self.position.z,
                    target.position.x,
                    target.position.z,
                    self.path_floor,
                ))
        {
            return BehaviorStatus::Failure;
        }

        let target_id = target.id;
        self.rotation = self
            .position
            .bearing_xz_to(&target.position)
            .unwrap_or(self.rotation);
        if self.state != AiState::Attack {
            self.state = AiState::Attack;
            self.state_timer_ms = 0.0;
            self.target_position = None;
            self.waypoints.clear();
            self.clear_path_bend();
            commands.push(self.make_move_cmd());
        }

        if self.attack_cooldown_left_ms <= 0.0 {
            self.attack_cooldown_left_ms = self.attack_cooldown_ms;
            self.swing_left_ms = self.swing_commit_ms;
            self.clear_path_bend();
            commands.push(self.make_move_cmd());
            commands.push(AiCommand::Attack {
                monster_id: self.monster_id.clone(),
                target_player_id: target_id,
            });
        }

        BehaviorStatus::Running
    }

    fn bt_chase_target(
        &mut self,
        params: &HashMap<String, f32>,
        delta_ms: f32,
        nearby_players: &[NearbyPlayer],
        commands: &mut Vec<AiCommand>,
        path_provider: &dyn PathProvider,
    ) -> BehaviorStatus {
        let target = match self.current_target(nearby_players) {
            Some(target) => target,
            None => return BehaviorStatus::Failure,
        };

        let target_pos = target.position;
        let path_recalc_ms = param(params, "pathRecalcMs", DEFAULT_PATH_RECALC_MS);
        let target_move_threshold =
            param(params, "targetMoveThreshold", DEFAULT_TARGET_MOVE_THRESHOLD);

        self.move_speed = self.run_speed;
        if self.state != AiState::Hold {
            self.state = AiState::Chase;
        }

        // While waiting on an unreachable target (empty waypoints, Hold),
        // retry only at repath cadence — every tick would flood A* at 60Hz.
        let exhausted = self.current_waypoint_idx >= self.waypoints.len();
        let target_moved = self.target_moved_significantly_by(&target_pos, target_move_threshold);
        let needs_repath = (exhausted && self.state != AiState::Hold)
            || self.path_elapsed_ms > path_recalc_ms
            || target_moved;

        if needs_repath {
            if target_moved {
                self.detour_goal = None;
            }
            self.last_known_target_pos = Some(target_pos);
            if !self.continue_detour(path_provider)
                && !self.path_to_chase_slot(&target_pos, path_provider)
            {
                // No reachable free cell — head for the target itself; the
                // advance gate still queues us behind standers. A* answers an
                // unreachable target with a partial path (found=false):
                // walking it brings the pack up to the door, and once the
                // partial leg stops covering ground it counts as no path, so
                // the chase waits instead of re-arriving every tick.
                let result = self.query_path(target_pos.x, target_pos.z, path_provider);
                let keep = result.found
                    || path_len(self.position.x, self.position.z, &result.waypoints)
                        > MIN_PARTIAL_PROGRESS_METERS;
                self.install_path(if keep { result.waypoints } else { Vec::new() });
            }
            if self.waypoints.is_empty() {
                // Unreachable target (a shut door): wait here instead of
                // failing into a frantic wander. A bunched wait still
                // spreads — a cell-sharer steps one cell aside when it can.
                if !(self.cell_yield && self.try_sidestep(&target_pos, path_provider, false)) {
                    self.enter_hold(commands);
                    return BehaviorStatus::Running;
                }
            }
        }

        if self.state == AiState::Hold && self.current_waypoint_idx >= self.waypoints.len() {
            return BehaviorStatus::Running;
        }

        // A chase transition syncs before its first step: remote clients
        // still show the old motion, and a post-step sync jolts them a whole
        // tick. It also stands in for this tick's interval sync.
        let pre_synced = self.state == AiState::Chase && self.state != self.last_synced_state;
        if pre_synced {
            self.sync_chase(target_pos, commands);
        }
        let (reached, held) =
            self.follow_path_engaging(delta_ms, target_pos, self.chase_stop_range());
        if held {
            // Flow around the blocker, or back out and round the queue, when
            // possible; retry at repath cadence, not every tick.
            if (self.state != AiState::Hold || needs_repath)
                && (self.try_sidestep(&target_pos, path_provider, true)
                    || self.try_detour(&target_pos, path_provider))
            {
                self.state = AiState::Chase;
                if !pre_synced {
                    self.sync_chase(target_pos, commands);
                }
                return BehaviorStatus::Running;
            }
            self.enter_hold(commands);
            return BehaviorStatus::Running;
        }
        // A Hold that got a path again resumes; the state change makes
        // `should_sync_move` report the transition at once.
        self.state = AiState::Chase;
        if reached {
            self.reslotting = false;
            self.detour_goal = None;
        }
        if !pre_synced {
            self.sync_chase(target_pos, commands);
        }
        BehaviorStatus::Running
    }

    /// Radius the walk (and the position it reports to remote clients) stops
    /// on, `None` while walking to a standing cell: that path runs to the
    /// cell, since stopping short leaves us mid-cell, in someone else's. Just
    /// inside `engage_limit` so the attack's range check is not a float
    /// coin-flip at the boundary.
    fn chase_stop_range(&self) -> Option<f32> {
        if self.chase_goal_cell.is_some() {
            None
        } else {
            Some(engage_limit(self.attack_range) - ENGAGE_INSET_METERS)
        }
    }

    /// No closer approach is coming — standing at the cell the separation grid
    /// handed us with the path spent, or held with nowhere to step (queued
    /// behind a stander, or the target walled off) — so the full reach counts.
    fn as_close_as_we_get(&self) -> bool {
        self.state == AiState::Hold
            || (self.current_waypoint_idx >= self.waypoints.len()
                && (self.chase_goal_cell.is_none()
                    || self.chase_goal_cell == Some(cell_of(self.position.x, self.position.z))))
    }

    fn sync_chase(&mut self, target_pos: crate::Position, commands: &mut Vec<AiCommand>) {
        if self.should_sync_move() {
            commands.push(self.make_chase_move_cmd(target_pos));
        }
    }

    /// Enter (or stay in) the chase hold — queued behind a stander, or
    /// waiting on an unreachable target. `AiState::Hold` reports as Idle so
    /// remote clients stop the model.
    fn enter_hold(&mut self, commands: &mut Vec<AiCommand>) {
        self.reslotting = false;
        if self.state == AiState::Hold {
            return;
        }
        self.state = AiState::Hold;
        self.state_timer_ms = 0.0;
        self.target_position = None;
        // Consume the state-change sync trigger; the pose goes out with this
        // command instead.
        self.should_sync_move();
        commands.push(self.make_move_cmd());
    }

    /// Remote clients walk toward `target_position` until the next sync, so
    /// it must lie on the path, a couple of sync intervals ahead — aiming
    /// them at the target itself overshoots the standing cell the chase
    /// actually stops at, and every sync yanks the model back.
    fn make_chase_move_cmd(&self, target_pos: crate::Position) -> AiCommand {
        let lookahead = self.move_speed * NETWORK_SYNC_INTERVAL_MS / 1000.0 * 2.0;
        let engage = self.chase_stop_range().map(|r| (target_pos, r));
        self.move_cmd_to(self.path_lookahead(lookahead, engage))
    }

    /// A held chaser flows around the blocker NetHack-style: step into an
    /// adjacent free cell strictly closer to its goal. Cardinal only — a
    /// diagonal's swept line clips the very cell that blocked us. Replaces the
    /// current leg; the next repath rebuilds the full path from the new cell.
    fn try_sidestep(
        &mut self,
        target_pos: &crate::Position,
        path_provider: &dyn PathProvider,
        require_closer: bool,
    ) -> bool {
        let (goal_x, goal_z) = match self.chase_goal_cell {
            Some(cell) => cell_center(cell),
            None => (target_pos.x, target_pos.z),
        };
        let here = cell_of(self.position.x, self.position.z);
        let target_cell = cell_of(target_pos.x, target_pos.z);
        let cur_dx = shortest_world_delta_x(self.position.x, goal_x);
        let cur_dz = goal_z - self.position.z;
        let cur_d = cur_dx * cur_dx + cur_dz * cur_dz;

        // Route-closer, not just euclidean-closer: near a wall the
        // straight-line gradient points into dead pockets, and a sidestep
        // picked by it undoes the goal path every cycle — walk 0.6m up,
        // repath walks 0.6m back down, forever (the dungeon-wall jog). Only
        // a candidate whose own route to the goal is strictly shorter than
        // the one we hold makes progress.
        let cur_route = path_len(
            self.position.x,
            self.position.z,
            &self.waypoints[self.current_waypoint_idx.min(self.waypoints.len())..],
        );
        if require_closer && cur_route <= 0.0 {
            // No candidate route can be strictly shorter than nothing.
            return false;
        }

        // Score the cardinals by cheap checks first; A* runs below, in order,
        // only until one succeeds.
        let mut candidates: Vec<((f32, f32), f32)> = Vec::new();
        for &(ox, oz) in crate::pathfinding::DIRS.iter() {
            let cell = (here.0 + ox, here.1 + oz);
            let (cx, cz) = cell_center(cell);
            if cell == target_cell
                || self.occupied_cells.contains(&cell)
                || !path_provider.cell_passable(cx, cz, self.path_floor)
            {
                continue;
            }
            let dx = shortest_world_delta_x(cx, goal_x);
            let dz = goal_z - cz;
            let d = dx * dx + dz * dz;
            if require_closer && d >= cur_d {
                continue;
            }
            candidates.push(((cx, cz), d));
        }
        candidates.sort_by(|a, b| a.1.total_cmp(&b.1));

        for ((cx, cz), _) in candidates {
            let result = self.query_path(cx, cz, path_provider);
            // The leg must not cross an occupied cell either: a wall can make
            // A* route to a free neighbor *through* someone else's cell, and
            // the advance gate would stop that leg on its first crossing —
            // then the next tick re-picks the exact same sidestep, forever
            // (the stair-wall in-place jog).
            if !result.found
                || result.waypoints.is_empty()
                || path_len(self.position.x, self.position.z, &result.waypoints)
                    > SIDESTEP_MAX_PATH_METERS
                || leg_crosses_occupied(
                    self.position.x,
                    self.position.z,
                    &result.waypoints,
                    &self.occupied_cells,
                )
            {
                continue;
            }
            if require_closer {
                let cand_route = self.query_path_from(cx, cz, goal_x, goal_z, path_provider);
                if !cand_route.found || path_len(cx, cz, &cand_route.waypoints) >= cur_route {
                    continue;
                }
            }
            // Deliberately not `compute_path`: the repath timer keeps running
            // so the interrupted goal is re-tried on its normal cadence.
            self.waypoints = result.waypoints;
            self.current_waypoint_idx = 0;
            self.face_first_waypoint();
            return true;
        }
        false
    }

    /// Back out and re-route around the standers; the goal sticks in
    /// `detour_goal` so repaths keep avoiding instead of rejoining the queue.
    fn try_detour(
        &mut self,
        target_pos: &crate::Position,
        path_provider: &dyn PathProvider,
    ) -> bool {
        let goal = match self.chase_goal_cell {
            Some(cell) => cell_center(cell),
            None => (target_pos.x, target_pos.z),
        };
        self.detour_goal = Some(goal);
        self.continue_detour(path_provider)
    }

    /// Re-route toward `detour_goal` around the standers. Drops the detour
    /// (false) when no local one remains — the caller re-slots as usual.
    fn continue_detour(&mut self, path_provider: &dyn PathProvider) -> bool {
        let Some((goal_x, goal_z)) = self.detour_goal else {
            return false;
        };
        let local_goal_x = self.position.x + shortest_world_delta_x(self.position.x, goal_x);
        let mut result = path_provider.find_path_avoiding(
            self.position.x,
            self.position.z,
            self.path_floor,
            local_goal_x,
            goal_z,
            self.path_floor,
            &self.occupied_cells,
            DETOUR_MAX_NODES,
        );
        for wp in &mut result.waypoints {
            wp.x = wrap_world_x(wp.x);
        }
        if !result.found
            || result.waypoints.is_empty()
            || path_len(self.position.x, self.position.z, &result.waypoints)
                > DETOUR_MAX_PATH_METERS
        {
            self.detour_goal = None;
            return false;
        }
        self.install_path(result.waypoints);
        self.face_first_waypoint();
        true
    }

    /// Rule 1 of doc/MONSTER_SEPARATION.md: path to a free standing cell near
    /// the target, nearest to the target first, trying the next-best cell when
    /// one is unreachable — the nearest may sit outside a corridor. Keeps the
    /// previous goal cell while it stays valid and reachable so re-slotting
    /// doesn't oscillate. False = no reachable free cell (waypoints left
    /// empty; caller falls back to the raw target position).
    fn path_to_chase_slot(
        &mut self,
        target_pos: &crate::Position,
        path_provider: &dyn PathProvider,
    ) -> bool {
        let max_d = self.attack_range - CHASE_CELL_RANGE_MARGIN;
        if max_d <= 0.0 {
            self.chase_goal_cell = None;
            return false;
        }

        if let Some(cell) = self.chase_goal_cell {
            if self.chase_cell_valid(cell, target_pos, max_d, path_provider)
                && self.try_goal_cell(cell, path_provider)
            {
                return true;
            }
        }

        let target_cell = cell_of(target_pos.x, target_pos.z);
        let r = max_d.ceil() as i32;
        let base_x = target_pos.x.floor();
        let base_z = target_pos.z.floor();
        // Our side of the target first (side 0), nearest the engage distance
        // within that side: ranking the whole ring by our own distance let a
        // chaser settle on the first cell that merely had the target in reach
        // and swing from the edge of its range, and ranking by closeness alone
        // walks a long-reach monster into the target's lap. Cells past the
        // target rank last, by our distance — reaching one means rounding the
        // target, and the leg would cross its cell.
        let engage = engage_limit(self.attack_range);
        let approach_x = shortest_world_delta_x(target_pos.x, self.position.x);
        let approach_z = self.position.z - target_pos.z;
        let mut candidates: Vec<((i32, i32), (u8, f32))> = Vec::new();
        for ox in -r..=r {
            for oz in -r..=r {
                let cx = wrap_world_x(base_x + ox as f32 + 0.5);
                let cz = base_z + oz as f32 + 0.5;
                let cell = cell_of(cx, cz);
                let Some(target_d_sq) = self.chase_cell_open(cell, target_cell, target_pos, max_d)
                else {
                    continue;
                };
                let tdx = shortest_world_delta_x(target_pos.x, cx);
                let tdz = cz - target_pos.z;
                let cost = if tdx * approach_x + tdz * approach_z >= 0.0 {
                    (0, (target_d_sq.sqrt() - engage).abs())
                } else {
                    let dx = shortest_world_delta_x(self.position.x, cx);
                    let dz = cz - self.position.z;
                    (1, dx * dx + dz * dz)
                };
                candidates.push((cell, cost));
            }
        }
        candidates.sort_by(|a, b| a.1 .0.cmp(&b.1 .0).then(a.1 .1.total_cmp(&b.1 .1)));

        // The raycast half of validity is deferred to here — the sort
        // discards most of the scan, so only cells about to be path-tested
        // pay it. Invalid cells don't consume path tries.
        let mut tries = 0;
        for (cell, _) in candidates {
            if tries >= MAX_SLOT_PATH_TRIES {
                break;
            }
            if !self.chase_cell_attackable(cell, target_pos, path_provider) {
                continue;
            }
            tries += 1;
            if self.try_goal_cell(cell, path_provider) {
                return true;
            }
        }
        self.chase_goal_cell = None;
        false
    }

    /// Commit a goal cell: path to its center, keeping the cell only when the
    /// goal is actually reachable. `found` is load-bearing: A* answers an
    /// unreachable goal with a partial path toward it, and accepting that
    /// made the two front monsters at a shut door ping-pong forever — each
    /// "reached" the partial end in the other's lane and re-slotted to the
    /// opposite player-side cell. (The raw-target fallback deliberately keeps
    /// partial paths: its goal is fixed, so it converges at the door and
    /// holds.)
    fn try_goal_cell(&mut self, cell: (i32, i32), path_provider: &dyn PathProvider) -> bool {
        let (gx, gz) = cell_center(cell);
        let result = self.query_path(gx, gz, path_provider);
        if !result.found || result.waypoints.is_empty() {
            return false;
        }
        self.install_path(result.waypoints);
        self.chase_goal_cell = Some(cell);
        true
    }

    /// Cheap half of candidate validity: not the target's own cell,
    /// unoccupied, and close enough that standing at its center can deliver
    /// the attack. `Some` carries the cell's squared distance to the target,
    /// which the caller ranks by.
    fn chase_cell_open(
        &self,
        cell: (i32, i32),
        target_cell: (i32, i32),
        target_pos: &crate::Position,
        max_d: f32,
    ) -> Option<f32> {
        if cell == target_cell || self.occupied_cells.contains(&cell) {
            return None;
        }
        let (cx, cz) = cell_center(cell);
        let dx = shortest_world_delta_x(cx, target_pos.x);
        let dz = target_pos.z - cz;
        let d_sq = dx * dx + dz * dz;
        (d_sq <= max_d * max_d).then_some(d_sq)
    }

    /// Raycast half of candidate validity: standable, and with a clear attack
    /// line. The line check is load-bearing: a target up a stair (or behind a
    /// shut door) projects XZ-near cells on our own floor — settling in one
    /// would leave us attack-refused against the wall, re-arriving every
    /// tick, running in place. Rejecting them falls back to chasing the raw
    /// target position, whose stair-cell goal is what lets the path climb
    /// after it.
    fn chase_cell_attackable(
        &self,
        cell: (i32, i32),
        target_pos: &crate::Position,
        path_provider: &dyn PathProvider,
    ) -> bool {
        let (cx, cz) = cell_center(cell);
        path_provider.cell_passable(cx, cz, self.path_floor)
            && !path_provider.attack_line_blocked(
                cx,
                cz,
                target_pos.x,
                target_pos.z,
                self.path_floor,
            )
    }

    fn chase_cell_valid(
        &self,
        cell: (i32, i32),
        target_pos: &crate::Position,
        max_d: f32,
        path_provider: &dyn PathProvider,
    ) -> bool {
        self.chase_cell_open(cell, cell_of(target_pos.x, target_pos.z), target_pos, max_d)
            .is_some()
            && self.chase_cell_attackable(cell, target_pos, path_provider)
    }

    fn select_target_in_range(&mut self, nearby_players: &[NearbyPlayer], range: f32) -> bool {
        let range_sq = range * range;

        if let Some(target_id) = &self.target_player_id {
            return nearby_players.iter().any(|p| {
                p.id == *target_id
                    && p.health > 0
                    && self.position.dist_xz_sq(&p.position) <= range_sq
            });
        }

        let selected = nearby_players
            .iter()
            .filter_map(|p| {
                let dist_sq = self.position.dist_xz_sq(&p.position);
                (p.health > 0 && dist_sq <= range_sq).then_some((dist_sq, p))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, p)| p.id);

        if let Some(id) = selected {
            self.target_player_id = Some(id);
            true
        } else {
            false
        }
    }

    pub(super) fn current_target<'a>(
        &self,
        nearby_players: &'a [NearbyPlayer],
    ) -> Option<&'a NearbyPlayer> {
        let target_id = self.target_player_id.as_ref()?;
        nearby_players
            .iter()
            .find(|p| p.id == *target_id && p.health > 0)
    }
}
