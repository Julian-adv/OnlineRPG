//! Server-side dungeon runtime. Layouts are regenerated deterministically
//! from the entrance id (same shared-crate generator the client runs via
//! wasm), so the runtime holds only live state: cached layouts and spawn
//! slots. That state is in-memory — after a restart, reconnecting players
//! rehydrate from the generator plus their persisted position/floor_level.
//! The one exception is the treasure-chest claim, which is DB-backed
//! (`GameState::chest_opens`) because a lost claim is free loot.

use std::collections::{HashMap, HashSet};

use onlinerpg_shared::dungeon::{
    cell_center, dungeon_origin, floor_world_y, generate_dungeon_for, interior_doors,
    monster_level_for_depth, FloorLayout, PropKind, ENTRANCE_DOOR_ID, FLOOR_Y_TOLERANCE,
};
use onlinerpg_shared::inventory::GroundItem;
use onlinerpg_shared::{Position, ServerMessage};
use rand::Rng;
use tracing::{info, warn};

use crate::types::PlayerId;

use super::GameState;

const MONSTER_RESPAWN_MS: u64 = 5 * 60 * 1000;
const BOSS_RESPAWN_MS: u64 = 30 * 60 * 1000;
/// Retry delay when a spawn attempt failed (e.g. global monster cap).
const SPAWN_RETRY_MS: u64 = 10 * 1000;
const CHEST_INTERACT_RANGE: f32 = 2.5;
const CHEST_ITEM_MIN_PRICE: i64 = 2000;
/// How close a player must stand to a prop to break (barrel/crate) or open
/// (chest) it.
const PROP_INTERACT_RANGE: f32 = 2.5;
/// How close a player must stand to a doorway to work it; the client gates the
/// click at 2m from the leaf it hit. `toggle_dungeon_door` adds half the
/// opening on top, the reach of a leaf standing open.
const DOOR_INTERACT_RANGE: f32 = 2.5;
/// Chance that a freshly-broken barrel/crate spills a loose coin pile.
const BROKEN_PROP_COIN_DROP_CHANCE: f64 = 0.20;

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
}

pub(super) struct FloorRuntime {
    /// One slot per layout SpawnSpec, same order.
    pub slots: Vec<SpawnSlot>,
    pub players: HashSet<PlayerId>,
    /// Whether this floor's boss has been killed and not yet respawned.
    /// Tracked explicitly rather than inferred from "no boss slot is alive":
    /// a slot also reads as empty while a spawn is pending or was refused by
    /// the global monster cap, which would otherwise open the chest gate for
    /// a boss nobody fought.
    pub boss_defeated: bool,
}

pub(super) struct SpawnSlot {
    pub alive_monster_id: Option<String>,
    pub respawn_at_ms: u64,
    pub is_boss: bool,
}

