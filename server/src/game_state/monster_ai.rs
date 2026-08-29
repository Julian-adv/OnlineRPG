//! Server-driven monster brains (doc/SERVER_SIDE_MONSTER_AI.md). The same
//! `shared::monster_ai` runtime the clients ran, ticked here so a modified
//! client cannot park, herd or aim the monsters it used to own. Ownership in
//! the registry stays as spawn-cap bookkeeping; on the wire it reads `None`.

use crate::types::{Monster, MonsterState, PlayerId, Position, ServerMessage};
use onlinerpg_shared::dungeon::passability_floor_for_level;
use onlinerpg_shared::monster_ai::{
    self, AiCommand, BehaviorTree, CachePathProvider, MonsterBrain, NearbyMonster, NearbyPlayer,
    PathProvider, AGGRESSIVE_BEHAVIOR, DEFAULT_BEHAVIOR,
};
use onlinerpg_shared::pathfinding::{is_movement_blocked, PathResult};
use onlinerpg_shared::shortest_world_delta_x;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// CPU spent on brains per tick before the rest wait for the next one.
const TICK_BUDGET: Duration = Duration::from_millis(40);
/// A brain skipped for long (budget starvation, or nobody near it) resumes
/// with at most this much simulated time, so it can't leap a whole path.
const MAX_BRAIN_DELTA_MS: f32 = 1000.0;
const STATS_LOG_PERIOD: Duration = Duration::from_secs(30);

struct Entry {
    brain: MonsterBrain,
    last_tick: Instant,
}

impl Entry {
    /// Simulated time owed to the brain up to `now`, and mark it paid.
    fn owed_ms(&mut self, now: Instant, forced: Option<f32>) -> f32 {
        let delta = forced.unwrap_or_else(|| (now - self.last_tick).as_secs_f32() * 1000.0);
        self.last_tick = now;
        delta.min(MAX_BRAIN_DELTA_MS)
    }
}

#[derive(Default)]
struct Stats {
    ticks: u32,
    ticked: u64,
    pathfinds: u64,
    commands: u64,
    over_budget: u32,
    worst_ms: f32,
}

pub(crate) struct ServerBrains {
    entries: HashMap<String, Entry>,
    trees: HashMap<String, BehaviorTree>,
    /// Round-robin start so budget starvation rotates instead of always
    /// hitting the same tail.
    cursor: usize,
    stats: Stats,
    stats_since: Instant,
}

