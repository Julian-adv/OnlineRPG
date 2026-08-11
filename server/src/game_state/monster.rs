use crate::types::{MonsterState, PlayerId, Position, ServerMessage};
use std::collections::hash_map::Entry;
use std::collections::HashSet;
use tracing::{debug, warn};

/// Keep spawns this many meters clear of every no-spawn zone (towns), so the
/// area *around* a town stays empty too. Mirrors the client's TOWN_MARGIN.
const NO_SPAWN_MARGIN: f32 = 30.0;

/// Headroom over a monster's run speed at which its move token bucket refills,
/// absorbing jitter between the owner's simulation clock and packet arrival.
const MONSTER_MOVE_SPEED_SLACK: f32 = 1.2;
/// Capacity of a monster's move token bucket (meters). Bounds the jump an idle
/// monster could bank up — set just above the ~10m longest legitimate wander
/// leg (`DEFAULT_MAX_MOVE_DIST`) — while still absorbing a burst of frames that
/// the network delivered bunched together.
const MONSTER_MOVE_BUDGET_CAP_METERS: f32 = 12.0;
/// How far a reported monster Y may sit from the server's own ground sample
/// before the move is refused. Absorbs the mismatch between the client's
/// terrain snap and the server's sample without leaving enough room to clear
/// the furniture the passability sweep guards.
const MONSTER_GROUND_Y_TOLERANCE_METERS: f32 = 0.25;
/// Run speed assumed for a monster whose type has no definition (only test /
/// misconfigured types). Kept just above the player's own speed so an unknown
/// type stays tightly bounded rather than inheriting a fast monster's leeway.
const DEFAULT_MONSTER_RUN_SPEED: f32 = 3.5;
const AMBIENT_SPAWN_ALLOWANCE_TTL_MS: u64 = 30_000;

/// One tick's view of the roster, bucketed by cell, so the ownership sweep
/// locks it once rather than once per monster.
type PlayerSnapshot = std::collections::HashMap<super::SpatialCell, Vec<(PlayerId, Position, i8)>>;

/// Who, if anyone, is inside a monster's AOI.
enum Attendance {
    /// Nobody — invisible to every client.
    Nobody,
    /// The owner is there, so its client is simulating the monster.
    Owner,
    /// Someone else is, but the owner is not: the nearest candidate to adopt it.
    Bystander(PlayerId),
}

/// Alive-monster tallies, kept current by spawn/kill rather than recounted.
/// Its own struct so `MonsterRegistry` can update it while holding a
/// `&mut Monster` — separate fields, so the borrows stay disjoint.
#[derive(Default)]
struct AliveCounts {
    total: usize,
    /// owner → type → count. Nested so the per-player cap looks up by `&str`
    /// rather than allocating an owned key on each of its ~25k calls a tick.
    by_owner: std::collections::HashMap<PlayerId, std::collections::HashMap<String, u32>>,
}

impl AliveCounts {
    fn credit(&mut self, owner: Option<PlayerId>, monster_type: &str) {
        self.total += 1;
        let Some(owner) = owner else {
            return;
        };
        let by_type = self.by_owner.entry(owner).or_default();
        match by_type.get_mut(monster_type) {
            Some(count) => *count += 1,
            None => {
                by_type.insert(monster_type.to_string(), 1);
            }
        }
    }

    fn debit(&mut self, owner: Option<PlayerId>, monster_type: &str) {
        self.total = self.total.saturating_sub(1);
        let Some(owner) = owner else {
            return;
        };
        let Some(by_type) = self.by_owner.get_mut(&owner) else {
            return;
        };
        // Zero counts are dropped: a stale entry would pin a disconnected
        // player's key forever.
        if let Some(count) = by_type.get_mut(monster_type) {
            *count -= 1;
            if *count == 0 {
                by_type.remove(monster_type);
            }
        }
        if by_type.is_empty() {
            self.by_owner.remove(&owner);
        }
    }

    fn for_owner(&self, owner: &PlayerId, monster_type: &str) -> u32 {
        self.by_owner
            .get(owner)
            .and_then(|by_type| by_type.get(monster_type))
            .copied()
            .unwrap_or(0)
    }
}

