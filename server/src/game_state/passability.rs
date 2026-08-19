//! Server-side passability cache: the same shared `PassabilityCache` the
//! browser (wasm) and agent-client build, fed from the server's own data
//! (housing files, region objects, dungeon layouts). `tick_player_movement`
//! checks untrusted simulated steps against it so players can't walk
//! through walls. Dungeon interior doors seal their corridor mouth while
//! shut — `interior_doors` derives the same door list (and ids) the client
//! renders, and every toggle rebuilds the floor's cells.

use crate::terrain::io::TerrainIO;
use onlinerpg_shared::dungeon::{
    dungeon_cache_key, dungeon_passability, floor_cells, generate_dungeon_for, set_floor_cells,
};
use onlinerpg_shared::furniture::{self, FurniturePlacement};
use onlinerpg_shared::housing::HouseData;
use onlinerpg_shared::pathfinding;
use onlinerpg_shared::{WORLD_MAX_X, WORLD_MIN_X, WORLD_WIDTH_X};
use serde::Deserialize;
use tracing::{info, warn};

/// Region object file shape (`data/terrain/objects/r{rx}_{rz}.json`).
#[derive(Deserialize)]
struct RegionObjects {
    #[serde(default)]
    placements: Vec<FurniturePlacement>,
}

/// Query a short local movement sweep on both representations of the wrapped
/// X seam. The player's stored position is canonical, while a seam-crossing
/// step is deliberately left unwrapped so it remains a short segment. Shifting
/// that segment by one world width lets it see passability near the destination
/// edge as well as the source edge.
///
/// See `pathfinding::blocking_entry_for_mover` for why a sealed-in player is
/// let out.
pub(super) fn wrapped_block_info<'a>(
    cache: &'a pathfinding::PassabilityCache,
    from_x: f32,
    from_z: f32,
    to_x: f32,
    to_z: f32,
    floor_level: u8,
    y: f32,
) -> Option<pathfinding::BlockInfo<'a>> {
    let y = collision_y(cache, from_x, from_z, to_x, to_z, floor_level, y);
    wrapped_block_info_at(cache, from_x, from_z, to_x, to_z, floor_level, y)
}

/// The Y to test obstacles against, derived from the cache rather than taken
/// from the client.
///
/// `obstacle_reaches_y` stops applying an obstacle once the mover is above it,
/// and the storey is only ever inferred back from that same Y — so a client
/// reporting a hand's breadth over the wall tops walks through every wall and
/// table on the floor, and no amount of X/Z simulation catches it. A storey has
/// one real floor height: take it from the storey the swept leg crosses.
///
/// Two cases keep the reported Y: a leg crossing no floor at all — open
/// terrain, a bridge deck, where there is nothing to derive from and nothing to
/// collide with — and a climber mid-flight (`in_stairwell_span`).
///
/// The no-floor case is tested first because it is the common one, and it
/// settles the answer on its own.
fn collision_y(
    cache: &pathfinding::PassabilityCache,
    from_x: f32,
    from_z: f32,
    to_x: f32,
    to_z: f32,
    floor_level: u8,
    reported: f32,
) -> f32 {
    let Some(floor_y) =
        pathfinding::supporting_floor_y(cache, from_x, from_z, to_x, to_z, floor_level, reported)
    else {
        return reported;
    };
    if pathfinding::in_stairwell_span(cache, from_x, from_z, reported) {
        return reported;
    }
    floor_y
}

/// The seam sweep itself, at a Y the caller has already derived — so a step and
/// the slide candidates it falls back to are judged at one height.
fn wrapped_block_info_at<'a>(
    cache: &'a pathfinding::PassabilityCache,
    from_x: f32,
    from_z: f32,
    to_x: f32,
    to_z: f32,
    floor_level: u8,
    y: f32,
) -> Option<pathfinding::BlockInfo<'a>> {
    if let Some(info) = pathfinding::blocking_entry_for_mover(
        cache,
        from_x,
        from_z,
        to_x,
        to_z,
        floor_level,
        Some(y),
    ) {
        return Some(info);
    }

    let seam_offset = if to_x >= WORLD_MAX_X {
        -WORLD_WIDTH_X
    } else if to_x < WORLD_MIN_X {
        WORLD_WIDTH_X
    } else {
        return None;
    };
    pathfinding::blocking_entry_for_mover(
        cache,
        from_x + seam_offset,
        from_z,
        to_x + seam_offset,
        to_z,
        floor_level,
        Some(y),
    )
}

