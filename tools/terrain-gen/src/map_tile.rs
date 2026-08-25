use crate::map_color::{lerp, mix, scale, smoothstep, to_rgb};
use anyhow::{bail, Context, Result};
use image::{Rgb, RgbImage};
use onlinerpg_shared::tree_format::{
    TREE_EXCLUSION_RADIUS, TREE_V1_BYTES_PER_INSTANCE, TREE_V1_HEADER_BYTES, TREE_V1_MAGIC,
    TREE_V1_SCALE,
};
use onlinerpg_shared::worldgen::tile_bake::{
    HEIGHT_BIAS, HEIGHT_STEP, PAL_PAVING, PAL_RIVER_BED, PAL_ROAD, PAL_STONE_PATH, TILE_DIM,
    VERTS_PER_SIDE,
};
use onlinerpg_terrain::coords;
use std::path::Path;

pub const TILES_PER_REGION: i32 = 16;
pub const REGION_PX: u32 = TILES_PER_REGION as u32 * TILE_DIM as u32;

const MAX_PALETTE: usize = 16;
const SEA_LEVEL_M: f32 = -0.25;
const COLOR_FALLBACK: [u8; 3] = [120, 120, 100];
const HEIGHTMAP_BYTES: usize = VERTS_PER_SIDE * VERTS_PER_SIDE * 2;
const SPLATMAP_BYTES: usize = TILE_DIM * TILE_DIM * 4;
const RELIEF_BORDER: usize = 32;
const RELIEF_RADIUS: usize = 10;
const RELIEF_OFFSET: usize = 18;

pub type MinimapPalette = [[u8; 3]; MAX_PALETTE];
type RegionBounds = ((i32, i32), (i32, i32));

pub fn new_region_image() -> RgbImage {
    RgbImage::new(REGION_PX, REGION_PX)
}

pub static MINIMAP_PALETTE: std::sync::LazyLock<MinimapPalette> =
    std::sync::LazyLock::new(load_minimap_palette);

fn load_minimap_palette() -> MinimapPalette {
    const PALETTE_JSON: &str = include_str!("../../../shared/palette.json");
    let value: serde_json::Value =
        serde_json::from_str(PALETTE_JSON).expect("shared/palette.json is valid JSON");
    let layers = value["layers"]
        .as_array()
        .expect("palette.json: layers array");
    let mut palette = [COLOR_FALLBACK; MAX_PALETTE];
    for (index, layer) in layers.iter().enumerate().take(MAX_PALETTE) {
        let color = layer["minimapColor"]
            .as_array()
            .expect("palette.json: minimapColor array");
        palette[index] = [
            color[0].as_u64().expect("minimapColor[0]") as u8,
            color[1].as_u64().expect("minimapColor[1]") as u8,
            color[2].as_u64().expect("minimapColor[2]") as u8,
        ];
    }
    palette
}

pub fn render_region_to_path(
    terrain: &Path,
    region_x: i32,
    region_z: i32,
    output: &Path,
) -> Result<()> {
    let image = render_region_image(terrain, region_x, region_z, None)?;
    save_image(&image, output)
}

pub fn render_region_pyramid(
    terrain: &Path,
    output: &Path,
    region_x: i32,
    region_z: i32,
    bounds: Option<RegionBounds>,
) -> Result<()> {
    let mut mip = render_region_image(terrain, region_x, region_z, bounds)?;
    save_image(&mip, &coords::minimap_path(output, region_x, region_z))?;
    for size in coords::MINIMAP_LOD_SIZES.into_iter().rev() {
        mip = downsample_half(&mip);
        save_image(
            &mip,
            &coords::minimap_lod_path(output, region_x, region_z, size),
        )?;
    }
    Ok(())
}

