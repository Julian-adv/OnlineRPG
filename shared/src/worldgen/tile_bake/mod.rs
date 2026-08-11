//! Phase 7: high-resolution tile baking.
//!
//! Consumes the low-resolution `GlobalMap` (elevation + land mask), the
//! flow/river map from Phase 4, and the road network from Phase 6, and
//! produces per-tile binary artifacts that match the runtime
//! `terrain::TerrainIO` layout:
//!
//! * 65×65 uint16 heightmap (`defaults::HEIGHTMAP_SIZE = 8,450` bytes)
//! * 64×64×4 byte V2 splatmap (`defaults::SPLATMAP_SIZE = 16,384` bytes)
//!
//! The global map lives at `meters_per_cell` m/cell (typically 8). Tile
//! vertices are 1 m apart, so each vertex sample is a bilinear interpolation
//! of 2×2 global cells plus a high-frequency detail noise term. Rivers and
//! roads are handled as world-space polylines (Chaikin-smoothed) and queried
//! by point-to-segment distance during bake. The splat layer uses a fixed
//! five-slot region palette; primary/secondary indices and the `blend` byte
//! are chosen per-cell by a priority ladder (river > road > sea > cliff >
//! alpine > slope > coast > plain). River bed wins over road so road→river
//! crossings keep their wet-pebble texture for a future bridge mesh; cliff
//! wins over alpine so a vertical marble face on a snowy peak still reads
//! as bare rock.

pub mod bridges;
mod constants;
mod context;
mod heightmap;
mod river_field;
pub mod river_geom;
pub mod settlement_flatten;
mod splatmap;
mod water_field;

use serde::{Deserialize, Serialize};

use super::global_map::GlobalMap;
use super::vector_features::{
    river_segments_near_tile_wrap_x, segments_near_tile, segments_near_tile_wrap_x,
};

pub use constants::{
    HEIGHT_BIAS, HEIGHT_STEP, PAL_CLIFF, PAL_DIRT, PAL_GROUND, PAL_PAVING, PAL_RIVER_BED, PAL_ROAD,
    PAL_SAND, PAL_SNOW, PAL_STONE_PATH, RIVER_MAX_WIDTH_M, RIVER_MIN_WIDTH_M, TILE_DIM,
    VERTS_PER_SIDE,
};
pub use context::BakeContext;
pub use river_field::bake_river_field;
pub use water_field::{
    bake_water_field, WATER_FIELD_BIN_MAGIC, WATER_FIELD_HEADER_SIZE, WATER_FIELD_PIXEL_SIZE,
    WATER_FIELD_TOTAL_SIZE,
};

/// Decomposed height-sample result for one world point. Each field is one
/// step of `sample_elevation_m` / `carve_at_point`, surfaced so the
/// `terrain-gen probe-point` CLI can show how a vertex got to its final
/// value.
#[derive(Debug, Clone, Copy)]
pub struct PointProbe {
    pub world_x: f32,
    pub world_z: f32,
    pub global_cell: (i32, i32),
    pub land_mask: u8,
    pub dist_to_land: u16,
    pub natural_height: f32,
    pub final_height: f32,
    pub river: Option<NearestRiver>,
}

#[derive(Debug, Clone, Copy)]
pub struct NearestRiver {
    pub seg_idx: usize,
    pub t: f32,
    pub d_m: f32,
    pub signed_d_m: f32,
    pub width: f32,
    pub flow_norm: f32,
    pub half_width: f32,
    pub taper: f32,
    pub depth_uncapped: f32,
    pub bed_floor: f32,
    pub max_carve_depth: f32,
    pub carve: f32,
}

/// Compute a `PointProbe` for `(wx, wz)`, matching what
/// `apply_river_carve_to_tile` would have written to the tile heightmap.
pub fn probe_point(map: &GlobalMap, ctx: &BakeContext, wx: f32, wz: f32) -> PointProbe {
    heightmap::probe_point_impl(map, ctx, wx, wz)
}

use constants::{
    COAST_FADE_SPAN_M, COAST_HEIGHT_BLEND_M, COAST_SAND_M, RIVER_CARVE_TAPER_EXTRA_M,
    RIVER_CARVE_TAPER_MIN_M, RIVER_FADE_SPAN_M, RIVER_MOUTH_FAN_EXTRA, RIVER_SAND_WIDTH_MULT,
    ROAD_FADE_SPAN_M, ROAD_HALF_WIDTH_M,
};
use heightmap::{apply_river_carve_to_tile, encode_heightmap, sample_tile_heights_no_carve};
use splatmap::bake_splatmap;

