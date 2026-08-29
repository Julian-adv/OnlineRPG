//! Server-side dungeon runtime. Layouts are regenerated deterministically
//! from the entrance id (same shared-crate generator the client runs via
//! wasm), so the runtime holds only live state: cached layouts and spawn
//! slots. That state is in-memory — after a restart, reconnecting players
//! rehydrate from the generator plus their persisted position/floor_level.
//! The one exception is the treasure-chest claim, which is DB-backed
//! (`GameState::chest_opens`) because a lost claim is free loot. Locked
//! floors, keys and the keyed chest: doc/DUNGEON_REWARD.md.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use onlinerpg_shared::dungeon::{
    cell_center, dungeon_origin, floor_height_at, floor_level_for_passability, floor_world_y,
    generate_dungeon_for, interior_doors, key_depth_for, last_locked_depth, leg_touches_shaft,
    locked_depths, locked_door_ids, monster_level_for_depth, on_stair_shaft, world_to_cell,
    FloorLayout, PropKind, DUNGEON_FLOOR_HEIGHT, ENTRANCE_DOOR_ID, FLOOR_CHANGE_LEG_MAX, GRID,
    SHAFT_CHANGE_MARGIN,
};
use onlinerpg_shared::inventory::GroundItem;
use onlinerpg_shared::{wrap_world_x, Position, ServerMessage};
use rand::Rng;
use tracing::{info, warn};

use crate::types::PlayerId;

use super::monster::Handoff;
use super::GameState;

const MONSTER_RESPAWN_MS: u64 = 5 * 60 * 1000;
/// Held until `reset_dungeons`: one guardian per night.
pub(super) const BOSS_RESPAWN_NEVER: u64 = u64::MAX;
/// Per-kill chance that a monster in the section above a locked floor drops
/// that floor's key (doc/DUNGEON_REWARD.md — a starting value).
const KEY_DROP_CHANCE: f64 = 0.05;
/// Same, for a barrel, crate or clutter chest cracked open.
const PROP_KEY_DROP_CHANCE: f64 = 0.01;
/// A door opened with a key shuts itself again after this.
pub(super) const LOCKED_DOOR_OPEN_DURATION: Duration = Duration::from_secs(30);
/// How long after arriving on a floor a player in its stair room stays out
/// of monster sight — the floor's loading hitch, not a hideout.
pub(super) const STAIR_ROOM_ARRIVAL_GRACE_MS: u64 = 10_000;
/// Retry delay when a spawn attempt failed (e.g. global monster cap).
const SPAWN_RETRY_MS: u64 = 10 * 1000;
/// Ejected treasure-chest loot lands this far from the chest, scattered at a
/// random angle. The minimum keeps items out from under the chest itself.
const CHEST_LOOT_SCATTER_MIN: f32 = 0.8;
const CHEST_LOOT_SCATTER_MAX: f32 = 3.0;
pub(super) const DUNGEON_RESET_WARNING_DURATION: Duration = Duration::from_millis(4_640);

/// Loot-pickup grace before a chest opener is sent back to town.
const CHEST_RETURN_DELAY: Duration = Duration::from_secs(60);
/// How close a player must stand to a prop to break (barrel/crate) or open
/// (chest, the treasure chest included) it.
const PROP_INTERACT_RANGE: f32 = 2.5;
/// How close a player must stand to a doorway to work it; the client gates the
/// click at 2m from the leaf it hit. `toggle_dungeon_door` adds half the
/// opening on top, the reach of a leaf standing open.
const DOOR_INTERACT_RANGE: f32 = 2.5;
/// Chance that a freshly-broken barrel/crate spills a loose coin pile.
const BROKEN_PROP_COIN_DROP_CHANCE: f64 = 0.20;

/// What `validated_dungeon_floor` settled on: the floor to store, and the Y
/// to store — the floor's own ground height underground, the reported one
/// above ground where terrain owns Y.
pub(super) struct DungeonFloorVerdict {
    pub(super) floor: i8,
    pub(super) y: f32,
}

/// A world-X range split at the wrap seam into canonical segments, so cell
/// enumeration over it matches the cells wrapped player positions produce.
fn split_wrapped_x(min_x: f32, max_x: f32) -> Vec<(f32, f32)> {
    use onlinerpg_shared::{WORLD_MAX_X, WORLD_MIN_X};
    if min_x < WORLD_MIN_X {
        vec![(WORLD_MIN_X, max_x), (wrap_world_x(min_x), WORLD_MAX_X)]
    } else if max_x >= WORLD_MAX_X {
        vec![(min_x, WORLD_MAX_X), (WORLD_MIN_X, wrap_world_x(max_x))]
    } else {
        vec![(min_x, max_x)]
    }
}

/// Player-spatial-hash cell → entrances whose discovery region — the sight
/// circle around the entrance plus its grid footprint — overlaps it.
/// Coverage is exact by construction (integer cell enumeration over each
/// region's bounding box); a stray extra cell only costs a redundant precise
/// check, and `discovery_cell_prefilter_covers_the_discovery_region` guards
/// the whole property against regressions.
#[allow(clippy::type_complexity)]
pub(super) fn discovery_cells(
    dungeon_defs: &crate::dungeon_defs::DungeonDefs,
) -> HashMap<super::SpatialCell, Vec<&'static crate::dungeon_defs::DungeonEntranceDef>> {
    let r = super::EVENT_DELIVERY_RADIUS;
    let mut cells: HashMap<_, Vec<&'static crate::dungeon_defs::DungeonEntranceDef>> =
        HashMap::new();
    for e in dungeon_defs.all() {
        let (ox, oz) = dungeon_origin(e.x, e.z);
        let min_z = (e.z - r).min(oz);
        let max_z = (e.z + r).max(oz + GRID as f32);
        for (min_x, max_x) in split_wrapped_x((e.x - r).min(ox), (e.x + r).max(ox + GRID as f32)) {
            for cell in super::SpatialCell::covering(min_x, max_x, min_z, max_z) {
                cells.entry(cell).or_default().push(e);
            }
        }
    }
    cells
}

pub(super) struct DungeonRuntime {
    pub layouts: Vec<FloorLayout>,
    /// Live per-floor state, keyed by depth. Created when a player first
    /// enters the floor.
    pub floors: HashMap<u8, FloorRuntime>,
    /// Broken props per depth (indices into that floor's `props`). Shared across
    /// the instance, persists across re-entry; resets on server restart. Kept on
    /// the dungeon (not the per-floor runtime) so a break still records even if
    /// the floor's `FloorRuntime` hasn't been created (e.g. a relog rehydrate
    /// that didn't replay the floor-entry transition).
    pub broken_props: HashMap<u8, HashSet<u32>>,
    /// Opened chest props per depth (indices into that floor's `props`). Same
    /// lifetime/scope as `broken_props`; chests stay solid when opened (only the
    /// lid animates), so this drives no passability change.
    pub opened_props: HashMap<u8, HashSet<u32>>,
    /// Open doors per depth (depth 0 = the surface entrance door; ≥1 = interior
    /// room doors). Both sides derive `door_id` from the door's geometry, and
    /// `toggle_dungeon_door` only admits ids that resolve, so this stays
    /// bounded by the layout. Same lifetime as `broken_props`.
    pub open_doors: HashMap<u8, HashSet<u32>>,
    /// Key-gated door ids per floor (index = depth - 1); empty on unlocked floors.
    pub locked_doors: Vec<Vec<u32>>,
    /// Which opening each open locked door is on, so the timed close can
    /// tell its own opening from a later one.
    pub locked_door_opens: HashMap<(u8, u32), u64>,
    pub locked_door_generation: u64,
}

pub(super) struct FloorRuntime {
    /// One slot per layout SpawnSpec, same order.
    pub slots: Vec<SpawnSlot>,
    /// Occupants, with when each stepped onto the floor (arrival grace).
    pub players: HashMap<PlayerId, u64>,
}

pub(super) struct SpawnSlot {
    pub alive_monster_id: Option<String>,
    pub respawn_at_ms: u64,
    pub is_boss: bool,
}

impl SpawnSlot {
    /// A slain boss, waiting on the dungeon reset rather than a timer.
    fn held_until_reset(&self) -> bool {
        self.respawn_at_ms == BOSS_RESPAWN_NEVER
    }
}

/// Reverse index entry: which dungeon slot a live monster belongs to.
pub(super) struct DungeonMonsterRef {
    pub entrance_id: String,
    pub depth: u8,
    pub slot: usize,
    pub is_boss: bool,
}

