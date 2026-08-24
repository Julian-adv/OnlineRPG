//! `bake` command: runs the full worldgen pipeline, then for every tile in
//! the chosen region range writes heightmap + splatmap bin files plus a
//! top-level `worldgen.json` index with the seed, config, settlements, and
//! road polylines in world coordinates. The texture palette is global
//! (`shared/palette.json`, shared with the client at bundle time), so no
//! per-region or per-bake meta files are emitted.
//!
//! The file layout matches `terrain::TerrainIO` so the runtime can load the
//! output without any format conversion.

use crate::map_tile::{render_region_pyramid_bounded, TILES_PER_REGION};
use anyhow::{Context, Result};
use onlinerpg_shared::worldgen::{
    coasts, continent, elevation, erosion, rivers, roads, settlements,
    tile_bake::{self, bridges},
    vegetation, GlobalMap, WorldGenConfig,
};
use onlinerpg_terrain::coords;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

fn remove_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::Error::from(e).context(format!("remove {}", path.display()))),
    }
}

pub fn run(
    config: &WorldGenConfig,
    out: &Path,
    region_min: (i32, i32),
    region_max: (i32, i32),
    water_field_only: bool,
) -> Result<()> {
    let overall = Instant::now();

    // --- Phase 1–6 pipeline: exactly mirrors `preview::run` so the same
    // seed produces the same macro world in both commands. ---------------
    eprintln!(
        "Generating {}×{} global map (seed={:#x})…",
        config.global_res, config.global_res, config.seed
    );
    let t = Instant::now();
    let mut map = continent::generate_continent_mask(config);
    eprintln!("  Phase 1 (continent): {:.2}s", t.elapsed().as_secs_f32());

    let t = Instant::now();
    elevation::generate_elevation(&mut map);
    eprintln!("  Phase 2 (elevation): {:.2}s", t.elapsed().as_secs_f32());

    let t = Instant::now();
    erosion::erode_hydraulic(&mut map);
    eprintln!(
        "  Phase 3 (erosion):   {:.2}s  (sim_res {})",
        t.elapsed().as_secs_f32(),
        if config.erosion_sim_res == 0 {
            config.global_res
        } else {
            config.erosion_sim_res
        }
    );

    let t = Instant::now();
    let mut river_map = rivers::compute_flow(&map);
    let min_peak = config.max_elevation_m * rivers::RIVER_PEAK_ELEVATION_FRAC;
    rivers::extract_rivers(&map, &mut river_map, min_peak, 20);
    eprintln!(
        "  Phase 4 (rivers):    {:.2}s  ({} polylines)",
        t.elapsed().as_secs_f32(),
        river_map.rivers.len()
    );

    let added_hotspots = elevation::seed_river_gap_mountains(&mut map, &river_map);
    if !added_hotspots.is_empty() {
        let t = Instant::now();
        river_map = rivers::compute_flow(&map);
        rivers::extract_rivers(&map, &mut river_map, min_peak, 20);
        eprintln!(
            "  Phase 4b (gap fill): {:.2}s  (+{} mountain hotspots, {} polylines)",
            t.elapsed().as_secs_f32(),
            added_hotspots.len(),
            river_map.rivers.len()
        );
    }

    let t = Instant::now();
    let island_out = elevation::seed_small_island_hills(&mut map);
    if !island_out.hotspots.is_empty() {
        let saved_rivers = std::mem::take(&mut river_map.rivers);
        river_map = rivers::compute_flow(&map);
        river_map.rivers = saved_rivers;
        let before = river_map.rivers.len();
        rivers::extract_small_island_rivers(
            &map,
            &mut river_map,
            &island_out.island_cells,
            config.small_island_river_min_peak_m,
            config.small_island_river_min_length as usize,
            config.small_island_river_peak_spacing_cells,
        );
        eprintln!(
            "  Phase 4c (islands):  {:.2}s  (+{} hotspots, +{} island rivers)",
            t.elapsed().as_secs_f32(),
            island_out.hotspots.len(),
            river_map.rivers.len() - before,
        );
    }

    let t = Instant::now();
    let fields = settlements::compute_habitability_fields(&map, &river_map);
    let mut settlements_list =
        settlements::place_settlements_with_fields(&map, &river_map, &fields);
    eprintln!(
        "  Phase 5a (cities):   {:.2}s  ({} cities)",
        t.elapsed().as_secs_f32(),
        settlements_list.len()
    );

    let t = Instant::now();
    let mut road_net = roads::compute_roads(&map, &settlements_list, &river_map);
    // Fuse pairs of roads that emerge from the same town and run nearly
    // parallel for a long stretch — replace the duplicate prefixes with a
    // shared trunk so the network reads as one road that Y-forks at the
    // divergence point instead of two near-parallel routes. Run before
    // grid-snap so the snap pass sees the merged geometry.
    roads::merge_parallel_runs(&mut road_net, map.config.global_res as usize);
    // Second pass picks up parallel pairs that don't share an endpoint —
    // e.g. two roads heading from a coastal corridor toward different
    // hub cities a few hundred meters apart. The endpoint-anchored pass
    // can't see those because their start/end cells differ.
    roads::merge_parallel_interiors(&mut road_net, map.config.global_res as usize);
    // Bridges in the runtime are placed on a 90°-grid only, so snap a small
    // window of cells at every road↔river crossing into pure cardinal
    // strips before tile baking — otherwise a diagonal A* crossing would
    // leave a sub-cell gap that no axis-aligned bridge mesh fits.
    roads::snap_crossings_to_grid(
        &mut road_net,
        &mut river_map,
        map.config.global_res as usize,
    );
    eprintln!(
        "  Phase 6 (roads):     {:.2}s  ({} roads)",
        t.elapsed().as_secs_f32(),
        road_net.roads.len()
    );

    let t = Instant::now();
    let extras = settlements::place_settlements_along_roads_with_fields(
        &map,
        &road_net,
        &settlements_list,
        config.settlement_along_road_count as usize,
        &fields,
    );
    let added = extras.len();
    settlements_list.extend(extras);
    eprintln!(
        "  Phase 5b (villages): {:.2}s  (+{} along-road villages, total {})",
        t.elapsed().as_secs_f32(),
        added,
        settlements_list.len()
    );

    // --- Phase 7 prep: build shared per-cell context. -------------------
    let t = Instant::now();
    let coast_polys = coasts::extract_coasts(&map.land_mask, map.config.global_res as usize);
    let ctx = tile_bake::BakeContext::new(&map, &river_map, &road_net, &coast_polys);
    eprintln!(
        "  Phase 7 prep:        {:.2}s  ({} coast polylines + river/road fields)",
        t.elapsed().as_secs_f32(),
        coast_polys.len()
    );

    // --- Settlement pads: 50 m circular flatten around each settlement so
    // houses sit on level ground regardless of the underlying hills. The
    // splatmap is left untouched (roads still paint through). Built before
    // bridge detection so bridge bank probes can read the post-pad surface
    // when the bridge lands inside a town. ------------------------------
    let t = Instant::now();
    let settlement_directives =
        tile_bake::settlement_flatten::build_directives(&settlements_list, &map.config, &map, &ctx);
    let settlement_flattens =
        tile_bake::settlement_flatten::group_flattens_by_tile(&settlement_directives);
    eprintln!(
        "  Phase 7 pads:        {:.2}s  ({} settlements across {} tiles)",
        t.elapsed().as_secs_f32(),
        settlements_list.len(),
        settlement_flattens.len()
    );

    // --- Bridge placement: detect road↔river crossings, write region
    // object JSONs, and pre-bucket per-tile flatten directives so the
    // parallel bake can apply them inline with heightmap sampling. -------
    let t = Instant::now();
    let bridge_catalog = load_bridge_catalog().ok_or_else(|| {
        anyhow::anyhow!("bridge catalog entries 'stone_bridge' and 'bridge_wood_long' missing")
    })?;
    let bridge_placements = bridges::detect_bridges(
        &map,
        &river_map,
        &road_net,
        &ctx,
        &bridge_catalog,
        &settlement_directives,
    );
    let bridge_flattens = bridges::group_flattens_by_tile(&bridge_placements, &bridge_catalog);
    eprintln!(
        "  Phase 7 bridges:     {:.2}s  ({} bridges across {} tiles)",
        t.elapsed().as_secs_f32(),
        bridge_placements.len(),
        bridge_flattens.len()
    );

    // --- Directory scaffolding. ------------------------------------------
    let region_xs: Vec<i32> = (region_min.0..=region_max.0).collect();
    let region_zs: Vec<i32> = (region_min.1..=region_max.1).collect();
    let region_count = region_xs.len() * region_zs.len();

    if !water_field_only {
        std::fs::create_dir_all(out.join("height"))
            .with_context(|| format!("create {}/height", out.display()))?;
        std::fs::create_dir_all(out.join("splat"))
            .with_context(|| format!("create {}/splat", out.display()))?;
        std::fs::create_dir_all(out.join("trees"))
            .with_context(|| format!("create {}/trees", out.display()))?;
        std::fs::create_dir_all(out.join("grass"))
            .with_context(|| format!("create {}/grass", out.display()))?;
        std::fs::create_dir_all(out.join("river-field"))
            .with_context(|| format!("create {}/river-field", out.display()))?;
        std::fs::create_dir_all(out.join("minimap"))
            .with_context(|| format!("create {}/minimap", out.display()))?;
        std::fs::create_dir_all(out.join("objects"))
            .with_context(|| format!("create {}/objects", out.display()))?;
    }
    std::fs::create_dir_all(out.join("water-field"))
        .with_context(|| format!("create {}/water-field", out.display()))?;

    for &rx in &region_xs {
        for &rz in &region_zs {
            if !water_field_only {
                std::fs::create_dir_all(coords::height_region_dir(out, rx, rz))?;
                std::fs::create_dir_all(coords::splat_region_dir(out, rx, rz))?;
                std::fs::create_dir_all(coords::tree_region_dir(out, rx, rz))?;
                std::fs::create_dir_all(coords::grass_region_dir(out, rx, rz))?;
                std::fs::create_dir_all(coords::river_field_region_dir(out, rx, rz))?;
            }
            std::fs::create_dir_all(coords::water_field_region_dir(out, rx, rz))?;
        }
    }

    // --- Tile enumeration + parallel bake. -------------------------------
    let mut tile_coords: Vec<(i32, i32)> = Vec::with_capacity(region_count * 256);
    for &rx in &region_xs {
        for &rz in &region_zs {
            for j in 0..TILES_PER_REGION {
                for i in 0..TILES_PER_REGION {
                    tile_coords.push((rx * TILES_PER_REGION + i, rz * TILES_PER_REGION + j));
                }
            }
        }
    }
    let total_tiles = tile_coords.len();
    eprintln!(
        "Baking {} regions × 256 tiles = {} tiles (x=[{:+},{:+}] z=[{:+},{:+}])",
        region_count, total_tiles, region_min.0, region_max.0, region_min.1, region_max.1
    );

    let done = AtomicUsize::new(0);
    let next_report = AtomicUsize::new(4096);
    let t_bake = Instant::now();

    tile_coords
        .par_iter()
        .try_for_each(|&(tx, tz)| -> Result<()> {
            let flattens = bridge_flattens
                .get(&(tx, tz))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let pads = settlement_flattens
                .get(&(tx, tz))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let baked = tile_bake::bake_tile_with_bridges(&map, &ctx, tx, tz, flattens, pads);

            if !water_field_only {
                let hpath = coords::heightmap_path(out, tx, tz);
                let spath = coords::splatmap_path(out, tx, tz);
                std::fs::write(&hpath, &baked.heightmap)
                    .with_context(|| format!("write {}", hpath.display()))?;
                std::fs::write(&spath, &baked.splatmap)
                    .with_context(|| format!("write {}", spath.display()))?;

                // Drop stale pre-edit snapshots from the previous bake;
                // `ensureOriginalHeightmap` will re-snapshot the fresh terrain
                // on first edit. Without this, restoring after a house delete
                // would write the old bake's shape back into the tile.
                remove_if_exists(&coords::original_heightmap_path(out, tx, tz))?;
                remove_if_exists(&coords::original_grass_path(out, tx, tz))?;

                // Phase 8: tree + grass placement files. The worldgen pipeline
                // doesn't lay houses, so no exclusion rects — empty slice keeps
                // the call compatible with a future replat that does.
                let tree_bin =
                    vegetation::bake_trees(tx, tz, &baked.splatmap, &baked.heightmap, &[]);
                let grass_bin = vegetation::bake_grass(tx, tz, &baked.splatmap, &baked.heightmap);
                let tpath = coords::tree_path(out, tx, tz);
                let gpath = coords::grass_path(out, tx, tz);
                std::fs::write(&tpath, &tree_bin)
                    .with_context(|| format!("write {}", tpath.display()))?;
                std::fs::write(&gpath, &grass_bin)
                    .with_context(|| format!("write {}", gpath.display()))?;

                // RFD1 river-field. Stale file is removed when this tile has
                // no segments so a previous-world bin doesn't outlive its bake.
                let rfpath = coords::river_field_path(out, tx, tz);
                if let Some(field_bin) = baked.river_field.as_ref() {
                    std::fs::write(&rfpath, field_bin)
                        .with_context(|| format!("write {}", rfpath.display()))?;
                } else {
                    remove_if_exists(&rfpath)?;
                }
            }

            // WFD1 unified water-field, same stale-removal discipline.
            let wfpath = coords::water_field_path(out, tx, tz);
            if let Some(field_bin) = baked.water_field.as_ref() {
                std::fs::write(&wfpath, field_bin)
                    .with_context(|| format!("write {}", wfpath.display()))?;
            } else {
                remove_if_exists(&wfpath)?;
            }

            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            let report_at = next_report.load(Ordering::Relaxed);
            if n >= report_at || n == total_tiles {
                // Best-effort CAS so only one thread logs per threshold.
                if next_report
                    .compare_exchange(
                        report_at,
                        report_at + 4096,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    )
                    .is_ok()
                {
                    eprintln!(
                        "  {}/{} tiles ({:.1}%)",
                        n,
                        total_tiles,
                        n as f32 * 100.0 / total_tiles as f32
                    );
                }
            }
            Ok(())
        })?;

    eprintln!(
        "Baked {} tiles in {:.2}s",
        total_tiles,
        t_bake.elapsed().as_secs_f32()
    );

    // Water-field-only mode ends here: everything below (minimaps, object
    // JSONs, worldgen.json, preview PNGs) would clobber live-world state
    // that the full bake owns — most critically the pre-edit heightmap
    // snapshots removed in the per-tile loop above.
    if water_field_only {
        eprintln!(
            "Water-field-only bake complete ({:.2}s total)",
            overall.elapsed().as_secs_f32()
        );
        return Ok(());
    }

    // --- Region map PNGs. ------------------------------------------------
    let t_mini = Instant::now();
    let region_coords: Vec<(i32, i32)> = region_xs
        .iter()
        .flat_map(|&rx| region_zs.iter().map(move |&rz| (rx, rz)))
        .collect();
    region_coords
        .into_par_iter()
        .try_for_each(|(rx, rz)| -> Result<()> {
            render_region_pyramid_bounded(out, out, rx, rz, region_min, region_max)
        })?;
    eprintln!(
        "Wrote {} region map pyramids in {:.2}s",
        region_count,
        t_mini.elapsed().as_secs_f32()
    );

    // --- Region object JSONs (bridges grouped by region). ---------------
    let t_obj = Instant::now();
    let region_count_written = write_object_regions(
        out,
        &bridge_placements,
        &bridge_catalog,
        region_min,
        region_max,
    )?;
    eprintln!(
        "Wrote {} region object JSONs in {:.2}s",
        region_count_written,
        t_obj.elapsed().as_secs_f32()
    );

    // --- Top-level worldgen.json index. ----------------------------------
    let worldgen_json = build_worldgen_json(
        config,
        &map,
        &settlements_list,
        &road_net,
        region_min,
        region_max,
    );
    let wpath = out.join("worldgen.json");
    std::fs::write(&wpath, serde_json::to_string_pretty(&worldgen_json)?)
        .with_context(|| format!("write {}", wpath.display()))?;

    let preview_dir = out
        .join("worldgen_preview")
        .join(crate::preview::seed_subdir(config.seed));
    std::fs::create_dir_all(&preview_dir)
        .with_context(|| format!("create {}", preview_dir.display()))?;
    crate::preview::write_pngs(&preview_dir, &map, &river_map, &road_net, &settlements_list)?;

    eprintln!(
        "Wrote {} (bake total {:.2}s)",
        wpath.display(),
        overall.elapsed().as_secs_f32()
    );
    Ok(())
}

