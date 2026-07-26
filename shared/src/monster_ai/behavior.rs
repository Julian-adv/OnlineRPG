//! Behavior tree execution for [`MonsterBrain`]: node traversal plus the
//! condition and action handlers (`bt_*`) that drive the brain's state.

use super::tree::BehaviorStatus;
use super::{
    param, AiCommand, AiState, BehaviorNode, MonsterBrain, NearbyPlayer, PathProvider,
    ATTACK_RELEASE_MARGIN_METERS, DEFAULT_FLEE_HEALTH_RATIO, DEFAULT_FLEE_MAX_DURATION_MS,
    DEFAULT_IDLE_CHECK_MS, DEFAULT_LEASH_RANGE, DEFAULT_MAX_MOVE_DIST, DEFAULT_MIN_MOVE_DIST,
    DEFAULT_PATH_RECALC_MS, DEFAULT_RETURN_ARRIVE_DIST, DEFAULT_TARGET_MOVE_THRESHOLD,
    FLEE_SAFE_DIST_MARGIN,
};
use crate::MonsterState;
use rand::Rng;
use std::collections::HashMap;

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
                let dx = self.position.x - self.spawn_position.x;
                let dz = self.position.z - self.spawn_position.z;
                (self.state == AiState::Return || dx * dx + dz * dz > range * range).into()
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
            "attack_target" => self.bt_attack_target(params, nearby_players, commands),
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
        let dx = self.spawn_position.x - self.position.x;
        let dz = self.spawn_position.z - self.position.z;
        if dx * dx + dz * dz <= arrive_dist * arrive_dist {
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
    ) -> BehaviorStatus {
        let target = match self.current_target(nearby_players) {
            Some(target) => target,
            None => return BehaviorStatus::Failure,
        };

        let dx = target.position.x - self.position.x;
        let dz = target.position.z - self.position.z;
        let range = param(params, "range", self.attack_range);
        // Engage at `range`, release further out: see ATTACK_RELEASE_MARGIN_METERS.
        let limit = if self.state == AiState::Attack {
            range + ATTACK_RELEASE_MARGIN_METERS
        } else {
            range
        };
        // A swing already under way finishes; see `swing_left_ms`.
        if self.swing_left_ms <= 0.0 && dx * dx + dz * dz > limit * limit {
            return BehaviorStatus::Failure;
        }

        let target_id = target.id;
        self.rotation = dx.atan2(dz);
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

        self.state = AiState::Chase;
        self.move_speed = self.run_speed;

        let needs_repath = self.waypoints.is_empty()
            || self.current_waypoint_idx >= self.waypoints.len()
            || self.path_elapsed_ms > path_recalc_ms
            || self.target_moved_significantly_by(&target_pos, target_move_threshold);

        if needs_repath {
            self.compute_path(target_pos.x, target_pos.z, path_provider);
            self.last_known_target_pos = Some(target_pos);
            if self.waypoints.is_empty() {
                return BehaviorStatus::Failure;
            }
        }

        self.follow_path(delta_ms);
        if self.should_sync_move() {
            commands.push(AiCommand::Move {
                monster_id: self.monster_id.clone(),
                position: self.position,
                rotation: self.rotation,
                state: MonsterState::Run,
                target_position: target_pos,
            });
        }
        BehaviorStatus::Running
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