fn render_region_image(
    terrain: &Path,
    region_x: i32,
    region_z: i32,
    bounds: Option<RegionBounds>,
) -> Result<RgbImage> {
    let palette = &*MINIMAP_PALETTE;
    let heights = load_height_neighborhood(terrain, region_x, region_z, bounds)?;
    let splatmap = load_region_splatmap(terrain, region_x, region_z)?;
    let height_side = REGION_PX as usize + RELIEF_BORDER * 2 + 1;
    let integral = build_integral(&heights, height_side);
    let integral_side = height_side + 1;
    let mut image = new_region_image();

    for cell_z in 0..REGION_PX as usize {
        for cell_x in 0..REGION_PX as usize {
            let grid_x = cell_x + RELIEF_BORDER;
            let grid_z = cell_z + RELIEF_BORDER;
            let h00 = heights[grid_z * height_side + grid_x];
            let h10 = heights[grid_z * height_side + grid_x + 1];
            let h01 = heights[(grid_z + 1) * height_side + grid_x];
            let h11 = heights[(grid_z + 1) * height_side + grid_x + 1];
            let detail_x = ((h10 + h11) - (h00 + h01)) * 0.5;
            let detail_z = ((h01 + h11) - (h00 + h10)) * 0.5;
            let left = box_mean(
                &integral,
                integral_side,
                grid_x - RELIEF_OFFSET,
                grid_z,
                RELIEF_RADIUS,
            );
            let right = box_mean(
                &integral,
                integral_side,
                grid_x + RELIEF_OFFSET,
                grid_z,
                RELIEF_RADIUS,
            );
            let top = box_mean(
                &integral,
                integral_side,
                grid_x,
                grid_z - RELIEF_OFFSET,
                RELIEF_RADIUS,
            );
            let bottom = box_mean(
                &integral,
                integral_side,
                grid_x,
                grid_z + RELIEF_OFFSET,
                RELIEF_RADIUS,
            );
            let span = (RELIEF_OFFSET * 2) as f32;
            let slope_x = (right - left) / span * 0.94 + detail_x * 0.06;
            let slope_z = (bottom - top) / span * 0.94 + detail_z * 0.06;
            let splat_index = (cell_z * REGION_PX as usize + cell_x) * 4;
            let rgb = render_cell(
                region_x * REGION_PX as i32 + cell_x as i32,
                region_z * REGION_PX as i32 + cell_z as i32,
                [h00, h10, h01, h11],
                slope_x,
                slope_z,
                &splatmap[splat_index..splat_index + 4],
                palette,
            );
            image.put_pixel(cell_x as u32, cell_z as u32, Rgb(rgb));
        }
    }
    overlay_trees(&mut image, terrain, region_x, region_z, bounds)?;

    Ok(image)
}

fn save_image(image: &RgbImage, output: &Path) -> Result<()> {
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    image
        .save(output)
        .with_context(|| format!("write {}", output.display()))
}

fn downsample_half(source: &RgbImage) -> RgbImage {
    let mut output = RgbImage::new(source.width() / 2, source.height() / 2);
    for y in 0..output.height() {
        for x in 0..output.width() {
            let pixels = [
                source.get_pixel(x * 2, y * 2),
                source.get_pixel(x * 2 + 1, y * 2),
                source.get_pixel(x * 2, y * 2 + 1),
                source.get_pixel(x * 2 + 1, y * 2 + 1),
            ];
            let mut color = [0u8; 3];
            for channel in 0..3 {
                let sum = pixels
                    .iter()
                    .map(|pixel| pixel[channel] as u16)
                    .sum::<u16>();
                color[channel] = ((sum + 2) / 4) as u8;
            }
            output.put_pixel(x, y, Rgb(color));
        }
    }
    output
}