/// The monster map plus alive counts, so the spawn caps are O(1) instead of a
/// scan. Corpses linger 30s (combat.rs) and must not hold a spawn slot, so the
/// counts track alive monsters only.
///
/// `get_mut` hands out a plain `&mut Monster` for position and health edits.
/// Changing `state` to Dead or `owner_id` through it would desync the counts —
/// route those through `mark_dead` / `reassign_owner`.
#[derive(Default)]
pub(crate) struct MonsterRegistry {
    monsters: std::collections::HashMap<String, crate::types::Monster>,
    alive: AliveCounts,
}

impl MonsterRegistry {
    fn credit(&mut self, monster: &crate::types::Monster) {
        if monster.state != MonsterState::Dead {
            self.alive.credit(monster.owner_id, &monster.monster_type);
        }
    }

    fn debit(&mut self, monster: &crate::types::Monster) {
        if monster.state != MonsterState::Dead {
            self.alive.debit(monster.owner_id, &monster.monster_type);
        }
    }

    pub(crate) fn insert(
        &mut self,
        id: String,
        monster: crate::types::Monster,
    ) -> Option<crate::types::Monster> {
        self.credit(&monster);
        let replaced = self.monsters.insert(id, monster);
        if let Some(old) = &replaced {
            self.debit(old);
        }
        replaced
    }

    pub(crate) fn remove(&mut self, id: &str) -> Option<crate::types::Monster> {
        let removed = self.monsters.remove(id);
        if let Some(monster) = &removed {
            self.debit(monster);
        }
        removed
    }

    pub(crate) fn get(&self, id: &str) -> Option<&crate::types::Monster> {
        self.monsters.get(id)
    }

    pub(crate) fn get_mut(&mut self, id: &str) -> Option<&mut crate::types::Monster> {
        self.monsters.get_mut(id)
    }

    pub(crate) fn values(
        &self,
    ) -> std::collections::hash_map::Values<'_, String, crate::types::Monster> {
        self.monsters.values()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.monsters.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.monsters.is_empty()
    }

    /// Alive monsters server-wide, for the global cap.
    pub(crate) fn alive_total(&self) -> usize {
        self.alive.total
    }

    /// Alive monsters of one type owned by one player, for the per-player cap.
    pub(crate) fn alive_for(&self, owner: &PlayerId, monster_type: &str) -> u32 {
        self.alive.for_owner(owner, monster_type)
    }

    /// Kill a monster, freeing its spawn slot while the corpse lingers.
    pub(crate) fn mark_dead(&mut self, id: &str) {
        let Some(monster) = self.monsters.get_mut(id) else {
            return;
        };
        if monster.state == MonsterState::Dead {
            return;
        }
        monster.state = MonsterState::Dead;
        self.alive.debit(monster.owner_id, &monster.monster_type);
    }

    #[cfg(test)]
    pub(crate) fn alive_by_owner_type_len(&self) -> usize {
        self.alive.by_owner.values().map(|by| by.len()).sum()
    }

    /// Owners with at least one alive monster. Pins that an owner whose last
    /// monster went away leaves no empty map behind.
    #[cfg(test)]
    pub(crate) fn alive_owner_count(&self) -> usize {
        self.alive.by_owner.len()
    }

    /// Returns the updated monster, so callers can announce the handoff
    /// without looking it up again.
    pub(crate) fn reassign_owner(
        &mut self,
        id: &str,
        new_owner: PlayerId,
    ) -> Option<&crate::types::Monster> {
        let monster = self.monsters.get_mut(id)?;
        if monster.state != MonsterState::Dead {
            self.alive.debit(monster.owner_id, &monster.monster_type);
            self.alive.credit(Some(new_owner), &monster.monster_type);
        }
        monster.owner_id = Some(new_owner);
        Some(monster)
    }
}

impl std::ops::Index<&str> for MonsterRegistry {
    type Output = crate::types::Monster;

    fn index(&self, id: &str) -> &Self::Output {
        &self.monsters[id]
    }
}

