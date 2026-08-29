use crate::map_color::{lerp, mix, scale, smooth_curve, smoothstep, to_rgb, unit_to_u8};
use anyhow::{bail, Context, Result};
use image::{Rgb, RgbImage};
use onlinerpg_shared::worldgen::{
    coasts, continent, elevation, erosion, rivers, roads, settlements,
    tile_bake::{
        BakeContext, HEIGHT_BIAS, HEIGHT_STEP, PAL_CLIFF, PAL_DIRT, PAL_PAVING, PAL_RIVER_BED,
        PAL_ROAD, PAL_SAND, PAL_SNOW, PAL_STONE_PATH, TILE_DIM, VERTS_PER_SIDE,
    },
    GlobalMap, WorldGenConfig,
};
use onlinerpg_terrain::coords;
use rayon::prelude::*;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

const REGION_PX: usize = 1024;
const FANTASY_FAMILY: coords::MinimapFamily = coords::MinimapFamily::Fantasy;
const LEGACY_FAMILY: coords::MinimapFamily = coords::MinimapFamily::Legacy;
/// Every tile size a region is written at, largest first — each LOD is the
/// half-scale of the one before it.
const TILE_SIZES: [u32; 4] = [
    coords::MINIMAP_BASE_SIZE,
    coords::MINIMAP_LOD_SIZES[2],
    coords::MINIMAP_LOD_SIZES[1],
    coords::MINIMAP_LOD_SIZES[0],
];
const TILES_PER_REGION: i32 = 16;
const WORLD_MIN_REGION: i32 = -16;
const WORLD_MAX_REGION: i32 = 15;
const BASE_GUIDE_WEIGHT: f32 = 0.46;
/// Side length of the baked world in meters. This module renders the fixed
/// 32x32-region world, so it is derived from the region bounds rather than
/// read from `WorldGenConfig` — the two must agree.
const WORLD_SIZE_M: f32 =
    ((WORLD_MAX_REGION - WORLD_MIN_REGION + 1) * TILES_PER_REGION) as f32 * TILE_SIZE_M;
const TILE_SIZE_M: f32 = 64.0;
const LEGACY_COLORS: [[u8; 3]; 11] = [
    [80, 140, 50],
    [210, 185, 110],
    [180, 100, 60],
    [240, 240, 245],
    [140, 135, 125],
    [130, 110, 90],
    [100, 160, 220],
    [160, 155, 150],
    [170, 160, 145],
    [30, 60, 150],
    [100, 160, 220],
];

pub fn run(
    config: &WorldGenConfig,
    legacy_source: &Path,
    terrain: &Path,
    out: &Path,
    region_min: (i32, i32),
    region_max: (i32, i32),
) -> Result<()> {
    validate_inputs(legacy_source, out, region_min, region_max)?;
    let overall = Instant::now();
    let macro_world = generate_macro_world(config)?;
    let textures = Textures::load()?;
    let gamma = GammaLut::new();
    let regions = region_coordinates(region_min, region_max);
    let expected_files = regions.len() * TILE_SIZES.len();
    let staging = staging_path(out)?;

    std::fs::create_dir_all(&staging)
        .with_context(|| format!("create staging directory {}", staging.display()))?;
    for size in coords::MINIMAP_LOD_SIZES {
        std::fs::create_dir_all(staging.join(size.to_string()))
            .with_context(|| format!("create staging LOD {}", size))?;
    }

    eprintln!(
        "Rendering {} regions ({} WebP tiles) to staging {}",
        regions.len(),
        expected_files,
        staging.display()
    );
    let completed = AtomicUsize::new(0);
    let raw_count = AtomicUsize::new(0);
    let legacy_count = AtomicUsize::new(0);
    let generated_count = AtomicUsize::new(0);
    let render_start = Instant::now();

    regions.par_iter().try_for_each(|&(rx, rz)| -> Result<()> {
        let source = load_semantic_source(legacy_source, terrain, rx, rz)?;
        match &source {
            SemanticSource::Raw(_) => {
                raw_count.fetch_add(1, Ordering::Relaxed);
            }
            SemanticSource::Legacy(_) => {
                legacy_count.fetch_add(1, Ordering::Relaxed);
            }
            SemanticSource::Generated => {
                generated_count.fetch_add(1, Ordering::Relaxed);
            }
        }
        let mut render = render_region(rx, rz, &macro_world, &textures, &source);
        for size in TILE_SIZES {
            if size < coords::MINIMAP_BASE_SIZE {
                render = render.downsampled(&gamma);
            }
            save_render(
                &render,
                rx,
                rz,
                &textures,
                &tile_path(&staging, size, rx, rz),
            )?;
        }

        let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
        let stride = (regions.len() / 20).max(1);
        if done == regions.len() || done.is_multiple_of(stride) {
            eprintln!(
                "  rendered {}/{} regions ({:.1}s)",
                done,
                regions.len(),
                render_start.elapsed().as_secs_f32()
            );
        }
        Ok(())
    })?;

    for size in coords::MINIMAP_LOD_SIZES {
        std::fs::create_dir_all(out.join(size.to_string()))
            .with_context(|| format!("create output LOD {}", size))?;
    }

    regions.par_iter().try_for_each(|&(rx, rz)| -> Result<()> {
        for size in TILE_SIZES {
            publish_file(
                &tile_path(&staging, size, rx, rz),
                &tile_path(out, size, rx, rz),
            )?;
        }
        Ok(())
    })?;

    for size in coords::MINIMAP_LOD_SIZES {
        let _ = std::fs::remove_dir(staging.join(size.to_string()));
    }
    let _ = std::fs::remove_dir(&staging);
    eprintln!(
        "Fantasy map complete in {:.1}s: {} legacy, {} raw, {} generated fallback; {} files at {}",
        overall.elapsed().as_secs_f32(),
        legacy_count.load(Ordering::Relaxed),
        raw_count.load(Ordering::Relaxed),
        generated_count.load(Ordering::Relaxed),
        expected_files,
        out.display()
    );
    Ok(())
}

fn validate_inputs(
    legacy_source: &Path,
    out: &Path,
    region_min: (i32, i32),
    region_max: (i32, i32),
) -> Result<()> {
    if !legacy_source.is_dir() {
        bail!(
            "legacy minimap source is not a directory: {}",
            legacy_source.display()
        );
    }
    if path_key(legacy_source)? == path_key(out)? {
        bail!("legacy source and output must be different directories");
    }
    if region_min.0 < WORLD_MIN_REGION
        || region_min.1 < WORLD_MIN_REGION
        || region_max.0 > WORLD_MAX_REGION
        || region_max.1 > WORLD_MAX_REGION
    {
        bail!(
            "map region range must stay within -16..15: x[{},{}] z[{},{}]",
            region_min.0,
            region_max.0,
            region_min.1,
            region_max.1
        );
    }
    Ok(())
}