impl ServerBrains {
    pub(crate) fn new() -> Self {
        let trees =
            monster_ai::load_behavior_trees(include_str!("../../../data-src/behavior_trees.json"))
                .expect("behavior_trees.json is malformed");
        Self {
            entries: HashMap::new(),
            trees,
            cursor: 0,
            stats: Stats::default(),
            stats_since: Instant::now(),
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Counts path queries for the tick stats.
struct CountingPath<'a> {
    inner: CachePathProvider<'a>,
    count: Cell<u64>,
}

impl PathProvider for CountingPath<'_> {
    fn find_path(&self, sx: f32, sz: f32, sf: u8, gx: f32, gz: f32, gf: u8) -> PathResult {
        self.count.set(self.count.get() + 1);
        self.inner.find_path(sx, sz, sf, gx, gz, gf)
    }

    fn attack_line_blocked(&self, fx: f32, fz: f32, tx: f32, tz: f32, floor: u8) -> bool {
        self.inner.attack_line_blocked(fx, fz, tx, tz, floor)
    }

    fn cell_passable(&self, x: f32, z: f32, floor: u8) -> bool {
        self.inner.cell_passable(x, z, floor)
    }

    fn find_path_avoiding(
        &self,
        sx: f32,
        sz: f32,
        sf: u8,
        gx: f32,
        gz: f32,
        gf: u8,
        blocked: &[(i32, i32)],
        max_nodes: usize,
    ) -> PathResult {
        self.count.set(self.count.get() + 1);
        self.inner
            .find_path_avoiding(sx, sz, sf, gx, gz, gf, blocked, max_nodes)
    }
}

type Roster = HashMap<super::SpatialCell, Vec<(PlayerId, Position, u32, i8)>>;

/// A live monster someone can see this tick.
struct Active {
    id: String,
    floor_level: i8,
    players: Vec<NearbyPlayer>,
}

impl super::GameState {
    pub(crate) fn server_monster_ai(&self) -> bool {
        self.server_monster_ai
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn enable_server_monster_ai(&self) {
        self.server_monster_ai
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// The monster as clients should see it: no owner while the server drives
    /// it, or the cap holder's client would start a brain of its own.
    pub(super) fn wire_monster(&self, monster: &Monster) -> Monster {
        let mut monster = monster.clone();
        monster.owner_id = self.wire_owner(monster.owner_id);
        monster
    }

    pub(super) fn wire_owner(&self, owner_id: Option<PlayerId>) -> Option<PlayerId> {
        if self.server_monster_ai() {
            None
        } else {
            owner_id
        }
    }

    fn new_brain(&self, monster: &Monster) -> MonsterBrain {
        let def = self.monster_defs.get(&monster.monster_type);
        let behavior = if monster.aggressive {
            AGGRESSIVE_BEHAVIOR.to_string()
        } else {
            def.and_then(|d| d.behavior.clone())
                .unwrap_or_else(|| DEFAULT_BEHAVIOR.to_string())
        };
        let mut brain = MonsterBrain::new(
            monster.id.clone(),
            monster.monster_type.clone(),
            behavior,
            monster.position,
            monster.health,
            monster.max_health,
            def.map_or(monster_ai::DEFAULT_WALK_SPEED, |d| d.walk_speed),
            def.map_or(monster_ai::DEFAULT_RUN_SPEED, |d| d.run_speed),
            def.map_or(monster_ai::DEFAULT_ATTACK_RANGE, |d| d.attack_range),
            def.map_or(monster_ai::DEFAULT_CHASE_RANGE, |d| d.chase_range),
            def.map_or(monster_ai::DEFAULT_ATTACK_COOLDOWN_MS, |d| {
                d.attack_cooldown as f32
            }),
        );
        brain.path_floor = passability_floor_for_level(monster.floor_level);
        brain.rotation = monster.rotation;
        brain
    }

    /// One 200ms step of every brain someone can see, within `TICK_BUDGET`.
    pub async fn tick_monster_ai(&self) {
        self.tick_monster_ai_with(None).await;
    }

    /// Tests drive simulated time explicitly instead of waiting it out.
    #[cfg(test)]
    pub(crate) async fn tick_monster_ai_by(&self, delta_ms: f32) {
        self.tick_monster_ai_with(Some(delta_ms)).await;
    }

    async fn tick_monster_ai_with(&self, forced_delta_ms: Option<f32>) {
        if !self.server_monster_ai() {
            return;
        }
        let started = Instant::now();
        let roster: Roster = {
            let now = Self::now_ms();
            let players = self.players.read().await;
            let mut roster = Roster::default();
            for (id, p) in players.iter() {
                if !p.is_ready(now) {
                    continue;
                }
                roster
                    .entry(super::SpatialCell::from_position(&p.position))
                    .or_default()
                    .push((*id, p.position, p.health, p.floor_level));
            }
            roster
        };
        let radius = super::EVENT_DELIVERY_RADIUS;
        let radius_sq = radius * radius;
        let players_near = |position: &Position, floor: i8| -> Vec<NearbyPlayer> {
            let mut seen = HashSet::new();
            super::SpatialCell::within_radius(position, radius)
                .filter_map(|cell| roster.get(&cell))
                .flatten()
                .filter(|(id, p, _, f)| {
                    *f == floor && position.dist_xz_sq(p) <= radius_sq && seen.insert(*id)
                })
                .map(|(id, p, health, _)| NearbyPlayer {
                    id: *id,
                    position: *p,
                    health: *health,
                })
                .collect()
        };

        let mut brains = self.monster_brains.lock().await;
        // Reconcile against the registry and snapshot what to tick, in one
        // pass under the read lock.
        let (active, standing) = {
            let monsters = self.monsters.read().await;
            brains.entries.retain(|id, _| {
                monsters
                    .get(id)
                    .is_some_and(|m| m.state != MonsterState::Dead)
            });
            let mut active = Vec::new();
            let mut standing: HashMap<super::SpatialCell, Vec<NearbyMonster>> = HashMap::new();
            for m in monsters.values() {
                if m.state == MonsterState::Dead || m.health == 0 {
                    continue;
                }
                let players = players_near(&m.position, m.floor_level);
                if players.is_empty() {
                    continue;
                }
                if m.state.is_stationary() {
                    standing
                        .entry(super::SpatialCell::from_position(&m.position))
                        .or_default()
                        .push(NearbyMonster {
                            id: m.id.clone(),
                            position: m.position,
                            state: m.state,
                            path_floor: passability_floor_for_level(m.floor_level),
                        });
                }
                if !brains.entries.contains_key(&m.id) {
                    brains.entries.insert(
                        m.id.clone(),
                        Entry {
                            brain: self.new_brain(m),
                            last_tick: started,
                        },
                    );
                }
                active.push(Active {
                    id: m.id.clone(),
                    floor_level: m.floor_level,
                    players,
                });
            }
            (active, standing)
        };
        if active.is_empty() {
            return;
        }

        let mut commands: Vec<(String, i8, AiCommand)> = Vec::new();
        let pathfinds = {
            let cache = self.passability_read();
            let path = CountingPath {
                inner: CachePathProvider { cache: &cache },
                count: Cell::new(0),
            };
            let mut rng = rand::thread_rng();
            let mut ticked = 0usize;
            let n = active.len();
            let start = brains.cursor % n;
            let mut over_budget = false;
            for i in 0..n {
                if started.elapsed() > TICK_BUDGET {
                    over_budget = true;
                    brains.cursor = (start + i) % n;
                    break;
                }
                let a = &active[(start + i) % n];
                let ServerBrains { entries, trees, .. } = &mut *brains;
                let Some(entry) = entries.get_mut(&a.id) else {
                    continue;
                };
                let Some(tree) = monster_ai::behavior_tree_for(trees, &entry.brain.behavior) else {
                    continue;
                };
                let delta_ms = entry.owed_ms(Instant::now(), forced_delta_ms);
                let monsters: Vec<NearbyMonster> =
                    super::SpatialCell::within_radius(&entry.brain.position, radius)
                        .filter_map(|cell| standing.get(&cell))
                        .flatten()
                        .filter(|m| m.id != a.id)
                        .cloned()
                        .collect();
                let result = entry.brain.tick_with_behavior_tree(
                    delta_ms, &a.players, &monsters, tree, &path, &mut rng,
                );
                ticked += 1;
                commands.extend(
                    result
                        .commands
                        .into_iter()
                        .map(|c| (a.id.clone(), a.floor_level, c)),
                );
            }
            if !over_budget {
                brains.cursor = 0;
            }
            let s = &mut brains.stats;
            s.ticks += 1;
            s.ticked += ticked as u64;
            s.commands += commands.len() as u64;
            s.over_budget += over_budget as u32;
            path.count.get()
        };
        brains.stats.pathfinds += pathfinds;
        let elapsed_ms = started.elapsed().as_secs_f32() * 1000.0;
        brains.stats.worst_ms = brains.stats.worst_ms.max(elapsed_ms);
        if brains.stats_since.elapsed() >= STATS_LOG_PERIOD {
            let s = std::mem::take(&mut brains.stats);
            info!(
                "monster ai: brains {} active {} ticks {} ticked/tick {:.0} pathfinds/s {:.1} commands/s {:.1} over_budget {} worst {:.1}ms",
                brains.entries.len(),
                active.len(),
                s.ticks,
                s.ticked as f32 / s.ticks.max(1) as f32,
                s.pathfinds as f32 / STATS_LOG_PERIOD.as_secs_f32(),
                s.commands as f32 / STATS_LOG_PERIOD.as_secs_f32(),
                s.over_budget,
                s.worst_ms
            );
            brains.stats_since = Instant::now();
        }
        drop(brains);

        for (monster_id, floor_level, command) in commands {
            self.apply_ai_command(&monster_id, floor_level, command)
                .await;
        }
    }

    async fn apply_ai_command(&self, monster_id: &str, floor_level: i8, command: AiCommand) {
        match command {
            AiCommand::Move {
                position,
                rotation,
                state,
                target_position,
                ..
            } => {
                self.apply_ai_move(
                    monster_id,
                    floor_level,
                    position,
                    rotation,
                    state,
                    target_position,
                )
                .await
            }
            AiCommand::Attack {
                target_player_id, ..
            } => {
                self.monster_attack(None, monster_id, &target_player_id)
                    .await
            }
        }
    }

    async fn apply_ai_move(
        &self,
        monster_id: &str,
        floor_level: i8,
        position: Position,
        rotation: f32,
        state: MonsterState,
        target_position: Position,
    ) {
        if !position.is_finite() || !rotation.is_finite() || state == MonsterState::Dead {
            warn!("Brain emitted a bad move for {monster_id}: {position:?} {state:?}");
            return;
        }
        let mut position = position.wrapped_x();
        let target_position = target_position.wrapped_x();
        let from = {
            let monsters = self.monsters.read().await;
            match monsters.get(monster_id) {
                Some(m) if m.state != MonsterState::Dead => m.position,
                _ => return,
            }
        };
        // A brain step through a wall means its path and the cache disagree;
        // refuse it and pull the brain back so it re-plans from the truth.
        let blocked = (from.x != position.x || from.z != position.z) && {
            let cache = self.passability_read();
            let to_x = from.x + shortest_world_delta_x(from.x, position.x);
            let floor = passability_floor_for_level(floor_level);
            is_movement_blocked(&cache, from.x, from.z, to_x, position.z, floor, None)
        };
        if blocked {
            warn!(
                "Refused wall-crossing brain move for {monster_id} on floor {floor_level}: {from:?} -> {position:?}"
            );
            let mut brains = self.monster_brains.lock().await;
            if let Some(entry) = brains.entries.get_mut(monster_id) {
                entry.brain.apply_authoritative_position(from);
            }
            return;
        }
        // Brains keep their spawn Y; the ground is the server's to settle.
        let y = self
            .expected_monster_move_y(floor_level, from, position)
            .await;
        position.y = y.unwrap_or(from.y);
        let (old_position, owner_id, monster) = {
            let mut monsters = self.monsters.write().await;
            let Some(m) = monsters.get_mut(monster_id) else {
                return;
            };
            if m.state == MonsterState::Dead {
                return;
            }
            m.rotation = rotation;
            m.state = state;
            let old = m.position;
            let Some(m) = monsters.set_position(monster_id, position) else {
                return;
            };
            (old, m.owner_id, m.clone())
        };
        {
            let mut brains = self.monster_brains.lock().await;
            if let Some(entry) = brains.entries.get_mut(monster_id) {
                entry.brain.position.y = position.y;
            }
        }
        self.fanout_monster_position_update(
            &monster,
            old_position,
            ServerMessage::MonsterMoved {
                monster_id: monster_id.to_string(),
                position,
                rotation,
                state,
                target_position: Position {
                    y: position.y,
                    ..target_position
                },
                owner_id: self.wire_owner(owner_id),
            },
            None,
        )
        .await;
    }

    /// Feed a player's swing (hit or miss — a miss still aggros) to the brain
    /// and act on what it decides.
    pub(super) async fn brain_hit(
        &self,
        monster_id: &str,
        attacker_id: &PlayerId,
        hit: bool,
        damage: u32,
    ) {
        if !self.server_monster_ai() {
            return;
        }
        let commands = {
            let mut brains = self.monster_brains.lock().await;
            let Some(entry) = brains.entries.get_mut(monster_id) else {
                return;
            };
            // A swing lands between ticks; the hit pose must not rewind the
            // monster to the last tick's step.
            let owed = entry.owed_ms(Instant::now(), None);
            entry.brain.catch_up(owed);
            entry
                .brain
                .handle_hit_with_behavior_tree(attacker_id, hit, damage)
        };
        if commands.is_empty() {
            return;
        }
        let floor_level = {
            let monsters = self.monsters.read().await;
            match monsters.get(monster_id) {
                Some(m) => m.floor_level,
                None => return,
            }
        };
        for command in commands {
            self.apply_ai_command(monster_id, floor_level, command)
                .await;
        }
    }

    /// Where the brain has the monster right now; the registry trails it by
    /// up to a sync interval while it runs.
    pub(super) async fn brain_position_now(&self, monster_id: &str) -> Option<Position> {
        if !self.server_monster_ai() {
            return None;
        }
        let mut brains = self.monster_brains.lock().await;
        let entry = brains.entries.get_mut(monster_id)?;
        let owed = entry.owed_ms(Instant::now(), None);
        entry.brain.catch_up(owed);
        Some(entry.brain.position)
    }

    pub(super) async fn brain_death(&self, monster_id: &str) {
        if !self.server_monster_ai() {
            return;
        }
        let mut brains = self.monster_brains.lock().await;
        if brains.entries.remove(monster_id).is_some() {
            debug!("Brain dropped for dead monster {monster_id}");
        }
    }

    #[cfg(test)]
    pub(crate) async fn brain_count(&self) -> usize {
        self.monster_brains.lock().await.len()
    }
}