fn prop_wall_opposite_dir(layout: &FloorLayout, x: i32, z: i32) -> (i32, i32) {
    // Pick the adjacent wall the same way the client orients chest props
    // (N, S, W, E), then step toward the opposite/open side.
    if !layout.is_carved(x, z - 1) {
        (0, 1)
    } else if !layout.is_carved(x, z + 1) {
        (0, -1)
    } else if !layout.is_carved(x - 1, z) {
        (1, 0)
    } else if !layout.is_carved(x + 1, z) {
        (-1, 0)
    } else {
        (0, 0)
    }
}

/// Squared XZ distance from `pos` to a door's blocking line, measured to the
/// whole segment so a wide opening is in reach from either end. `seg` is
/// floor-local grid lines, and those sit on integer world coordinates, so
/// cells need no half-cell offset here. `InteriorDoorSpec::seg` always runs
/// low corner to high, which the clamp relies on.
pub(super) fn door_line_dist_sq(entrance: &Position, seg: [i32; 4], pos: &Position) -> f32 {
    let (ox, oz) = dungeon_origin(entrance.x, entrance.z);
    let [ax, az, bx, bz] = seg;
    // Offset past each end, zero along the segment. The fixed axis has zero
    // length, so its offset falls through untouched.
    let dx = onlinerpg_shared::shortest_world_delta_x(ox + ax as f32, pos.x);
    let dz = pos.z - (oz + az as f32);
    let dx = dx - dx.clamp(0.0, (bx - ax) as f32);
    let dz = dz - dz.clamp(0.0, (bz - az) as f32);
    dx * dx + dz * dz
}

impl GameState {
    /// Lazily generate and cache the layouts for a dungeon.
    pub(super) async fn ensure_dungeon_runtime(&self, entrance_id: &str) {
        {
            let dungeons = self.dungeons.read().await;
            if dungeons.contains_key(entrance_id) {
                return;
            }
        }
        let layouts = generate_dungeon_for(entrance_id);
        let locked_doors = layouts.iter().map(locked_door_ids).collect();
        info!(
            "Dungeon '{}' runtime created ({} floors)",
            entrance_id,
            layouts.len()
        );
        let mut dungeons = self.dungeons.write().await;
        dungeons
            .entry(entrance_id.to_string())
            .or_insert(DungeonRuntime {
                layouts,
                floors: HashMap::new(),
                broken_props: HashMap::new(),
                opened_props: HashMap::new(),
                open_doors: HashMap::new(),
                locked_doors,
                locked_door_opens: HashMap::new(),
                locked_door_generation: 0,
            });
    }

    /// Toggle a dungeon door's open state and return the new state: the server
    /// flips the stored state, reseals the floor's passability (a shut interior
    /// door blocks movement and pathing) and lets the connection layer
    /// broadcast. Interior door ids map to corridor-mouth segments via
    /// `interior_doors`.
    ///
    /// The door is shared state, so nothing changes until the request is
    /// authorized against the floor, the door, and the reach to it. A locked
    /// door also wants the floor's key in the bag, from either side and in
    /// either direction; the key is not spent, and an opened locked door shuts
    /// itself after `LOCKED_DOOR_OPEN_DURATION`.
    pub async fn toggle_dungeon_door(
        &self,
        player_id: &PlayerId,
        entrance_id: &str,
        depth: u8,
        door_id: u32,
    ) -> Option<bool> {
        let entrance = self.dungeon_defs.get(entrance_id)?;
        let expected_floor = -i8::try_from(depth).ok()?;
        let (player_pos, _, player_floor, player_name) = self.player_pose(player_id).await?;
        if player_floor != expected_floor {
            warn!(
                "Door toggle refused: {player_name} on floor {player_floor} asked for '{entrance_id}' depth {depth} door {door_id}"
            );
            return None;
        }

        if depth == 0 {
            // The entrance shed is client-side geometry — the server has the
            // shaft but not the doorway within it — so the entrance marker's
            // delivery circle stands in for reach. Loose next to the 2.5m
            // interior gate, but the toggle still drives collision on every
            // client nearby, so it is not left open to the whole world.
            if door_id != ENTRANCE_DOOR_ID {
                return None;
            }
            if player_pos.dist_xz_sq(&entrance.position())
                > super::EVENT_DELIVERY_RADIUS * super::EVENT_DELIVERY_RADIUS
            {
                return None;
            }
        }
        self.ensure_dungeon_runtime(entrance_id).await;
        let mut locked = false;
        if depth > 0 {
            // Needs the cached layouts, so this gate runs after the ensure;
            // layouts are immutable, so the scan holds only the read lock.
            let door = {
                let dungeons = self.dungeons.read().await;
                let layout = dungeons
                    .get(entrance_id)?
                    .layouts
                    .get((depth - 1) as usize)?;
                interior_doors(layout)
                    .into_iter()
                    .find(|d| d.door_id == door_id)?
            };
            let reach = DOOR_INTERACT_RANGE + door.len as f32 * 0.5;
            let dist_sq = door_line_dist_sq(&entrance.position(), door.seg(), &player_pos);
            if dist_sq > reach * reach {
                warn!(
                    "Door toggle refused: {player_name} is {:.1}m from '{entrance_id}' depth {depth} door {door_id} (reach {reach:.1})",
                    dist_sq.sqrt()
                );
                return None;
            }
            locked = door.locked;
            if locked && !self.require_key(player_id, entrance, depth, "door").await {
                return None;
            }
        }

        let (is_open, opening) = {
            let mut dungeons = self.dungeons.write().await;
            let rt = dungeons.get_mut(entrance_id)?;
            let set = rt.open_doors.entry(depth).or_default();
            let is_open = if set.remove(&door_id) {
                false
            } else {
                set.insert(door_id);
                true
            };
            let opening = (locked && is_open).then(|| {
                rt.locked_door_generation += 1;
                rt.locked_door_opens
                    .insert((depth, door_id), rt.locked_door_generation);
                rt.locked_door_generation
            });
            if locked && !is_open {
                rt.locked_door_opens.remove(&(depth, door_id));
            }
            (is_open, opening)
        };
        if let Some(opening) = opening {
            let game_state = self.clone();
            let entrance_id = entrance_id.to_string();
            tokio::spawn(async move {
                tokio::time::sleep(LOCKED_DOOR_OPEN_DURATION).await;
                game_state
                    .close_locked_door(&entrance_id, depth, door_id, opening)
                    .await;
            });
        }
        self.rebuild_dungeon_floor_passability(entrance_id, depth)
            .await;
        info!(
            "Player {player_name} {} '{entrance_id}' depth {depth} door {door_id} at ({:.1},{:.1})",
            if is_open { "opened" } else { "closed" },
            player_pos.x,
            player_pos.z
        );
        Some(is_open)
    }

    /// Shut a locked door `LOCKED_DOOR_OPEN_DURATION` after `opening`, unless
    /// it was closed and reopened since (a later opening), and tell the
    /// floor's occupants. Anyone who slipped through without a key is now on
    /// the far side for good — that is the tailgater's own choice.
    async fn close_locked_door(&self, entrance_id: &str, depth: u8, door_id: u32, opening: u64) {
        let occupants: Vec<PlayerId> = {
            let mut dungeons = self.dungeons.write().await;
            let Some(rt) = dungeons.get_mut(entrance_id) else {
                return;
            };
            if rt.locked_door_opens.get(&(depth, door_id)) != Some(&opening) {
                return;
            }
            rt.locked_door_opens.remove(&(depth, door_id));
            if !rt
                .open_doors
                .get_mut(&depth)
                .is_some_and(|set| set.remove(&door_id))
            {
                return;
            }
            rt.floors
                .get(&depth)
                .map(|fr| fr.players.keys().copied().collect())
                .unwrap_or_default()
        };
        self.rebuild_dungeon_floor_passability(entrance_id, depth)
            .await;
        info!("'{entrance_id}' depth {depth} door {door_id} locked itself again");
        self.send_direct_message_to_players(
            &occupants,
            ServerMessage::DungeonDoorToggled {
                entrance_id: entrance_id.to_string(),
                depth,
                door_id,
                is_open: false,
            },
        )
        .await;
    }

    /// The key gate: whether the player carries the key to `entrance`'s
    /// locked floor `depth`; a refusal tells them which key `what` takes.
    async fn require_key(
        &self,
        player_id: &PlayerId,
        entrance: &crate::dungeon_defs::DungeonEntranceDef,
        depth: u8,
        what: &str,
    ) -> bool {
        let key = entrance.key_item_id(depth);
        if self.holds_item(player_id, &key).await {
            return true;
        }
        self.send_direct_message(
            player_id,
            ServerMessage::InteractionRejected {
                reason: format!("The {what} is locked. It takes a {}.", self.item_name(&key)),
            },
        )
        .await;
        false
    }