fn generate_macro_world(config: &WorldGenConfig) -> Result<MacroWorld> {
    eprintln!(
        "Generating {}x{} seed {:#x} macro world in memory",
        config.global_res, config.global_res, config.seed
    );
    let t = Instant::now();
    let mut map = continent::generate_continent_mask(config);
    eprintln!("  Phase 1 continent: {:.2}s", t.elapsed().as_secs_f32());

    let t = Instant::now();
    elevation::generate_elevation(&mut map);
    eprintln!("  Phase 2 elevation: {:.2}s", t.elapsed().as_secs_f32());

    let t = Instant::now();
    erosion::erode_hydraulic(&mut map);
    eprintln!("  Phase 3 erosion: {:.2}s", t.elapsed().as_secs_f32());

    let t = Instant::now();
    let mut river_map = rivers::compute_flow(&map);
    let min_peak = config.max_elevation_m * rivers::RIVER_PEAK_ELEVATION_FRAC;
    rivers::extract_rivers(&map, &mut river_map, min_peak, 20);
    let added_hotspots = elevation::seed_river_gap_mountains(&mut map, &river_map);
    if !added_hotspots.is_empty() {
        river_map = rivers::compute_flow(&map);
        rivers::extract_rivers(&map, &mut river_map, min_peak, 20);
    }
    let island_out = elevation::seed_small_island_hills(&mut map);
    if !island_out.hotspots.is_empty() {
        let saved_rivers = std::mem::take(&mut river_map.rivers);
        river_map = rivers::compute_flow(&map);
        river_map.rivers = saved_rivers;
        rivers::extract_small_island_rivers(
            &map,
            &mut river_map,
            &island_out.island_cells,
            config.small_island_river_min_peak_m,
            config.small_island_river_min_length as usize,
            config.small_island_river_peak_spacing_cells,
        );
    }
    eprintln!(
        "  Phase 4 rivers: {:.2}s ({} polylines)",
        t.elapsed().as_secs_f32(),
        river_map.rivers.len()
    );

    let t = Instant::now();
    let fields = settlements::compute_habitability_fields(&map, &river_map);
    let mut settlement_list = settlements::place_settlements_with_fields(&map, &river_map, &fields);
    eprintln!(
        "  Phase 5 settlements: {:.2}s ({} cities)",
        t.elapsed().as_secs_f32(),
        settlement_list.len()
    );

    let t = Instant::now();
    let mut road_net = roads::compute_roads(&map, &settlement_list, &river_map);
    roads::merge_parallel_runs(&mut road_net, config.global_res as usize);
    roads::merge_parallel_interiors(&mut road_net, config.global_res as usize);
    roads::snap_crossings_to_grid(&mut road_net, &mut river_map, config.global_res as usize);
    let extras = settlements::place_settlements_along_roads_with_fields(
        &map,
        &road_net,
        &settlement_list,
        config.settlement_along_road_count as usize,
        &fields,
    );
    settlement_list.extend(extras);
    eprintln!(
        "  Phase 6 roads: {:.2}s ({} roads, {} settlements)",
        t.elapsed().as_secs_f32(),
        road_net.roads.len(),
        settlement_list.len()
    );

    let t = Instant::now();
    let coast_polys = coasts::extract_coasts(&map.land_mask, config.global_res as usize);
    let context = BakeContext::new(&map, &river_map, &road_net, &coast_polys);
    let river_cells = raster_rivers(&river_map, config.global_res as usize);
    let road_cells = raster_roads(&road_net, config.global_res as usize);
    let dist_to_land = context.dist_to_land;
    let derived = derive_relief_fields(&map);
    eprintln!(
        "  map fields: {:.2}s ({} coast polylines)",
        t.elapsed().as_secs_f32(),
        coast_polys.len()
    );

    Ok(MacroWorld {
        config: map.config,
        land: map.land_mask,
        elevation: map.elevation_m,
        dist_to_land,
        river: river_cells,
        road: road_cells,
        coast: derived.coast,
        slope: derived.slope,
        shade: derived.shade,
        ridge: derived.ridge,
        forest: derived.forest,
    })
}

struct DerivedFields {
    coast: Vec<u8>,
    slope: Vec<f32>,
    shade: Vec<u8>,
    ridge: Vec<u8>,
    forest: Vec<u8>,
}

fn derive_relief_fields(map: &GlobalMap) -> DerivedFields {
    let res = map.config.global_res as usize;
    let mpc = map.config.meters_per_cell();
    let small = box_blur(&map.elevation_m, res, 1);
    let broad = box_blur(&map.elevation_m, res, 7);
    let mut coast = vec![0u8; res * res];
    let mut slope = vec![0.0f32; res * res];
    let mut shade = vec![0u8; res * res];
    let mut ridge = vec![0u8; res * res];
    let mut forest = vec![0u8; res * res];

    for z in 0..res {
        let zn = z.saturating_sub(1);
        let zp = (z + 1).min(res - 1);
        for x in 0..res {
            let xm = (x + res - 1) % res;
            let xp = (x + 1) % res;
            let i = z * res + x;
            let dx_small = (small[z * res + xp] - small[z * res + xm]) / (2.0 * mpc);
            let dz_small = (small[zp * res + x] - small[zn * res + x]) / (2.0 * mpc);
            let dx_broad = (broad[z * res + xp] - broad[z * res + xm]) / (2.0 * mpc);
            let dz_broad = (broad[zp * res + x] - broad[zn * res + x]) / (2.0 * mpc);
            let dx = dx_small * 0.68 + dx_broad * 0.32;
            let dz = dz_small * 0.68 + dz_broad * 0.32;
            let local_slope = dx.hypot(dz);
            slope[i] = local_slope;
            shade[i] = unit_to_u8((hillshade(dx, dz) - 0.55) / 0.9);
            let ridge_strength = smoothstep(8.0, 90.0, (small[i] - broad[i]).max(0.0))
                * smoothstep(0.08, 0.8, local_slope);
            ridge[i] = unit_to_u8(ridge_strength);

            if map.land_mask[i] != 0 {
                let mut touches_sea = false;
                for oz in -1..=1 {
                    let nz = (z as i32 + oz).clamp(0, res as i32 - 1) as usize;
                    for ox in -1..=1 {
                        let nx = (x as i32 + ox).rem_euclid(res as i32) as usize;
                        if map.land_mask[nz * res + nx] == 0 {
                            touches_sea = true;
                        }
                    }
                }
                coast[i] = if touches_sea { 255 } else { 0 };
                let nx = x as f32 / res as f32;
                let nz = z as f32 / res as f32;
                let large = value_noise(config_seed(map) ^ 0xF043_57A1, nx * 32.0, nz * 32.0, 32);
                let detail =
                    value_noise(config_seed(map) ^ 0xC4A0_9E21, nx * 128.0, nz * 128.0, 128);
                let clusters = large * 0.72 + detail * 0.28;
                let elevation_gate = 1.0 - smoothstep(720.0, 1500.0, map.elevation_m[i]);
                let slope_gate = 1.0 - smoothstep(0.34, 1.35, local_slope);
                forest[i] =
                    unit_to_u8(smoothstep(0.37, 0.54, clusters) * elevation_gate * slope_gate);
            }
        }
    }

    DerivedFields {
        coast,
        slope,
        shade,
        ridge,
        forest,
    }
}

fn config_seed(map: &GlobalMap) -> u64 {
    map.config.seed
}

fn box_blur(input: &[f32], side: usize, radius: usize) -> Vec<f32> {
    if radius == 0 {
        return input.to_vec();
    }
    let width = radius * 2 + 1;
    let mut horizontal = vec![0.0f32; input.len()];
    for z in 0..side {
        let row = z * side;
        let mut sum = 0.0;
        for dx in -(radius as i32)..=radius as i32 {
            let x = dx.rem_euclid(side as i32) as usize;
            sum += input[row + x];
        }
        for x in 0..side {
            horizontal[row + x] = sum / width as f32;
            let remove_x = (x as i32 - radius as i32).rem_euclid(side as i32) as usize;
            let add_x = (x + radius + 1) % side;
            sum += input[row + add_x] - input[row + remove_x];
        }
    }

    let mut output = vec![0.0f32; input.len()];
    for x in 0..side {
        let mut sum = 0.0;
        for dz in -(radius as i32)..=radius as i32 {
            let z = dz.clamp(0, side as i32 - 1) as usize;
            sum += horizontal[z * side + x];
        }
        for z in 0..side {
            output[z * side + x] = sum / width as f32;
            let remove_z = (z as i32 - radius as i32).clamp(0, side as i32 - 1) as usize;
            let add_z = (z + radius + 1).min(side - 1);
            sum += horizontal[add_z * side + x] - horizontal[remove_z * side + x];
        }
    }
    output
}