fn render_cell(
    global_x: i32,
    global_z: i32,
    heights: [f32; 4],
    slope_x: f32,
    slope_z: f32,
    splat: &[u8],
    palette: &MinimapPalette,
) -> [u8; 3] {
    let [h00, h10, h01, h11] = heights;
    let height = (h00 + h10 + h01 + h11) * 0.25;
    let shade = hillshade(slope_x, slope_z);
    let shoreline = [h00, h10, h01, h11]
        .iter()
        .any(|value| *value < SEA_LEVEL_M)
        && [h00, h10, h01, h11]
            .iter()
            .any(|value| *value >= SEA_LEVEL_M);

    let packed = splat[0];
    let primary = ((packed >> 4) & 0x0f) as usize;
    let secondary = (packed & 0x0f) as usize;
    let secondary_weight = splat[2] as f32 / 255.0;
    let primary_weight = 1.0 - secondary_weight;
    let river_weight = layer_weight(
        primary,
        secondary,
        primary_weight,
        secondary_weight,
        PAL_RIVER_BED as usize,
    );
    let road_weight = [PAL_ROAD, PAL_STONE_PATH, PAL_PAVING]
        .iter()
        .map(|layer| {
            layer_weight(
                primary,
                secondary,
                primary_weight,
                secondary_weight,
                *layer as usize,
            )
        })
        .sum::<f32>()
        .clamp(0.0, 1.0);

    if height < SEA_LEVEL_M && river_weight < 0.6 {
        return render_sea(global_x, global_z, height, shade, shoreline);
    }

    let primary_color = layer_color(primary, palette);
    let secondary_color = layer_color(secondary, palette);
    let mut color = mix(primary_color, secondary_color, secondary_weight);
    let slope = slope_x.hypot(slope_z);
    let highland = smoothstep(70.0, 520.0, height);
    let exposed = smoothstep(0.8, 4.5, slope);
    color = mix(
        color,
        [118.0, 112.0, 101.0],
        highland * 0.22 + exposed * 0.12,
    );

    let vegetation = vegetation_density(splat[3]) * (1.0 - road_weight) * (1.0 - river_weight);
    color = mix(color, [52.0, 78.0, 45.0], vegetation * 0.04);

    if river_weight > 0.0 {
        let river = [49.0, 111.0, 133.0];
        color = mix(color, river, river_weight * 0.92);
    }
    if shoreline && height >= SEA_LEVEL_M {
        color = mix(color, [204.0, 189.0, 139.0], 0.42);
    }

    let terrain_shade = lerp(shade, 1.0, road_weight * 0.42 + river_weight * 0.76);
    color = scale(color, terrain_shade);
    to_rgb(color)
}

fn render_sea(global_x: i32, global_z: i32, height: f32, shade: f32, shoreline: bool) -> [u8; 3] {
    let depth = (SEA_LEVEL_M - height).max(0.0);
    let depth_mix = (depth / 26.0).sqrt().clamp(0.0, 1.0);
    let mut color = mix([61.0, 137.0, 151.0], [17.0, 47.0, 76.0], depth_mix);
    let phase = (global_x + global_z * 2).rem_euclid(32) as f32 / 32.0;
    let wave = (1.0 - (phase * 2.0 - 1.0).abs() - 0.5) * 0.02;
    color = scale(color, 1.0 + wave + (shade - 1.0) * (1.0 - depth_mix) * 0.18);
    if shoreline {
        color = mix(color, [96.0, 172.0, 169.0], 0.58);
    }
    to_rgb(color)
}