/// What the movement sim may do with one step.
pub(super) enum StepOutcome<'a> {
    Clear,
    /// Diagonal refused, one axis still open — the destination to take instead.
    Slid(f32, f32),
    Blocked(pathfinding::BlockInfo<'a>),
}

/// Resolve one step, falling back to a single axis when the diagonal is refused.
///
/// The axis fallback mirrors the client's `resolveWallSlide`
/// (`client/src/lib/components/player-control/fsm/movement-substrate.ts`),
/// including its `dx >= dz` tie-break. Without it the server refuses corner
/// grazes the client walks through, and the two simulations drift apart.
pub(super) fn resolve_step<'a>(
    cache: &'a pathfinding::PassabilityCache,
    from_x: f32,
    from_z: f32,
    to_x: f32,
    to_z: f32,
    floor_level: u8,
    y: f32,
) -> StepOutcome<'a> {
    const EPS: f32 = 1e-6;
    let y = collision_y(cache, from_x, from_z, to_x, to_z, floor_level, y);
    let Some(info) = wrapped_block_info_at(cache, from_x, from_z, to_x, to_z, floor_level, y)
    else {
        return StepOutcome::Clear;
    };
    let clear =
        |tx, tz| wrapped_block_info_at(cache, from_x, from_z, tx, tz, floor_level, y).is_none();
    let (dx, dz) = ((to_x - from_x).abs(), (to_z - from_z).abs());
    let x_ok = dx > EPS && clear(to_x, from_z);
    // Both axes open means only the exact diagonal grazed a corner tip; keep the
    // one making more progress. Testing X first lets the common case skip the
    // second sweep, which scans the whole cache.
    if x_ok && dx >= dz {
        return StepOutcome::Slid(to_x, from_z);
    }
    if dz > EPS && clear(from_x, to_z) {
        return StepOutcome::Slid(from_x, to_z);
    }
    if x_ok {
        return StepOutcome::Slid(to_x, from_z);
    }
    StepOutcome::Blocked(info)
}

/// Whether a mover in this cell has no legal step in any direction.
///
/// Not [`pathfinding::is_cell_sealed`]: that one reads the mask alone, so it
/// also fires on the furniture parked on a standing player, whom
/// `blocking_entry_for_mover` deliberately lets walk out. Asking the same way
/// the mover's own steps are judged keeps that case out.
pub(super) fn sealed_in(
    cache: &pathfinding::PassabilityCache,
    position: &crate::types::Position,
    floor_level: u8,
) -> bool {
    let (cx, cz) = (position.x.floor() + 0.5, position.z.floor() + 0.5);
    pathfinding::DIRS.iter().all(|&(dx, dz)| {
        pathfinding::is_movement_blocked_for_mover(
            cache,
            cx,
            cz,
            cx + dx as f32,
            cz + dz as f32,
            floor_level,
            Some(position.y),
        )
    })
}