fn raster_rivers(river_map: &rivers::RiverMap, res: usize) -> Vec<u8> {
    let mut mask = vec![0u8; res * res];
    let max_flow = river_map.max_flow().max(1.0).ln_1p();
    for river in &river_map.rivers {
        for (index, &(x, z)) in river.points.iter().enumerate() {
            let flow = river.flow.get(index).copied().unwrap_or(1.0).ln_1p() / max_flow;
            let radius = if flow > 0.72 { 1 } else { 0 };
            stamp_mask(&mut mask, res, x as i32, z as i32, radius, unit_to_u8(flow));
        }
    }
    mask
}

fn raster_roads(road_net: &roads::RoadNetwork, res: usize) -> Vec<u8> {
    let mut mask = vec![0u8; res * res];
    let max_length = road_net
        .roads
        .iter()
        .map(|road| road.points.len())
        .max()
        .unwrap_or(1) as f32;
    let max_log_length = max_length.ln_1p();
    for road in &road_net.roads {
        let normalized_length = (road.points.len() as f32).ln_1p() / max_log_length;
        let importance = smoothstep(0.34, 1.0, normalized_length);
        let value = unit_to_u8(0.12 + importance * 0.88);
        for &(x, z) in &road.points {
            stamp_mask(&mut mask, res, x as i32, z as i32, 0, value);
        }
    }
    mask
}

fn stamp_mask(mask: &mut [u8], res: usize, x: i32, z: i32, radius: i32, value: u8) {
    for oz in -radius..=radius {
        let nz = z + oz;
        if !(0..res as i32).contains(&nz) {
            continue;
        }
        for ox in -radius..=radius {
            if ox * ox + oz * oz > radius * radius {
                continue;
            }
            let nx = (x + ox).rem_euclid(res as i32) as usize;
            let i = nz as usize * res + nx;
            mask[i] = mask[i].max(value);
        }
    }
}

struct MacroWorld {
    config: WorldGenConfig,
    land: Vec<u8>,
    elevation: Vec<f32>,
    dist_to_land: Vec<u16>,
    river: Vec<u8>,
    road: Vec<u8>,
    coast: Vec<u8>,
    slope: Vec<f32>,
    shade: Vec<u8>,
    ridge: Vec<u8>,
    forest: Vec<u8>,
}

#[derive(Clone, Copy)]
struct MacroSample {
    land: bool,
    height: f32,
    dist_to_land_m: f32,
    river: f32,
    road: f32,
    coast: f32,
    slope: f32,
    shade: f32,
    ridge: f32,
    forest: f32,
}

impl MacroWorld {
    fn sample(&self, wx: f32, wz: f32) -> MacroSample {
        let res = self.config.global_res as usize;
        let mpc = self.config.meters_per_cell();
        let half = self.config.world_size_m as f32 * 0.5;
        let world = self.config.world_size_m as f32;
        let fx = (wx + half).rem_euclid(world) / mpc - 0.5;
        let fz = ((wz + half) / mpc - 0.5).clamp(0.0, res as f32 - 1.0);
        let x0 = fx.floor() as i32;
        let z0 = fz.floor() as i32;
        let tx = fx - x0 as f32;
        let tz = fz - z0 as f32;
        let x1 = x0 + 1;
        let z1 = (z0 + 1).min(res as i32 - 1);
        let x0 = x0.rem_euclid(res as i32) as usize;
        let x1 = x1.rem_euclid(res as i32) as usize;
        let z0 = z0.clamp(0, res as i32 - 1) as usize;
        let z1 = z1 as usize;
        let indices = [z0 * res + x0, z0 * res + x1, z1 * res + x0, z1 * res + x1];
        let land = bilinear_u8(&self.land, indices, tx, tz) >= 0.5;
        MacroSample {
            land,
            height: bilinear_f32(&self.elevation, indices, tx, tz),
            dist_to_land_m: bilinear_u16(&self.dist_to_land, indices, tx, tz) * mpc,
            river: bilinear_u8(&self.river, indices, tx, tz),
            road: bilinear_u8(&self.road, indices, tx, tz),
            coast: bilinear_u8(&self.coast, indices, tx, tz),
            slope: bilinear_f32(&self.slope, indices, tx, tz),
            shade: 0.55 + bilinear_u8(&self.shade, indices, tx, tz) * 0.9,
            ridge: bilinear_u8(&self.ridge, indices, tx, tz),
            forest: bilinear_u8(&self.forest, indices, tx, tz),
        }
    }
}

enum SemanticSource {
    Raw(RawRegion),
    Legacy(RgbImage),
    Generated,
}

fn load_semantic_source(
    legacy_source: &Path,
    terrain: &Path,
    rx: i32,
    rz: i32,
) -> Result<SemanticSource> {
    if let Some(raw) = RawRegion::load(terrain, rx, rz)? {
        return Ok(SemanticSource::Raw(raw));
    }
    let path = legacy_tile_path(legacy_source, rx, rz);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SemanticSource::Generated)
        }
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    let image = image::load_from_memory(&bytes)
        .with_context(|| format!("decode {}", path.display()))?
        .to_rgb8();
    if image.dimensions() != (REGION_PX as u32, REGION_PX as u32) {
        return Ok(SemanticSource::Generated);
    }
    if legacy_palette_score(&image) >= 0.82 {
        Ok(SemanticSource::Legacy(image))
    } else {
        Ok(SemanticSource::Generated)
    }
}

struct RawRegion {
    heights: Vec<f32>,
    splat: Vec<u8>,
}

impl RawRegion {
    fn load(terrain: &Path, rx: i32, rz: i32) -> Result<Option<Self>> {
        if !coords::height_region_dir(terrain, rx, rz).is_dir()
            || !coords::splat_region_dir(terrain, rx, rz).is_dir()
        {
            return Ok(None);
        }
        let mut heights = vec![0.0f32; (REGION_PX + 1) * (REGION_PX + 1)];
        let mut splat = vec![0u8; REGION_PX * REGION_PX * 4];
        for local_z in 0..TILES_PER_REGION {
            for local_x in 0..TILES_PER_REGION {
                let tx = rx * TILES_PER_REGION + local_x;
                let tz = rz * TILES_PER_REGION + local_z;
                let height_path = coords::heightmap_path(terrain, tx, tz);
                let splat_path = coords::splatmap_path(terrain, tx, tz);
                let height_bytes = match std::fs::read(&height_path) {
                    Ok(bytes) if bytes.len() == VERTS_PER_SIDE * VERTS_PER_SIDE * 2 => bytes,
                    Ok(_) => return Ok(None),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(error) => {
                        return Err(error)
                            .with_context(|| format!("read {}", height_path.display()))
                    }
                };
                let splat_bytes = match std::fs::read(&splat_path) {
                    Ok(bytes) if bytes.len() == TILE_DIM * TILE_DIM * 4 => bytes,
                    Ok(_) => return Ok(None),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(error) => {
                        return Err(error).with_context(|| format!("read {}", splat_path.display()))
                    }
                };
                let base_x = local_x as usize * TILE_DIM;
                let base_z = local_z as usize * TILE_DIM;
                for vz in 0..VERTS_PER_SIDE {
                    for vx in 0..VERTS_PER_SIDE {
                        let source = (vz * VERTS_PER_SIDE + vx) * 2;
                        let encoded =
                            u16::from_le_bytes([height_bytes[source], height_bytes[source + 1]]);
                        heights[(base_z + vz) * (REGION_PX + 1) + base_x + vx] =
                            encoded as f32 * HEIGHT_STEP - HEIGHT_BIAS;
                    }
                }
                for z in 0..TILE_DIM {
                    let source = z * TILE_DIM * 4;
                    let target = ((base_z + z) * REGION_PX + base_x) * 4;
                    splat[target..target + TILE_DIM * 4]
                        .copy_from_slice(&splat_bytes[source..source + TILE_DIM * 4]);
                }
            }
        }
        Ok(Some(Self { heights, splat }))
    }

