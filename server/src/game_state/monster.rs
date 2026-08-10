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
/// Run speed assumed for a monster whose type has no definition (only test /
/// misconfigured types). Kept just above the player's own speed so an unknown
/// type stays tightly bounded rather than inheriting a fast monster's leeway.
const DEFAULT_MONSTER_RUN_SPEED: f32 = 3.5;
const AMBIENT_SPAWN_ALLOWANCE_TTL_MS: u64 = 30_000;
/// How long a surface monster may sit with no player inside its AOI before it
/// despawns. Long enough that stepping out of range and back doesn't churn.
const ABANDONED_MONSTER_GRACE_MS: u64 = 60_000;

/// Player positions bucketed by spatial cell for one abandonment sweep.
type PlayerCells = std::collections::HashMap<super::SpatialCell, Vec<(Position, i8)>>;

/// The monster map plus incrementally maintained population counts.
///
/// Every spawn checks the global and per-player caps. At 5,000 users that is
/// tens of thousands of checks per 10s tick, so the caps cannot be answered by
/// scanning the map. Counts track *alive* monsters only: a corpse lingers 30s
/// (combat.rs) and must not hold a spawn slot.
///
/// `get_mut` hands out a plain `&mut Monster` for position and health edits.
/// Changing a monster's `state` to Dead or its `owner_id` through it would
/// desync the counts — route those through `mark_dead` / `reassign_owner`.
#[derive(Default)]
pub(crate) struct MonsterRegistry {
    monsters: std::collections::HashMap<String, crate::types::Monster>,
    alive_total: usize,
    alive_by_owner_type: std::collections::HashMap<(PlayerId, String), u32>,
}

impl MonsterRegistry {
    fn credit(&mut self, monster: &crate::types::Monster) {
        if monster.state == MonsterState::Dead {
            return;
        }
        self.alive_total += 1;
        if let Some(owner) = monster.owner_id {
            *self
                .alive_by_owner_type
                .entry((owner, monster.monster_type.clone()))
                .or_insert(0) += 1;
        }
    }

    fn debit(&mut self, monster: &crate::types::Monster) {
        if monster.state == MonsterState::Dead {
            return;
        }
        self.alive_total = self.alive_total.saturating_sub(1);
        if let Some(owner) = monster.owner_id {
            let key = (owner, monster.monster_type.clone());
            if let Entry::Occupied(mut entry) = self.alive_by_owner_type.entry(key) {
                *entry.get_mut() -= 1;
                if *entry.get() == 0 {
                    entry.remove();
                }
            }
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

    pub(crate) fn iter(
        &self,
    ) -> std::collections::hash_map::Iter<'_, String, crate::types::Monster> {
        self.monsters.iter()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.monsters.len()
    }

    /// Alive monsters server-wide, for the global cap.
    pub(crate) fn alive_total(&self) -> usize {
        self.alive_total
    }

    /// Alive monsters of one type owned by one player, for the per-player cap.
    pub(crate) fn alive_for(&self, owner: &PlayerId, monster_type: &str) -> u32 {
        self.alive_by_owner_type
            .get(&(*owner, monster_type.to_string()))
            .copied()
            .unwrap_or(0)
    }

    /// Kill a monster, freeing its spawn slot while the corpse lingers.
    pub(crate) fn mark_dead(&mut self, id: &str) {
        let Some(monster) = self.monsters.get(id) else {
            return;
        };
        if monster.state == MonsterState::Dead {
            return;
        }
        let snapshot = monster.clone();
        self.debit(&snapshot);
        if let Some(monster) = self.monsters.get_mut(id) {
            monster.state = MonsterState::Dead;
        }
    }

    #[cfg(test)]
    pub(crate) fn alive_by_owner_type_len(&self) -> usize {
        self.alive_by_owner_type.len()
    }

    pub(crate) fn reassign_owner(&mut self, id: &str, new_owner: PlayerId) {
        let Some(monster) = self.monsters.get(id) else {
            return;
        };
        let snapshot = monster.clone();
        self.debit(&snapshot);
        if let Some(monster) = self.monsters.get_mut(id) {
            monster.owner_id = Some(new_owner);
        }
        let updated = self.monsters[id].clone();
        self.credit(&updated);
    }
}

impl<Q: AsRef<str>> std::ops::Index<Q> for MonsterRegistry {
    type Output = crate::types::Monster;