/// Convert a global-cell center to its world-space position. World origin
/// sits at the center of the map (X wraps, Z clamps).
fn cell_to_world(cell_x: u32, cell_y: u32, cfg: &WorldGenConfig) -> (f32, f32) {
    let mpc = cfg.meters_per_cell();
    let origin_offset = cfg.world_size_m as f32 * 0.5;
    (
        (cell_x as f32 + 0.5) * mpc - origin_offset,
        (cell_y as f32 + 0.5) * mpc - origin_offset,
    )
}

fn build_worldgen_json(
    cfg: &WorldGenConfig,
    map: &GlobalMap,
    settlements_list: &[settlements::Settlement],
    road_net: &roads::RoadNetwork,
    region_min: (i32, i32),
    region_max: (i32, i32),
) -> serde_json::Value {
    let settlements_json: Vec<_> = settlements_list
        .iter()
        .map(|s| {
            let (wx, wz) = cell_to_world(s.cell_x, s.cell_y, cfg);
            serde_json::json!({
                "cell_x": s.cell_x,
                "cell_y": s.cell_y,
                "world_x": wx,
                "world_z": wz,
                "tile_x": coords::world_to_tile(wx),
                "tile_z": coords::world_to_tile(wz),
                "score": s.score,
            })
        })
        .collect();
    let roads_json: Vec<_> = road_net
        .roads
        .iter()
        .map(|r| {
            let points: Vec<_> = r
                .points
                .iter()
                .map(|&(x, y)| {
                    let (wx, wz) = cell_to_world(x, y, cfg);
                    serde_json::json!([wx, wz])
                })
                .collect();
            serde_json::json!({ "points": points })
        })
        .collect();
    serde_json::json!({
        "seed": cfg.seed,
        "config": cfg,
        "measured_sea_ratio": map.measured_sea_ratio(),
        "baked_region_range": {
            "x_min": region_min.0,
            "x_max": region_max.0,
            "z_min": region_min.1,
            "z_max": region_max.1,
        },
        "settlements": settlements_json,
        "roads": roads_json,
    })
}

