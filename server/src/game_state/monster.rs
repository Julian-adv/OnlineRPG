use crate::types::{MonsterLifecycle, MonsterState, PlayerId, Position, ServerMessage};
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
/// Metres from town before an ambient type one level higher starts spawning.
/// What you meet on the surface follows where you stand, not your level.
const AMBIENT_SPAWN_METERS_PER_LEVEL: f32 = 70.0;
/// Monsters the ownership sweep visits per lock acquisition, so it never holds
/// the registry — written by every kill and every move — for all 135k. A write
/// between chunks can reorder the map and hide monsters from that sweep; the
/// next tick restarts from zero over a fresh order, so a reconcile is at worst
/// delayed a few ticks, never skipped.
const OWNERSHIP_SCAN_CHUNK: usize = 4_000;

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

/// One reassignment for `hand_off_monsters` to apply. Picking the candidate is
/// the caller's job; this is just the request.
pub(super) struct Handoff {
    pub monster_id: String,
    pub new_owner: PlayerId,
    pub old_owner: Option<PlayerId>,
}

/// owner → the ids it owns, so the spawn cap is O(1) and a disconnect finds
/// its handful of monsters without walking the whole map. Its own struct so
/// `MonsterRegistry` can update it while holding a `&mut Monster` — separate
/// fields, so the borrows stay disjoint. State plays no part: a corpse stays
/// filed under its owner until it is removed, because a disconnect has to
/// clear corpses too.
#[derive(Default, PartialEq)]
struct OwnedIds {
    by_owner: std::collections::HashMap<PlayerId, HashSet<String>>,
}

impl OwnedIds {
    fn add(&mut self, owner: Option<PlayerId>, id: &str) {
        let Some(owner) = owner else {
            return;
        };
        self.by_owner
            .entry(owner)
            .or_default()
            .insert(id.to_string());
    }

    fn remove(&mut self, owner: Option<PlayerId>, id: &str) {
        let Some(owner) = owner else {
            return;
        };
        // Empty sets are dropped, so a disconnected player's key does not
        // linger.
        let Entry::Occupied(mut entry) = self.by_owner.entry(owner) else {
            return;
        };
        entry.get_mut().remove(id);
        if entry.get().is_empty() {
            entry.remove();
        }
    }

    fn len_for(&self, owner: &PlayerId) -> usize {
        self.by_owner.get(owner).map(|ids| ids.len()).unwrap_or(0)
    }

    fn for_owner(&self, owner: &PlayerId) -> impl Iterator<Item = &str> {
        self.by_owner
            .get(owner)
            .into_iter()
            .flatten()
            .map(String::as_str)
    }
}

/// The monster map plus an owner index and a cell index, so the spawn cap is
/// O(1) instead of a scan and an AOI query visits only nearby monsters instead
/// of the whole map. Both indexes carry corpses, since clients still see them.
///
/// `get_mut` hands out a plain `&mut Monster` for health and timestamp edits.
/// Changing `owner_id` through it would desync the owner index and `position`
/// the cell index — route those through `reassign_owner` / `set_position`.
#[derive(Default)]
pub(crate) struct MonsterRegistry {
    monsters: std::collections::HashMap<String, crate::types::Monster>,
    ids_by_owner: OwnedIds,
    /// Stored positions are canonical in X
    /// (`client_monster_move_stores_canonical_world_x`), which is what lets the
    /// index find them across the world seam.
    cells: super::SpatialIndex<String>,
}

impl MonsterRegistry {
    /// Monsters within AOI range of either position — a superset of the two
    /// AOIs, so the only monsters a move between them can bring into or out of
    /// view. Callers still test the exact distance.
    pub(crate) fn near_either(
        &self,
        a: &Position,
        b: &Position,
    ) -> impl Iterator<Item = &crate::types::Monster> {
        self.cells
            .keys_near_either(a, b, super::EVENT_DELIVERY_RADIUS)
            .filter_map(|id| self.monsters.get(id.as_str()))
    }

