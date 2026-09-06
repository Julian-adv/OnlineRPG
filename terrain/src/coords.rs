use std::path::{Path, PathBuf};

/// Full baked X circumference in terrain tiles. The canonical files cover
/// tile X -256 through 255; runtime render grids may request one tile beyond
/// either edge and must read the periodic tile from the opposite side.
pub const WORLD_TILES_X: i32 = 512;
pub const WORLD_MIN_TILE_X: i32 = -256;
pub const WORLD_MAX_TILE_X: i32 = WORLD_MIN_TILE_X + WORLD_TILES_X;
pub const FANTASY_MINIMAP_DIR: &str = "minimap-fantasy";
pub const MINIMAP_DIR: &str = "minimap";
/// Full-resolution region tile edge, in pixels.
pub const MINIMAP_BASE_SIZE: u32 = 1024;
/// Downscaled tiles baked below the base size, coarsest first.
pub const MINIMAP_LOD_SIZES: [u32; 3] = [128, 256, 512];

/// Normalize a render/data tile X into the canonical baked file range.
#[inline]
pub fn wrap_tile_x(tile_x: i32) -> i32 {
    (tile_x - WORLD_MIN_TILE_X).rem_euclid(WORLD_TILES_X) + WORLD_MIN_TILE_X
}

/// Region equivalent of `wrap_tile_x` for the 32 baked X regions.
#[inline]
pub fn wrap_region_x(region_x: i32) -> i32 {
    (region_x + 16).rem_euclid(32) - 16
}

/// Convert tile coordinate to region coordinate (floor division).
/// Region = 16x16 tiles. Negative coords round toward negative infinity.
pub fn tile_to_region(tile: i32) -> i32 {
    tile.div_euclid(16)
}

/// Convert a world-space coordinate (X or Z, meters) to the tile index that
/// contains it. Tile 0 spans [-32, 32), tile 1 [32, 96), etc.
pub fn world_to_tile(world_coord: f32) -> i32 {
    ((world_coord + 32.0) / 64.0).floor() as i32
}

/// Format region directory name: "r+00_+00", "r-01_+02"
fn region_dir_name(rx: i32, rz: i32) -> String {
    format!("r{:+03}_{:+03}", rx, rz)
}

/// Build filesystem path for a heightmap tile file.
pub fn heightmap_path(base: &Path, tx: i32, tz: i32) -> PathBuf {
    let tx = wrap_tile_x(tx);
    let (rx, rz) = (tile_to_region(tx), tile_to_region(tz));
    height_region_dir(base, rx, rz).join(format!("h_{:+05}_{:+05}.bin", tx, tz))
}

/// Build filesystem path for a splatmap tile file.
pub fn splatmap_path(base: &Path, tx: i32, tz: i32) -> PathBuf {
    let tx = wrap_tile_x(tx);
    let (rx, rz) = (tile_to_region(tx), tile_to_region(tz));
    splat_region_dir(base, rx, rz).join(format!("s_{:+05}_{:+05}.bin", tx, tz))
}

/// Build filesystem path for a region's height tile directory.
pub fn height_region_dir(base: &Path, rx: i32, rz: i32) -> PathBuf {
    base.join("height").join(region_dir_name(rx, rz))
}

/// Build filesystem path for a region's splat tile directory.
pub fn splat_region_dir(base: &Path, rx: i32, rz: i32) -> PathBuf {
    base.join("splat").join(region_dir_name(rx, rz))
}

pub fn landscaping_region_dir(base: &Path, rx: i32, rz: i32) -> PathBuf {
    base.join("landscaping").join(region_dir_name(rx, rz))
}

pub fn landscaping_path(base: &Path, tx: i32, tz: i32) -> PathBuf {
    let tx = wrap_tile_x(tx);
    landscaping_region_dir(base, tile_to_region(tx), tile_to_region(tz))
        .join(format!("l_{tx:+05}_{tz:+05}.bin"))
}

/// Build filesystem path for a grass placement data file.
pub fn grass_path(base: &Path, tx: i32, tz: i32) -> PathBuf {
    let tx = wrap_tile_x(tx);
    let (rx, rz) = (tile_to_region(tx), tile_to_region(tz));
    grass_region_dir(base, rx, rz).join(format!("g_{:+05}_{:+05}.bin", tx, tz))
}

/// Build filesystem path for a region's grass tile directory.
pub fn grass_region_dir(base: &Path, rx: i32, rz: i32) -> PathBuf {
    base.join("grass").join(region_dir_name(rx, rz))
}

/// Build filesystem path for an original (pre-housing) heightmap tile file.
pub fn original_heightmap_path(base: &Path, tx: i32, tz: i32) -> PathBuf {
    let tx = wrap_tile_x(tx);
    let (rx, rz) = (tile_to_region(tx), tile_to_region(tz));
    original_height_region_dir(base, rx, rz).join(format!("o_{:+05}_{:+05}.bin", tx, tz))
}

/// Build filesystem path for a region's original height tile directory.
pub fn original_height_region_dir(base: &Path, rx: i32, rz: i32) -> PathBuf {
    base.join("height-original").join(region_dir_name(rx, rz))
}

/// Build filesystem path for an original (pre-housing) grass placement data file.
pub fn original_grass_path(base: &Path, tx: i32, tz: i32) -> PathBuf {
    let tx = wrap_tile_x(tx);
    let (rx, rz) = (tile_to_region(tx), tile_to_region(tz));
    original_grass_region_dir(base, rx, rz).join(format!("g_{:+05}_{:+05}.bin", tx, tz))
}