/// Where to put a mover [`sealed_in`] its own cell, or `None` when every
/// neighbour is sealed too — a position wrong in a way no nudge repairs.
///
/// Only furniture yields to a trapped mover; a dungeon's own entry holds its
/// walls, so it cannot without becoming a wall hack. That leaves a player
/// standing where a broken crate used to be when the server restarted — the
/// runtime's broken-prop set is memory only — inside a solid pillar with no
/// legal step in any direction, and what seals a cell that way is a 1x1 pillar
/// or a shut door, so the way out is the adjoining cell.
///
/// `stands_on` keeps the nudge on the level's own cells — and keeps it from
/// becoming a lift: rock is sealed on every side, so a mover put there by a
/// floor change reads as trapped too, and the carved cell next door may be
/// another floor's room. Callers who know the floor's shape say so.
pub(super) fn escape_from_sealed_cell(
    cache: &pathfinding::PassabilityCache,
    position: &crate::types::Position,
    floor_level: u8,
    stands_on: impl Fn(f32, f32) -> bool,
) -> Option<crate::types::Position> {
    if !stands_on(position.x, position.z) || !sealed_in(cache, position, floor_level) {
        return None;
    }
    let (cx, cz) = (position.x.floor() + 0.5, position.z.floor() + 0.5);
    pathfinding::DIRS
        .iter()
        .map(|&(dx, dz)| (cx + dx as f32, cz + dz as f32))
        .find(|&(x, z)| {
            stands_on(x, z)
                && !pathfinding::is_cell_sealed(cache, x, z, floor_level, Some(position.y))
        })
        .map(|(x, z)| crate::types::Position {
            x: onlinerpg_shared::wrap_world_x(x),
            y: pathfinding::get_floor_y_base(cache, x, z, floor_level).unwrap_or(position.y),
            z,
        })
}

/// Cache floor index for a player, derived from the server's own position
/// rather than the floor the client reported.
///
/// Underground and in a house the position is server-owned in all three axes
/// (floor changes only on stairs, Y from the storey's own height — see
/// `surface_ground_y`), so this agrees with the validated floor.
pub(super) fn authoritative_floor(
    cache: &pathfinding::PassabilityCache,
    position: &crate::types::Position,
) -> u8 {
    pathfinding::get_floor_at_position(cache, position.x, position.z, position.y)
}

impl super::GameState {
    /// Surface counterpart of `validated_dungeon_floor`: a storey changes one
    /// at a time, on a short leg touching the stairwell joining the two
    /// (`from == to` for a standalone change). Also accepts dropping to the
    /// ground where the held storey has no floor under the mover at all (the
    /// room was edited away). Otherwise keeps the leg's starting floor.
    /// Official NPCs walk their schedules across storeys unvalidated, as they
    /// do through walls.
    pub(super) async fn validated_house_floor(
        &self,
        player_id: &crate::types::PlayerId,
        current_floor: i8,
        requested_floor: i8,
        from: &crate::types::Position,
        to: &crate::types::Position,
        is_official_npc: bool,
    ) -> i8 {
        if requested_floor == current_floor || is_official_npc {
            return requested_floor;
        }
        let accepted = {
            let cache = self.passability_read();
            let lower = current_floor.min(requested_floor) as u8;
            let upper = current_floor.max(requested_floor) as u8;
            let to_x = from.x + onlinerpg_shared::shortest_world_delta_x(from.x, to.x);
            (upper - lower == 1
                && pathfinding::leg_touches_stairwell(
                    &cache,
                    lower,
                    upper,
                    (from.x, from.z),
                    (to_x, to.z),
                ))
                || (requested_floor == 0
                    && pathfinding::storey_ground_y(
                        &cache,
                        current_floor as u8,
                        from.x,
                        from.z,
                        from.y,
                    )
                    .is_none())
        };
        if accepted {
            return requested_floor;
        }
        warn!(
            "Player {} storey change {} -> {} off the stairs at ({:.1},{:.1}), kept floor {}",
            self.player_name_of(player_id).await,
            current_floor,
            requested_floor,
            to.x,
            to.z,
            current_floor
        );
        current_floor
    }

    /// The Y to store for a mover on surface floor `floor` at `to`: the
    /// dungeon entrance ramp, house storey or stairwell the server can model.
    /// Open terrain keeps the reported Y — the server holds no bridge decks,
    /// and nothing server-side consumes Y where no floor grid stands.
    pub(super) async fn surface_ground_y(&self, floor: u8, to: &crate::types::Position) -> f32 {
        if floor == 0 {
            if let Some(entrance) = self.dungeon_defs.entrance_at(to.x, to.z) {
                let dungeons = self.dungeons.read().await;
                let ramp_y = dungeons.get(&entrance.id).and_then(|rt| {
                    onlinerpg_shared::dungeon::ground_y_for_floor(
                        &entrance.position(),
                        &rt.layouts,
                        floor,
                        to.x,
                        to.z,
                    )
                });
                if let Some(y) = ramp_y {
                    return y;
                }
            }
        }
        let cache = self.passability_read();
        pathfinding::storey_ground_y(&cache, floor, to.x, to.z, to.y).unwrap_or(to.y)
    }