    pub(crate) fn insert(
        &mut self,
        id: String,
        monster: crate::types::Monster,
    ) -> Option<crate::types::Monster> {
        // Unfile whatever this key held first, so a replacement cannot leave
        // the old monster's entries behind — or drop its own.
        let replaced = self.remove(&id);
        let position = monster.position;
        let owner = monster.owner_id;
        self.monsters.insert(id.clone(), monster);
        self.ids_by_owner.add(owner, id.as_str());
        self.cells.insert(id, &position);
        replaced
    }

    pub(crate) fn remove(&mut self, id: &str) -> Option<crate::types::Monster> {
        let removed = self.monsters.remove(id);
        if let Some(monster) = &removed {
            self.cells.remove(id, &monster.position);
            self.ids_by_owner.remove(monster.owner_id, id);
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

    /// Monsters on the server, corpses included.
    pub(crate) fn len(&self) -> usize {
        self.monsters.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.monsters.is_empty()
    }

    /// Monsters a player owns — how much its client is simulating, which is
    /// what an ownership handoff balances on. Broader than the ambient spawn
    /// cap it also gates: dungeon and admin-spawned monsters are exempt from
    /// that cap but still counted here.
    pub(crate) fn owned_by(&self, owner: &PlayerId) -> usize {
        self.ids_by_owner.len_for(owner)
    }

    /// Every monster id a player owns — what a disconnect has to clear,
    /// without scanning the map.
    pub(crate) fn ids_owned_by(&self, owner: &PlayerId) -> impl Iterator<Item = &str> {
        self.ids_by_owner.for_owner(owner)
    }

    pub(crate) fn mark_dead(&mut self, id: &str) {
        if let Some(monster) = self.monsters.get_mut(id) {
            monster.state = MonsterState::Dead;
        }
    }

    /// Returns the updated monster, so callers can announce the handoff
    /// without looking it up again.
    pub(crate) fn reassign_owner(
        &mut self,
        id: &str,
        new_owner: PlayerId,
    ) -> Option<&crate::types::Monster> {
        let monster = self.monsters.get_mut(id)?;
        self.ids_by_owner.remove(monster.owner_id, id);
        self.ids_by_owner.add(Some(new_owner), id);
        monster.owner_id = Some(new_owner);
        Some(monster)
    }

    /// Move a monster, keeping its index entry in step, and return it so callers
    /// can fan the move out without a second lookup. A position written through
    /// `get_mut` instead would leave the monster indexed where it used to stand:
    /// invisible to arriving players, and announced as departed to the ones it
    /// walked toward.
    pub(crate) fn set_position(
        &mut self,
        id: &str,
        position: Position,
    ) -> Option<&crate::types::Monster> {
        let monster = self.monsters.get_mut(id)?;
        let old_position = monster.position;
        monster.position = position;
        self.cells.moved(id, &old_position, &position);
        Some(monster)
    }

    /// Whether the owner index files exactly what the map says — the invariant
    /// the spawn cap and the disconnect cleanup depend on.
    #[cfg(test)]
    pub(crate) fn owner_index_matches_map(&self) -> bool {
        let mut expected = OwnedIds::default();
        for (id, monster) in &self.monsters {
            expected.add(monster.owner_id, id);
        }
        self.ids_by_owner == expected
    }

    /// Whether the cell index holds exactly the buckets the map implies — the
    /// invariant every AOI query depends on.
    #[cfg(test)]
    pub(crate) fn cell_index_matches_map(&self) -> bool {
        let mut expected = super::SpatialIndex::default();
        for (id, monster) in &self.monsters {
            expected.insert(id.clone(), &monster.position);
        }
        self.cells.matches(&expected)
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
    /// `lifecycle` names who owns the monster's removal; `level_override`
    /// applies depth scaling (health here, combat stats in combat.rs).
    /// Dungeon-slot spawns skip the ambient per-player cap — their spawn
    /// slots are the cap.
    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_monster(
        &self,
        monster_type: String,
        position: Position,
        rotation: f32,
        owner_id: Option<PlayerId>,
        floor_level: i8,
        lifecycle: MonsterLifecycle,
        level_override: Option<u8>,
        aggressive: bool,
    ) -> Option<crate::types::Monster> {
        // Ambient lifecycle only: a slot monster's cap is its floor's slots,
        // and an admin-spawned type with no ambient rule has none.
        let capped_owner = owner_id.filter(|_| {
            lifecycle == MonsterLifecycle::Ambient
                && Self::find_ambient_rule(&monster_type).is_some()
        });

        // O(1) against the owner index: this runs on every spawn, tens of
        // thousands of times per tick at target population.
        if let Some(owner) = capped_owner {
            let max_per_player =
                crate::world_config::world_config().max_monsters_per_player as usize;
            let owned = self.monsters.read().await.owned_by(&owner);
            if owned >= max_per_player {
                warn!("Monster spawn rejected: player {owner} already owns {owned} monsters");
                return None;
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
            lifecycle,
            last_attack_at: 0,
            last_move_at: Self::now_ms(),
            // Starts empty: the monster spawns beside its owner and its first
            // reported position is the spawn point, so nothing legitimate needs
            // budget yet. The bucket then fills as real time passes.
            move_budget: 0.0,
        };

        let mut monsters = self.monsters.write().await;
        monsters.insert(id.clone(), monster.clone());
        let total = monsters.len();
        debug!(
            "Spawned monster {} [owner #{}, spawn #{}] (Total: {})",
            id, owner_number, spawn_count, total
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
            monster.rotation = rotation;
            monster.state = state;
            // Through the registry so the cell index follows the monster.
            let Some(monster) = monsters.set_position(&monster_id, new_position) else {
                return;
            };
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
            .collect();
        let new_visible: HashSet<_> = self
            .player_ids_within_position(
                &monster.position,
                monster.floor_level,
                super::EVENT_DELIVERY_RADIUS,
            )
            .await
            .into_iter()
            .collect();
        // The owner simulates from wherever it stands and player moves are the
        // other ownership event, so a monster wandering out of a stationary
        // owner's AOI is visible only here. Read off the sets before they are
        // trimmed for messaging.
        let departed_owner = monster
            .owner_id
            .filter(|owner| old_visible.contains(owner) && !new_visible.contains(owner));

        let excluded = |id: &&PlayerId| skip_player_id == Some(*id);
        let left: Vec<_> = old_visible
            .difference(&new_visible)
            .filter(|id| !excluded(id))
            .cloned()
            .collect();
        let entered: Vec<_> = new_visible
            .difference(&old_visible)
            .filter(|id| !excluded(id))
            .cloned()
            .collect();
        let stayed: Vec<_> = new_visible
            .intersection(&old_visible)
            .filter(|id| !excluded(id))
            .cloned()
            .collect();

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

        if let Some(owner) = departed_owner {
            self.release_monsters_left_behind(
                &owner,
                vec![(
                    monster.id.clone(),
                    monster.position,
                    monster.floor_level,
                    monster.lifecycle,
                )],
            )
            .await;
        }
    }

    /// Pick an adopter inside each monster's AOI: least loaded first, nearest
    /// on ties. The load is seeded from what each candidate already simulates
    /// and carries this batch's not-yet-applied picks, so a scattering party's
    /// monsters spread instead of piling on whoever is nearest. Monsters with
    /// nobody in range come back as orphans, with their lifecycle, for the
    /// caller's policy — despawn or park.
    async fn plan_handoffs(
        &self,
        abandoned: Vec<(String, Position, i8, MonsterLifecycle)>,
        old_owner: &PlayerId,
    ) -> (Vec<Handoff>, Vec<(String, MonsterLifecycle)>) {
        let mut load: std::collections::HashMap<PlayerId, usize> = std::collections::HashMap::new();
        let mut handoffs = Vec::new();
        let mut orphans = Vec::new();
        for (monster_id, position, floor_level, lifecycle) in abandoned {
            // The old owner may still be in the roster (a disconnect removes it
            // later; a walk-out has merely moved), so never let it adopt its
            // own monsters.
            let candidates = self
                .players_within_position(
                    &position,
                    floor_level,
                    super::EVENT_DELIVERY_RADIUS,
                    Some(old_owner),
                )
                .await;
            if candidates.is_empty() {
                orphans.push((monster_id, lifecycle));
                continue;
            }
            for (candidate, _) in &candidates {
                if let std::collections::hash_map::Entry::Vacant(entry) = load.entry(*candidate) {
                    entry.insert(self.monsters.read().await.owned_by(candidate));
                }
            }
            let Some((new_owner, _)) =
                candidates
                    .into_iter()
                    .min_by(|(a_id, a_dist), (b_id, b_dist)| {
                        load[a_id]
                            .cmp(&load[b_id])
                            .then_with(|| a_dist.total_cmp(b_dist))
                    })
            else {
                continue;
            };
            *load.entry(new_owner).or_default() += 1;
            handoffs.push(Handoff {
                monster_id,
                new_owner,
                old_owner: Some(*old_owner),
            });
        }
        (handoffs, orphans)
    }

    /// Hand a departing player's monsters to players still inside their AOI,
    /// least loaded first, and despawn the rest. Same rule as the walk-out
    /// release, except the owner is not coming back, so a dungeon monster that
    /// gets this far is despawned rather than left for the floor. (In practice
    /// `remove_player` walks a dungeon player off its floor first, so those are
    /// already reassigned by the time this runs.)
    pub async fn remove_monsters_by_owner(&self, owner_id: &PlayerId) {
        let owned: Vec<(String, Position, i8, MonsterLifecycle)> = {
            let monsters = self.monsters.read().await;
            monsters
                .ids_owned_by(owner_id)
                .filter_map(|id| monsters.get(id))
                .map(|m| (m.id.clone(), m.position, m.floor_level, m.lifecycle))
                .collect()
        };
        if owned.is_empty() {
            return;
        }

        let (handoffs, orphans) = self.plan_handoffs(owned, owner_id).await;
        debug!(
            "Owner {} left: {} monsters handed off, {} despawned",
            owner_id,
            handoffs.len(),
            orphans.len()
        );
        self.hand_off_monsters(handoffs).await;
        self.despawn_monsters(orphans.into_iter().map(|(id, _)| id).collect())
            .await;
    }

    /// The owner can no longer see these monsters and its client is dropping
    /// their brains, so ownership is settled by the same event instead of by a
    /// later sweep. Each monster goes to a player still inside its AOI when
    /// there is one. With nobody in range, an ambient orphan is despawned on
    /// the spot — nobody can see it, and keeping it would only hold a cap
    /// slot — while a dungeon-slot orphan is parked: its floor owns removal,
    /// and `adopt_unattended_monsters` re-arms it when someone walks back in.
    /// Every branch tells the old owner `MonsterRemoved` exactly once, which
    /// is both its release and its AOI removal.
    pub(super) async fn release_monsters_left_behind(
        &self,
        owner_id: &PlayerId,
        abandoned: Vec<(String, Position, i8, MonsterLifecycle)>,
    ) {
        let (handoffs, orphans) = self.plan_handoffs(abandoned, owner_id).await;
        self.hand_off_monsters(handoffs).await;
        let (expired, parked): (Vec<_>, Vec<_>) = orphans
            .into_iter()
            .partition(|(_, lifecycle)| lifecycle.despawns_when_unattended());
        self.despawn_monsters(expired.into_iter().map(|(id, _)| id).collect())
            .await;
        for (monster_id, _) in parked {
            debug!("Monster {} parked: owner {} left", monster_id, owner_id);
            self.send_direct_message(owner_id, ServerMessage::MonsterRemoved { monster_id })
                .await;
        }
    }

    /// Monsters that just entered `player_id`'s view with no owner inside
    /// their own AOI — parked dungeon monsters, or strays a race left behind.
    /// Nobody is simulating them, so the viewer adopts them on sight.
    pub(super) async fn adopt_unattended_monsters(
        &self,
        player_id: &PlayerId,
        entered: &[crate::types::Monster],
    ) {
        if entered.is_empty() {
            return;
        }
        let radius_sq = super::EVENT_DELIVERY_RADIUS * super::EVENT_DELIVERY_RADIUS;
        let adoptions: Vec<Handoff> = {
            let players = self.players.read().await;
            entered
                .iter()
                .filter(|monster| {
                    let attended = monster
                        .owner_id
                        .and_then(|id| players.get(&id))
                        .is_some_and(|owner| {
                            owner.floor_level == monster.floor_level
                                && monster.position.dist_xz_sq(&owner.position) <= radius_sq
                        });
                    !attended
                })
                .map(|monster| Handoff {
                    monster_id: monster.id.clone(),
                    new_owner: *player_id,
                    old_owner: monster.owner_id,
                })
                .collect()
        };
        self.hand_off_monsters(adoptions).await;
    }

    /// How far from town an ambient type starts spawning — one step per level
    /// above the first, so the gate follows `monsters.csv` instead of being
    /// authored per rule. Unknown types gate nothing.
    pub(crate) fn min_ambient_town_distance(&self, monster_type: &str) -> f32 {
        self.monster_defs.get(monster_type).map_or(0.0, |def| {
            f32::from(def.level.saturating_sub(1)) * AMBIENT_SPAWN_METERS_PER_LEVEL
        })
    }

    /// Server-driven monster spawn tick. Each player below their cap gets at
    /// most one SpawnMonsterRequest per tick — a randomly picked eligible type —
    /// so the field fills in gradually instead of a full set of types landing at
    /// once. The client picks a valid position near itself (grassland, not
    /// water, away from towns). Each request records an expiring allowance the
    /// client's response must consume via take_spawn_allowance.
    pub async fn tick_monster_spawns(&self) {
        let ambient_spawns = &crate::world_config::world_config().ambient_spawns;
        if ambient_spawns.is_empty() {
            return;
        }

        let max_per_player = crate::world_config::world_config().max_monsters_per_player as usize;

        // Players eligible for ambient spawns this tick. NPC players only
        // qualify when a human is within sight range (no point spawning monsters
        // around an agent nobody is watching); humans always qualify. Computed
        // once under a single read lock so the loop below needs none.
        let mut candidates: Vec<(PlayerId, f32)> = {
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
                .map(|(id, player)| (*id, self.town_distance(&player.position)))
                .collect()
        };
        if candidates.is_empty() {
            return;
        }

        // The owner index answers each cap in O(1), so this reads only the
        // players the tick asks about instead of walking every monster on the
        // server. The expiry sweep below still runs even if nobody is left.
        {
            let monsters = self.monsters.read().await;
            candidates.retain(|(id, _)| monsters.owned_by(id) < max_per_player);
        }

        // Indexed like `ambient_spawns`, so the distance gate costs one lookup
        // per rule instead of one per rule per player.
        let min_distances: Vec<f32> = ambient_spawns
            .iter()
            .map(|rule| self.min_ambient_town_distance(&rule.monster_type))
            .collect();

        let now = Self::now_ms();
        let recipients_by_rule = {
            use rand::seq::SliceRandom;
            let mut allowances = self.ambient_spawn_allowances.write().await;
            allowances.retain(|_, expires_at| *expires_at > now);
            let mut by_rule: Vec<Vec<PlayerId>> = vec![Vec::new(); ambient_spawns.len()];
            let mut rng = rand::thread_rng();
            let mut order: Vec<usize> = (0..ambient_spawns.len()).collect();

            for (player_id, town_distance) in &candidates {
                // One request per player per tick. The scan runs in shuffled
                // order, so the type on offer is uniform over the ones the
                // player can take rather than following the rule list.
                order.shuffle(&mut rng);
                for &r in &order {
                    if *town_distance < min_distances[r] {
                        continue;
                    }
                    // The owned key is only built once the distance gate passes.
                    let key = (*player_id, ambient_spawns[r].monster_type.clone());
                    if let Entry::Vacant(entry) = allowances.entry(key) {
                        entry.insert(now + AMBIENT_SPAWN_ALLOWANCE_TTL_MS);
                        by_rule[r].push(*player_id);
                        break;
                    }
                }
            }
            by_rule
        };

        // One send per type: the payload is encoded once and shared, so the
        // type name is cloned per batch rather than per recipient.
        for (r, recipients) in recipients_by_rule.into_iter().enumerate() {
            if recipients.is_empty() {
                continue;
            }
            self.send_direct_message_to_players(
                &recipients,
                ServerMessage::SpawnMonsterRequest {
                    monster_type: ambient_spawns[r].monster_type.clone(),
                },
            )
            .await;
        }
    }

    /// Same predicate as `player_ids_within_position(.., EVENT_DELIVERY_RADIUS)`
    /// but against a per-tick snapshot, so a sweep over every monster locks the
    /// roster once instead of twice per monster.
    /// The adopter is the least-loaded candidate, nearest on ties — one player
    /// must not inherit a scattering party's whole population while others
    /// stand idle. `inflight` counts this tick's not-yet-applied handoffs on
    /// top of what `owned_by` already answers.
    fn attendance(
        monsters: &MonsterRegistry,
        inflight: &std::collections::HashMap<PlayerId, usize>,
        roster: &PlayerSnapshot,
        monster: &crate::types::Monster,
    ) -> Attendance {
        let mut best: Option<(PlayerId, usize, f32)> = None;
        for (id, dist_sq) in Self::players_in_aoi(roster, &monster.position, monster.floor_level) {
            if monster.owner_id == Some(id) {
                return Attendance::Owner;
            }
            let load = monsters.owned_by(&id) + inflight.get(&id).copied().unwrap_or(0);
            if best.is_none_or(|(_, best_load, best_dist)| {
                load < best_load || (load == best_load && dist_sq < best_dist)
            }) {
                best = Some((id, load, dist_sq));
            }
        }
        match best {
            Some((id, ..)) => Attendance::Bystander(id),
            None => Attendance::Nobody,
        }
    }

    /// Every player inside the AOI around `position` on `floor_level`, with the
    /// squared distance. Lazy so `attendance` can stop at the owner; the seam
    /// can yield a player twice, which neither caller minds.
    fn players_in_aoi<'a>(
        roster: &'a PlayerSnapshot,
        position: &'a Position,
        floor_level: i8,
    ) -> impl Iterator<Item = (PlayerId, f32)> + 'a {
        let radius_sq = super::EVENT_DELIVERY_RADIUS * super::EVENT_DELIVERY_RADIUS;
        super::SpatialCell::within_radius(position, super::EVENT_DELIVERY_RADIUS)
            .filter_map(move |cell| roster.get(&cell))
            .flatten()
            .filter_map(move |(id, player_position, floor)| {
                let dist_sq = position.dist_xz_sq(player_position);
                (*floor == floor_level && dist_sq <= radius_sq).then_some((*id, dist_sq))
            })
    }

    /// Safety-net reconcile of every monster against who is actually near it.
    ///
    /// Ownership is settled event-side — the player-move AOI diff releases
    /// what a mover leaves behind, the move fanout catches a monster wandering
    /// off, and entering AOI adopts unattended monsters — so on a healthy
    /// server this sweep finds nothing. What it repairs is the race those
    /// events can lose (two players crossing a monster's AOI boundary at the
    /// same instant, each seeing the other as still inside) and whatever a bug
    /// strands: a bystander adopts, and a monster nobody can see is deleted,
    /// since keeping it would only hold a cap slot against a monster that
    /// exists for no player.
    ///
    /// Handoff applies to every lifecycle; the despawn only to monsters whose
    /// lifecycle leaves removal here — a dungeon floor despawns its own slot
    /// spawns on exit.
    pub async fn tick_monster_ownership(&self) {
        let roster = {
            let players = self.players.read().await;
            let mut roster = PlayerSnapshot::default();
            for (id, player) in players.iter() {
                roster
                    .entry(super::SpatialCell::from_position(&player.position))
                    .or_default()
                    .push((*id, player.position, player.floor_level));
            }
            roster
        };

        let mut expired: Vec<String> = Vec::new();
        let mut handoffs: Vec<Handoff> = Vec::new();
        let mut inflight: std::collections::HashMap<PlayerId, usize> =
            std::collections::HashMap::new();
        let mut scanned = 0usize;
        loop {
            let mut chunk = 0usize;
            let monsters = self.monsters.read().await;
            for monster in monsters.values().skip(scanned).take(OWNERSHIP_SCAN_CHUNK) {
                chunk += 1;
                match Self::attendance(&monsters, &inflight, &roster, monster) {
                    Attendance::Owner => {}
                    Attendance::Bystander(new_owner) => {
                        *inflight.entry(new_owner).or_default() += 1;
                        handoffs.push(Handoff {
                            monster_id: monster.id.clone(),
                            new_owner,
                            old_owner: monster.owner_id,
                        })
                    }
                    Attendance::Nobody => {
                        if monster.lifecycle.despawns_when_unattended() {
                            expired.push(monster.id.clone());
                        }
                    }
                }
            }
            scanned += chunk;
            if chunk < OWNERSHIP_SCAN_CHUNK {
                break;
            }
        }

        self.hand_off_monsters(handoffs).await;
        self.despawn_monsters(expired).await;
    }

    pub(super) async fn despawn_monsters(&self, expired: Vec<String>) {
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
            debug!("Despawned monster {}", monster.id);
            self.announce_monster_removed(&monster).await;
        }
    }

    /// Deliberately ignores the per-player cap: the monster already exists, and
    /// refusing would strand it. The adopter simply gets no new ambient spawns
    /// until back under the cap.
    pub(super) async fn hand_off_monsters(&self, handoffs: Vec<Handoff>) {
        if handoffs.is_empty() {
            return;
        }
        // Chunked like the ownership scan: a mass handoff (server-wide
        // reconcile after a burst of disconnects) must not hold the registry
        // write lock, or the channel map, across all of it at once.
        for batch in handoffs.chunks(OWNERSHIP_SCAN_CHUNK) {
            let reassigned: Vec<(crate::types::Monster, PlayerId, Option<PlayerId>)> = {
                let mut monsters = self.monsters.write().await;
                batch
                    .iter()
                    .filter_map(|h| {
                        let monster = monsters.reassign_owner(&h.monster_id, h.new_owner)?;
                        Some((monster.clone(), h.new_owner, h.old_owner))
                    })
                    .collect()
            };
            // One channel-map guard per batch; `tx.send` is synchronous.
            let channels = self.direct_channels.read().await;
            for (monster, new_owner, old_owner) in reassigned {
                debug!("Monster {} handed to {}", monster.id, new_owner);
                if let Some(tx) = old_owner.and_then(|id| channels.get(&id)) {
                    let _ = tx.send(super::DirectMessage::Typed(ServerMessage::MonsterRemoved {
                        monster_id: monster.id.clone(),
                    }));
                }
                if let Some(tx) = channels.get(&new_owner) {
                    let _ = tx.send(super::DirectMessage::Typed(
                        ServerMessage::MonsterAssigned { monster },
                    ));
                }
            }
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

        // The distance gate, measured where the monster lands. The offer-time
        // check reads the player's distance and is one max_distance looser.
        if self.town_distance(&position) < self.min_ambient_town_distance(monster_type) {
            return None;
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