    /// Spend one of each of the dungeon's floor keys the player carries and
    /// push the bag. The deepest one is required (checked by the caller); the
    /// shallower ones just go with it.
    async fn take_dungeon_keys(
        &self,
        player_id: &PlayerId,
        entrance: &crate::dungeon_defs::DungeonEntranceDef,
        total: u8,
    ) {
        let snapshot = {
            let mut inventories = self.inventories.write().await;
            let Some(inv) = inventories.get_mut(player_id) else {
                return;
            };
            for depth in locked_depths(total) {
                super::inventory::draw_from_bag(&mut inv.bag, &entrance.key_item_id(depth), 1);
            }
            inv.clone()
        };
        self.mark_dirty(player_id).await;
        self.mark_inventory_dirty(player_id).await;
        self.send_inventory_snapshot(player_id, snapshot).await;
    }

    /// Players in a floor's arrival stair room (`rooms[0]`) that monster AI
    /// must not see: fresh arrivals (`STAIR_ROOM_ARRIVAL_GRACE_MS`) and anyone
    /// behind shut locked doors (doc/DUNGEON_REWARD.md §4).
    pub(super) async fn players_hidden_in_stair_rooms(
        &self,
        underground: &[(PlayerId, Position, i8)],
    ) -> HashSet<PlayerId> {
        let mut hidden = HashSet::new();
        if underground.is_empty() {
            return hidden;
        }
        let now = Self::now_ms();
        let dungeons = self.dungeons.read().await;
        for (id, pos, floor) in underground {
            let depth = floor.unsigned_abs();
            let Some(entrance) = self.dungeon_defs.entrance_at(pos.x, pos.z) else {
                continue;
            };
            let Some(rt) = dungeons.get(&entrance.id) else {
                continue;
            };
            let Some(layout) = rt.layouts.get(depth as usize - 1) else {
                continue;
            };
            let (cx, cz) = world_to_cell(&entrance.position(), pos.x, pos.z);
            if !layout.rooms.first().is_some_and(|r| r.contains(cx, cz)) {
                continue;
            }
            let just_arrived = rt
                .floors
                .get(&depth)
                .and_then(|fr| fr.players.get(id))
                .is_some_and(|&at| now.saturating_sub(at) < STAIR_ROOM_ARRIVAL_GRACE_MS);
            let open = rt.open_doors.get(&depth);
            let sealed = rt.locked_doors.get(depth as usize - 1).is_some_and(|ids| {
                !ids.is_empty() && !ids.iter().any(|d| open.is_some_and(|o| o.contains(d)))
            });
            if just_arrived || sealed {
                hidden.insert(*id);
            }
        }
        hidden
    }

    /// Deliver a toggle to players near the door on the door's own floor.
    /// Depth 0 centers on the entrance so the circle matches the client's
    /// snapshot re-pull boundary; interior doors center on the toggler, whom
    /// `toggle_dungeon_door` has already put within reach. The toggler is also
    /// sent directly, so their own reply never rides on the radius sweep.
    pub(crate) async fn publish_dungeon_door_toggle(
        &self,
        player_id: &PlayerId,
        entrance_id: String,
        depth: u8,
        door_id: u32,
        is_open: bool,
    ) {
        let (center, floor_level) = if depth == 0 {
            let Some(def) = self.dungeon_defs.get(&entrance_id) else {
                return;
            };
            (def.position(), 0)
        } else {
            let Some((position, _, _)) = self.get_player_position(player_id).await else {
                return;
            };
            (position, -(depth as i8))
        };
        let toggled = ServerMessage::DungeonDoorToggled {
            entrance_id,
            depth,
            door_id,
            is_open,
        };
        self.send_direct_message(player_id, toggled.clone()).await;
        self.send_direct_message_to_players_within_position(
            &center,
            floor_level,
            super::EVENT_DELIVERY_RADIUS,
            toggled,
            Some(player_id),
        )
        .await;
    }

    /// Every currently-open door in a dungeon as (depth, door_id) pairs, for the
    /// RequestDungeonDoors snapshot. Reads without creating the runtime — an
    /// untouched dungeon simply has no open doors.
    pub async fn dungeon_open_doors(&self, entrance_id: &str) -> Vec<(u8, u32)> {
        let dungeons = self.dungeons.read().await;
        let Some(rt) = dungeons.get(entrance_id) else {
            return Vec::new();
        };
        rt.open_doors
            .iter()
            .flat_map(|(depth, ids)| ids.iter().map(move |id| (*depth, *id)))
            .collect()
    }

    /// Seed a character's chest history at login. Without it a reconnect
    /// would read as "never opened".
    pub async fn set_chest_opens(&self, character_id: i64, opens: Vec<(String, i64)>) {
        if opens.is_empty() {
            return;
        }
        let mut chest_opens = self.chest_opens.write().await;
        for (entrance_id, opened_game_seconds) in opens {
            chest_opens.insert((character_id, entrance_id), opened_game_seconds);
        }
    }

    /// Seed a player's discovered-entrance set at login so known entrances
    /// are not re-announced. A load failure refuses game entry, so this
    /// always receives the character's full persisted history.
    pub async fn set_dungeon_discoveries(&self, player_id: &PlayerId, ids: Vec<String>) {
        let mut discoveries = self.dungeon_discoveries.write().await;
        discoveries.insert(*player_id, ids.into_iter().collect());
    }

    pub async fn remove_dungeon_discoveries(&self, player_id: &PlayerId) {
        let mut discoveries = self.dungeon_discoveries.write().await;
        discoveries.remove(player_id);
    }

    /// Record the first time a player comes within event-delivery range of a
    /// dungeon entrance (or stands on its footprint, which rehydrated logins
    /// deep inside a dungeon may reach first) and push the updated snapshot
    /// to them. Runs on every position change: the static cell index rejects
    /// moves nowhere near an entrance without touching a lock, and the
    /// near-entrance case tests only that cell's entrances under a read
    /// lock, geometry before id hashing.
    pub(super) async fn check_dungeon_discovery(&self, player_id: &PlayerId, position: &Position) {
        let Some(candidates) = self
            .dungeon_discovery_cells
            .get(&super::SpatialCell::from_position(position))
        else {
            return;
        };
        let radius_sq = super::EVENT_DELIVERY_RADIUS * super::EVENT_DELIVERY_RADIUS;
        let entrance = {
            let discoveries = self.dungeon_discoveries.read().await;
            let known = discoveries.get(player_id);
            let Some(entrance) = candidates.iter().find(|e| {
                (position.dist_xz_sq(&e.position()) <= radius_sq
                    || e.footprint_contains(position.x, position.z))
                    && !known.is_some_and(|k| k.contains(&e.id))
            }) else {
                return;
            };
            *entrance
        };
        let ids = {
            let mut discoveries = self.dungeon_discoveries.write().await;
            let known = discoveries.entry(*player_id).or_default();
            // Re-checked under the write lock: a concurrent update may have
            // recorded the same entrance between the two locks.
            if !known.insert(entrance.id.clone()) {
                return;
            }
            known.iter().cloned().collect::<Vec<String>>()
        };
        if let Some(character_id) = self.character_id_of(player_id).await {
            let mut pending = self.pending_discovery_saves.write().await;
            pending.push((character_id, entrance.id.clone()));
        }
        self.send_direct_message(
            player_id,
            ServerMessage::DungeonDiscoveries { entrance_ids: ids },
        )
        .await;
    }

    /// Drain the queued discovery rows for the next `save_batch`.
    pub(super) async fn take_pending_discovery_saves(&self) -> Vec<(i64, String)> {
        std::mem::take(&mut *self.pending_discovery_saves.write().await)
    }

    /// Re-queue rows whose batch failed, like the other dirty-set restores.
    pub(super) async fn restore_pending_discovery_saves(&self, rows: Vec<(i64, String)>) {
        self.pending_discovery_saves.write().await.extend(rows);
    }