/// Embedded copy of `client/public/models/objects/catalog.json` so the bake
/// runs without a runtime config path. Compiled-in rather than read from
/// disk because the bake binary runs from arbitrary working directories.
const CATALOG_JSON: &str = include_str!("../../../client/public/models/objects/catalog.json");

fn load_bridge_catalog() -> Option<bridges::BridgeCatalog> {
    let entries: serde_json::Value = serde_json::from_str(CATALOG_JSON).ok()?;
    let arr = entries.as_array()?;
    let parse = |id: &str| -> Option<bridges::BridgeModel> {
        let entry = arr
            .iter()
            .find(|e| e.get("id").and_then(|v| v.as_str()) == Some(id))?;
        let bridge = entry.get("bridge")?;
        let opt_f32 = |key: &str| entry.get(key).and_then(|v| v.as_f64()).map(|v| v as f32);
        let deck_max_z = bridge.get("deckMaxZ")?.as_f64()? as f32;
        let deck_min_x = bridge.get("deckMinX")?.as_f64()? as f32;
        let deck_max_x = bridge.get("deckMaxX")?.as_f64()? as f32;
        // Explicit `flattenFootLengthZ` in the catalog wins so the design
        // can override the deckYSamples-derived value when a model's arch
        // shape isn't a good proxy for how far onto the bank the foundation
        // should land.
        let foot_length = opt_f32("flattenFootLengthZ").unwrap_or_else(|| {
            let samples: Vec<f32> = bridge
                .get("deckYSamples")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_f64().map(|f| f as f32))
                        .collect()
                })
                .unwrap_or_default();
            bridges::foot_length_from_samples(&samples, deck_max_z)
        });
        // `flattenFootWidthX` is the full lateral width of the foot rect;
        // store it as half-width since the rect is symmetric. Default to
        // the deck's own X half-width so models without an explicit value
        // flatten exactly under the deck.
        let foot_half_width_x = opt_f32("flattenFootWidthX")
            .map(|w| w * 0.5)
            .unwrap_or((deck_max_x - deck_min_x) * 0.5);
        Some(bridges::BridgeModel {
            id: id.to_string(),
            deck_min_x,
            deck_max_x,
            deck_min_z: bridge.get("deckMinZ")?.as_f64()? as f32,
            deck_max_z,
            min_local_y: opt_f32("minLocalY").unwrap_or(0.0),
            flatten_bury_depth: opt_f32("flattenBuryDepth").unwrap_or(0.0),
            flatten_foot_length_z: foot_length,
            flatten_foot_half_width_x: foot_half_width_x,
        })
    };
    Some(bridges::BridgeCatalog {
        narrow: parse("stone_bridge")?,
        wide: parse("bridge_wood_long")?,
    })
}

