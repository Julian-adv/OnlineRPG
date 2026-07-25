use crate::types::{MonsterState, PlayerId, Position, ServerMessage};
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

        // Read lock: single-pass check of both global and per-player limits
        {
            let monsters = self.monsters.read().await;
            let mut alive_count = 0usize;
            let mut owned_alive = 0usize;
            for m in monsters.values() {
                if m.state != MonsterState::Dead {
                    alive_count += 1;
                    if let Some(ref owner) = owner_id {
                        if m.owner_id.as_ref() == Some(owner) && m.monster_type == monster_type {
                            owned_alive += 1;
                        }
                    }
                }
            }
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
        let alive = monsters
            .values()
            .filter(|m| m.state != MonsterState::Dead)
            .count();
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
        new_position: Position,
        rotation: f32,
        state: MonsterState,
        target_position: Position,
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
            if !new_position.is_finite() || !rotation.is_finite() || !target_position.is_finite() {
                let correction = Self::move_correction(monster_id, monster);
                drop(monsters);
                self.send_direct_message(mover_id, correction).await;
                return;
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

        // Single lock: count alive monsters per (owner, type) and total
        let (owner_type_counts, total_alive) = {
            let monsters = self.monsters.read().await;
            let mut counts = std::collections::HashMap::new();
            let mut alive = 0usize;
            for m in monsters.values() {
                if m.state != MonsterState::Dead {
                    alive += 1;
                    if let Some(ref owner) = m.owner_id {
                        *counts.entry((*owner, m.monster_type.clone())).or_insert(0) += 1;
                    }
                }
            }
            (counts, alive)
        };

        let mut requested_this_tick = 0usize;

        for rule in ambient_spawns {
            for player_id in &player_ids {
                if total_alive + requested_this_tick >= max_total {
                    return;
                }

                let owned = owner_type_counts
                    .get(&(*player_id, rule.monster_type.clone()))
                    .copied()
                    .unwrap_or(0);

                if owned >= rule.max_per_player {
                    continue;
                }

                // Ask the client to find a valid position near itself and spawn
                self.send_direct_message(
                    player_id,
                    ServerMessage::SpawnMonsterRequest {
                        monster_type: rule.monster_type.clone(),
                    },
                )
                .await;

                requested_this_tick += 1;
            }
        }
    }

    /// Validate a client-requested spawn: it must carry finite values, be a
    /// configured ambient type, sit outside every no-spawn zone, and be within
    /// range of the requesting player. Terrain checks (grassland, water) are
    /// the client's responsibility — the server has no terrain data.
    pub async fn validate_spawn_request(
        &self,
        player_id: &PlayerId,
        monster_type: &str,
        position: &Position,
        rotation: f32,
    ) -> bool {
        // The range check below only reads x/z, so without this a non-finite y
        // or rotation would reach MonsterSpawned.
        if !position.is_finite() || !rotation.is_finite() {
            return false;
        }
        let rule = match Self::find_ambient_rule(monster_type) {
            Some(r) => r,
            None => return false,
        };

        // Reject if inside any no-spawn zone (towns, safe areas) + margin
        for zone in &self.no_spawn_zones {
            if zone.contains_with_margin(position.x, position.z, NO_SPAWN_MARGIN) {
                return false;
            }
        }

        // Must be reasonably close to the requesting player (anti-cheat sanity)
        let player_pos = {
            let players = self.players.read().await;
            match players.get(player_id) {
                Some(p) => p.position,
                None => return false,
            }
        };
        let dx = onlinerpg_shared::shortest_world_delta_x(player_pos.x, position.x);
        let dz = position.z - player_pos.z;
        let max = rule.max_distance + 10.0; // tolerance
        dx * dx + dz * dz <= max * max
    }
}