    /// Claim this character's chest open for the current night, returning
    /// false when they already took it. The chest refills at nightfall, so
    /// two opens are only ever a night apart.
    ///
    /// Entries from earlier nights carry no information — the chest has
    /// already refilled — so the claim drops them on its way through,
    /// keeping the map bounded without a logout hook (see `buybacks` for the
    /// same sweep-as-you-touch-it approach).
    async fn claim_chest_open(
        &self,
        character_id: i64,
        entrance_id: &str,
        now_seconds: i64,
    ) -> bool {
        let tonight = Self::night_epoch(now_seconds);
        let key = (character_id, entrance_id.to_string());
        let mut chest_opens = self.chest_opens.write().await;
        if Self::opened_since(chest_opens.get(&key), tonight) {
            return false;
        }
        chest_opens.retain(|_, &mut opened| Self::night_epoch(opened) >= tonight);
        chest_opens.insert(key, now_seconds);
        true
    }

    /// Whether a recorded chest open falls on or after the night `epoch`.
    fn opened_since(opened: Option<&i64>, epoch: i64) -> bool {
        opened.is_some_and(|&opened| Self::night_epoch(opened) >= epoch)
    }

    /// Release a claim whose DB write failed, so a repaired storage layer can
    /// retry tonight. Removes only the timestamp this attempt inserted — a
    /// relog can rehydrate the key from the DB mid-write (`set_chest_opens`),
    /// and that value is the durable one.
    async fn rollback_chest_open_claim(
        &self,
        character_id: i64,
        entrance_id: &str,
        opened_game_seconds: i64,
    ) {
        let key = (character_id, entrance_id.to_string());
        let mut chest_opens = self.chest_opens.write().await;
        if chest_opens.get(&key) == Some(&opened_game_seconds) {
            chest_opens.remove(&key);
        }
    }

    /// Open the final-floor treasure chest: next to it on the deepest floor,
    /// carrying the deepest locked floor's key, one open per character per
    /// night. The open spends one of each of the dungeon's keys the opener
    /// holds. The rolled items burst out of the chest as ground drops
    /// scattered around it (anyone nearby may grab them); the depth-scaled
    /// gold goes straight to the opener. The open is broadcast nearby.
    /// Re-opening an already-claimed chest still swings the lid: the clicker
    /// alone gets an item-less `DungeonChestOpened` showing an empty box, and
    /// keeps the keys.
    pub async fn open_dungeon_chest(
        &self,
        player_id: &PlayerId,
        entrance_id: &str,
        auth_service: &crate::auth::AuthService,
    ) {
        let Some(entrance) = self.dungeon_defs.get(entrance_id) else {
            return;
        };
        let Some(character_id) = self.character_id_of(player_id).await else {
            return;
        };
        self.ensure_dungeon_runtime(entrance_id).await;

        let (player_pos, player_floor) = {
            let players = self.players.read().await;
            match players.get(player_id) {
                Some(p) if p.health > 0 => (p.position, p.floor_level),
                _ => return,
            }
        };

        let (chest_pos, total, position_check) = {
            let dungeons = self.dungeons.read().await;
            let Some(rt) = dungeons.get(entrance_id) else {
                return;
            };
            let total = rt.layouts.len() as u8;
            let last = match rt.layouts.last() {
                Some(l) => l,
                None => return,
            };
            let Some(chest) = last.chest else { return };
            let chest_pos = cell_center(&entrance.position(), total, chest);

            let check = if player_floor != -(total as i8) {
                Some("You must be on the deepest floor")
            } else {
                let dx = onlinerpg_shared::shortest_world_delta_x(chest_pos.x, player_pos.x);
                let dz = player_pos.z - chest_pos.z;
                (dx * dx + dz * dz > PROP_INTERACT_RANGE * PROP_INTERACT_RANGE)
                    .then_some("Too far from the chest")
            };
            (chest_pos, total, check)
        };
        if let Some(reason) = position_check {
            self.send_direct_message(
                player_id,
                ServerMessage::InteractionRejected {
                    reason: reason.to_string(),
                },
            )
            .await;
            return;
        }
        if let Some(depth) = last_locked_depth(total) {
            if !self.require_key(player_id, entrance, depth, "chest").await {
                return;
            }
        }
        let now_seconds = self.current_total_game_seconds();
        if !self
            .claim_chest_open(character_id, entrance_id, now_seconds)
            .await
        {
            // Already claimed tonight: an item-less open swings the lid on
            // an empty box, for the clicker only.
            self.send_direct_message(
                player_id,
                ServerMessage::DungeonChestOpened {
                    entrance_id: entrance_id.to_string(),
                    player_id: *player_id,
                    item_def_ids: Vec::new(),
                    gold: 0,
                },
            )
            .await;
            return;
        }
        // Persist the claim before handing out loot: a crash between the two
        // costs the player a chest, the other order costs everyone a refill.
        let auth = auth_service.clone();
        let owner = entrance_id.to_string();
        if let Err(err) = super::auth_db(move || {
            auth.record_dungeon_chest_open(character_id, &owner, now_seconds)
        })
        .await
        {
            warn!("Failed to persist chest open for character {character_id}: {err}");
            self.rollback_chest_open_claim(character_id, entrance_id, now_seconds)
                .await;
            self.send_direct_message(
                player_id,
                ServerMessage::InteractionRejected {
                    reason: "The chest could not be saved; try again".to_string(),
                },
            )
            .await;
            return;
        }

        self.take_dungeon_keys(player_id, entrance, total).await;

        // Roll loot: guaranteed signature drops, then an independent chance
        // roll per pool item (doc/ITEM_TIERS.md — 던전당 기대 ~5회).
        let depth = total as i64;
        let (item_def_ids, gold) = {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            let mut items = entrance.chest_drops.clone();
            let table = self.item_defs.chest_roll_table(entrance.chest_tier);
            items.extend(crate::world_drop_defs::roll_independent(
                table
                    .iter()
                    .filter(|(id, _)| !entrance.chest_drops.contains(id))
                    .map(|(id, chance)| (id.as_str(), *chance)),
                &mut rng,
            ));
            let gold = rng.gen_range(depth * 500..=depth * 1500);
            (items, gold)
        };

        self.eject_chest_loot(item_def_ids.clone(), chest_pos, player_floor);
        self.schedule_chest_return(player_id).await;
        let new_gold = {
            let mut gold_map = self.player_gold.write().await;
            let wallet = gold_map.entry(*player_id).or_insert(0);
            *wallet += gold;
            *wallet
        };
        self.mark_dirty(player_id).await;
        self.send_direct_message(player_id, ServerMessage::GoldUpdate { gold: new_gold })
            .await;

        info!(
            "Player {} opened dungeon chest '{}': {:?} + {} gold",
            self.player_name_of(player_id).await,
            entrance_id,
            item_def_ids,
            gold
        );
        self.send_direct_message_to_players_within_position(
            &player_pos,
            player_floor,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::DungeonChestOpened {
                entrance_id: entrance_id.to_string(),
                player_id: *player_id,
                item_def_ids,
                gold,
            },
            None,
        )
        .await;
    }

    /// Arm the vault's return: `CHEST_RETURN_DELAY` after a real open the
    /// opener is carried back to town unless they already left the dungeon.
    /// A disconnect in between runs it first (`cleanup_player_session`), so
    /// the deepest floor cannot be a parking spot between refills.
    async fn schedule_chest_return(&self, player_id: &PlayerId) {
        if !self.chest_returns.write().await.insert(*player_id) {
            return;
        }
        self.send_system_message(
            player_id,
            "The vault's magic stirs — you will be carried back to town in a minute.",
        )
        .await;
        let game_state = self.clone();
        let player_id = *player_id;
        tokio::spawn(async move {
            tokio::time::sleep(CHEST_RETURN_DELAY).await;
            game_state.fire_chest_return(&player_id).await;
        });
    }

    /// Run a pending chest return now; no-op without one or once back on
    /// the surface.
    pub(super) async fn fire_chest_return(&self, player_id: &PlayerId) {
        if !self.chest_returns.write().await.remove(player_id) {
            return;
        }
        let in_dungeon = {
            let players = self.players.read().await;
            players.get(player_id).is_some_and(|p| p.floor_level < 0)
        };
        if !in_dungeon {
            return;
        }
        self.teleport_to_town(player_id).await;
        self.send_system_message(player_id, "The vault's magic carries you back to town.")
            .await;
    }