    fn semantic(&self, x: usize, z: usize) -> Semantic {
        let height = self.cell_height(x, z);
        let left = self.cell_height(x.saturating_sub(1), z);
        let right = self.cell_height((x + 1).min(REGION_PX - 1), z);
        let top = self.cell_height(x, z.saturating_sub(1));
        let bottom = self.cell_height(x, (z + 1).min(REGION_PX - 1));
        let slope_x = (right - left) * 0.5;
        let slope_z = (bottom - top) * 0.5;
        let i = (z * REGION_PX + x) * 4;
        let packed = self.splat[i];
        let primary = (packed >> 4) & 0x0f;
        let secondary = packed & 0x0f;
        let secondary_weight = self.splat[i + 2] as f32 / 255.0;
        let primary_weight = 1.0 - secondary_weight;
        let weight = |layer: u8| {
            (if primary == layer {
                primary_weight
            } else {
                0.0
            }) + if secondary == layer {
                secondary_weight
            } else {
                0.0
            }
        };
        Semantic {
            land: height >= -0.25,
            river: weight(PAL_RIVER_BED),
            road: (weight(PAL_ROAD) + weight(PAL_STONE_PATH) + weight(PAL_PAVING)).clamp(0.0, 1.0),
            sand: weight(PAL_SAND),
            rock: (weight(PAL_CLIFF) + weight(PAL_DIRT) * 0.6).clamp(0.0, 1.0),
            snow: weight(PAL_SNOW),
            height: Some(height),
            slope: Some((slope_x, slope_z)),
        }
    }

    fn cell_height(&self, x: usize, z: usize) -> f32 {
        let side = REGION_PX + 1;
        let h00 = self.heights[z * side + x];
        let h10 = self.heights[z * side + x + 1];
        let h01 = self.heights[(z + 1) * side + x];
        let h11 = self.heights[(z + 1) * side + x + 1];
        (h00 + h10 + h01 + h11) * 0.25
    }
}

#[derive(Clone, Copy)]
struct Semantic {
    land: bool,
    river: f32,
    road: f32,
    sand: f32,
    rock: f32,
    snow: f32,
    height: Option<f32>,
    slope: Option<(f32, f32)>,
}

impl Semantic {
    fn generated(sample: MacroSample) -> Self {
        Self {
            land: sample.land,
            river: sample.river,
            road: sample.road,
            sand: sample.coast,
            rock: 0.0,
            snow: 0.0,
            height: None,
            slope: None,
        }
    }

    fn legacy(pixel: [u8; 3], sample: MacroSample) -> Self {
        let deep_water = proximity(pixel, [30, 60, 150], 26.0);
        let shallow_water = proximity(pixel, [100, 160, 220], 28.0);
        let river_blue = ((pixel[2] as f32 - pixel[0] as f32 - 35.0) / 75.0).clamp(0.0, 1.0)
            * ((pixel[2] as f32 - pixel[1] as f32 - 12.0) / 55.0).clamp(0.0, 1.0);
        let land = deep_water < 0.58 && (shallow_water <= 0.65 || sample.land);
        let spread = *pixel.iter().max().unwrap() as f32 - *pixel.iter().min().unwrap() as f32;
        let neutral = (1.0 - smoothstep(18.0, 42.0, spread)).clamp(0.0, 1.0);
        let road = proximity(pixel, [140, 135, 125], 38.0).max(
            proximity(pixel, [160, 155, 150], 34.0).max(proximity(pixel, [170, 160, 145], 34.0)),
        ) * neutral;
        let exact_river = shallow_water.max(river_blue);
        let river_importance = smoothstep(0.025, 0.48, sample.river);
        let road_importance = smoothstep(0.035, 0.58, sample.road);
        Self {
            land,
            river: if land {
                exact_river * river_importance
            } else {
                0.0
            },
            road: road * road_importance,
            sand: proximity(pixel, [210, 185, 110], 44.0).max(sample.coast * 0.5),
            // Bare-mountain browns must survive even where the macro world has
            // no relief (data-only massifs like the snow mountains), so the
            // baseline carries most of the weight and macro terms only add.
            rock: proximity(pixel, [130, 110, 90], 52.0).max(proximity(
                pixel,
                [180, 100, 60],
                54.0,
            )) * (0.52
                + smoothstep(300.0, 1050.0, sample.height) * 0.18
                + smoothstep(0.12, 0.72, sample.slope) * 0.18
                + sample.ridge * 0.12),
            snow: proximity(pixel, [240, 240, 245], 42.0)
                * smoothstep(650.0, 1450.0, sample.height),
            height: None,
            slope: None,
        }
    }
}

struct Textures {
    atlas_guide: RgbImage,
    ocean: RgbImage,
    lowland: RgbImage,
    forest: RgbImage,
    rock: RgbImage,
}

impl Textures {
    fn load() -> Result<Self> {
        let atlas_guide = decode_texture(
            include_bytes!("../assets/world-map/world-atlas-guide.png"),
            "world atlas guide",
        )?;
        Ok(Self {
            atlas_guide,
            ocean: decode_texture(include_bytes!("../assets/world-map/ocean.png"), "ocean")?,
            lowland: decode_texture(include_bytes!("../assets/world-map/lowland.png"), "lowland")?,
            forest: decode_texture(include_bytes!("../assets/world-map/forest.png"), "forest")?,
            rock: decode_texture(
                include_bytes!("../assets/world-map/rock-albedo.png"),
                "rock albedo",
            )?,
        })
    }
}

fn decode_texture(bytes: &[u8], name: &str) -> Result<RgbImage> {
    Ok(image::load_from_memory(bytes)
        .with_context(|| format!("decode embedded {} map texture", name))?
        .to_rgb8())
}

struct RegionRender {
    image: RgbImage,
    road: Vec<u8>,
    river: Vec<u8>,
    land: Vec<u8>,
    forest: Vec<u8>,
}

impl RegionRender {
    fn size(&self) -> u32 {
        self.image.width()
    }

    fn downsampled(&self, gamma: &GammaLut) -> Self {
        downsample_half(self, gamma)
    }
}

fn render_region(
    rx: i32,
    rz: i32,
    world: &MacroWorld,
    textures: &Textures,
    source: &SemanticSource,
) -> RegionRender {
    let mut image = RgbImage::new(REGION_PX as u32, REGION_PX as u32);
    let mut road = vec![0u8; REGION_PX * REGION_PX];
    let mut river = vec![0u8; REGION_PX * REGION_PX];
    let mut land = vec![0u8; REGION_PX * REGION_PX];
    let mut forest = vec![0u8; REGION_PX * REGION_PX];
    for z in 0..REGION_PX {
        for x in 0..REGION_PX {
            let wx = region_world_coord(rx, x);
            let wz = region_world_coord(rz, z);
            let sample = world.sample(wx, wz);
            let semantic = match source {
                SemanticSource::Raw(raw) => raw.semantic(x, z),
                SemanticSource::Legacy(legacy) => {
                    Semantic::legacy(legacy.get_pixel(x as u32, z as u32).0, sample)
                }
                SemanticSource::Generated => Semantic::generated(sample),
            };
            let i = z * REGION_PX + x;
            let color = if semantic.land {
                let (color, forest_strength) = render_land(wx, wz, sample, semantic, textures);
                forest[i] = unit_to_u8(forest_strength);
                color
            } else {
                render_ocean(wx, wz, sample, textures)
            };
            image.put_pixel(x as u32, z as u32, Rgb(color));
            road[i] = unit_to_u8(semantic.road);
            river[i] = unit_to_u8(semantic.river);
            land[i] = if semantic.land { 255 } else { 0 };
        }
    }
    RegionRender {
        image,
        road,
        river,
        land,
        forest,
    }
}

