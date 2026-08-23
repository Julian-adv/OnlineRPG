//! Move-coupled ambient spawning (doc/REPEAT_FARMING.md, part 1).
//!
//! Monsters are granted by distance walked, not by the clock: every movement
//! tick rolls `1 - (1 - p)^d` over that tick's displacement, so standing still
//! yields nothing and how the client chops its moves up does not matter. The
//! server picks the position — just outside the screen, on the side the player
//! is heading — validates it and walks the line back to the player, so a
//! monster never appears somewhere it could not walk in from.

use crate::types::{MonsterLifecycle, PlayerId, Position, ServerMessage};
use onlinerpg_shared::{shortest_world_delta_x, wrap_world_x, MAX_MOVE_TARGET_DISTANCE};
use rand::Rng;
use tracing::debug;

/// Chance of a spawn per metre walked: one monster per 24.5m, capping a
/// nonstop runner near 660 kills/hour. Tuned by playtest
/// (doc/REPEAT_FARMING.md).
const SPAWN_CHANCE_PER_METER: f64 = 0.04;
/// Half-edge of the screen-aligned square spawns land on. Covers the visible
/// ground up to a 2.0 aspect ratio, and its corner stays inside AOI
/// (`EVENT_DELIVERY_RADIUS`) so the owner's client can simulate the monster.
const SPAWN_HALF_EDGE: f32 = 20.0;
// The corner is the far point of that square: outside AOI it would spawn
// monsters the owner can neither see nor simulate.
const _: () = assert!(
    2.0 * SPAWN_HALF_EDGE * SPAWN_HALF_EDGE
        < super::EVENT_DELIVERY_RADIUS * super::EVENT_DELIVERY_RADIUS
);
/// Cell size of the reachability walk — the resolution of both the passability
/// cache and the terrain grid.
const REACH_STEP_METERS: f32 = 1.0;
/// Ground rise a monster may cross in one cell before the line reads as a
/// cliff (45°).
const MAX_CLIMB_PER_CELL: f32 = 1.0;

/// One player's displacement over a movement tick.
pub(super) struct MoveStep {
    pub player_id: PlayerId,
    pub from: Position,
    pub to: Position,
    pub floor_level: i8,
    pub is_official_npc: bool,
}

impl super::GameState {
    /// Roll each mover's walked distance for an ambient spawn and place the
    /// winners. Runs after `tick_player_movement` drops its locks.
    pub(super) async fn spawn_along_movement(&self, steps: &[MoveStep]) {
        #[cfg(test)]
        if !self
            .ambient_spawns_enabled
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        if crate::world_config::world_config()
            .ambient_spawns
            .is_empty()
        {
            return;
        }
        for step in steps {
            // Surface only: house storeys and dungeons have their own spawns,
            // and the placement below is XZ-only.
            if step.floor_level != 0 {
                continue;
            }
            let dx = shortest_world_delta_x(step.from.x, step.to.x);
            let dz = step.to.z - step.from.z;
            let distance = (dx * dx + dz * dz).sqrt();
            // A jump longer than any legitimate move leg is a teleport, not a
            // walk: it earns nothing rather than a near-certain spawn.
            if !distance.is_finite() || distance <= 0.0 || distance > MAX_MOVE_TARGET_DISTANCE {
                continue;
            }
            let chance = 1.0 - (1.0 - SPAWN_CHANCE_PER_METER).powf(f64::from(distance));
            if !rand::thread_rng().gen_bool(chance) {
                continue;
            }
            self.spawn_ahead_of(step, dx, dz).await;
        }
    }

    /// Place one monster just off the screen edge the player is heading for.
    /// Every gate is a plain reject — the roll is what rations spawns — and
    /// they run cheapest first: arithmetic, then locks, then terrain.
    async fn spawn_ahead_of(&self, step: &MoveStep, dx: f32, dz: f32) {
        let point = Self::screen_edge_point(&step.to, dx, dz);
        for zone in &self.no_spawn_zones {
            if zone.contains_with_margin(point.x, point.z, super::monster::NO_SPAWN_MARGIN) {
                return;
            }
        }
        let Some(monster_type) = self.pick_ambient_type(&point) else {
            return;
        };
        let max_per_player = crate::world_config::world_config().max_monsters_per_player as usize;
        if self.monsters.read().await.owned_alive_by(&step.player_id) >= max_per_player {
            return;
        }
        // An agent nobody is watching draws no monsters.
        if step.is_official_npc && !self.human_watching(&step.to).await {
            return;
        }
        if !self
            .splat_sampler
            .is_vegetation_base_at(point.x, point.z)
            .await
        {
            return;
        }
        let Some(y) = self.dry_ground_at(point.x, point.z).await else {
            return;
        };
        let position = Position { y, ..point };
        if !self.walkable_line(&position, &step.to).await {
            return;
        }

        let rotation = position.bearing_xz_to(&step.to).unwrap_or(0.0);
        if let Some(monster) = self
            .spawn_monster(
                monster_type.to_string(),
                position,
                rotation,
                Some(step.player_id),
                0,
                MonsterLifecycle::Ambient,
                None,
                false,
            )
            .await
        {
            // The owner runs the AI, and it only starts on this message.
            self.send_direct_message(&step.player_id, ServerMessage::MonsterAssigned { monster })
                .await;
        }
    }