/// Build filesystem path for a region's original grass tile directory.
pub fn original_grass_region_dir(base: &Path, rx: i32, rz: i32) -> PathBuf {
    base.join("grass-original").join(region_dir_name(rx, rz))
}

/// Build filesystem path for a region zone JSON file.
pub fn zone_path(base: &Path, rx: i32, rz: i32) -> PathBuf {
    base.join("zones")
        .join(format!("r{:+03}_{:+03}.json", rx, rz))
}

/// Build filesystem path for a region land-grade file (one byte per plot).
pub fn land_grade_path(base: &Path, rx: i32, rz: i32) -> PathBuf {
    base.join("land-grades")
        .join(format!("r{:+03}_{:+03}.bin", rx, rz))
}

/// Build filesystem path for a region object JSON file.
pub fn object_path(base: &Path, rx: i32, rz: i32) -> PathBuf {
    base.join("objects")
        .join(format!("r{:+03}_{:+03}.json", rx, rz))
}

/// Build filesystem path for a tree placement data file.
pub fn tree_path(base: &Path, tx: i32, tz: i32) -> PathBuf {
    let tx = wrap_tile_x(tx);
    let (rx, rz) = (tile_to_region(tx), tile_to_region(tz));
    tree_region_dir(base, rx, rz).join(format!("t_{:+05}_{:+05}.bin", tx, tz))
}

/// Build filesystem path for a region's tree tile directory.
pub fn tree_region_dir(base: &Path, rx: i32, rz: i32) -> PathBuf {
    base.join("trees").join(region_dir_name(rx, rz))
}

/// The two parallel minimap tile families. Everything that differs between
/// them — directory, extension, MIME type — lives here so the rest of the
/// codebase never re-derives it (and never sniffs magic bytes for it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinimapFamily {
    /// Painterly world-map tiles, baked by `terrain-gen render-map-world`.
    Fantasy,
    /// Semantic splat-colored tiles written by the editor and the bake.
    Legacy,
}

impl MinimapFamily {
    pub fn dir(self) -> &'static str {
        match self {
            Self::Fantasy => FANTASY_MINIMAP_DIR,
            Self::Legacy => MINIMAP_DIR,
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Self::Fantasy => "webp",
            Self::Legacy => "png",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Fantasy => "image/webp",
            Self::Legacy => "image/png",
        }
    }

    pub fn base_path(self, base: &Path, rx: i32, rz: i32) -> PathBuf {
        base.join(self.dir())
            .join(region_tile_name(rx, rz, self.ext()))
    }

    pub fn lod_path(self, base: &Path, rx: i32, rz: i32, size: u32) -> PathBuf {
        if size >= MINIMAP_BASE_SIZE {
            return self.base_path(base, rx, rz);
        }
        base.join(self.dir())
            .join(size.to_string())
            .join(region_tile_name(rx, rz, self.ext()))
    }
}

/// Filename a region tile takes in any family root, e.g. `r-02_+04.webp`.
pub fn region_tile_name(rx: i32, rz: i32, ext: &str) -> String {
    let rx = wrap_region_x(rx);
    format!("r{rx:+03}_{rz:+03}.{ext}")
}

/// Build filesystem path for a region minimap PNG file.
pub fn minimap_path(base: &Path, rx: i32, rz: i32) -> PathBuf {
    MinimapFamily::Legacy.base_path(base, rx, rz)
}

/// Fantasy world-map tiles are baked as lossy WebP; the gameplay minimap stays PNG.
pub fn fantasy_minimap_path(base: &Path, rx: i32, rz: i32) -> PathBuf {
    MinimapFamily::Fantasy.base_path(base, rx, rz)
}

pub fn minimap_lod_path(base: &Path, rx: i32, rz: i32, size: u32) -> PathBuf {
    MinimapFamily::Legacy.lod_path(base, rx, rz, size)
}

pub fn fantasy_minimap_lod_path(base: &Path, rx: i32, rz: i32, size: u32) -> PathBuf {
    MinimapFamily::Fantasy.lod_path(base, rx, rz, size)
}

/// Build filesystem path for the per-tile river-field binary (RFD1) —
/// pixel-aligned surfaceY + flowDir lookup table consumed by the runtime
/// quad-mesh river renderer. See `shared/src/worldgen/tile_bake/river_field.rs`.
pub fn river_field_path(base: &Path, tx: i32, tz: i32) -> PathBuf {
    let tx = wrap_tile_x(tx);
    let (rx, rz) = (tile_to_region(tx), tile_to_region(tz));
    river_field_region_dir(base, rx, rz).join(format!("rf_{:+05}_{:+05}.bin", tx, tz))
}

/// Build filesystem path for a region's river-field tile directory.
pub fn river_field_region_dir(base: &Path, rx: i32, rz: i32) -> PathBuf {
    base.join("river-field").join(region_dir_name(rx, rz))
}

/// Build filesystem path for the per-tile unified water-field binary
/// (WFD1) — surfaceY + flow + riverness lookup table consumed by the
/// runtime's single water mesh per tile. See
/// `shared/src/worldgen/tile_bake/water_field.rs`.
pub fn water_field_path(base: &Path, tx: i32, tz: i32) -> PathBuf {
    let tx = wrap_tile_x(tx);
    let (rx, rz) = (tile_to_region(tx), tile_to_region(tz));
    water_field_region_dir(base, rx, rz).join(format!("wf_{:+05}_{:+05}.bin", tx, tz))
}

/// Build filesystem path for a region's water-field tile directory.
pub fn water_field_region_dir(base: &Path, rx: i32, rz: i32) -> PathBuf {
    base.join("water-field").join(region_dir_name(rx, rz))
}