fn render_land(
    wx: f32,
    wz: f32,
    sample: MacroSample,
    semantic: Semantic,
    textures: &Textures,
) -> ([u8; 3], f32) {
    let height = semantic.height.unwrap_or(sample.height);
    let (slope, shade) = if let Some((dx, dz)) = semantic.slope {
        let local = hillshade(dx, dz);
        (dx.hypot(dz), sample.shade * 0.58 + local * 0.42)
    } else {
        (sample.slope, sample.shade)
    };
    let relief_gate = smoothstep(0.28, 0.95, slope).max(sample.ridge * 0.65);
    let mut rock = smoothstep(680.0, 1500.0, height) * (0.12 + relief_gate * 0.34);
    rock = rock
        .max(smoothstep(0.42, 1.35, slope) * smoothstep(220.0, 900.0, height) * 0.52)
        .max(sample.ridge * 0.46)
        .max(semantic.rock * 0.95)
        .max(semantic.snow * 0.72)
        .clamp(0.0, 1.0);
    let forest = (sample.forest.powf(0.72) * (1.0 - rock * 0.55) * (1.0 - semantic.sand * 0.72))
        .clamp(0.0, 1.0);
    let valley_green = (1.0 - smoothstep(420.0, 1100.0, height))
        * (1.0 - smoothstep(0.14, 0.80, slope))
        * (1.0 - semantic.sand);
    let lowland = sample_texture(&textures.lowland, wx, wz, 48.0);
    let mut local = tint(lowland, [0.72, 0.96 + valley_green * 0.08, 0.62]);
    if forest > 0.01 {
        let forest_color = sample_texture(&textures.forest, wx, wz, 32.0);
        local = mix(local, tint(forest_color, [0.76, 0.98, 0.68]), forest * 0.82);
    }
    if rock > 0.01 {
        let rock_color = sample_texture(&textures.rock, wx, wz, 52.0);
        local = mix(local, tint(rock_color, [0.94, 0.93, 0.88]), rock * 0.68);
    }
    let shore = semantic.sand.max(sample.coast * 0.55) * (1.0 - rock);
    local = mix(local, [185.0, 166.0, 107.0], shore * 0.48);
    let snow = semantic.snow.clamp(0.0, 1.0);
    if snow > 0.01 {
        local = mix(local, [236.0, 241.0, 247.0], snow * 0.9);
    }
    let relief = 0.88 + shade * 0.12 + sample.ridge * rock * 0.035;
    local = scale(local, relief.clamp(0.84, 1.16));
    let guide = sample_world_guide(&textures.atlas_guide, wx, wz);
    let guide_weight =
        BASE_GUIDE_WEIGHT * guide_material_match(guide, 1.0, forest) * (1.0 - snow * 0.7);
    (to_rgb(mix(local, guide, guide_weight)), forest)
}

fn render_ocean(wx: f32, wz: f32, sample: MacroSample, textures: &Textures) -> [u8; 3] {
    let texture = sample_texture(&textures.ocean, wx, wz, 48.0);
    let mut local = mix([5.0, 25.0, 55.0], tint(texture, [0.56, 0.68, 0.80]), 0.74);
    let shelf = 1.0 - smoothstep(12.0, 125.0, sample.dist_to_land_m);
    let foam = 1.0 - smoothstep(4.0, 18.0, sample.dist_to_land_m);
    local = mix(local, [18.0, 112.0, 139.0], shelf * 0.58);
    let luminance = (texture[0] + texture[1] + texture[2]) / (255.0 * 3.0);
    let sparkle = luminance * luminance * luminance;
    local = mix(local, [93.0, 199.0, 196.0], foam * (0.24 + sparkle * 0.34));
    local = scale(local, 0.97 + (sample.shade - 1.0) * shelf * 0.06);
    let guide = sample_world_guide(&textures.atlas_guide, wx, wz);
    let guide_weight = BASE_GUIDE_WEIGHT * guide_water_likelihood(guide);
    to_rgb(mix(local, guide, guide_weight))
}

fn sample_world_guide(image: &RgbImage, wx: f32, wz: f32) -> [f32; 3] {
    let world = WORLD_SIZE_M;
    let nx = (wx + world * 0.5).rem_euclid(world) / world;
    let nz = ((wz + world * 0.5) / world).clamp(0.0, 1.0);
    let x = nx * image.width() as f32;
    let y = nz * (image.height() - 1) as f32;
    let x0 = x.floor() as i64;
    let y0 = y.floor() as u32;
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let x0 = x0.rem_euclid(image.width() as i64) as u32;
    let x1 = (x0 + 1) % image.width();
    let y1 = (y0 + 1).min(image.height() - 1);
    let p00 = image.get_pixel(x0, y0).0;
    let p10 = image.get_pixel(x1, y0).0;
    let p01 = image.get_pixel(x0, y1).0;
    let p11 = image.get_pixel(x1, y1).0;
    let mut color = [0.0; 3];
    for channel in 0..3 {
        let top = p00[channel] as f32 * (1.0 - tx) + p10[channel] as f32 * tx;
        let bottom = p01[channel] as f32 * (1.0 - tx) + p11[channel] as f32 * tx;
        color[channel] = top * (1.0 - ty) + bottom * ty;
    }
    color
}

fn guide_water_likelihood(color: [f32; 3]) -> f32 {
    let blue_over_green = smoothstep(9.0, 38.0, color[2] - color[1]);
    let blue_over_red = smoothstep(18.0, 62.0, color[2] - color[0]);
    blue_over_green * blue_over_red
}

/// How strongly a guide pixel reads as forest canopy: dark, green-dominant.
/// Bright field greens and gray rock both fall to 0.
fn guide_forest_likelihood(color: [f32; 3]) -> f32 {
    let luminance = (color[0] + color[1] + color[2]) / 3.0;
    let darkness = 1.0 - smoothstep(55.0, 95.0, luminance);
    let green_excess = smoothstep(2.0, 14.0, color[1] - color[0].max(color[2]));
    darkness * green_excess
}

fn sample_texture(image: &RgbImage, wx: f32, wz: f32, cycles: f32) -> [f32; 3] {
    let world = WORLD_SIZE_M;
    let x = (wx + world * 0.5).rem_euclid(world) / world;
    let z = (wz + world * 0.5) / world;
    let base = even_cycles(cycles);
    let layers = [
        sample_texture_layer(image, x, z, base, [0.173, 0.611]),
        sample_texture_layer(image, z, 1.0 - x, base + 14, [0.439, 0.127]),
        sample_texture_layer(image, 1.0 - x, 1.0 - z, base * 2 - 2, [0.733, 0.347]),
    ];
    [
        layers[0][0] * 0.42 + layers[1][0] * 0.34 + layers[2][0] * 0.24,
        layers[0][1] * 0.42 + layers[1][1] * 0.34 + layers[2][1] * 0.24,
        layers[0][2] * 0.42 + layers[1][2] * 0.34 + layers[2][2] * 0.24,
    ]
}

fn sample_texture_layer(
    image: &RgbImage,
    x: f32,
    z: f32,
    cycles: i32,
    offset: [f32; 2],
) -> [f32; 3] {
    let u = (x * cycles as f32 + offset[0]) * image.width() as f32;
    let v = (z * cycles as f32 + offset[1]) * image.height() as f32;
    let x = mirror_index(u.floor() as i64, image.width());
    let y = mirror_index(v.floor() as i64, image.height());
    let pixel = image.get_pixel(x, y).0;
    [pixel[0] as f32, pixel[1] as f32, pixel[2] as f32]
}

fn even_cycles(cycles: f32) -> i32 {
    let rounded = (cycles.round() as i32).max(2);
    if rounded % 2 == 0 {
        rounded
    } else {
        rounded + 1
    }
}

fn mirror_index(value: i64, size: u32) -> u32 {
    let size = size as i64;
    let value = value.rem_euclid(size * 2);
    if value < size {
        value as u32
    } else {
        (size * 2 - 1 - value) as u32
    }
}

struct GammaLut {
    to_linear: [f32; 256],
    to_srgb: [u8; 4097],
}