fn overlay_trees(
    image: &mut RgbImage,
    terrain: &Path,
    region_x: i32,
    region_z: i32,
    bounds: Option<RegionBounds>,
) -> Result<()> {
    for local_z in -1..=TILES_PER_REGION {
        for local_x in -1..=TILES_PER_REGION {
            let tile_x = region_x * TILES_PER_REGION + local_x;
            let tile_z = region_z * TILES_PER_REGION + local_z;
            if !tile_region_allowed(tile_x, tile_z, bounds) {
                continue;
            }
            let path = coords::tree_path(terrain, tile_x, tile_z);
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error).with_context(|| format!("read {}", path.display()))
                }
            };
            if bytes.len() < TREE_V1_HEADER_BYTES {
                bail!("{} has a truncated tree header", path.display());
            }
            let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
            if magic != TREE_V1_MAGIC {
                bail!("{} has unsupported tree data", path.display());
            }
            let counts = [
                u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize,
                u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
            ];
            let expected =
                TREE_V1_HEADER_BYTES + counts.iter().sum::<usize>() * TREE_V1_BYTES_PER_INSTANCE;
            if bytes.len() != expected {
                bail!(
                    "{} has {} bytes; expected {}",
                    path.display(),
                    bytes.len(),
                    expected
                );
            }

            let mut offset = TREE_V1_HEADER_BYTES;
            for tree_type in 0..2 {
                for _ in 0..counts[tree_type] {
                    let instance = &bytes[offset..offset + TREE_V1_BYTES_PER_INSTANCE];
                    offset += TREE_V1_BYTES_PER_INSTANCE;
                    let local_tree_x = u16::from_le_bytes(instance[0..2].try_into().unwrap())
                        as f32
                        * TILE_DIM as f32
                        / 65535.0;
                    let local_tree_z = u16::from_le_bytes(instance[2..4].try_into().unwrap())
                        as f32
                        * TILE_DIM as f32
                        / 65535.0;
                    let (scale_min, scale_range) = TREE_V1_SCALE[tree_type];
                    let tree_scale = scale_min + instance[5] as f32 / 255.0 * scale_range;
                    let radius =
                        (TREE_EXCLUSION_RADIUS[tree_type] * tree_scale * 0.82).clamp(1.15, 4.8);
                    let center_x = local_x as f32 * TILE_DIM as f32 + local_tree_x;
                    let center_z = local_z as f32 * TILE_DIM as f32 + local_tree_z;
                    draw_disc(
                        image,
                        center_x + radius * 0.28,
                        center_z + radius * 0.34,
                        radius * 1.04,
                        [24, 38, 24],
                        0.42,
                    );
                    let crown = if tree_type == 0 {
                        [43, 72, 39]
                    } else {
                        [57, 83, 45]
                    };
                    draw_disc(image, center_x, center_z, radius, crown, 0.78);
                    draw_disc(
                        image,
                        center_x - radius * 0.28,
                        center_z - radius * 0.32,
                        radius * 0.42,
                        [111, 130, 74],
                        0.34,
                    );
                }
            }
        }
    }
    Ok(())
}

fn draw_disc(
    image: &mut RgbImage,
    center_x: f32,
    center_z: f32,
    radius: f32,
    color: [u8; 3],
    alpha: f32,
) {
    let min_x = (center_x - radius - 1.0).floor() as i32;
    let max_x = (center_x + radius + 1.0).ceil() as i32;
    let min_z = (center_z - radius - 1.0).floor() as i32;
    let max_z = (center_z + radius + 1.0).ceil() as i32;
    for z in min_z..=max_z {
        for x in min_x..=max_x {
            if x < 0 || z < 0 || x >= REGION_PX as i32 || z >= REGION_PX as i32 {
                continue;
            }
            let distance =
                ((x as f32 + 0.5 - center_x).powi(2) + (z as f32 + 0.5 - center_z).powi(2)).sqrt();
            let coverage = (radius + 0.65 - distance).clamp(0.0, 1.0) * alpha;
            if coverage <= 0.0 {
                continue;
            }
            let pixel = image.get_pixel_mut(x as u32, z as u32);
            for channel in 0..3 {
                pixel[channel] = (pixel[channel] as f32 * (1.0 - coverage)
                    + color[channel] as f32 * coverage)
                    .round() as u8;
            }
        }
    }
}