    /// A point on the screen-aligned square of half-edge `SPAWN_HALF_EDGE`
    /// around `center`, on the edge the player is walking toward.
    ///
    /// The camera is isometric (yaw -45°), so the screen axes are the world
    /// diagonals: `u = (x + z)/√2` runs right, `v = (x - z)/√2` runs up. The
    /// √2 cancels out of the comparison, leaving two adds and an absolute
    /// value to decide which edge the heading points at.
    pub(super) fn screen_edge_point(center: &Position, dx: f32, dz: f32) -> Position {
        let free = rand::Rng::gen_range(&mut rand::thread_rng(), -SPAWN_HALF_EDGE..SPAWN_HALF_EDGE);
        let edge = |along: f32| SPAWN_HALF_EDGE * along.signum();
        let (u, v) = if (dx + dz).abs() >= (dx - dz).abs() {
            (edge(dx + dz), free)
        } else {
            (free, edge(dx - dz))
        };
        let inv_sqrt2 = std::f32::consts::FRAC_1_SQRT_2;
        Position {
            x: wrap_world_x(center.x + (u + v) * inv_sqrt2),
            y: center.y,
            z: center.z + (u - v) * inv_sqrt2,
        }
    }

    /// Uniform over the ambient types allowed this far from town. What you
    /// meet still follows where you stand, as it did before.
    pub(super) fn pick_ambient_type(&self, point: &Position) -> Option<&'static str> {
        let town_distance = self.town_distance(point);
        rand::seq::IteratorRandom::choose(
            crate::world_config::world_config()
                .ambient_spawns
                .iter()
                .map(|rule| rule.monster_type.as_str())
                .filter(|ty| town_distance >= self.min_ambient_town_distance(ty)),
            &mut rand::thread_rng(),
        )
    }

    /// Ground height at a point, or `None` if it stands in water (sea or
    /// river) or the terrain cannot be sampled.
    async fn dry_ground_at(&self, x: f32, z: f32) -> Option<f32> {
        let (ground, depth) = self.ground_and_depth_at(x, z).await?;
        (depth <= 0.0).then_some(ground)
    }

    /// Can a monster walk from the spawn point to the player? Buildings and
    /// furniture answer from the passability cache in one sweep; the terrain
    /// is checked cell by cell, since the cache holds no terrain at all.
    /// No pathfinding — a blocked line just loses this spawn.
    async fn walkable_line(&self, from: &Position, to: &Position) -> bool {
        if onlinerpg_shared::pathfinding::attack_line_blocked(
            &self.passability_read(),
            from.x,
            from.z,
            to.x,
            to.z,
            onlinerpg_shared::dungeon::passability_floor_for_level(0),
        ) {
            return false;
        }
        let dx = shortest_world_delta_x(from.x, to.x);
        let dz = to.z - from.z;
        let length = (dx * dx + dz * dz).sqrt();
        let mut travelled = REACH_STEP_METERS;
        let mut last_ground = from.y;
        while travelled < length {
            let t = travelled / length;
            let x = wrap_world_x(from.x + dx * t);
            let z = from.z + dz * t;
            let Some(ground) = self.dry_ground_at(x, z).await else {
                return false;
            };
            if (ground - last_ground).abs() > MAX_CLIMB_PER_CELL {
                debug!("Ambient spawn dropped: line to player blocked at ({x:.1},{z:.1})");
                return false;
            }
            last_ground = ground;
            travelled += REACH_STEP_METERS;
        }
        true
    }

    /// Opt this state into move-coupled ambient spawning (off by default in
    /// tests, so walking a player around draws no monsters unasked).
    #[cfg(test)]
    pub(crate) fn enable_ambient_spawns(&self) {
        self.ambient_spawns_enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Is a human player close enough to see monsters spawned here?
    async fn human_watching(&self, position: &Position) -> bool {
        let nearby = self
            .player_ids_within_position(position, 0, super::EVENT_DELIVERY_RADIUS)
            .await;
        let players = self.players.read().await;
        nearby
            .iter()
            .any(|id| players.get(id).is_some_and(|p| !p.is_official_npc))
    }
}