impl GammaLut {
    fn new() -> Self {
        let mut to_linear = [0.0; 256];
        for (index, value) in to_linear.iter_mut().enumerate() {
            *value = srgb_to_linear(index as f32 / 255.0);
        }
        let mut to_srgb = [0u8; 4097];
        for (index, value) in to_srgb.iter_mut().enumerate() {
            *value = (linear_to_srgb(index as f32 / 4096.0) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
        Self { to_linear, to_srgb }
    }

    fn encode(&self, linear: f32) -> u8 {
        let index = (linear.clamp(0.0, 1.0) * 4096.0).round() as usize;
        self.to_srgb[index]
    }
}

fn downsample_half(source: &RegionRender, gamma: &GammaLut) -> RegionRender {
    let width = source.image.width() as usize;
    let height = source.image.height() as usize;
    let out_width = width / 2;
    let out_height = height / 2;
    let mut image = RgbImage::new(out_width as u32, out_height as u32);
    let mut road = vec![0u8; out_width * out_height];
    let mut river = vec![0u8; out_width * out_height];
    let mut land = vec![0u8; out_width * out_height];
    let mut forest = vec![0u8; out_width * out_height];
    for y in 0..out_height {
        for x in 0..out_width {
            let source_pixels = [
                source.image.get_pixel((x * 2) as u32, (y * 2) as u32),
                source.image.get_pixel((x * 2 + 1) as u32, (y * 2) as u32),
                source.image.get_pixel((x * 2) as u32, (y * 2 + 1) as u32),
                source
                    .image
                    .get_pixel((x * 2 + 1) as u32, (y * 2 + 1) as u32),
            ];
            let mut rgb = [0u8; 3];
            for channel in 0..3 {
                let linear = source_pixels
                    .iter()
                    .map(|pixel| gamma.to_linear[pixel[channel] as usize])
                    .sum::<f32>()
                    * 0.25;
                rgb[channel] = gamma.encode(linear);
            }
            let source_indices = [
                y * 2 * width + x * 2,
                y * 2 * width + x * 2 + 1,
                (y * 2 + 1) * width + x * 2,
                (y * 2 + 1) * width + x * 2 + 1,
            ];
            let target = y * out_width + x;
            road[target] = source_indices
                .iter()
                .map(|&i| source.road[i])
                .max()
                .unwrap_or(0);
            river[target] = source_indices
                .iter()
                .map(|&i| source.river[i])
                .max()
                .unwrap_or(0);
            land[target] = ((source_indices
                .iter()
                .map(|&i| source.land[i] as u16)
                .sum::<u16>()
                + 2)
                / 4) as u8;
            forest[target] = ((source_indices
                .iter()
                .map(|&i| source.forest[i] as u16)
                .sum::<u16>()
                + 2)
                / 4) as u8;
            image.put_pixel(x as u32, y as u32, Rgb(rgb));
        }
    }
    RegionRender {
        image,
        road,
        river,
        land,
        forest,
    }
}

fn legacy_palette_score(image: &RgbImage) -> f32 {
    let mut matching = 0usize;
    let mut total = 0usize;
    for y in (0..REGION_PX).step_by(16) {
        for x in (0..REGION_PX).step_by(16) {
            let pixel = image.get_pixel(x as u32, y as u32).0;
            let mut best = f32::MAX;
            for a in LEGACY_COLORS {
                for b in LEGACY_COLORS {
                    best = best.min(point_segment_distance_sq(pixel, a, b));
                }
            }
            if best <= 49.0 {
                matching += 1;
            }
            total += 1;
        }
    }
    matching as f32 / total as f32
}

fn point_segment_distance_sq(point: [u8; 3], a: [u8; 3], b: [u8; 3]) -> f32 {
    let p = [point[0] as f32, point[1] as f32, point[2] as f32];
    let a = [a[0] as f32, a[1] as f32, a[2] as f32];
    let d = [b[0] as f32 - a[0], b[1] as f32 - a[1], b[2] as f32 - a[2]];
    let length_sq = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    let t = if length_sq > 0.0 {
        (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1] + (p[2] - a[2]) * d[2]) / length_sq)
            .clamp(0.0, 1.0)
    } else {
        0.0
    };
    let dx = p[0] - (a[0] + d[0] * t);
    let dy = p[1] - (a[1] + d[1] * t);
    let dz = p[2] - (a[2] + d[2] * t);
    dx * dx + dy * dy + dz * dz
}

fn value_noise(seed: u64, x: f32, z: f32, period_x: i32) -> f32 {
    let x0 = x.floor() as i32;
    let z0 = z.floor() as i32;
    let tx = smooth_curve(x - x0 as f32);
    let tz = smooth_curve(z - z0 as f32);
    let value = |ix: i32, iz: i32| hash_unit(seed, ix.rem_euclid(period_x), iz);
    let a = lerp(value(x0, z0), value(x0 + 1, z0), tx);
    let b = lerp(value(x0, z0 + 1), value(x0 + 1, z0 + 1), tx);
    lerp(a, b, tz)
}