    /// Cache guards recover from poisoning: a panic mid-update at worst
    /// leaves one stale entry, which must not take down the movement tick.
    pub(super) fn passability_read(
        &self,
    ) -> std::sync::RwLockReadGuard<'_, pathfinding::PassabilityCache> {
        self.passability.read().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn passability_write(
        &self,
    ) -> std::sync::RwLockWriteGuard<'_, pathfinding::PassabilityCache> {
        self.passability.write().unwrap_or_else(|e| e.into_inner())
    }

    /// Build the boot-time cache: every house, every region's solid
    /// furniture and every dungeon layout. Read failures propagate — an entry
    /// silently missing from this cache is a wall players can walk through.
    pub async fn init_passability(&self, terrain_io: &TerrainIO) -> std::io::Result<()> {
        let houses = self.housing_io.read_all_houses().await?;
        for house in &houses {
            self.passability_add_house(house).await;
        }
        let regions = self.load_region_furniture(terrain_io).await?;
        let mut dungeons = 0usize;
        for def in self.dungeon_defs.all() {
            let layouts = generate_dungeon_for(&def.id);
            let rp = dungeon_passability(&def.position(), &layouts);
            self.passability_write()
                .insert(dungeon_cache_key(&def.id), rp);
            dungeons += 1;
        }
        // Counts are cache entries, not files scanned: most region files hold
        // only decorative objects and seal nothing.
        info!(
            "Passability cache ready: {} entries ({} houses, {} furniture regions, {} dungeons)",
            houses.len() + regions + dungeons,
            houses.len(),
            regions,
            dungeons
        );
        Ok(())
    }

    async fn load_region_furniture(&self, terrain_io: &TerrainIO) -> std::io::Result<usize> {
        let mut count = 0;
        for (rx, rz) in terrain_io.list_object_regions().await? {
            let json = terrain_io.read_object(rx, rz).await?;
            let objs = serde_json::from_value::<RegionObjects>(json).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("bad region objects r{rx:+03}_{rz:+03}: {e}"),
                )
            })?;
            if self.sync_region_furniture(rx, rz, &objs.placements) {
                count += 1;
            }
        }
        Ok(count)
    }

    /// Insert or replace a house's cache entry: base grids plus the door
    /// overlays persisted in its data. The in-memory open-door state for the
    /// house is reset — the incoming data is authoritative after an edit.
    pub async fn passability_add_house(&self, house: &HouseData) {
        self.clear_open_doors_for_house(&house.id).await;
        let rp = pathfinding::build_runtime_passability(house);
        let mut cache = self.passability_write();
        cache.insert(house.id.clone(), rp);
        pathfinding::apply_door_overlays(&mut cache, house);
    }

    pub async fn passability_remove_house(&self, house_id: &str) {
        self.clear_open_doors_for_house(house_id).await;
        self.passability_write().remove(house_id);
    }

    /// Mirror of the client's `passability_set_furniture` for one region:
    /// solid placements become sealed cells, empty regions clear the entry.
    /// Returns whether the region left an entry behind — most regions hold only
    /// decorative objects and contribute nothing.
    pub fn sync_region_furniture(
        &self,
        rx: i32,
        rz: i32,
        placements: &[FurniturePlacement],
    ) -> bool {
        let key = furniture::region_cache_key(rx, rz);
        let mut cache = self.passability_write();
        match furniture::build_furniture_passability_for_placements(placements) {
            Some(rp) => {
                cache.insert(key, rp);
                true
            }
            None => {
                cache.remove(&key);
                false
            }
        }
    }

    /// Validate the map editor's region-object payload before it is persisted,
    /// returning only the fields needed by collision caching.
    pub(crate) fn parse_region_furniture(
        body: &serde_json::Value,
    ) -> Result<Vec<FurniturePlacement>, serde_json::Error> {
        RegionObjects::deserialize(body).map(|objects| objects.placements)
    }

    /// Re-derive one dungeon floor's cells from its current dynamic state
    /// (shared `dungeon::floor_cells`); this is the adapter that hands it the
    /// server's own live door/prop state.
    ///
    /// The cells are computed before the passability write lock is taken:
    /// `tick_player_movement` holds that lock read-side for every moving
    /// player, so a 6400-cell rebuild under it would stall the whole tick.
    pub(super) async fn rebuild_dungeon_floor_passability(&self, entrance_id: &str, depth: u8) {
        let cells = {
            let dungeons = self.dungeons.read().await;
            let Some(rt) = dungeons.get(entrance_id) else {
                return;
            };
            let broken: Vec<u32> = rt
                .broken_props
                .get(&depth)
                .map(|s| s.iter().copied().collect())
                .unwrap_or_default();
            floor_cells(&rt.layouts, depth, &broken, rt.open_doors.get(&depth))
        };
        let Some(cells) = cells else {
            return;
        };
        set_floor_cells(&mut self.passability_write(), entrance_id, depth, cells);
    }
}