/// Group placements by region and write `objects/r±NN_±NN.json`. Skips
/// regions outside the requested bake range to match the existing per-tile
/// output discipline. Returns the number of region files written.
///
/// Stale bake-emitted bridges are pruned here: each region in the bake
/// range is read, placements whose `type` is in `BridgeCatalog::model_ids`
/// are dropped, and surviving non-bridge placements are merged with this
/// bake's bridges. New bridge IDs continue past the highest kept ID so
/// editor-side `id`-keyed references on user-placed objects stay stable
/// across bakes. A region whose merged result is empty has its file
/// removed.
fn write_object_regions(
    out: &Path,
    placements: &[bridges::BridgePlacement],
    catalog: &bridges::BridgeCatalog,
    region_min: (i32, i32),
    region_max: (i32, i32),
) -> Result<usize> {
    let mut by_region: HashMap<(i32, i32), Vec<&bridges::BridgePlacement>> = HashMap::new();
    for p in placements {
        let tx = coords::world_to_tile(p.x);
        let tz = coords::world_to_tile(p.z);
        let rx = coords::tile_to_region(tx);
        let rz = coords::tile_to_region(tz);
        if rx < region_min.0 || rx > region_max.0 || rz < region_min.1 || rz > region_max.1 {
            continue;
        }
        by_region.entry((rx, rz)).or_default().push(p);
    }

    let bridge_ids = catalog.model_ids();
    let written = AtomicUsize::new(0);
    let region_pairs: Vec<(i32, i32)> = (region_min.1..=region_max.1)
        .flat_map(|rz| (region_min.0..=region_max.0).map(move |rx| (rx, rz)))
        .collect();
    region_pairs
        .into_par_iter()
        .try_for_each(|(rx, rz)| -> Result<()> {
            let path = coords::object_path(out, rx, rz);
            let existing: Vec<serde_json::Value> = match std::fs::read_to_string(&path) {
                Ok(s) => {
                    let val: serde_json::Value = serde_json::from_str(&s)
                        .with_context(|| format!("parse {}", path.display()))?;
                    val.get("placements")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default()
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(e) => {
                    return Err(anyhow::Error::from(e).context(format!("read {}", path.display())));
                }
            };
            let kept: Vec<serde_json::Value> = existing
                .iter()
                .filter(|p| {
                    let t = p.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    !bridge_ids.contains(&t)
                })
                .cloned()
                .collect();
            let stripped = kept.len() != existing.len();
            let new_bridges = by_region.get(&(rx, rz));
            // Region untouched: file's bridges (if any) match what we'd
            // re-emit, and there's nothing new to add. Skip the rewrite.
            if !stripped && new_bridges.is_none() {
                return Ok(());
            }
            let max_kept_id = kept
                .iter()
                .filter_map(|p| p.get("id").and_then(|v| v.as_u64()))
                .max()
                .unwrap_or(0);
            let mut merged = kept;
            if let Some(region_placements) = new_bridges {
                for (i, p) in region_placements.iter().enumerate() {
                    merged.push(serde_json::json!({
                        "floorLevel": 0,
                        "id": max_kept_id + 1 + i as u64,
                        "rotation": p.rotation,
                        "type": p.model_id,
                        "x": p.x,
                        "y": p.y,
                        "z": p.z,
                    }));
                }
            }
            if merged.is_empty() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("remove {}", path.display()))?;
            } else {
                let json = serde_json::json!({ "placements": merged });
                std::fs::write(&path, serde_json::to_string_pretty(&json)?)
                    .with_context(|| format!("write {}", path.display()))?;
                written.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        })?;
    Ok(written.into_inner())
}