fn hash_unit(seed: u64, x: i32, z: i32) -> f32 {
    let mut value = seed
        ^ (x as u32 as u64).wrapping_mul(0x9E37_79B1_85EB_CA87)
        ^ (z as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    value ^= value >> 30;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^= value >> 31;
    (value >> 40) as f32 / (1u32 << 24) as f32
}

fn hillshade(dx: f32, dz: f32) -> f32 {
    let nx = -dx;
    let ny = 1.0;
    let nz = -dz;
    let inv_len = 1.0 / (nx * nx + ny * ny + nz * nz).sqrt().max(1e-5);
    let dot = nx * inv_len * -0.48 + ny * inv_len * 0.75 + nz * inv_len * -0.45;
    (0.44 + dot * 0.75).clamp(0.55, 1.35)
}

fn bilinear_f32(values: &[f32], indices: [usize; 4], tx: f32, tz: f32) -> f32 {
    let top = lerp(values[indices[0]], values[indices[1]], tx);
    let bottom = lerp(values[indices[2]], values[indices[3]], tx);
    lerp(top, bottom, tz)
}

fn bilinear_u8(values: &[u8], indices: [usize; 4], tx: f32, tz: f32) -> f32 {
    let top = lerp(
        values[indices[0]] as f32 / 255.0,
        values[indices[1]] as f32 / 255.0,
        tx,
    );
    let bottom = lerp(
        values[indices[2]] as f32 / 255.0,
        values[indices[3]] as f32 / 255.0,
        tx,
    );
    lerp(top, bottom, tz)
}

fn bilinear_u16(values: &[u16], indices: [usize; 4], tx: f32, tz: f32) -> f32 {
    let top = lerp(values[indices[0]] as f32, values[indices[1]] as f32, tx);
    let bottom = lerp(values[indices[2]] as f32, values[indices[3]] as f32, tx);
    lerp(top, bottom, tz)
}

fn proximity(pixel: [u8; 3], target: [u8; 3], radius: f32) -> f32 {
    let dr = pixel[0] as f32 - target[0] as f32;
    let dg = pixel[1] as f32 - target[1] as f32;
    let db = pixel[2] as f32 - target[2] as f32;
    (1.0 - (dr * dr + dg * dg + db * db) / (radius * radius)).clamp(0.0, 1.0)
}

fn tint(color: [f32; 3], tint: [f32; 3]) -> [f32; 3] {
    [color[0] * tint[0], color[1] * tint[1], color[2] * tint[2]]
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

fn region_world_coord(region: i32, pixel: usize) -> f32 {
    region as f32 * REGION_PX as f32 - 32.0 + pixel as f32 + 0.5
}

fn lod_region_world_coord(region: i32, pixel: usize, size: u32) -> f32 {
    let footprint = REGION_PX as f32 / size as f32;
    region as f32 * REGION_PX as f32 - 32.0 + (pixel as f32 + 0.5) * footprint
}

fn region_coordinates(region_min: (i32, i32), region_max: (i32, i32)) -> Vec<(i32, i32)> {
    let mut regions = Vec::new();
    for rz in region_min.1..=region_max.1 {
        for rx in region_min.0..=region_max.0 {
            regions.push((rx, rz));
        }
    }
    regions
}

/// `root` is the family directory itself (the CLI's `--legacy-source` /
/// `--out`), so these build the in-family layout rather than taking a terrain
/// base dir — the filename format still comes from `coords`.
fn legacy_tile_path(root: &Path, rx: i32, rz: i32) -> PathBuf {
    root.join(coords::region_tile_name(rx, rz, LEGACY_FAMILY.ext()))
}

fn tile_path(root: &Path, size: u32, rx: i32, rz: i32) -> PathBuf {
    let name = coords::region_tile_name(rx, rz, FANTASY_FAMILY.ext());
    if size >= coords::MINIMAP_BASE_SIZE {
        root.join(name)
    } else {
        root.join(size.to_string()).join(name)
    }
}

#[derive(Clone, Copy)]
struct FeatureStyle {
    river_threshold: f32,
    river_opacity: f32,
    road_threshold: f32,
    road_opacity: f32,
}

impl FeatureStyle {
    fn for_size(size: u32) -> Self {
        match size {
            128 => Self {
                river_threshold: 0.42,
                river_opacity: 0.52,
                road_threshold: 0.52,
                road_opacity: 0.36,
            },
            256 => Self {
                river_threshold: 0.24,
                river_opacity: 0.64,
                road_threshold: 0.34,
                road_opacity: 0.48,
            },
            512 => Self {
                river_threshold: 0.11,
                river_opacity: 0.76,
                road_threshold: 0.19,
                road_opacity: 0.60,
            },
            _ => Self {
                river_threshold: 0.035,
                river_opacity: 0.88,
                road_threshold: 0.075,
                road_opacity: 0.72,
            },
        }
    }
}

fn compose_features(image: &mut RgbImage, road: &[u8], river: &[u8], style: FeatureStyle) {
    for (index, pixel) in image.pixels_mut().enumerate() {
        let mut color = [pixel[0] as f32, pixel[1] as f32, pixel[2] as f32];
        let river_strength = river[index] as f32 / 255.0;
        if river_strength > style.river_threshold {
            let alpha =
                smoothstep(style.river_threshold, 1.0, river_strength) * style.river_opacity;
            color = mix(color, [34.0, 119.0, 151.0], alpha);
        }
        let road_strength = road[index] as f32 / 255.0;
        if road_strength > style.road_threshold {
            let alpha = smoothstep(style.road_threshold, 1.0, road_strength) * style.road_opacity;
            color = mix(color, [168.0, 149.0, 111.0], alpha);
        }
        *pixel = Rgb(to_rgb(color));
    }
}

fn guide_weight_for_size(size: u32) -> f32 {
    match size {
        128 => 0.92,
        256 => 0.80,
        512 => 0.64,
        _ => BASE_GUIDE_WEIGHT,
    }
}

/// How much the guide may show through at this pixel. Water/land agreement
/// gates everywhere; on land, painted canopy must also agree with the real
/// forest strength so art direction cannot swallow real fields at far zoom.
fn guide_material_match(guide: [f32; 3], land_coverage: f32, real_forest: f32) -> f32 {
    let guide_water = guide_water_likelihood(guide);
    let veg_match = 1.0 - (guide_forest_likelihood(guide) - real_forest).abs();
    (land_coverage * (1.0 - guide_water) + (1.0 - land_coverage) * guide_water)
        * (land_coverage * veg_match + (1.0 - land_coverage))
}

fn additional_guide_alpha(target_weight: f32, material_match: f32) -> f32 {
    let base = (BASE_GUIDE_WEIGHT * material_match).clamp(0.0, 1.0);
    let target = (target_weight * material_match).clamp(base, 1.0);
    if target <= base || base >= 1.0 {
        0.0
    } else {
        (target - base) / (1.0 - base)
    }
}

fn blend_lod_guide(
    image: &mut RgbImage,
    land: &[u8],
    forest: &[u8],
    rx: i32,
    rz: i32,
    textures: &Textures,
) {
    let size = image.width();
    let target_weight = guide_weight_for_size(size);
    if target_weight <= BASE_GUIDE_WEIGHT {
        return;
    }
    let width = size as usize;
    for y in 0..image.height() {
        for x in 0..image.width() {
            let index = y as usize * width + x as usize;
            let wx = lod_region_world_coord(rx, x as usize, size);
            let wz = lod_region_world_coord(rz, y as usize, size);
            let guide = sample_world_guide(&textures.atlas_guide, wx, wz);
            let land_coverage = land[index] as f32 / 255.0;
            let material_match =
                guide_material_match(guide, land_coverage, forest[index] as f32 / 255.0);
            let alpha = additional_guide_alpha(target_weight, material_match);
            if alpha > 0.0 {
                let pixel = image.get_pixel_mut(x, y);
                let color = [pixel[0] as f32, pixel[1] as f32, pixel[2] as f32];
                *pixel = Rgb(to_rgb(mix(color, guide, alpha)));
            }
        }
    }
}

fn save_render(
    render: &RegionRender,
    rx: i32,
    rz: i32,
    textures: &Textures,
    path: &Path,
) -> Result<()> {
    let size = render.size();
    let mut image = render.image.clone();
    blend_lod_guide(&mut image, &render.land, &render.forest, rx, rz, textures);
    compose_features(
        &mut image,
        &render.road,
        &render.river,
        FeatureStyle::for_size(size),
    );
    save_webp(&image, path)
}

// Lossy quality 82: visually transparent for painterly tiles at ~1/8 the PNG size.
const WEBP_QUALITY: f32 = 82.0;

fn save_webp(image: &RgbImage, path: &Path) -> Result<()> {
    let encoded =
        webp::Encoder::from_rgb(image.as_raw(), image.width(), image.height()).encode(WEBP_QUALITY);
    std::fs::write(path, &*encoded).with_context(|| format!("write {}", path.display()))
}

fn publish_file(staged: &Path, final_path: &Path) -> Result<()> {
    // Also drop the PNG-era tile at this slot: the server no longer looks for
    // fantasy PNGs, so leaving one would waste disk and mislead a partial
    // rebake into thinking the region still serves.
    for stale in [final_path.to_path_buf(), final_path.with_extension("png")] {
        match std::fs::remove_file(&stale) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("replace {}", stale.display()))
            }
        }
    }
    std::fs::rename(staged, final_path).with_context(|| format!("publish {}", final_path.display()))
}

fn staging_path(out: &Path) -> Result<PathBuf> {
    let parent = out.parent().unwrap_or_else(|| Path::new("."));
    let name = out
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("minimap-fantasy");
    Ok(parent.join(format!(
        ".{}.world-map-staging-{}",
        name,
        std::process::id()
    )))
}