/// Largest radius at which any river segment can still affect a tile vertex
/// (carve taper or splat fade). Computed from the global maxima so every
/// caller — bake, splat, diagnostic probes — agrees on which segments to
/// pull in around a given point. The fan flare at the mouth pushes the
/// effective half-width to `MAX_WIDTH * (1 + FAN_EXTRA)`; clipping the
/// margin to the natural max would drop adjacent-tile segments inside
/// wide wedges and leave seams in the visible bank.
#[inline]
pub fn river_margin_m() -> f32 {
    let max_half_width = RIVER_MAX_WIDTH_M * (1.0 + RIVER_MOUTH_FAN_EXTRA) * 0.5;
    let max_taper = RIVER_CARVE_TAPER_MIN_M + RIVER_CARVE_TAPER_EXTRA_M;
    let max_sand_half_width = RIVER_MAX_WIDTH_M * RIVER_SAND_WIDTH_MULT;
    (max_half_width + max_taper).max(max_sand_half_width + RIVER_FADE_SPAN_M)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BakedTile {
    /// Row-major uint16 heightmap (little-endian), 65×65 × 2 bytes.
    pub heightmap: Vec<u8>,
    /// Row-major V2 splatmap, 64×64 × 4 bytes.
    pub splatmap: Vec<u8>,
    /// Per-tile RFD1 river field (surfaceY + flowDir, 65×65). `None`
    /// when the tile sees no river segment within the bake margin —
    /// runtime treats missing as "no river quad in this tile".
    /// Legacy — kept during the WFD1 rollout for old clients.
    pub river_field: Option<Vec<u8>>,
    /// Per-tile WFD1 unified water field (surfaceY + flow + riverness,
    /// 65×65). `None` when the tile sees no river segment — runtime
    /// synthesizes a flat sea field for sea-only tiles.
    pub water_field: Option<Vec<u8>>,
}

/// Bake one tile at signed tile coordinate (tx, tz).
pub fn bake_tile(map: &GlobalMap, ctx: &BakeContext, tx: i32, tz: i32) -> BakedTile {
    bake_tile_with_bridges(map, ctx, tx, tz, &[], &[])
}

/// Like `bake_tile` but applies per-tile heightmap flatten passes — bridge
/// decks (rotated rect) and settlements (circular pads). The splatmap
/// is unchanged: bridge piers and house pads paint through their models
/// at runtime, not via splat. Caller obtains the per-tile lists from
/// `bridges::group_flattens_by_tile` and
/// `settlement_flatten::group_flattens_by_tile`.
pub fn bake_tile_with_bridges(
    map: &GlobalMap,
    ctx: &BakeContext,
    tx: i32,
    tz: i32,
    bridge_flattens: &[bridges::BridgeFlatten],
    settlement_flattens: &[settlement_flatten::SettlementFlatten],
) -> BakedTile {
    let tile_min_x = tx as f32 * TILE_DIM as f32 - TILE_DIM as f32 * 0.5;
    let tile_min_z = tz as f32 * TILE_DIM as f32 - TILE_DIM as f32 * 0.5;
    let tile_max_x = tile_min_x + TILE_DIM as f32;
    let tile_max_z = tile_min_z + TILE_DIM as f32;

    let river_margin = river_margin_m();
    let river_segs = river_segments_near_tile_wrap_x(
        &ctx.rivers_world,
        tile_min_x,
        tile_min_z,
        tile_max_x,
        tile_max_z,
        river_margin,
        map.config.world_size_m as f32,
    );
    // `* 2.0`: the plain branch reads road distance up to HALF + FADE*2 via
    // `road_fade`. Shrinking the margin would desynchronize the fade across
    // tile boundaries.
    let road_margin = ROAD_HALF_WIDTH_M + ROAD_FADE_SPAN_M * 2.0;
    let road_segs = segments_near_tile(
        &ctx.roads_world,
        tile_min_x,
        tile_min_z,
        tile_max_x,
        tile_max_z,
        road_margin,
    );
    // Coast margin must reach as far as any classification branch consults
    // it: sand-dominant out to COAST_SAND_M, then a slope-dirt fade out to
    // COAST_SAND_M + COAST_FADE_SPAN_M. A cell within the fade span that
    // doesn't see the segment would otherwise reach `min_distance_to_segments
    // = INFINITY` and resolve to full-dirt, popping at the tile boundary.
    // The splat `is_sea` gate additionally reads the polyline side within
    // COAST_HEIGHT_BLEND_M of the shore, so the filter must reach at least
    // that far too — otherwise a near-edge pixel inside the blend band would
    // see no segment and fall back to the 8 m land_mask, re-popping the
    // staircase the height blend removes. (`.max` is a no-op today since the
    // sand reach is wider, but keeps the two coupled if either constant moves.)
    let coast_margin = (COAST_SAND_M + COAST_FADE_SPAN_M).max(COAST_HEIGHT_BLEND_M);
    let coast_segs = segments_near_tile_wrap_x(
        &ctx.coasts_world,
        tile_min_x,
        tile_min_z,
        tile_max_x,
        tile_max_z,
        coast_margin,
        map.config.world_size_m as f32,
    );

    // Order: natural surface → settlement pad → river carve → bridge deck.
    // Carving after the pad lets rivers cut a real channel through the
    // flattened settlement, and bridges run last so the deck rect fills the
    // channel back up at the crossing.
    let mut heights = sample_tile_heights_no_carve(map, ctx, tx, tz);
    if !settlement_flattens.is_empty() {
        settlement_flatten::apply_settlement_flatten(
            &mut heights,
            tile_min_x,
            tile_min_z,
            settlement_flattens,
            &ctx.detail_noise,
        );
    }
    apply_river_carve_to_tile(&mut heights, map, tile_min_x, tile_min_z, &river_segs);
    if !bridge_flattens.is_empty() {
        bridges::apply_bridge_flatten(&mut heights, tile_min_x, tile_min_z, bridge_flattens);
    }
    let river_field = bake_river_field(map, ctx, &heights, tile_min_x, tile_min_z, &river_segs);
    let water_field = bake_water_field(map, ctx, &heights, tile_min_x, tile_min_z, &river_segs);
    let heightmap = encode_heightmap(&heights);
    let splatmap = bake_splatmap(
        map,
        ctx,
        tx,
        tz,
        &heights,
        &river_segs,
        &road_segs,
        &coast_segs,
    );
    BakedTile {
        heightmap,
        splatmap,
        river_field,
        water_field,
    }
}

/// World-space X (or Z) → signed tile index. The +TILE_DIM/2 shift puts the
/// world origin at a tile center (matching `tile_origin_x = tx*TILE_DIM -
/// TILE_DIM/2`), so a world coord falls in tile `tx` when `tx*TILE_DIM -
/// TILE_DIM/2 ≤ wx < (tx+1)*TILE_DIM - TILE_DIM/2`.
#[inline]
pub(super) fn world_to_tile(wx: f32) -> i32 {
    ((wx + TILE_DIM as f32 * 0.5) / TILE_DIM as f32).floor() as i32
}

#[cfg(test)]
mod tests {
    use super::super::{continent, elevation, rivers, roads, settlements};
    use super::constants::{HEIGHT_BIAS, HEIGHT_STEP};
    use super::*;
    use crate::worldgen::config::WorldGenConfig;

    fn small_config() -> WorldGenConfig {
        WorldGenConfig {
            seed: 0xBEEF_7777,
            world_size_m: 1024,
            global_res: 128,
            reference_res: 128,
            sea_ratio: 0.35,
            continent_frequency: 1.0 / 64.0,
            continent_seed_count: 3,
            continent_seed_min_distance_cells: 20,
            target_continent_count: 1,
            continent_gap_cells: 0,
            small_island_count: 0,
            min_island_cells: 0,
            min_strait_width_cells: 0,
            max_isthmus_width_cells: 0,
            settlement_target_count: 3,
            settlement_min_spacing_cells: 10,
            settlement_inland_buffer_cells: 0,
            settlement_river_flow_threshold: 20.0,
            settlement_along_road_count: 0,
            y_border_wall_cells: 0,
            y_border_wall_height_m: 0.0,
            ..WorldGenConfig::default()
        }
    }

    fn build_context() -> (GlobalMap, BakeContext) {
        let cfg = small_config();
        let mut map = continent::generate_continent_mask(&cfg);
        elevation::generate_elevation(&mut map);
        let mut rm = rivers::compute_flow(&map);
        rivers::extract_rivers(&map, &mut rm, 50.0, 4);
        let s = settlements::place_settlements(&map, &rm);
        let net = roads::compute_roads(&map, &s, &rm);
        let coast_polys =
            super::super::coasts::extract_coasts(&map.land_mask, map.config.global_res as usize);
        let ctx = BakeContext::new(&map, &rm, &net, &coast_polys);
        (map, ctx)
    }

    #[test]
    fn output_byte_sizes_match_terrain_io() {
        let (map, ctx) = build_context();
        let baked = bake_tile(&map, &ctx, 0, 0);
        // These constants duplicate `terrain::defaults::{HEIGHTMAP_SIZE,
        // SPLATMAP_SIZE}` on purpose — the shared crate can't depend on
        // the terrain crate, so the fixed sizes are asserted here as a
        // contract pin.
        assert_eq!(baked.heightmap.len(), VERTS_PER_SIDE * VERTS_PER_SIDE * 2);
        assert_eq!(baked.splatmap.len(), TILE_DIM * TILE_DIM * 4);
    }

    #[test]
    fn deterministic_for_same_seed() {
        let (a_map, a_ctx) = build_context();
        let (b_map, b_ctx) = build_context();
        for &(tx, tz) in &[(0, 0), (-1, 1), (3, -2)] {
            let a = bake_tile(&a_map, &a_ctx, tx, tz);
            let b = bake_tile(&b_map, &b_ctx, tx, tz);
            assert_eq!(a.heightmap, b.heightmap);
            assert_eq!(a.splatmap, b.splatmap);
        }
    }

    #[test]
    fn sea_tiles_encode_below_sea_level() {
        // Pick a tile far inside the sea (corner of the small test world) and
        // verify the uint16 values decode to negative meters.
        let (map, ctx) = build_context();
        let world_size = map.config.world_size_m as i32;
        let tile_edge = world_size / (TILE_DIM as i32) / 2 - 1;
        let baked = bake_tile(&map, &ctx, tile_edge, tile_edge);
        let mut any_below = false;
        for chunk in baked.heightmap.chunks_exact(2) {
            let v = u16::from_le_bytes([chunk[0], chunk[1]]);
            let meters = v as f32 * HEIGHT_STEP - HEIGHT_BIAS;
            if meters < 0.0 {
                any_below = true;
                break;
            }
        }
        // Not every seed puts sea at the edge, but for this config we expect
        // at least some sub-zero vertices in the ocean corner tile.
        assert!(
            any_below,
            "expected some sub-zero vertices in an offshore tile"
        );
    }

    #[test]
    fn shoreline_hugs_smoothed_coast_polyline() {
        // The visible waterline is the heightmap's sea-level contour. With
        // the shoreline blend it must sit on the smoothed coast polyline:
        // sample segments across every chain and check (a) the natural
        // height ON the polyline is ~0 (detail noise wobbles it a few tens
        // of cm), and (b) 6 m to the oriented left is above water while
        // 6 m to the right is below. Cliffy coasts legitimately bend the
        // profile (the natural terrain blends back in), so this is a
        // large-majority check, not a per-point one.
        let (map, ctx) = build_context();
        let mut total = 0u32;
        let mut on_line_ok = 0u32;
        let mut sides_ok = 0u32;
        for poly in &ctx.coasts_world {
            if poly.points.len() < 2 {
                continue;
            }
            for i in (0..poly.points.len() - 1).step_by(7) {
                let a = poly.points[i];
                let b = poly.points[i + 1];
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let len = (dx * dx + dz * dz).sqrt();
                if len < 1e-3 {
                    continue;
                }
                let (nx, nz) = (-dz / len, dx / len);
                let (mx, mz) = ((a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5);
                total += 1;
                let on_line = probe_point(&map, &ctx, mx, mz).natural_height;
                if on_line.abs() < 0.35 {
                    on_line_ok += 1;
                }
                let land = probe_point(&map, &ctx, mx + nx * 6.0, mz + nz * 6.0).natural_height;
                let sea = probe_point(&map, &ctx, mx - nx * 6.0, mz - nz * 6.0).natural_height;
                if land > 0.0 && sea < 0.0 {
                    sides_ok += 1;
                }
            }
        }
        assert!(total >= 20, "expected a real coastline to sample: {total}");
        assert!(
            on_line_ok * 10 >= total * 9,
            "waterline off the polyline too often: {on_line_ok}/{total}"
        );
        assert!(
            sides_ok * 10 >= total * 9,
            "land/sea sides wrong too often: {sides_ok}/{total}"
        );
    }

    #[test]
    fn splat_bytes_reference_valid_palette_slots() {
        let (map, ctx) = build_context();
        let baked = bake_tile(&map, &ctx, 0, 0);
        for chunk in baked.splatmap.chunks_exact(4) {
            let primary = (chunk[0] >> 4) & 0x0F;
            let secondary = chunk[0] & 0x0F;
            assert!(
                primary <= PAL_RIVER_BED,
                "primary slot {} out of palette",
                primary
            );
            assert!(
                secondary <= PAL_RIVER_BED,
                "secondary slot {} out of palette",
                secondary
            );
            // veg byte is either 0, short grass (230..=239), or tall grass
            // (240..=249). The patch field chooses tall/short per-patch, so
            // both ranges can appear in the same tile.
            let veg = chunk[3];
            assert!(
                veg == 0 || (230..=249).contains(&veg),
                "unexpected veg byte {}",
                veg
            );
        }
    }
}