    /// Burst a treasure chest's loot out as scattered ground items once the
    /// lid has swung open. The wait lives server-side so a client that skips
    /// the lid animation can't reach the loot early (same rule as kill loot).
    fn eject_chest_loot(&self, item_def_ids: Vec<String>, chest_pos: Position, floor_level: i8) {
        let game_state = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(*super::combat::CHEST_LOOT_EJECT_DELAY).await;
            game_state
                .spawn_scattered_items(
                    item_def_ids,
                    chest_pos,
                    floor_level,
                    CHEST_LOOT_SCATTER_MIN,
                    CHEST_LOOT_SCATTER_MAX,
                )
                .await;
            // Rare bonus world drops burst out with the rest.
            game_state
                .spawn_world_drops(chest_pos, floor_level, None, Vec::new())
                .await;
        });
    }

    /// Break a destructible dungeon prop (barrel/crate): requires standing
    /// next to it on its floor. Records the break for the instance, makes the
    /// cell walkable (client-side, on receipt) and broadcasts it to nearby
    /// players (the breaker included). On a fresh break, has a small chance to
    /// spill the same loose coin pile that an opened chest prop uses. No-op if
    /// it's already broken.
    pub async fn break_dungeon_prop(
        &self,
        player_id: &PlayerId,
        entrance_id: &str,
        depth: u8,
        prop_id: u32,
    ) {
        let broken_at = self
            .interact_with_dungeon_prop(
                player_id,
                entrance_id,
                depth,
                prop_id,
                "prop",
                |kind| matches!(kind, PropKind::Barrel | PropKind::Crate),
                |rt| &mut rt.broken_props,
                ServerMessage::DungeonPropBroken {
                    entrance_id: entrance_id.to_string(),
                    depth,
                    prop_id,
                },
            )
            .await;

        if let Some(prop_pos) = broken_at {
            self.rebuild_dungeon_floor_passability(entrance_id, depth)
                .await;
            if rand::thread_rng().gen_bool(BROKEN_PROP_COIN_DROP_CHANCE) {
                let drop_pos = self
                    .prop_wall_opposite_drop_position(entrance_id, depth, prop_id, prop_pos)
                    .await;
                self.spawn_dungeon_coin_pile(drop_pos, -(depth as i8)).await;
            }
            let key = self.roll_prop_key_drop(player_id, entrance_id, depth).await;
            self.spawn_world_drops(prop_pos, -(depth as i8), None, key.into_iter().collect())
                .await;
        }
    }

    /// Open an interactive chest prop: requires standing next to it on its
    /// floor. Records the open for the instance and broadcasts it to nearby
    /// players (the opener included) so every client plays the lid animation.
    /// The chest stays solid — opening changes no passability. No-op if it's
    /// already open. A fresh open also spills a loose coin pile next to the
    /// chest for anyone nearby to grab (1–10 copper on pickup).
    pub async fn open_dungeon_prop(
        &self,
        player_id: &PlayerId,
        entrance_id: &str,
        depth: u8,
        prop_id: u32,
    ) {
        let opened_at = self
            .interact_with_dungeon_prop(
                player_id,
                entrance_id,
                depth,
                prop_id,
                "chest",
                |kind| matches!(kind, PropKind::Chest),
                |rt| &mut rt.opened_props,
                ServerMessage::DungeonPropOpened {
                    entrance_id: entrance_id.to_string(),
                    depth,
                    prop_id,
                },
            )
            .await;

        if let Some(chest_pos) = opened_at {
            let drop_pos = self
                .prop_wall_opposite_drop_position(entrance_id, depth, prop_id, chest_pos)
                .await;
            self.spawn_dungeon_coin_pile(drop_pos, -(depth as i8)).await;
            let key = self.roll_prop_key_drop(player_id, entrance_id, depth).await;
            self.spawn_world_drops(chest_pos, -(depth as i8), None, key.into_iter().collect())
                .await;
        }
    }

    async fn spawn_dungeon_coin_pile(&self, position: Position, floor_level: i8) {
        let instance_id = self.next_instance_id().await;
        self.spawn_ground_item(GroundItem {
            instance_id,
            item_def_id: super::COIN_PILE_ITEM_ID.to_string(),
            position,
            floor_level,
            quantity: 1,
            enchant: 0,
            dropped_by: None,
            cape_color: None,
            cape_texture: None,
        })
        .await;
    }

    /// Where a dungeon prop drops its coin pile: a short step away from the
    /// prop cell toward the carved side opposite the wall it was placed
    /// against. This matches the way chests face into the room and keeps coins
    /// out from under broken debris. Falls back to the prop cell center when
    /// the facing can't be read or the opening cell isn't floor.
    async fn prop_wall_opposite_drop_position(
        &self,
        entrance_id: &str,
        depth: u8,
        prop_id: u32,
        cell_center_pos: Position,
    ) -> Position {
        /// How far out from the prop cell center the coins land.
        const DROP_DIST: f32 = 0.85;

        let dungeons = self.dungeons.read().await;
        let dir = dungeons
            .get(entrance_id)
            .and_then(|rt| rt.layouts.get((depth - 1) as usize))
            .and_then(|layout| {
                let prop = layout.props.get(prop_id as usize)?;
                let (x, z) = (prop.x, prop.z);
                let (cdx, cdz) = prop_wall_opposite_dir(layout, x, z);
                if (cdx, cdz) != (0, 0) && layout.is_carved(x + cdx, z + cdz) {
                    Some((cdx as f32, cdz as f32))
                } else {
                    None
                }
            });

        match dir {
            Some((dx, dz)) => Position {
                x: cell_center_pos.x + dx * DROP_DIST,
                y: cell_center_pos.y,
                z: cell_center_pos.z + dz * DROP_DIST,
            },
            None => cell_center_pos,
        }
    }

    /// Shared handler for a click-to-interact dungeon prop (break a barrel/crate
    /// or open a chest). Validates the prop's kind, the player's floor and
    /// proximity to it, then claims the interaction in the runtime set chosen by
    /// `select_state`. On a fresh claim it broadcasts `on_success` to nearby
    /// players (the actor included) and returns the prop's world position (so
    /// the caller can spawn loot there); a failed check rejects the actor with a
    /// reason built from `noun`. Returns `None` (silent no-op) for a missing
    /// dungeon/prop/player, the wrong prop kind, or an already-claimed prop.
    #[allow(clippy::too_many_arguments)]
    async fn interact_with_dungeon_prop(
        &self,
        player_id: &PlayerId,
        entrance_id: &str,
        depth: u8,
        prop_id: u32,
        noun: &str,
        is_kind: impl Fn(PropKind) -> bool,
        select_state: impl Fn(&mut DungeonRuntime) -> &mut HashMap<u8, HashSet<u32>>,
        on_success: ServerMessage,
    ) -> Option<Position> {
        if depth == 0 {
            return None;
        }
        let entrance = self.dungeon_defs.get(entrance_id)?;
        self.ensure_dungeon_runtime(entrance_id).await;

        let (player_pos, player_floor) = {
            let players = self.players.read().await;
            match players.get(player_id) {
                Some(p) if p.health > 0 => (p.position, p.floor_level),
                _ => return None,
            }
        };

        // Validate against the layout + claim the interaction under the dungeon
        // lock. `Some(Err)` → reject with a reason, `Some(Ok(pos))` → newly
        // claimed (with the prop's world position), `None` → already claimed or
        // missing (silent no-op).
        let outcome: Option<Result<Position, String>> = {
            let mut dungeons = self.dungeons.write().await;
            let rt = dungeons.get_mut(entrance_id)?;
            let prop = match rt
                .layouts
                .get((depth - 1) as usize)
                .and_then(|l| l.props.get(prop_id as usize))
            {
                Some(p) => *p,
                None => return None,
            };
            if !is_kind(prop.kind) {
                return None;
            }
            if player_floor != -(depth as i8) {
                Some(Err(format!("You must be on the {noun}'s floor")))
            } else {
                let prop_pos = cell_center(&entrance.position(), depth, (prop.x, prop.z));
                let dx = onlinerpg_shared::shortest_world_delta_x(prop_pos.x, player_pos.x);
                let dz = player_pos.z - prop_pos.z;
                if dx * dx + dz * dz > PROP_INTERACT_RANGE * PROP_INTERACT_RANGE {
                    Some(Err(format!("Too far from the {noun}")))
                } else if select_state(rt).entry(depth).or_default().insert(prop_id) {
                    Some(Ok(prop_pos))
                } else {
                    None
                }
            }
        };

        match outcome {
            Some(Err(reason)) => {
                self.send_direct_message(player_id, ServerMessage::InteractionRejected { reason })
                    .await;
                None
            }
            Some(Ok(prop_pos)) => {
                self.send_direct_message_to_players_within_position(
                    &player_pos,
                    player_floor,
                    super::EVENT_DELIVERY_RADIUS,
                    on_success,
                    None,
                )
                .await;
                Some(prop_pos)
            }
            None => None,
        }
    }

    /// Debug helper: reset all destructible/openable prop state for a dungeon
    /// instance and push empty snapshots to players currently on its floors.
    pub async fn debug_reset_dungeon_props(&self, entrance_id: &str) {
        if self.dungeon_defs.get(entrance_id).is_none() {
            return;
        }
        self.ensure_dungeon_runtime(entrance_id).await;
        let (total_depths, floor_players): (u8, Vec<(u8, Vec<PlayerId>)>) = {
            let mut dungeons = self.dungeons.write().await;
            let Some(rt) = dungeons.get_mut(entrance_id) else {
                return;
            };
            rt.broken_props.clear();
            rt.opened_props.clear();
            (
                rt.layouts.len() as u8,
                rt.floors
                    .iter()
                    .map(|(depth, floor)| (*depth, floor.players.keys().cloned().collect()))
                    .collect(),
            )
        };

        for depth in 1..=total_depths {
            self.rebuild_dungeon_floor_passability(entrance_id, depth)
                .await;
        }

        for (depth, players) in floor_players {
            if players.is_empty() {
                continue;
            }
            self.send_direct_message_to_players(
                &players,
                ServerMessage::DungeonPropsState {
                    entrance_id: entrance_id.to_string(),
                    depth,
                    broken: Vec::new(),
                    opened: Vec::new(),
                },
            )
            .await;
        }
    }

    /// Track floor occupancy and monster lifecycles across dungeon floor
    /// changes (stairs, death respawn, disconnect, login rehydrate).
    /// `old_pos`/`new_pos` locate the dungeon for each side — on respawn
    /// the new position is the world spawn, far from the footprint.
    pub(crate) async fn handle_player_floor_change(
        &self,
        player_id: &PlayerId,
        old_floor: i8,
        new_floor: i8,
        old_pos: &Position,
        new_pos: &Position,
    ) {
        if old_floor >= 0 && new_floor >= 0 {
            return;
        }
        if old_floor < 0 {
            if let Some(entrance) = self.dungeon_defs.entrance_at(old_pos.x, old_pos.z) {
                self.leave_dungeon_floor(player_id, &entrance.id, (-old_floor) as u8)
                    .await;
            }
        }
        if new_floor < 0 {
            if let Some(entrance) = self.dungeon_defs.entrance_at(new_pos.x, new_pos.z) {
                self.enter_dungeon_floor(player_id, &entrance.id, (-new_floor) as u8)
                    .await;
            }
        }
    }

    async fn enter_dungeon_floor(&self, player_id: &PlayerId, entrance_id: &str, depth: u8) {
        self.ensure_dungeon_runtime(entrance_id).await;
        let (broken, opened): (Vec<u32>, Vec<u32>) = {
            let mut dungeons = self.dungeons.write().await;
            let Some(rt) = dungeons.get_mut(entrance_id) else {
                return;
            };
            let Some(layout) = rt.layouts.get((depth - 1) as usize) else {
                return;
            };
            let slots: Vec<SpawnSlot> = layout
                .spawns
                .iter()
                .map(|s| SpawnSlot {
                    alive_monster_id: None,
                    respawn_at_ms: 0,
                    is_boss: s.is_boss,
                })
                .collect();
            rt.floors
                .entry(depth)
                .or_insert_with(|| FloorRuntime {
                    slots,
                    players: HashMap::new(),
                })
                .players
                .insert(*player_id, Self::now_ms());
            let broken = rt
                .broken_props
                .get(&depth)
                .map(|s| s.iter().copied().collect())
                .unwrap_or_default();
            let opened = rt
                .opened_props
                .get(&depth)
                .map(|s| s.iter().copied().collect())
                .unwrap_or_default();
            (broken, opened)
        };
        // Tell the arriving player which props are already broken (render the
        // broken variant + walk through those cells) or opened (chests in the
        // open pose) from the start (sent even when empty so re-entries reset
        // cleanly).
        self.send_direct_message(
            player_id,
            ServerMessage::DungeonPropsState {
                entrance_id: entrance_id.to_string(),
                depth,
                broken,
                opened,
            },
        )
        .await;
        // Same for doors — see `DungeonDoorsState` for why arrival needs it.
        let doors = self.dungeon_open_doors(entrance_id).await;
        self.send_direct_message(
            player_id,
            ServerMessage::DungeonDoorsState {
                entrance_id: entrance_id.to_string(),
                doors,
            },
        )
        .await;
        self.populate_dungeon_floor(entrance_id, depth, player_id)
            .await;
    }

    /// Spawn monsters into every free, respawn-ready slot of a floor and
    /// assign their AI to `owner`. Claims slots under the lock, spawns
    /// outside it, then records the ids.
    pub(crate) async fn populate_dungeon_floor(
        &self,
        entrance_id: &str,
        depth: u8,
        owner: &PlayerId,
    ) {
        let Some(entrance) = self.dungeon_defs.get(entrance_id) else {
            return;
        };
        let now = Self::now_ms();

        let to_spawn: Vec<(usize, i32, i32, String, bool)> = {
            let mut dungeons = self.dungeons.write().await;
            let Some(rt) = dungeons.get_mut(entrance_id) else {
                return;
            };
            let Some(layout) = rt.layouts.get((depth - 1) as usize) else {
                return;
            };
            let specs = layout.spawns.clone();
            let Some(fr) = rt.floors.get_mut(&depth) else {
                return;
            };
            let mut claimed = Vec::new();
            for (i, slot) in fr.slots.iter_mut().enumerate() {
                if slot.alive_monster_id.is_none() && now >= slot.respawn_at_ms {
                    // Claim under the lock so concurrent callers can't
                    // double-spawn the slot.
                    slot.alive_monster_id = Some(String::new());
                    let spec = &specs[i];
                    claimed.push((
                        i,
                        spec.x,
                        spec.z,
                        spec.monster_type.clone(),
                        spec.aggressive,
                    ));
                }
            }
            claimed
        };

        for (slot_idx, cx, cz, monster_type, aggressive) in to_spawn {
            let def_level = self
                .monster_defs
                .get(&monster_type)
                .map(|d| d.level)
                .unwrap_or(1);
            let level = monster_level_for_depth(def_level, depth);
            let pos = cell_center(&entrance.position(), depth, (cx, cz));
            let spawned = self
                .spawn_monster(
                    monster_type,
                    pos,
                    0.0,
                    Some(*owner),
                    -(depth as i8),
                    crate::types::MonsterLifecycle::DungeonSlot,
                    Some(level),
                    aggressive,
                )
                .await;

            let mut dungeons = self.dungeons.write().await;
            let slot = if let Some(fr) = dungeons
                .get_mut(entrance_id)
                .and_then(|rt| rt.floors.get_mut(&depth))
            {
                fr.slots.get_mut(slot_idx)
            } else {
                None
            };
            match (slot, spawned) {
                (Some(slot), Some(monster)) => {
                    slot.alive_monster_id = Some(monster.id.clone());
                    let is_boss = slot.is_boss;
                    drop(dungeons);
                    let mut index = self.dungeon_monsters.write().await;
                    index.insert(
                        monster.id.clone(),
                        DungeonMonsterRef {
                            entrance_id: entrance_id.to_string(),
                            depth,
                            slot: slot_idx,
                            is_boss,
                        },
                    );
                    drop(index);
                    if !self.server_monster_ai() {
                        self.send_direct_message(owner, ServerMessage::MonsterAssigned { monster })
                            .await;
                    }
                }
                (Some(slot), None) => {
                    slot.alive_monster_id = None;
                    slot.respawn_at_ms = now + SPAWN_RETRY_MS;
                }
                _ => {}
            }
        }
    }

    /// Sunset closes the dungeon day: everyone inside is put out at the
    /// entrance and the guardians rise again. Tied to `night_epoch` because
    /// the chest's one-open-per-character runs on that clock too.
    pub async fn tick_dungeon_reset(&self) {
        let epoch = Self::night_epoch(self.current_total_game_seconds());
        {
            let mut last = self.dungeon_reset_last_epoch.write().await;
            match *last {
                // First tick after boot: record it. A restart must not empty
                // every dungeon.
                None => {
                    *last = Some(epoch);
                    return;
                }
                Some(seen) if seen >= epoch => return,
                Some(_) => *last = Some(epoch),
            }
        }
        self.reset_dungeons().await;
    }

    /// Sweep the dungeons empty, then reset the floors that emptied — clearing
    /// slots under a live floor would orphan its monsters, which
    /// `leave_dungeon_floor` despawns by reading those very ids. Props and
    /// doors keep their state; their loot pays once per dungeon instance.
    ///
    /// A second pass catches anyone who started descending during the first.
    /// Whatever it still misses keeps its floor until the next sunset.
    async fn reset_dungeons(&self) {
        let warned: Vec<PlayerId> = {
            let dungeons = self.dungeons.read().await;
            dungeons
                .values()
                .flat_map(|rt| rt.floors.values())
                .flat_map(|fr| fr.players.keys().copied())
                .collect::<HashSet<_>>()
                .into_iter()
                .collect()
        };
        if !warned.is_empty() {
            self.send_direct_message_to_players(&warned, ServerMessage::DungeonReset)
                .await;
            tokio::time::sleep(DUNGEON_RESET_WARNING_DURATION).await;
        }

        let mut evicted = Vec::new();
        for _ in 0..2 {
            let occupants: Vec<(PlayerId, Position)> = {
                let dungeons = self.dungeons.read().await;
                dungeons
                    .iter()
                    .filter_map(|(id, rt)| {
                        self.dungeon_defs.get(id).map(|def| (def.position(), rt))
                    })
                    .flat_map(|(at, rt)| {
                        rt.floors
                            .values()
                            .flat_map(move |fr| fr.players.keys().map(move |id| (*id, at)))
                    })
                    .collect()
            };
            if occupants.is_empty() {
                break;
            }
            for (player_id, at) in occupants {
                // The normal exit path: hands off the floor's monsters, and a
                // corpse revives at the entrance rather than where it fell.
                self.teleport_player(&player_id, at, 0.0, 0).await;
                evicted.push(player_id);
                // Sunset can empty every dungeon at once; don't hold the tick.
                tokio::task::yield_now().await;
            }
        }
        self.send_direct_message_to_players(
            &evicted,
            ServerMessage::SystemMessage {
                message: "A roar wakes far below, the dark takes you, and you come to at the \
                          entrance."
                    .to_string(),
            },
        )
        .await;

        let mut dungeons = self.dungeons.write().await;
        for rt in dungeons.values_mut() {
            for fr in rt.floors.values_mut().filter(|fr| fr.players.is_empty()) {
                for slot in fr.slots.iter_mut() {
                    slot.alive_monster_id = None;
                    slot.respawn_at_ms = 0;
                }
            }
        }
        info!(
            "Dungeons reset for the new night; {} occupant(s) returned to the surface",
            evicted.len()
        );
    }

    async fn leave_dungeon_floor(&self, player_id: &PlayerId, entrance_id: &str, depth: u8) {
        // Occupancy + alive-monster snapshot under one lock.
        let (remaining_owner, alive_ids) = {
            let mut dungeons = self.dungeons.write().await;
            let Some(fr) = dungeons
                .get_mut(entrance_id)
                .and_then(|rt| rt.floors.get_mut(&depth))
            else {
                return;
            };
            fr.players.remove(player_id);
            let remaining = fr.players.keys().next().cloned();
            let alive: Vec<String> = fr
                .slots
                .iter()
                .filter_map(|s| s.alive_monster_id.clone())
                .filter(|id| !id.is_empty())
                .collect();
            if remaining.is_none() {
                // A slain boss keeps its slot; only reset_dungeons frees it.
                // A living boss despawns with the floor, so its slot is freed.
                for slot in fr.slots.iter_mut().filter(|s| !s.held_until_reset()) {
                    slot.alive_monster_id = None;
                    // Empty floors repopulate instantly on next entry.
                    slot.respawn_at_ms = 0;
                }
            }
            (remaining, alive)
        };

        match remaining_owner {
            Some(new_owner) => {
                // Any remaining occupant will do, rather than one inside the
                // monster's AOI as `tick_monster_ownership` picks: a floor is
                // wider than the AOI, so that rule would despawn monsters on a
                // floor someone is still standing on.
                let handoffs: Vec<Handoff> = {
                    let monsters = self.monsters.read().await;
                    alive_ids
                        .iter()
                        .filter(|id| {
                            monsters
                                .get(id)
                                .is_some_and(|m| m.owner_id.as_ref() == Some(player_id))
                        })
                        .map(|id| Handoff {
                            monster_id: id.clone(),
                            new_owner,
                        })
                        .collect()
                };
                self.hand_off_monsters(handoffs).await;
            }
            None => {
                // Floor emptied: despawn everything (only monsters respawn
                // in a shared dungeon — and this bounds live monster count).
                // The slot index goes first; `despawn_monsters` consumes the ids.
                {
                    let mut index = self.dungeon_monsters.write().await;
                    for id in &alive_ids {
                        index.remove(id);
                    }
                }
                self.despawn_monsters(alive_ids).await;
            }
        }
    }

    /// Periodic tick: refill expired spawn slots on occupied floors so
    /// monsters respawn while players camp a floor.
    pub async fn tick_dungeons(&self) {
        let occupied: Vec<(String, u8, PlayerId)> = {
            let dungeons = self.dungeons.read().await;
            dungeons
                .iter()
                .flat_map(|(id, rt)| {
                    rt.floors.iter().filter_map(|(depth, fr)| {
                        fr.players.keys().next().map(|p| (id.clone(), *depth, *p))
                    })
                })
                .collect()
        };
        for (entrance_id, depth, owner) in occupied {
            self.populate_dungeon_floor(&entrance_id, depth, &owner)
                .await;
        }
    }

    /// Pick where a slain monster's loot lands. On a dungeon floor the
    /// random scatter is clamped onto walkable floor so the item never ends
    /// up inside a wall, where the proximity-only pickup could never reach
    /// it. On the surface (floor >= 0) the scatter is used unchanged.
    pub(super) async fn loot_drop_position(
        &self,
        monster_position: Position,
        floor_level: i8,
        preferred: Position,
    ) -> Position {
        if floor_level >= 0 {
            return preferred;
        }
        let Some(entrance) = self
            .dungeon_defs
            .entrance_at(monster_position.x, monster_position.z)
        else {
            return preferred;
        };
        let depth = (-floor_level) as usize;
        let dungeons = self.dungeons.read().await;
        let Some(layout) = dungeons
            .get(&entrance.id)
            .and_then(|rt| rt.layouts.get(depth - 1))
        else {
            return preferred;
        };
        layout.walkable_drop_position(&entrance.position(), &monster_position, &preferred)
    }

    /// Where to put a player sealed into their own cell, or `None` when they
    /// are not sealed in. Underground `stands_on` is the carved floor: rock is
    /// sealed on every side too, and only a mover on the floor's own cells is
    /// walked out. The depth comes from the passability floor, never from the
    /// floor the client claims.
    pub(super) async fn sealed_player_escape(
        &self,
        position: &Position,
        floor: u8,
    ) -> Option<Position> {
        let depth = -floor_level_for_passability(floor);
        let entrance = (depth > 0)
            .then(|| self.dungeon_defs.entrance_at(position.x, position.z))
            .flatten();
        let dungeons = self.dungeons.read().await;
        let carved = entrance.and_then(|e| {
            let layout = dungeons
                .get(&e.id)
                .and_then(|rt| rt.layouts.get(depth as usize - 1))?;
            Some((layout, e.position()))
        });
        let cache = self.passability_read();
        super::passability::escape_from_sealed_cell(&cache, position, floor, |x, z| {
            carved.is_none_or(|(layout, entrance)| {
                let (cx, cz) = world_to_cell(&entrance, x, z);
                layout.is_carved(cx, cz)
            })
        })
    }

    /// Mark a dungeon monster's slot for respawn after it dies, and roll the
    /// floor key the monster may carry (returned for the killer's loot
    /// scatter). Called from the combat death path; `None` for non-dungeon
    /// monsters, bosses and locked floors.
    pub(super) async fn on_dungeon_monster_dead(
        &self,
        monster_id: &str,
        killer: &PlayerId,
    ) -> Option<String> {
        let entry = {
            let mut index = self.dungeon_monsters.write().await;
            index.remove(monster_id)
        };
        let entry = entry?;
        let now = Self::now_ms();

        let total = {
            let mut dungeons = self.dungeons.write().await;
            let rt = dungeons.get_mut(&entry.entrance_id)?;
            let total = rt.layouts.len() as u8;
            let slot = rt.floors.get_mut(&entry.depth)?.slots.get_mut(entry.slot)?;
            slot.alive_monster_id = None;
            if entry.is_boss {
                slot.respawn_at_ms = BOSS_RESPAWN_NEVER;
                info!(
                    "Guardian of '{}' fell at depth {}",
                    entry.entrance_id, entry.depth
                );
                return None;
            }
            slot.respawn_at_ms = now + MONSTER_RESPAWN_MS;
            total
        };
        self.roll_dungeon_key_drop(
            killer,
            &entry.entrance_id,
            entry.depth,
            total,
            KEY_DROP_CHANCE,
        )
        .await
    }

    /// The key a loot event at `depth` drops, if any: `chance` for the next
    /// locked floor's key, skipped when the finder already carries it or has
    /// opened this dungeon's chest tonight (the key would be dead weight).
    /// The dice go first — the candidate check takes three locks.
    async fn roll_dungeon_key_drop(
        &self,
        finder: &PlayerId,
        entrance_id: &str,
        depth: u8,
        total: u8,
        chance: f64,
    ) -> Option<String> {
        if !rand::thread_rng().gen_bool(chance) {
            return None;
        }
        self.dungeon_key_candidate(finder, entrance_id, depth, total)
            .await
    }

    async fn roll_prop_key_drop(
        &self,
        finder: &PlayerId,
        entrance_id: &str,
        depth: u8,
    ) -> Option<String> {
        if !rand::thread_rng().gen_bool(PROP_KEY_DROP_CHANCE) {
            return None;
        }
        let total = self.dungeons.read().await.get(entrance_id)?.layouts.len() as u8;
        self.dungeon_key_candidate(finder, entrance_id, depth, total)
            .await
    }

    /// The key a kill at `depth` is eligible to drop, before the chance roll.
    pub(super) async fn dungeon_key_candidate(
        &self,
        killer: &PlayerId,
        entrance_id: &str,
        depth: u8,
        total: u8,
    ) -> Option<String> {
        let entrance = self.dungeon_defs.get(entrance_id)?;
        let key = entrance.key_item_id(key_depth_for(depth, total)?);
        if self.holds_item(killer, &key).await {
            return None;
        }
        let character_id = self.character_id_of(killer).await?;
        let tonight = Self::night_epoch(self.current_total_game_seconds());
        let opened_tonight = Self::opened_since(
            self.chest_opens
                .read()
                .await
                .get(&(character_id, entrance_id.to_string())),
            tonight,
        );
        (!opened_tonight).then_some(key)
    }

    /// Validate a client-declared floor for the move leg `from`→`to`
    /// (`from == to` for a standalone floor change). A floor changes one
    /// storey at a time, on a short leg touching the shaft joining the two;
    /// anything else keeps the floor the leg started on. Y is never a reason
    /// to refuse — the verdict carries the ground height derived from the
    /// settled floor and `to`'s XZ.
    pub(super) async fn validated_dungeon_floor(
        &self,
        player_id: &PlayerId,
        current_floor: i8,
        requested_floor: i8,
        from: &Position,
        to: &Position,
    ) -> DungeonFloorVerdict {
        let keep = DungeonFloorVerdict {
            floor: current_floor,
            y: to.y,
        };
        let entrance = self
            .dungeon_defs
            .entrance_at(to.x, to.z)
            .or_else(|| self.dungeon_defs.entrance_at(from.x, from.z));
        let Some(entrance) = entrance else {
            if requested_floor < 0 {
                warn!(
                    "Player {} reported dungeon floor {} outside any dungeon footprint",
                    self.player_name_of(player_id).await,
                    requested_floor
                );
            }
            // Nothing underground to hold a mover outside every footprint.
            return DungeonFloorVerdict {
                floor: requested_floor.max(0),
                ..keep
            };
        };

        self.ensure_dungeon_runtime(&entrance.id).await;
        let dungeons = self.dungeons.read().await;
        let layouts = dungeons
            .get(&entrance.id)
            .map(|d| &d.layouts[..])
            .unwrap_or(&[]);

        if requested_floor != current_floor {
            let shallow = current_floor.max(requested_floor);
            let on_stairs = shallow <= 0
                && current_floor.min(requested_floor) == shallow - 1
                && from.dist_xz_sq(to) <= FLOOR_CHANGE_LEG_MAX * FLOOR_CHANGE_LEG_MAX
                && layouts
                    .get(shallow.unsigned_abs() as usize)
                    .is_some_and(|below| {
                        leg_touches_shaft(
                            &entrance.position(),
                            &below.up_shaft,
                            SHAFT_CHANGE_MARGIN,
                            (from.x, from.z),
                            (to.x, to.z),
                        )
                    });
            if !on_stairs {
                drop(dungeons);
                warn!(
                    "Player {} floor change {} -> {} off the stairs at ({:.1},{:.1}), kept floor {}",
                    self.player_name_of(player_id).await,
                    current_floor,
                    requested_floor,
                    to.x,
                    to.z,
                    current_floor
                );
                return keep;
            }
        }
        if requested_floor >= 0 {
            return DungeonFloorVerdict {
                floor: requested_floor,
                ..keep
            };
        }

        let depth = requested_floor.unsigned_abs();
        let expected_y = floor_height_at(&entrance.position(), layouts, depth, to.x, to.z);
        // Shaft ramps are geometry both floors share, so no Y there tells them
        // apart: Y is derived from (floor, XZ), never a veto. Off the ramps the
        // ground is flat, and a stored Y a whole floor away means the server's
        // own position drifted from the floor it holds — worth seeing.
        let drifted = expected_y.is_some_and(|y| (to.y - y).abs() > DUNGEON_FLOOR_HEIGHT)
            && !on_stair_shaft(&entrance.position(), layouts, depth, to.x, to.z);
        let total = layouts.len();
        drop(dungeons);
        let Some(expected_y) = expected_y else {
            warn!(
                "Player {} reported invalid dungeon depth {} (dungeon '{}' has {} floors)",
                player_id, depth, entrance.id, total
            );
            return keep;
        };
        if drifted {
            warn!(
                "Player {} floor {} Y drift: stored {:.1}, floor {:.1} at ({:.1},{:.1})",
                self.player_name_of(player_id).await,
                requested_floor,
                to.y,
                expected_y,
                to.x,
                to.z
            );
        }

        DungeonFloorVerdict {
            floor: requested_floor,
            y: expected_y,
        }
    }

    /// Infer the dungeon floor for an arbitrary position (used by debug
    /// teleports): if it lies in a dungeon footprint and its Y matches a
    /// floor's world Y, return that floor; otherwise 0 (surface). The only
    /// place a floor is read off a Y — everywhere else it is server state.
    pub(crate) async fn dungeon_floor_for_position(&self, position: &Position) -> i8 {
        /// How near a floor's world Y a position must sit to read as that floor.
        const FLOOR_Y_TOLERANCE: f32 = 2.5;
        let Some(entrance) = self.dungeon_defs.entrance_at(position.x, position.z) else {
            return 0;
        };
        self.ensure_dungeon_runtime(&entrance.id).await;
        let total = {
            let dungeons = self.dungeons.read().await;
            dungeons
                .get(&entrance.id)
                .map(|d| d.layouts.len())
                .unwrap_or(0)
        };
        for depth in 1..=total {
            let y = floor_world_y(entrance.y, depth as u8);
            if (position.y - y).abs() <= FLOOR_Y_TOLERANCE {
                return -(depth as i8);
            }
        }
        0
    }

    /// Called on login when the persisted floor_level is negative: verify
    /// the saved position still maps to a known dungeon and prime its
    /// runtime. Returns false when the dungeon no longer exists (caller
    /// should fall back to the world spawn).
    pub(crate) async fn rehydrate_dungeon_player(
        &self,
        player_id: &PlayerId,
        position: &Position,
        floor_level: i8,
    ) -> bool {
        let Some(entrance) = self.dungeon_defs.entrance_at(position.x, position.z) else {
            warn!(
                "Player {} saved at dungeon floor {} but no entrance covers ({:.1}, {:.1})",
                player_id, floor_level, position.x, position.z
            );
            return false;
        };
        self.ensure_dungeon_runtime(&entrance.id).await;
        let depth = (-floor_level) as usize;
        let valid = {
            let dungeons = self.dungeons.read().await;
            dungeons
                .get(&entrance.id)
                .is_some_and(|d| depth >= 1 && depth <= d.layouts.len())
        };
        if valid {
            info!(
                "Player {} rehydrated in dungeon '{}' at depth {}",
                self.player_name_of(player_id).await,
                entrance.id,
                depth
            );
        }
        valid
    }
}