fn load_height_neighborhood(
    terrain: &Path,
    region_x: i32,
    region_z: i32,
    bounds: Option<RegionBounds>,
) -> Result<Vec<f32>> {
    let side = REGION_PX as usize + RELIEF_BORDER * 2 + 1;
    let mut heights = vec![f32::NAN; side * side];

    for local_z in -1..=TILES_PER_REGION {
        for local_x in -1..=TILES_PER_REGION {
            let tile_x = region_x * TILES_PER_REGION + local_x;
            let tile_z = region_z * TILES_PER_REGION + local_z;
            if !tile_region_allowed(tile_x, tile_z, bounds) {
                continue;
            }
            let path = coords::heightmap_path(terrain, tile_x, tile_z);
            let required = (0..TILES_PER_REGION).contains(&local_x)
                && (0..TILES_PER_REGION).contains(&local_z);
            let bytes = match std::fs::read(&path) {
                Ok(bytes) if bytes.len() == HEIGHTMAP_BYTES => bytes,
                Ok(bytes) if required => {
                    bail!(
                        "{} has {} bytes; expected {}",
                        path.display(),
                        bytes.len(),
                        HEIGHTMAP_BYTES
                    )
                }
                Ok(_) => continue,
                Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error).with_context(|| format!("read {}", path.display()))
                }
            };
            let base_x = RELIEF_BORDER as isize + local_x as isize * TILE_DIM as isize;
            let base_z = RELIEF_BORDER as isize + local_z as isize * TILE_DIM as isize;
            for vertex_z in 0..VERTS_PER_SIDE {
                for vertex_x in 0..VERTS_PER_SIDE {
                    let target_x = base_x + vertex_x as isize;
                    let target_z = base_z + vertex_z as isize;
                    if target_x < 0
                        || target_z < 0
                        || target_x >= side as isize
                        || target_z >= side as isize
                    {
                        continue;
                    }
                    let source = (vertex_z * VERTS_PER_SIDE + vertex_x) * 2;
                    let encoded = u16::from_le_bytes([bytes[source], bytes[source + 1]]);
                    heights[target_z as usize * side + target_x as usize] =
                        encoded as f32 * HEIGHT_STEP - HEIGHT_BIAS;
                }
            }
        }
    }

    for z in 0..side {
        for x in 0..side {
            let index = z * side + x;
            if heights[index].is_finite() {
                continue;
            }
            let source_x = x.clamp(RELIEF_BORDER, RELIEF_BORDER + REGION_PX as usize);
            let source_z = z.clamp(RELIEF_BORDER, RELIEF_BORDER + REGION_PX as usize);
            heights[index] = heights[source_z * side + source_x];
        }
    }
    Ok(heights)
}

fn tile_region_allowed(tile_x: i32, tile_z: i32, bounds: Option<RegionBounds>) -> bool {
    let Some((region_min, region_max)) = bounds else {
        return true;
    };
    let region_x = coords::wrap_region_x(tile_x.div_euclid(TILES_PER_REGION));
    let region_z = tile_z.div_euclid(TILES_PER_REGION);
    region_x >= region_min.0
        && region_x <= region_max.0
        && region_z >= region_min.1
        && region_z <= region_max.1
}

fn load_region_splatmap(terrain: &Path, region_x: i32, region_z: i32) -> Result<Vec<u8>> {
    let side = REGION_PX as usize;
    let mut region = vec![0; side * side * 4];
    for local_z in 0..TILES_PER_REGION {
        for local_x in 0..TILES_PER_REGION {
            let tile_x = region_x * TILES_PER_REGION + local_x;
            let tile_z = region_z * TILES_PER_REGION + local_z;
            let path = coords::splatmap_path(terrain, tile_x, tile_z);
            let tile = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
            if tile.len() != SPLATMAP_BYTES {
                bail!(
                    "{} has {} bytes; expected {}",
                    path.display(),
                    tile.len(),
                    SPLATMAP_BYTES
                );
            }
            for row in 0..TILE_DIM {
                let source = row * TILE_DIM * 4;
                let target_x = local_x as usize * TILE_DIM;
                let target_z = local_z as usize * TILE_DIM + row;
                let target = (target_z * side + target_x) * 4;
                region[target..target + TILE_DIM * 4]
                    .copy_from_slice(&tile[source..source + TILE_DIM * 4]);
            }
        }
    }
    Ok(region)
}