#[cfg(test)]
mod tests {
    use super::collision_y;
    use onlinerpg_shared::pathfinding::{
        PassabilityCache, RuntimeFloorGrid, RuntimePassability, StairwellInfo,
    };

    /// One storey at y_base 0 over cells x 0..2, z 0..4, with a stairwell up to
    /// a second storey at 3.1 occupying x 0..1.
    fn two_storey_cache() -> PassabilityCache {
        let grid = |floor_level, y_base| RuntimeFloorGrid {
            floor_level,
            origin_x: 0,
            origin_z: 0,
            width: 2,
            depth: 4,
            y_base,
            wall_height: 3.0,
            cells: vec![0u8; 8],
        };
        let mut cache = PassabilityCache::new();
        cache.insert(
            "house".to_string(),
            RuntimePassability {
                house_origin_x: 0.0,
                house_origin_z: 0.0,
                min_x: 0.0,
                max_x: 2.0,
                min_z: 0.0,
                max_z: 4.0,
                floors: vec![grid(0, 0.0), grid(1, 3.1)],
                stairwells: vec![StairwellInfo {
                    local_min_x: 0,
                    local_min_z: 0,
                    local_max_x: 1,
                    local_max_z: 4,
                    lower_floor: 0,
                    upper_floor: 1,
                    along_z: true,
                    reversed: false,
                }],
                yields_to_trapped_mover: false,
                is_ground: true,
            },
        );
        cache
    }

    #[test]
    fn a_forged_height_is_pulled_down_to_the_storey_it_claims() {
        let cache = two_storey_cache();
        // Off the stairwell (x 1.5), so nothing holds the mover up: both a
        // hand's breadth over the wall tops and an absurd claim collapse to
        // the storey's own floor height.
        assert_eq!(collision_y(&cache, 1.5, 0.5, 1.5, 1.5, 0, 3.5), 0.0);
        assert_eq!(collision_y(&cache, 1.5, 0.5, 1.5, 1.5, 0, 1000.0), 0.0);
        // Keyed to the upper storey, it is held to that one instead.
        assert_eq!(collision_y(&cache, 1.5, 0.5, 1.5, 1.5, 1, 1000.0), 3.1);
        // An honest Y survives unchanged.
        assert_eq!(collision_y(&cache, 1.5, 0.5, 1.5, 1.5, 0, 0.0), 0.0);
    }

    /// The exemption. Without it a climber is pulled down to the storey it is
    /// keyed to and the tables under the stairs block it.
    #[test]
    fn a_climber_mid_flight_keeps_its_reported_height() {
        let cache = two_storey_cache();
        assert_eq!(collision_y(&cache, 0.5, 0.5, 0.5, 1.5, 0, 2.0), 2.0);
        // Above the flight entirely: no flight to be on.
        assert_eq!(collision_y(&cache, 0.5, 0.5, 0.5, 1.5, 0, 1000.0), 0.0);
    }

    #[test]
    fn a_leg_crossing_no_floor_keeps_its_reported_height() {
        let cache = two_storey_cache();
        assert_eq!(collision_y(&cache, 50.0, 50.0, 51.0, 50.0, 0, 7.0), 7.0);
    }
}