    fn index(&self, id: Q) -> &Self::Output {
        &self.monsters[id.as_ref()]
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

    pub async fn update_monster_position(
        &self,
        mover_id: &PlayerId,
        monster_id: String,
        mut new_position: Position,
        rotation: f32,
        state: MonsterState,
        mut target_position: Position,
    ) {
        let now = Self::now_ms();
        let (old_position, owner_id, monster) = {
            let mut monsters = self.monsters.write().await;

            let Some(monster) = monsters.get_mut(&monster_id) else {
                return;
            };
            if !monster.is_controllable_by(mover_id) {
                return;
            }
            // A corpse has already been debited from the population counts;
            // letting a client move it would also resurrect its `state`.
            if monster.state == MonsterState::Dead {
                return;
            }
            if !new_position.is_finite() || !rotation.is_finite() || !target_position.is_finite() {
                let correction = Self::move_correction(monster_id, monster);
                drop(monsters);
                self.send_direct_message(mover_id, correction).await;
                return;
            }
            // Store canonical X like player moves do; the budget and sweep
            // below are already periodic. See the regression test for what a
            // non-canonical stored X costs.
            new_position = new_position.wrapped_x();
            target_position = target_position.wrapped_x();
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
            let budget = (monster.move_budget + run_speed * MONSTER_MOVE_SPEED_SLACK * elapsed_s)
                .min(MONSTER_MOVE_BUDGET_CAP_METERS);
            monster.last_move_at = now;
            let dist = monster.position.dist_xz_sq(&new_position).sqrt();
            if dist > budget {
                // Bank the refill so the budget keeps recovering, but don't
                // spend it: the move stays where it was and isn't fanned out.
                monster.move_budget = budget;
                debug!(
                    "Rejected monster move {:.0}m (budget {:.1}m): monster {} by {}",
                    dist, budget, monster_id, mover_id
                );
                let correction = Self::move_correction(monster_id, monster);
                drop(monsters);
                self.send_direct_message(mover_id, correction).await;
                return;
            }
            // A move that reports an unchanged position can't cross anything,
            // and attack cadence reports plenty of them.
            let blocked = dist > 0.0 && {
                let cache = self.passability_read();
                let floor = super::passability::authoritative_floor(&cache, &monster.position);
                // Sweep in unwrapped X so a seam-crossing move stays the short
                // local segment `dist` measured.
                let to_x = monster.position.x
                    + onlinerpg_shared::shortest_world_delta_x(monster.position.x, new_position.x);
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
                monster.move_budget = budget;
                debug!("Rejected monster move through blocked terrain: {monster_id} by {mover_id}");
                let correction = Self::move_correction(monster_id, monster);
                drop(monsters);
                self.send_direct_message(mover_id, correction).await;
                return;
            }
            monster.move_budget = budget - dist;
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
                .iter()
                .filter(|(_, m)| m.owner_id.as_ref() == Some(owner_id))
                .map(|(id, _)| id.clone())
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

        // Single lock, no scan: the registry keeps both counts current, so
        // this reads only the (player, type) pairs this tick actually asks
        // about instead of walking every monster on the server.
        let (owner_type_counts, total_alive) = {
            let monsters = self.monsters.read().await;
            let mut counts = std::collections::HashMap::new();
            for rule in ambient_spawns {
                for player_id in &player_ids {
                    let owned = monsters.alive_for(player_id, &rule.monster_type);
                    if owned > 0 {
                        counts.insert((*player_id, rule.monster_type.clone()), owned);
                    }
                }
            }
            (counts, monsters.alive_total())
        };

        // Unconsumed allowances reserve slots against the global cap.
        let now = Self::now_ms();
        let requests = {
            let mut allowances = self.ambient_spawn_allowances.write().await;
            allowances.retain(|_, expires_at| *expires_at > now);
            let mut requests: Vec<(String, Vec<PlayerId>)> = Vec::new();

            for rule in ambient_spawns {
                if total_alive + allowances.len() >= max_total {
                    break;
                }

                let mut recipients = Vec::new();
                for player_id in &player_ids {
                    if total_alive + allowances.len() >= max_total {
                        break;
                    }

                    let key = (*player_id, rule.monster_type.clone());
                    if owner_type_counts.get(&key).copied().unwrap_or(0) >= rule.max_per_player {
                        continue;
                    }
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
    fn any_player_in_aoi(cells: &PlayerCells, position: &Position, floor_level: i8) -> bool {
        let radius_sq = super::EVENT_DELIVERY_RADIUS * super::EVENT_DELIVERY_RADIUS;
        // Mirror player_ids_within_position's seam handling: query translated
        // copies so cells from the opposite world edge participate too.
        [
            -onlinerpg_shared::WORLD_WIDTH_X,
            0.0,
            onlinerpg_shared::WORLD_WIDTH_X,
        ]
        .iter()
        .any(|shift_x| {
            let shifted = Position {
                x: position.x + shift_x,
                ..*position
            };
            let center = super::SpatialCell::from_position(&shifted);
            (center.x - 1..=center.x + 1).any(|x| {
                (center.z - 1..=center.z + 1).any(|z| {
                    cells
                        .get(&super::SpatialCell { x, z })
                        .is_some_and(|nearby| {
                            nearby.iter().any(|(pos, floor)| {
                                *floor == floor_level && position.dist_xz_sq(pos) <= radius_sq
                            })
                        })
                })
            })
        })
    }

    /// Despawn surface monsters no player has been near for the grace period.
    ///
    /// An ambient monster the owner walked away from is invisible to everyone
    /// (nobody is inside its AOI) yet still counts against both the owner's
    /// `max_per_player` and the global `max_monsters_total`. Roaming clients —
    /// agent bots especially — abandon monsters faster than they kill them, so
    /// without this the caps fill with unreachable monsters and ambient
    /// spawning stops server-wide. Dungeon monsters are excluded: their floor
    /// lifecycle already despawns them.
    pub async fn tick_abandoned_monsters(&self) {
        self.tick_abandoned_monsters_at(Self::now_ms()).await
    }

    /// Clock-injected body, so soak tests can compress hours into a loop.
    pub(crate) async fn tick_abandoned_monsters_at(&self, now: u64) {
        let player_cells: PlayerCells = {
            let players = self.players.read().await;
            let mut cells = PlayerCells::new();
            for player in players.values() {
                cells
                    .entry(super::SpatialCell::from_position(&player.position))
                    .or_default()
                    .push((player.position, player.floor_level));
            }
            cells
        };

        let unattended: Vec<String> = {
            let monsters = self.monsters.read().await;
            monsters
                .values()
                .filter(|m| {
                    m.floor_level >= 0
                        && !Self::any_player_in_aoi(&player_cells, &m.position, m.floor_level)
                })
                .map(|m| m.id.clone())
                .collect()
        };

        let expired: Vec<String> = {
            let mut tracker = self.abandoned_monsters.write().await;
            let unattended_set: HashSet<&String> = unattended.iter().collect();
            // Anything seen again (or already gone) stops accruing.
            tracker.retain(|id, _| unattended_set.contains(id));
            unattended
                .iter()
                .filter(|id| {
                    now.saturating_sub(*tracker.entry((*id).clone()).or_insert(now))
                        >= ABANDONED_MONSTER_GRACE_MS
                })
                .cloned()
                .collect()
        };
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
        {
            let mut tracker = self.abandoned_monsters.write().await;
            for id in &expired {
                tracker.remove(id);
            }
        }

        // Nobody is inside the AOI by definition, so only the owner needs
        // telling — its client still holds the monster and runs its AI.
        for monster in removed {
            debug!("Despawned abandoned monster {}", monster.id);
            if let Some(owner_id) = monster.owner_id {
                self.send_direct_message(
                    &owner_id,
                    ServerMessage::MonsterRemoved {
                        monster_id: monster.id,
                    },
                )
                .await;
            }
        }
    }

    /// Validate a client-requested spawn: it must carry finite values, be a
    /// configured ambient type, sit outside every no-spawn zone, and be within
    /// range of the requesting player. Terrain checks (grassland, water) are
    /// the client's responsibility — the server has no terrain data.
    ///
    /// Returns the position to store: the request with X wrapped into the
    /// canonical range, so validation and authoritative state share one
    /// representation.
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
        let dx = onlinerpg_shared::shortest_world_delta_x(player_pos.x, position.x);
        let dz = position.z - player_pos.z;
        let max = rule.max_distance + 10.0; // tolerance
        (dx * dx + dz * dz <= max * max).then_some(position)
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