fn build_integral(heights: &[f32], side: usize) -> Vec<f64> {
    let integral_side = side + 1;
    let mut integral = vec![0.0; integral_side * integral_side];
    for z in 0..side {
        let mut row_sum = 0.0;
        for x in 0..side {
            row_sum += heights[z * side + x] as f64;
            integral[(z + 1) * integral_side + x + 1] =
                integral[z * integral_side + x + 1] + row_sum;
        }
    }
    integral
}

fn box_mean(integral: &[f64], side: usize, x: usize, z: usize, radius: usize) -> f32 {
    let x0 = x - radius;
    let z0 = z - radius;
    let x1 = x + radius + 1;
    let z1 = z + radius + 1;
    let sum = integral[z1 * side + x1] - integral[z0 * side + x1] - integral[z1 * side + x0]
        + integral[z0 * side + x0];
    let width = radius * 2 + 1;
    (sum / (width * width) as f64) as f32
}

fn hillshade(slope_x: f32, slope_z: f32) -> f32 {
    let normal_x = -slope_x * 0.62;
    let normal_y = 1.0;
    let normal_z = -slope_z * 0.62;
    let length = (normal_x * normal_x + normal_y * normal_y + normal_z * normal_z).sqrt();
    let diffuse = ((normal_y * 0.72 + normal_z * -0.69) / length).max(0.0);
    (0.60 + diffuse * 0.56).clamp(0.56, 1.18)
}

fn layer_color(layer: usize, palette: &MinimapPalette) -> [f32; 3] {
    let color = match layer {
        0 => [82, 111, 55],
        1 => [190, 171, 113],
        2 => [157, 99, 63],
        3 => [224, 227, 226],
        4 => [154, 143, 121],
        5 => [128, 111, 88],
        6 => [49, 111, 133],
        7 => [151, 143, 127],
        8 => [164, 151, 130],
        _ => palette.get(layer).copied().unwrap_or(COLOR_FALLBACK),
    };
    [color[0] as f32, color[1] as f32, color[2] as f32]
}

fn layer_weight(
    primary: usize,
    secondary: usize,
    primary_weight: f32,
    secondary_weight: f32,
    layer: usize,
) -> f32 {
    let mut weight = 0.0;
    if primary == layer {
        weight += primary_weight;
    }
    if secondary == layer {
        weight += secondary_weight;
    }
    weight
}

fn vegetation_density(meta: u8) -> f32 {
    match meta {
        230..=239 => (meta - 230) as f32 / 9.0,
        240..=249 => (meta - 240) as f32 / 9.0,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_ground_has_neutral_light() {
        assert!((hillshade(0.0, 0.0) - 1.0).abs() < 0.01);
    }

    #[test]
    fn north_facing_slope_is_brighter() {
        assert!(hillshade(0.0, 2.0) > hillshade(0.0, -2.0));
    }

    #[test]
    fn deep_water_is_darker_than_shallow_water() {
        let shallow = render_sea(0, 0, -0.5, 1.0, false);
        let deep = render_sea(0, 0, -40.0, 1.0, false);
        assert!(
            deep.iter().map(|value| *value as u16).sum::<u16>()
                < shallow.iter().map(|value| *value as u16).sum::<u16>()
        );
    }

    #[test]
    fn mip_pixel_averages_one_source_block() {
        let source = RgbImage::from_fn(2, 2, |x, y| Rgb([(x * 40 + y * 80) as u8, 20, 40]));
        let mip = downsample_half(&source);
        assert_eq!(mip.get_pixel(0, 0).0, [60, 20, 40]);
    }
}