/// Reverse index entry: which dungeon slot a live monster belongs to.
pub(super) struct DungeonMonsterRef {
    pub entrance_id: String,
    pub depth: u8,
    pub slot: usize,
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
            });
    }

    /// Toggle a dungeon door's open state and return the new state: the server
    /// flips the stored state, reseals the floor's passability (a shut interior
    /// door blocks movement and pathing) and lets the connection layer
    /// broadcast. Interior door ids map to corridor-mouth segments via
    /// `interior_doors`.
    ///
    /// The door is shared state, so nothing changes until the request is
    /// authorized against the floor, the door, and the reach to it.
    pub async fn toggle_dungeon_door(
        &self,
        player_id: &PlayerId,
        entrance_id: &str,
        depth: u8,
        door_id: u32,
    ) -> Option<bool> {
        let entrance = self.dungeon_defs.get(entrance_id)?;
        let expected_floor = -i8::try_from(depth).ok()?;
        let (player_pos, player_floor) = {
            let players = self.players.read().await;
            let player = players.get(player_id)?;
            (player.position, player.floor_level)
        };
        if player_floor != expected_floor {
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
            if door_line_dist_sq(&entrance.position(), door.seg(), &player_pos) > reach * reach {
                return None;
            }
        }

        let is_open = {
            let mut dungeons = self.dungeons.write().await;
            let rt = dungeons.get_mut(entrance_id)?;
            let set = rt.open_doors.entry(depth).or_default();
            if set.remove(&door_id) {
                false
            } else {
                set.insert(door_id);
                true
            }
        };
        self.rebuild_dungeon_floor_passability(entrance_id, depth)
            .await;
        Some(is_open)
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
        if chest_opens
            .get(&key)
            .is_some_and(|&opened| Self::night_epoch(opened) >= tonight)
        {
            return false;
        }
        chest_opens.retain(|_, &mut opened| Self::night_epoch(opened) >= tonight);
        chest_opens.insert(key, now_seconds);
        true
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

    /// Open the final-floor treasure chest: requires standing next to it on
    /// the last floor with the boss dead, and one open per character per
    /// night. Loot (2–3 equipment rolls + depth-scaled gold) goes straight to
    /// the opener; the open is broadcast nearby.
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

        let position_check = {
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

            if player_floor != -(total as i8) {
                Some("You must be on the deepest floor")
            } else {
                let chest_pos = cell_center(&entrance.position(), total, chest);
                let dx = onlinerpg_shared::shortest_world_delta_x(chest_pos.x, player_pos.x);
                let dz = player_pos.z - chest_pos.z;
                if dx * dx + dz * dz > CHEST_INTERACT_RANGE * CHEST_INTERACT_RANGE {
                    Some("Too far from the chest")
                } else if !rt.floors.get(&total).is_some_and(|fr| fr.boss_defeated) {
                    Some("The guardian still lives")
                } else {
                    None
                }
            }
        };

        let now_seconds = self.current_total_game_seconds();
        let chest_check = if position_check.is_some() {
            position_check
        } else if self
            .claim_chest_open(character_id, entrance_id, now_seconds)
            .await
        {
            None
        } else {
            Some("The chest is empty (it refills at nightfall)")
        };
        if let Some(reason) = chest_check {
            self.send_direct_message(
                player_id,
                ServerMessage::InteractionRejected {
                    reason: reason.to_string(),
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

        // Roll loot: 2–3 distinct equipment items priced for endgame.
        let depth = {
            let dungeons = self.dungeons.read().await;
            dungeons
                .get(entrance_id)
                .map(|rt| rt.layouts.len() as i64)
                .unwrap_or(5)
        };
        let (item_def_ids, gold) = {
            use rand::seq::SliceRandom;
            use rand::Rng;
            let mut rng = rand::thread_rng();
            let mut pool = self
                .item_defs
                .equipment_ids_with_min_price(CHEST_ITEM_MIN_PRICE);
            // The guaranteed signature drops lead the list; keep the random
            // rolls distinct from them.
            pool.retain(|id| !entrance.chest_drops.contains(id));
            pool.shuffle(&mut rng);
            let count = rng.gen_range(2..=3).min(pool.len());
            let mut items = entrance.chest_drops.clone();
            items.extend(pool.into_iter().take(count));
            let gold = rng.gen_range(depth * 500..=depth * 1500);
            (items, gold)
        };

        for item_def_id in &item_def_ids {
            // give_item falls back to an inventory error when over carry
            // weight; the player keeps what fits.
            self.give_item(player_id, item_def_id).await;
        }
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

        // Rare bonus world drops land as ground items next to the opener,
        // on top of the equipment/gold that goes straight to their bags.
        self.spawn_world_drops(player_pos, player_floor).await;
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
            // Rare bonus world drops, independent of the coin roll.
            self.spawn_world_drops(prop_pos, -(depth as i8)).await;
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
            // Rare bonus world drops, in addition to the coin pile.
            self.spawn_world_drops(chest_pos, -(depth as i8)).await;
        }
    }

    async fn spawn_dungeon_coin_pile(&self, position: Position, floor_level: i8) {
        let instance_id = self.next_instance_id().await;
        self.spawn_ground_item(
            GroundItem {
                instance_id,
                item_def_id: super::COIN_PILE_ITEM_ID.to_string(),
                position,
                floor_level,
                enchant: 0,
            },
            None,
        )
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
                    .map(|(depth, floor)| (*depth, floor.players.iter().cloned().collect()))
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
                    players: HashSet::new(),
                    boss_defeated: false,
                })
                .players
                .insert(*player_id);
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
                    Some(level),
                    aggressive,
                )
                .await;

            let mut dungeons = self.dungeons.write().await;
            let slot = if let Some(fr) = dungeons
                .get_mut(entrance_id)
                .and_then(|rt| rt.floors.get_mut(&depth))
            {
                if spawned.is_some() && fr.slots.get(slot_idx).is_some_and(|s| s.is_boss) {
                    // The guardian walks again; the chest locks with it.
                    fr.boss_defeated = false;
                }
                fr.slots.get_mut(slot_idx)
            } else {
                None
            };
            match (slot, spawned) {
                (Some(slot), Some(monster)) => {
                    slot.alive_monster_id = Some(monster.id.clone());
                    drop(dungeons);
                    let mut index = self.dungeon_monsters.write().await;
                    index.insert(
                        monster.id.clone(),
                        DungeonMonsterRef {
                            entrance_id: entrance_id.to_string(),
                            depth,
                            slot: slot_idx,
                        },
                    );
                    drop(index);
                    self.send_direct_message(owner, ServerMessage::MonsterAssigned { monster })
                        .await;
                }
                (Some(slot), None) => {
                    slot.alive_monster_id = None;
                    slot.respawn_at_ms = now + SPAWN_RETRY_MS;
                }
                _ => {}
            }
        }
    }

    async fn leave_dungeon_floor(&self, player_id: &PlayerId, entrance_id: &str, depth: u8) {
        // Occupancy + alive-monster snapshot under one lock.
        let (remaining_owner, alive_ids, floor_emptied) = {
            let mut dungeons = self.dungeons.write().await;
            let Some(fr) = dungeons
                .get_mut(entrance_id)
                .and_then(|rt| rt.floors.get_mut(&depth))
            else {
                return;
            };
            fr.players.remove(player_id);
            let remaining = fr.players.iter().next().cloned();
            let alive: Vec<String> = fr
                .slots
                .iter()
                .filter_map(|s| s.alive_monster_id.clone())
                .filter(|id| !id.is_empty())
                .collect();
            if remaining.is_none() {
                for slot in fr.slots.iter_mut() {
                    slot.alive_monster_id = None;
                    // Empty floors repopulate instantly on next entry.
                    slot.respawn_at_ms = 0;
                }
                // The boss comes back with them, so its kill no longer counts.
                fr.boss_defeated = false;
            }
            (remaining, alive, false)
        };
        let _ = floor_emptied;

        match remaining_owner {
            Some(new_owner) => {
                // Hand the leaver's monsters to a player still on the floor.
                let reassigned: Vec<crate::types::Monster> = {
                    let mut monsters = self.monsters.write().await;
                    let mut out = Vec::new();
                    for id in &alive_ids {
                        if let Some(m) = monsters.get_mut(id) {
                            if m.owner_id.as_ref() == Some(player_id) {
                                m.owner_id = Some(new_owner);
                                out.push(m.clone());
                            }
                        }
                    }
                    out
                };
                for monster in reassigned {
                    info!("Dungeon monster {} reassigned to {}", monster.id, new_owner);
                    self.send_direct_message(
                        &new_owner,
                        ServerMessage::MonsterAssigned { monster },
                    )
                    .await;
                }
            }
            None => {
                // Floor emptied: despawn everything (only monsters respawn
                // in a shared dungeon — and this bounds live monster count).
                let removed: Vec<crate::types::Monster> = {
                    let mut monsters = self.monsters.write().await;
                    alive_ids
                        .iter()
                        .filter_map(|id| monsters.remove(id))
                        .collect()
                };
                {
                    let mut index = self.dungeon_monsters.write().await;
                    for id in &alive_ids {
                        index.remove(id);
                    }
                }
                for monster in removed {
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
                        fr.players.iter().next().map(|p| (id.clone(), *depth, *p))
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

    /// Mark a dungeon monster's slot for respawn after it dies. Called
    /// from the combat death path; no-op for non-dungeon monsters.
    pub(super) async fn on_dungeon_monster_dead(&self, monster_id: &str) {
        let entry = {
            let mut index = self.dungeon_monsters.write().await;
            index.remove(monster_id)
        };
        let Some(entry) = entry else { return };
        let now = Self::now_ms();
        let mut dungeons = self.dungeons.write().await;
        if let Some(fr) = dungeons
            .get_mut(&entry.entrance_id)
            .and_then(|rt| rt.floors.get_mut(&entry.depth))
        {
            if let Some(slot) = fr.slots.get_mut(entry.slot) {
                slot.alive_monster_id = None;
                if slot.is_boss {
                    slot.respawn_at_ms = now + BOSS_RESPAWN_MS;
                    fr.boss_defeated = true;
                } else {
                    slot.respawn_at_ms = now + MONSTER_RESPAWN_MS;
                }
            }
        }
    }

    /// Validate a client-reported floor change into/inside/out of a
    /// dungeon. Returns the floor to actually store: the requested floor
    /// when plausible, otherwise the player's current floor.
    ///
    /// Movement here mirrors the codebase's trust model (terrain
    /// collision is client-side): we only require that the position lies
    /// inside a known dungeon footprint and that the reported Y matches
    /// the floor's world Y, which is what walking the stair shafts
    /// produces naturally.
    pub(super) async fn validated_dungeon_floor(
        &self,
        player_id: &PlayerId,
        current_floor: i8,
        requested_floor: i8,
        position: &Position,
    ) -> i8 {
        if requested_floor >= 0 {
            return requested_floor;
        }

        let Some(entrance) = self.dungeon_defs.entrance_at(position.x, position.z) else {
            warn!(
                "Player {} reported dungeon floor {} outside any dungeon footprint",
                self.player_name_of(player_id).await,
                requested_floor
            );
            return current_floor.max(0);
        };

        self.ensure_dungeon_runtime(&entrance.id).await;
        let depth = (-requested_floor) as usize;
        let total = {
            let dungeons = self.dungeons.read().await;
            dungeons
                .get(&entrance.id)
                .map(|d| d.layouts.len())
                .unwrap_or(0)
        };
        if depth == 0 || depth > total {
            warn!(
                "Player {} reported invalid dungeon depth {} (dungeon '{}' has {} floors)",
                player_id, depth, entrance.id, total
            );
            return current_floor;
        }

        let expected_y = floor_world_y(entrance.y, depth as u8);
        if (position.y - expected_y).abs() > FLOOR_Y_TOLERANCE {
            warn!(
                "Player {} floor {} Y mismatch: reported {:.1}, expected {:.1}",
                self.player_name_of(player_id).await,
                requested_floor,
                position.y,
                expected_y
            );
            return current_floor;
        }

        requested_floor
    }

    /// Infer the dungeon floor for an arbitrary position (used by debug
    /// teleports): if it lies in a dungeon footprint and its Y matches a
    /// floor's world Y, return that floor; otherwise 0 (surface).
    pub(crate) async fn dungeon_floor_for_position(&self, position: &Position) -> i8 {
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