fn path_key(path: &Path) -> Result<String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn full_world_has_4096_unique_output_paths() {
        let regions = region_coordinates((-16, -16), (15, 15));
        let mut paths = HashSet::new();
        for (rx, rz) in regions {
            for size in [1024, 512, 256, 128] {
                paths.insert(tile_path(Path::new("out"), size, rx, rz));
            }
        }
        assert_eq!(paths.len(), 4096);
    }

    #[test]
    fn adjacent_regions_keep_one_meter_centers() {
        assert_eq!(region_world_coord(-2, 0), -2079.5);
        let last = region_world_coord(-2, REGION_PX - 1);
        let next = region_world_coord(-1, 0);
        assert_eq!(next - last, 1.0);
    }

    #[test]
    fn identical_input_and_output_paths_are_rejected() {
        assert_eq!(
            path_key(Path::new("data/terrain/minimap")).unwrap(),
            path_key(Path::new("./data/terrain/minimap")).unwrap()
        );
    }

    #[test]
    fn wrapped_x_boundary_samples_the_same_texture_phase() {
        let left = (-16384.0f32 + 16384.0).rem_euclid(32768.0);
        let right = (16384.0f32 + 16384.0).rem_euclid(32768.0);
        assert_eq!(left, right);
        let image = RgbImage::from_fn(7, 5, |x, y| {
            Rgb([(x * 31) as u8, (y * 47) as u8, ((x + y) * 19) as u8])
        });
        for z in [-16384.0, -1234.5, 0.0, 16383.0] {
            assert_eq!(
                sample_texture(&image, -16384.0, z, 32.0),
                sample_texture(&image, 16384.0, z, 32.0)
            );
        }
    }

    #[test]
    fn texture_sampler_uses_even_incommensurate_scales() {
        let base = even_cycles(32.0);
        assert_eq!([base, base + 14, base * 2 - 2], [32, 46, 62]);
        assert!([base, base + 14, base * 2 - 2]
            .iter()
            .all(|cycles| cycles % 2 == 0));
    }

    #[test]
    fn world_guide_is_bilinear_and_wraps_x() {
        let image = RgbImage::from_fn(2, 2, |x, y| Rgb([(x * 100) as u8, (y * 80) as u8, 20]));
        assert_eq!(
            sample_world_guide(&image, -16384.0, -16384.0),
            sample_world_guide(&image, 16384.0, -16384.0)
        );
        assert_eq!(
            sample_world_guide(&image, -8192.0, -16384.0),
            [50.0, 0.0, 20.0]
        );
    }

    #[test]
    fn guide_material_gate_rejects_obvious_mismatches() {
        assert!(guide_water_likelihood([8.0, 28.0, 92.0]) > 0.9);
        assert!(guide_water_likelihood([54.0, 86.0, 35.0]) < 0.01);
    }

    #[test]
    fn lod_guide_blend_reaches_target_weight() {
        for size in [1024, 512, 256, 128] {
            let target = guide_weight_for_size(size);
            for material_match in [0.0, 0.35, 1.0] {
                let base = BASE_GUIDE_WEIGHT * material_match;
                let alpha = additional_guide_alpha(target, material_match);
                let final_weight = base + (1.0 - base) * alpha;
                assert!((final_weight - target * material_match).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn lod_world_coordinates_use_pixel_footprint_centers() {
        assert_eq!(
            lod_region_world_coord(-2, 0, 1024),
            region_world_coord(-2, 0)
        );
        assert_eq!(lod_region_world_coord(0, 0, 512), -31.0);
        assert_eq!(
            lod_region_world_coord(0, 1, 128) - lod_region_world_coord(0, 0, 128),
            8.0
        );
    }

    #[test]
    fn river_raster_preserves_log_flow_strength() {
        let river_map = rivers::RiverMap {
            downstream: vec![None; 64],
            flow: vec![0.0; 64],
            rivers: vec![rivers::Polyline {
                points: vec![(0, 0), (3, 0), (6, 0)],
                flow: vec![1.0, 100.0, 10_000.0],
            }],
        };
        let mask = raster_rivers(&river_map, 8);
        assert!(mask[0] < mask[3]);
        assert!(mask[3] < mask[6]);
    }

    #[test]
    fn road_raster_prioritizes_long_routes() {
        let road_net = roads::RoadNetwork {
            roads: vec![
                roads::Road {
                    points: vec![(0, 0), (1, 0)],
                },
                roads::Road {
                    points: vec![(0, 3), (1, 3), (2, 3), (3, 3), (4, 3), (5, 3)],
                },
            ],
        };
        let mask = raster_roads(&road_net, 8);
        assert!(mask[0] < mask[3 * 8]);
    }

    #[test]
    fn lod_keeps_feature_mask_without_recomposing_color() {
        let image = RgbImage::from_pixel(2, 2, Rgb([80, 100, 70]));
        let source = RegionRender {
            image,
            road: vec![0; 4],
            river: vec![255, 0, 0, 0],
            land: vec![255, 255, 0, 0],
            forest: vec![255, 255, 0, 0],
        };
        let lod = downsample_half(&source, &GammaLut::new());
        assert_eq!(lod.river, vec![255]);
        assert_eq!(lod.land, vec![128]);
        assert_eq!(lod.forest, vec![128]);
        assert_eq!(lod.image.get_pixel(0, 0).0, [80, 100, 70]);

        let mut composed = lod.image.clone();
        compose_features(
            &mut composed,
            &lod.road,
            &lod.river,
            FeatureStyle::for_size(128),
        );
        assert!(composed.get_pixel(0, 0)[2] > lod.image.get_pixel(0, 0)[2]);
    }

    fn flat_lowland_sample() -> MacroSample {
        MacroSample {
            land: true,
            height: 30.0,
            dist_to_land_m: 0.0,
            river: 0.0,
            road: 0.0,
            coast: 0.0,
            slope: 0.0,
            shade: 1.0,
            ridge: 0.0,
            forest: 0.0,
        }
    }

    #[test]
    fn legacy_bare_mountains_stay_rocky_without_macro_relief() {
        let semantic = Semantic::legacy([130, 110, 90], flat_lowland_sample());
        assert!(
            semantic.rock >= 0.45,
            "data-only brown massifs need a strong rock signal: {}",
            semantic.rock
        );
    }

    #[test]
    fn snow_splat_renders_bright_snow() {
        let textures = Textures::load().expect("bundled textures decode");
        let mut semantic = Semantic::generated(flat_lowland_sample());
        semantic.snow = 1.0;
        semantic.rock = 0.4;
        let (snowy, _) = render_land(0.0, 0.0, flat_lowland_sample(), semantic, &textures);
        let brightness = snowy.iter().map(|&c| c as u32).sum::<u32>();
        assert!(
            brightness > 560,
            "full snow should render near-white, got {snowy:?}"
        );
        semantic.snow = 0.0;
        let (bare, _) = render_land(0.0, 0.0, flat_lowland_sample(), semantic, &textures);
        assert!(
            brightness > bare.iter().map(|&c| c as u32).sum::<u32>() + 150,
            "snow must clearly brighten the tile: {snowy:?} vs {bare:?}"
        );
    }

    #[test]
    fn guide_forest_likelihood_separates_canopy_field_and_rock() {
        let canopy = guide_forest_likelihood([44.0, 56.0, 30.0]);
        let field = guide_forest_likelihood([110.0, 120.0, 60.0]);
        let rock = guide_forest_likelihood([70.0, 70.0, 70.0]);
        assert!(
            canopy > 0.8,
            "dark canopy green should read as forest: {canopy}"
        );
        assert!(
            field < 0.05,
            "bright field green must not read as forest: {field}"
        );
        assert!(rock < 0.05, "gray rock must not read as forest: {rock}");
    }

    #[test]
    fn painted_canopy_cannot_swallow_real_fields() {
        let canopy = [44.0, 56.0, 30.0];
        let field_guide = [110.0, 120.0, 60.0];
        let over_field = guide_material_match(canopy, 1.0, 0.0);
        let over_forest = guide_material_match(canopy, 1.0, 1.0);
        let clearing_over_forest = guide_material_match(field_guide, 1.0, 1.0);
        let ocean = guide_material_match([20.0, 60.0, 120.0], 0.0, 0.0);
        assert!(
            over_field < 0.25,
            "canopy over real fields must be gated: {over_field}"
        );
        assert!(
            over_forest > 0.85,
            "canopy over real forest passes: {over_forest}"
        );
        assert!(
            clearing_over_forest < 0.25,
            "painted clearing over real forest must be gated: {clearing_over_forest}"
        );
        assert!(
            ocean > 0.85,
            "vegetation term must not affect water: {ocean}"
        );
    }

    #[test]
    fn smaller_lods_suppress_minor_features() {
        let terrain = RgbImage::from_pixel(1, 1, Rgb([80, 100, 70]));
        let road = [0];
        let river = [100];
        let mut base = terrain.clone();
        let mut overview = terrain.clone();
        compose_features(&mut base, &road, &river, FeatureStyle::for_size(1024));
        compose_features(&mut overview, &road, &river, FeatureStyle::for_size(128));
        assert_ne!(base.get_pixel(0, 0), terrain.get_pixel(0, 0));
        assert_eq!(overview.get_pixel(0, 0), terrain.get_pixel(0, 0));
    }
}