impl super::GameState {
    fn find_ambient_rule(
        monster_type: &str,
    ) -> Option<&'static crate::world_config::AmbientSpawnRule> {
        crate::world_config::world_config()
            .ambient_spawns
            .iter()
            .find(|r| r.monster_type == monster_type)
    }

    /// Create a monster, notify nearby players, and return it (or None if limit reached).
    /// `floor_level` < 0 marks dungeon monsters; `level_override` applies
    /// depth scaling (health here, combat stats in combat.rs). Dungeon
    /// spawns skip the ambient per-player cap — their spawn slots are the cap.
    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_monster(
        &self,
        monster_type: String,
        position: Position,
        rotation: f32,
        owner_id: Option<PlayerId>,
        floor_level: i8,
        level_override: Option<u8>,
        aggressive: bool,
    ) -> Option<crate::types::Monster> {
        let max_total = crate::world_config::world_config().max_monsters_total as usize;
        let max_per_player = if floor_level < 0 {
            None
        } else {
            Self::find_ambient_rule(&monster_type).map(|r| r.max_per_player as usize)
        };

        // O(1) against the registry's maintained counts: this runs on every
        // spawn, tens of thousands of times per tick at target population.
        {
            let monsters = self.monsters.read().await;
            let alive_count = monsters.alive_total();
            let owned_alive = owner_id
                .as_ref()
                .map(|owner| monsters.alive_for(owner, &monster_type) as usize)
                .unwrap_or(0);
            if alive_count >= max_total {
                warn!("Monster spawn rejected: limit reached ({})", alive_count);
                return None;
            }
            if let Some(max) = max_per_player {
                if owned_alive >= max {
                    warn!(
                        "Monster spawn rejected: player {:?} already owns {} alive {}",
                        owner_id, owned_alive, monster_type
                    );
                    return None;
                }
            }
        }

        let owner_number = match owner_id.as_ref() {
            Some(owner_id) => self.get_or_assign_player_number(owner_id).await,
            None => 0,
        };
        let spawn_count = {
            let mut id_state = self.id_state.write().await;
            let counter = id_state.owner_spawn_counts.entry(owner_number).or_insert(0);
            *counter = counter.saturating_add(1);
            *counter
        };
        let id = format!("m{}_{}", owner_number, spawn_count);

        let def = self.monster_defs.get(&monster_type);
        let base_health = def.map(|d| d.max_health()).unwrap_or(10);
        // Depth scaling never weakens a monster below its definition
        // health (bosses have a hand-tuned health larger than their
        // level's formula value).
        let health = match level_override {
            Some(level) => {
                base_health.max(crate::game::combat::monster_max_health_for_level(level))
            }
            None => base_health,
        };
        let monster = crate::types::Monster {
            id: id.clone(),
            monster_type: monster_type.clone(),
            position,
            rotation,
            state: MonsterState::Idle,
            owner_id,
            health,
            max_health: health,
            floor_level,
            level_override,
            aggressive,
            last_attack_at: 0,
            last_move_at: Self::now_ms(),
            // Starts empty: the monster spawns beside its owner and its first
            // reported position is the spawn point, so nothing legitimate needs
            // budget yet. The bucket then fills as real time passes.
            move_budget: 0.0,
        };

        let mut monsters = self.monsters.write().await;
        monsters.insert(id.clone(), monster.clone());
        let alive = monsters.alive_total();
        debug!(
            "Spawned monster {} [owner #{}, spawn #{}] (Alive: {})",
            id, owner_number, spawn_count, alive
        );

        self.send_direct_message_to_players_within_position(
            &monster.position,
            monster.floor_level,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::MonsterSpawned {
                monster: monster.clone(),
            },
            None,
        )
        .await;
        Some(monster)
    }

    /// The owner applies its moves optimistically and the normal fanout skips
    /// it, so a silent reject would desync it until reconnect. Echoes the
    /// authoritative state back to the mover instead.
    fn move_correction(monster_id: String, monster: &crate::types::Monster) -> ServerMessage {
        ServerMessage::MonsterMoved {
            monster_id,
            position: monster.position,
            rotation: monster.rotation,
            state: monster.state,
            target_position: monster.position,
            owner_id: monster.owner_id,
        }
    }

    /// The Y a move to `to` should land on: the ground delta applied to the
    /// stored Y, so an off-ground offset is carried rather than snapped away.
    /// Ambient spawns are grounded by `validate_spawn_request`, but `/spawnmob`
    /// seeds Y from the admin's pose, and such a monster stays off-ground for
    /// life. `None` means refuse the move, not "no opinion".
    async fn expected_monster_move_y(
        &self,
        floor_level: i8,
        from: Position,
        to: Position,
    ) -> Option<f32> {
        // Attack cadence reports plenty of unchanged positions.
        if from.x == to.x && from.z == to.z {
            return Some(from.y);
        }
        let (from_ground, to_ground) = if floor_level < 0 {
            let from_entrance = self.dungeon_defs.entrance_at(from.x, from.z)?;
            let to_entrance = self.dungeon_defs.entrance_at(to.x, to.z)?;
            if from_entrance.id != to_entrance.id {
                return None;
            }
            self.ensure_dungeon_runtime(&from_entrance.id).await;
            let dungeons = self.dungeons.read().await;
            let layouts = &dungeons.get(&from_entrance.id)?.layouts;
            let origin = from_entrance.position();
            let depth = floor_level.unsigned_abs();
            let height_at =
                |x, z| onlinerpg_shared::dungeon::floor_height_at(&origin, layouts, depth, x, z);
            (height_at(from.x, from.z)?, height_at(to.x, to.z)?)
        } else {
            (
                self.height_sampler
                    .sample_height(from.x, from.z)
                    .await
                    .ok()?,
                self.height_sampler.sample_height(to.x, to.z).await.ok()?,
            )
        };
        Some(from.y + to_ground - from_ground)
    }

    pub async fn update_monster_position(
        &self,
        mover_id: &PlayerId,
        monster_id: String,
        mut new_position: Position,
        rotation: f32,
        state: MonsterState,
        mut target_position: Position,
    ) {
        // Dead is rejected like malformed input: only server combat may kill a
        // monster.
        let input_valid = new_position.is_finite()
            && rotation.is_finite()
            && target_position.is_finite()
            && state != MonsterState::Dead;
        let Some((sample_from, floor_level)) = ({
            let monsters = self.monsters.read().await;
            monsters.get(&monster_id).and_then(|monster| {
                monster
                    .is_controllable_by(mover_id)
                    .then_some((monster.position, monster.floor_level))
            })
        }) else {
            return;
        };
        if input_valid {
            // Store canonical X like player moves do; see
            // client_monster_move_stores_canonical_world_x.
            new_position = new_position.wrapped_x();
            target_position = target_position.wrapped_x();
        }
        // Run speed is ground-projected, so charge horizontal travel once and
        // cap height changes independently. Euclidean distance would
        // double-charge ordinary slopes that the terrain snap adds.
        let dx = onlinerpg_shared::shortest_world_delta_x(sample_from.x, new_position.x);
        let dz = new_position.z - sample_from.z;
        let horizontal = (dx * dx + dz * dz).sqrt();
        let raw_dist = horizontal.max((new_position.y - sample_from.y).abs());
        // `f32::max` swallows NaN, so `input_valid` — not `raw_dist` — is what
        // keeps malformed input out of the height sample.
        let expected_y = if input_valid && raw_dist <= MONSTER_MOVE_BUDGET_CAP_METERS {
            self.expected_monster_move_y(floor_level, sample_from, new_position)
                .await
        } else {
            None
        };
        let now = Self::now_ms();
        let (old_position, owner_id, monster) = {
            let mut monsters = self.monsters.write().await;

            let Some(monster) = monsters.get_mut(&monster_id) else {
                return;
            };
            if !monster.is_controllable_by(mover_id) {
                return;
            }
            let accepted = 'check: {
                if !input_valid {
                    break 'check false;
                }
                // The height sample released the lock, so a concurrent move
                // would have left `expected_y` stale.
                if monster.position != sample_from {
                    break 'check false;
                }
                // Rate-limit client-reported movement with a token bucket that
                // refills at the monster's run speed. Movement is simulated by the
                // owning client, so without this an owner could teleport the monster
                // onto any player and use it as an unlimited-range weapon
                // (broadcast_monster_attack's reach check only sees the post-move
                // position). The bucket lets a legit burst of frames the network
                // delivered bunched together spend banked allowance, while its cap
                // bounds the jump an idle monster can bank, and its refill rate the
                // sustained speed.
                let run_speed = self
                    .monster_defs
                    .get(&monster.monster_type)
                    .map(|d| d.run_speed)
                    .unwrap_or(DEFAULT_MONSTER_RUN_SPEED);
                let elapsed_s = now.saturating_sub(monster.last_move_at) as f32 / 1000.0;
                let budget = (monster.move_budget
                    + run_speed * MONSTER_MOVE_SPEED_SLACK * elapsed_s)
                    .min(MONSTER_MOVE_BUDGET_CAP_METERS);
                monster.last_move_at = now;
                // Bank the refill up front so a refused move keeps recovering;
                // only an accepted one spends it.
                monster.move_budget = budget;
                if raw_dist > budget {
                    debug!(
                        "Rejected monster move {:.0}m (budget {:.1}m): monster {} by {}",
                        raw_dist, budget, monster_id, mover_id
                    );
                    break 'check false;
                }
                let Some(expected_y) = expected_y else {
                    debug!("No ground height for monster {monster_id} move by {mover_id}");
                    break 'check false;
                };
                if (new_position.y - expected_y).abs() > MONSTER_GROUND_Y_TOLERANCE_METERS {
                    debug!(
                        "Rejected monster Y {:.1} (expected {:.1}): monster {} by {}",
                        new_position.y, expected_y, monster_id, mover_id
                    );
                    break 'check false;
                }
                new_position.y = expected_y;
                let dist = horizontal.max((expected_y - monster.position.y).abs());
                if dist > budget {
                    break 'check false;
                }
                // A move that reports an unchanged position can't cross anything,
                // and attack cadence reports plenty of them.
                let blocked = dist > 0.0 && {
                    let cache = self.passability_read();
                    let floor = super::passability::authoritative_floor(&cache, &monster.position);
                    // Sweep in unwrapped X so a seam-crossing move stays the short
                    // local segment `dist` measured.
                    let to_x = monster.position.x + dx;
                    super::passability::wrapped_block_info(
                        &cache,
                        monster.position.x,
                        monster.position.z,
                        to_x,
                        new_position.z,
                        floor,
                        monster.position.y,
                    )
                    .is_some()
                };
                if blocked {
                    debug!(
                        "Rejected monster move through blocked terrain: {monster_id} by {mover_id}"
                    );
                    break 'check false;
                }
                monster.move_budget = budget - dist;
                true
            };
            if !accepted {
                let correction = Self::move_correction(monster_id, monster);
                drop(monsters);
                self.send_direct_message(mover_id, correction).await;
                return;
            }
            let old_position = monster.position;
            monster.position = new_position;
            monster.rotation = rotation;
            monster.state = state;
            (old_position, monster.owner_id, monster.clone())
        };

        self.fanout_monster_position_update(
            &monster,
            old_position,
            ServerMessage::MonsterMoved {
                monster_id,
                position: new_position,
                rotation,
                state,
                target_position,
                owner_id,
            },
            owner_id.as_ref(),
        )
        .await;
    }

    async fn fanout_monster_position_update(
        &self,
        monster: &crate::types::Monster,
        old_position: Position,
        update_msg: ServerMessage,
        skip_player_id: Option<&PlayerId>,
    ) {
        // Monsters never change floor mid-life (dungeon monsters are confined
        // to their floor), so both the old and new visibility sets gate on the
        // monster's own floor.
        let old_visible: HashSet<_> = self
            .player_ids_within_position(
                &old_position,
                monster.floor_level,
                super::EVENT_DELIVERY_RADIUS,
            )
            .await
            .into_iter()
            .filter(|id| skip_player_id != Some(id))
            .collect();
        let new_visible: HashSet<_> = self
            .player_ids_within_position(
                &monster.position,
                monster.floor_level,
                super::EVENT_DELIVERY_RADIUS,
            )
            .await
            .into_iter()
            .filter(|id| skip_player_id != Some(id))
            .collect();

        let left: Vec<_> = old_visible.difference(&new_visible).cloned().collect();
        let entered: Vec<_> = new_visible.difference(&old_visible).cloned().collect();
        let stayed: Vec<_> = new_visible.intersection(&old_visible).cloned().collect();

        self.send_direct_message_to_players(
            &left,
            ServerMessage::MonsterRemoved {
                monster_id: monster.id.clone(),
            },
        )
        .await;
        self.send_direct_message_to_players(
            &entered,
            ServerMessage::MonsterSpawned {
                monster: monster.clone(),
            },
        )
        .await;
        self.send_direct_message_to_players(&stayed, update_msg)
            .await;
    }

    pub async fn remove_monsters_by_owner(&self, owner_id: &PlayerId) {
        let removed_monsters = {
            let mut monsters = self.monsters.write().await;
            let owned_ids: Vec<String> = monsters
                .values()
                .filter(|m| m.owner_id.as_ref() == Some(owner_id))
                .map(|m| m.id.clone())
                .collect();

            owned_ids
                .into_iter()
                .filter_map(|monster_id| monsters.remove(&monster_id))
                .collect::<Vec<_>>()
        };

        for monster in removed_monsters {
            debug!(
                "Removed monster {} (owner {} disconnected)",
                monster.id, owner_id
            );
            self.send_direct_message_to_players_within_position(
                &monster.position,
                monster.floor_level,
                super::EVENT_DELIVERY_RADIUS,
                ServerMessage::MonsterRemoved {
                    monster_id: monster.id,
                },
                None,
            )
            .await;
        }
    }

    /// Server-driven monster spawn tick. For each ambient spawn type and each
    /// player below their cap, sends a SpawnMonsterRequest so the client can
    /// pick a valid position near itself (grassland, not water, away from towns).
    /// Each request records an expiring allowance the client's response must
    /// consume via take_spawn_allowance.
    pub async fn tick_monster_spawns(&self) {
        let ambient_spawns = &crate::world_config::world_config().ambient_spawns;
        if ambient_spawns.is_empty() {
            return;
        }

        let max_total = crate::world_config::world_config().max_monsters_total as usize;

        // Players eligible for ambient spawns this tick. NPC players only
        // qualify when a human is within sight range (no point spawning monsters
        // around an agent nobody is watching); humans always qualify. Computed
        // once under a single read lock so the per-rule loop below needs none.
        let player_ids: Vec<PlayerId> = {
            let players = self.players.read().await;
            let radius_sq = super::EVENT_DELIVERY_RADIUS * super::EVENT_DELIVERY_RADIUS;
            let human_positions: Vec<_> = players
                .values()
                .filter(|p| !p.is_official_npc)
                .map(|p| p.position)
                .collect();
            players
                .iter()
                .filter(|(_, player)| {
                    // Dungeon players get slot-based spawns, not ambient
                    // ones (spawn validation is XZ-only and would place
                    // surface monsters right above the dungeon).
                    player.floor_level >= 0
                        && (!player.is_official_npc
                            || human_positions
                                .iter()
                                .any(|hp| player.position.dist_xz_sq(hp) <= radius_sq))
                })
                .map(|(id, _)| *id)
                .collect()
        };
        if player_ids.is_empty() {
            return;
        }

        // One row per rule, indexed like `player_ids`: the registry answers each
        // cap in O(1), so this reads only the pairs the tick asks about instead
        // of walking every monster on the server.
        let (owned_per_rule, total_alive) = {
            let monsters = self.monsters.read().await;
            let counts: Vec<Vec<u32>> = ambient_spawns
                .iter()
                .map(|rule| {
                    player_ids
                        .iter()
                        .map(|player_id| monsters.alive_for(player_id, &rule.monster_type))
                        .collect()
                })
                .collect();
            (counts, monsters.alive_total())
        };

        // Unconsumed allowances reserve slots against the global cap.
        let now = Self::now_ms();
        let requests = {
            let mut allowances = self.ambient_spawn_allowances.write().await;
            allowances.retain(|_, expires_at| *expires_at > now);
            let mut requests: Vec<(String, Vec<PlayerId>)> = Vec::new();

            for (rule, owned) in ambient_spawns.iter().zip(&owned_per_rule) {
                if total_alive + allowances.len() >= max_total {
                    break;
                }

                let mut recipients = Vec::new();
                for (player_id, owned) in player_ids.iter().zip(owned) {
                    if total_alive + allowances.len() >= max_total {
                        break;
                    }
                    if *owned >= rule.max_per_player {
                        continue;
                    }
                    // The owned key is only built once the cap check passes.
                    let key = (*player_id, rule.monster_type.clone());
                    if let Entry::Vacant(entry) = allowances.entry(key) {
                        entry.insert(now + AMBIENT_SPAWN_ALLOWANCE_TTL_MS);
                        recipients.push(*player_id);
                    }
                }
                if !recipients.is_empty() {
                    requests.push((rule.monster_type.clone(), recipients));
                }
            }
            requests
        };

        for (monster_type, recipients) in requests {
            self.send_direct_message_to_players(
                &recipients,
                ServerMessage::SpawnMonsterRequest { monster_type },
            )
            .await;
        }
    }

    /// Same predicate as `player_ids_within_position(.., EVENT_DELIVERY_RADIUS)`
    /// but against a per-tick snapshot, so a sweep over every monster locks the
    /// roster once instead of twice per monster.
    ///
    /// Runs once per monster per tick, so it allocates nothing and stops early:
    /// finding the owner settles the question, and only the nearest bystander
    /// is worth tracking.
    fn attendance(roster: &PlayerSnapshot, monster: &crate::types::Monster) -> Attendance {
        let radius_sq = super::EVENT_DELIVERY_RADIUS * super::EVENT_DELIVERY_RADIUS;
        let mut nearest: Option<(PlayerId, f32)> = None;
        for cell in
            super::SpatialCell::within_radius(&monster.position, super::EVENT_DELIVERY_RADIUS)
        {
            let Some(nearby) = roster.get(&cell) else {
                continue;
            };
            for (id, position, floor) in nearby {
                if *floor != monster.floor_level {
                    continue;
                }
                let dist_sq = monster.position.dist_xz_sq(position);
                if dist_sq > radius_sq {
                    continue;
                }
                if monster.owner_id == Some(*id) {
                    return Attendance::Owner;
                }
                match nearest {
                    Some((_, best)) if best <= dist_sq => {}
                    _ => nearest = Some((*id, dist_sq)),
                }
            }
        }
        match nearest {
            Some((id, _)) => Attendance::Bystander(id),
            None => Attendance::Nobody,
        }
    }

    /// Reconcile every monster against who is actually near it.
    ///
    /// A monster's owner is the client simulating it, so an owner that walked
    /// out of its AOI leaves the monster frozen: still visible and attackable,
    /// but never chasing or retaliating. Where someone else is standing there,
    /// hand the monster over. Where nobody is, no client holds it and no client
    /// can see it, so it is deleted on the spot — keeping it would only hold a
    /// slot in `max_per_player` and `max_monsters_total` against a monster that
    /// does not exist for any player. Roaming clients abandon monsters faster
    /// than they kill them, and without this the caps fill with unreachable
    /// monsters and ambient spawning stops server-wide.
    ///
    /// Handoff applies to dungeon monsters too; the despawn does not, since a
    /// dungeon floor already despawns its own on exit.
    pub async fn tick_monster_ownership(&self) {
        self.tick_monster_ownership_body().await
    }

    fn player_snapshot(
        players: &std::collections::HashMap<PlayerId, crate::types::Player>,
    ) -> PlayerSnapshot {
        let mut roster = PlayerSnapshot::default();
        for (id, player) in players {
            roster
                .entry(super::SpatialCell::from_position(&player.position))
                .or_default()
                .push((*id, player.position, player.floor_level));
        }
        roster
    }

    pub(crate) async fn tick_monster_ownership_body(&self) {
        let roster = Self::player_snapshot(&*self.players.read().await);

        // Resolve every monster against the snapshot in one pass, then act.
        let mut expired: Vec<String> = Vec::new();
        let mut handoffs: Vec<(String, PlayerId, Option<PlayerId>)> = Vec::new();
        {
            let monsters = self.monsters.read().await;
            for monster in monsters.values() {
                match Self::attendance(&roster, monster) {
                    Attendance::Owner => {}
                    Attendance::Bystander(nearest) => {
                        handoffs.push((monster.id.clone(), nearest, monster.owner_id));
                    }
                    // A dungeon floor despawns its own on exit.
                    Attendance::Nobody if !monster.is_in_dungeon() => {
                        expired.push(monster.id.clone())
                    }
                    Attendance::Nobody => {}
                }
            }
        }

        self.hand_off_monsters(handoffs).await;

        if expired.is_empty() {
            return;
        }

        let removed: Vec<crate::types::Monster> = {
            let mut monsters = self.monsters.write().await;
            expired
                .iter()
                .filter_map(|id| monsters.remove(id))
                .collect()
        };

        for monster in removed {
            debug!("Despawned abandoned monster {}", monster.id);
            self.announce_monster_removed(&monster).await;
        }
    }

    /// Give a monster whose owner left its AOI to a player still inside it.
    ///
    /// The owner's client is what runs the monster's AI, and it tears the brain
    /// down the moment the monster leaves its own AOI. Without this the monster
    /// stays visible and attackable to whoever is standing there while nobody
    /// simulates it — it never chases, never retaliates. Applies in dungeons
    /// too: a floor is 80m across, wider than the 43m AOI, and the existing
    /// handoff only fires when a player leaves the floor entirely.
    ///
    /// Deliberately ignores `max_per_player`: the monster already exists, and
    /// refusing would strand it. The adopter simply gets no new ambient spawns
    /// until back under the cap.
    async fn hand_off_monsters(&self, handoffs: Vec<(String, PlayerId, Option<PlayerId>)>) {
        if handoffs.is_empty() {
            return;
        }
        let reassigned: Vec<(crate::types::Monster, PlayerId, Option<PlayerId>)> = {
            let mut monsters = self.monsters.write().await;
            handoffs
                .into_iter()
                .filter_map(|(id, new_owner, old_owner)| {
                    let monster = monsters.reassign_owner(&id, new_owner)?;
                    Some((monster.clone(), new_owner, old_owner))
                })
                .collect()
        };
        for (monster, new_owner, old_owner) in reassigned {
            debug!(
                "Monster {} handed to {} (owner left its AOI)",
                monster.id, new_owner
            );
            if let Some(old_owner) = old_owner {
                self.send_direct_message(
                    &old_owner,
                    ServerMessage::MonsterRemoved {
                        monster_id: monster.id.clone(),
                    },
                )
                .await;
            }
            self.send_direct_message(&new_owner, ServerMessage::MonsterAssigned { monster })
                .await;
        }
    }

    /// Tell everyone who can see a removed monster, and its owner, that it is
    /// gone. The owner is messaged separately: it may be outside the AOI and
    /// still holding the monster's AI.
    async fn announce_monster_removed(&self, monster: &crate::types::Monster) {
        let removal = ServerMessage::MonsterRemoved {
            monster_id: monster.id.clone(),
        };
        self.send_direct_message_to_players_within_position(
            &monster.position,
            monster.floor_level,
            super::EVENT_DELIVERY_RADIUS,
            removal.clone(),
            monster.owner_id.as_ref(),
        )
        .await;
        if let Some(owner_id) = monster.owner_id {
            self.send_direct_message(&owner_id, removal).await;
        }
    }

    /// Validate a client-requested spawn: it must carry finite values, be a
    /// configured ambient type, sit outside every no-spawn zone, and be within
    /// range of the requesting player. The server supplies the authoritative
    /// terrain Y. Placement stays client-selected: grassland has no
    /// server-side source at all, and water is simply not wired up yet
    /// (`water_depth_at` could check it, as `inventory.rs` already does).
    ///
    /// Returns the position to store with canonical X and server-sampled Y.
    pub async fn validate_spawn_request(
        &self,
        player_id: &PlayerId,
        monster_type: &str,
        position: &Position,
        rotation: f32,
    ) -> Option<Position> {
        // The range check below only reads x/z, so without this a non-finite y
        // or rotation would reach MonsterSpawned.
        if !position.is_finite() || !rotation.is_finite() {
            return None;
        }
        let mut position = *position;
        position.x = onlinerpg_shared::wrap_world_x(position.x);
        let rule = match Self::find_ambient_rule(monster_type) {
            Some(r) => r,
            None => return None,
        };

        // Reject if inside any no-spawn zone (towns, safe areas) + margin
        for zone in &self.no_spawn_zones {
            if zone.contains_with_margin(position.x, position.z, NO_SPAWN_MARGIN) {
                return None;
            }
        }

        // Must be reasonably close to the requesting player (anti-cheat sanity)
        let player_pos = {
            let players = self.players.read().await;
            match players.get(player_id) {
                Some(p) => p.position,
                None => return None,
            }
        };
        let max = rule.max_distance + 10.0; // tolerance
        if player_pos.dist_xz_sq(&position) > max * max {
            return None;
        }
        position.y = self
            .height_sampler
            .sample_height(position.x, position.z)
            .await
            .ok()?;
        Some(position)
    }

    /// Consume the player's unexpired allowance for this type, if any. Each
    /// tick-issued request authorizes exactly one accepted spawn response.
    pub async fn take_spawn_allowance(&self, player_id: &PlayerId, monster_type: &str) -> bool {
        let now = Self::now_ms();
        self.ambient_spawn_allowances
            .write()
            .await
            .remove(&(*player_id, monster_type.to_string()))
            .is_some_and(|expires_at| expires_at > now)
    }
}
