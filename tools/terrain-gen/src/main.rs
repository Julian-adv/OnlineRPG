//! Offline procedural world generator.
//!
//! Iterative workflow:
//!   1. `terrain-gen preview --seed N`  — dump PNGs for visual inspection
//!      (default `--out data/terrain/worldgen_preview` → files land in
//!      `data/terrain/worldgen_preview/<seed-hex>/`)
//!   2. tweak seed / config, repeat until satisfied
//!   3. `terrain-gen bake --seed N` — write per-tile height & splat files
//!      matching the runtime `TerrainIO` format (default
//!      `--out data/terrain`). Bake also re-emits the preview PNGs into the
//!      same `worldgen_preview/<seed-hex>/` folder so a single source of
//!      truth is kept across both commands.
//!
//! See `doc/TERRAIN_GENERATION.md` for the full design.

mod apply_houses;
mod bake;
mod inspect;
mod map_tile;
mod preview;
mod prune_house_trees;

use anyhow::Result;
use apply_houses::ApplyOptions;
use clap::{Args, Parser, Subcommand};
use onlinerpg_shared::worldgen::WorldGenConfig;
use prune_house_trees::PruneOptions;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "terrain-gen", version, about = "Procedural terrain generator")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// CLI parameters that map onto `WorldGenConfig`. Shared by `preview` and
/// `bake` so tuning established at preview time reproduces exactly during a
/// bake of the same seed.
#[derive(Args, Clone)]
struct GenArgs {
    /// Master seed.
    #[arg(long, default_value_t = 7)]
    seed: u64,

    /// Global map resolution (cells per side). 4096 is the design default.
    /// Lower values (e.g. 512, 1024) iterate faster during tuning.
    #[arg(long, default_value_t = 4096)]
    res: u32,

    /// Target sea fraction (0..1). Note: filters (top-N components, island
    /// culling, inter-continent gap) add ~5-20% extra sea on top of this
    /// target, so measured sea runs higher (e.g. target 0.3 → measured ~0.37).
    #[arg(long, default_value_t = 0.30)]
    sea: f32,

    /// Continent wavelength in global cells.
    #[arg(long, default_value_t = 700)]
    wavelength: u32,

    /// fBm octaves for continent shape.
    #[arg(long, default_value_t = 4)]
    octaves: u32,

    /// fBm gain (persistence) for continent shape.
    #[arg(long, default_value_t = 0.5)]
    gain: f32,

    /// Minimum land-component size, in global cells.
    #[arg(long, default_value_t = 400)]
    min_islands: u32,

    /// Minimum land-bridge width in global cells.
    #[arg(long, default_value_t = 10)]
    min_strait: u32,

    /// Sea-channel ridge noise strength.
    #[arg(long, default_value_t = 0.0)]
    channel_strength: f32,

    /// Sea-channel wavelength in global cells.
    #[arg(long, default_value_t = 1000.0)]
    channel_wavelength: f32,

    /// Isthmus-cut width in global cells.
    #[arg(long, default_value_t = 0)]
    max_isthmus: u32,

    /// Number of initial continent seed points.
    #[arg(long, default_value_t = 20)]
    seeds: u32,

    /// Minimum spacing between continent seeds in global cells.
    #[arg(long, default_value_t = 450)]
    seed_distance: u32,

    /// Target number of final continents.
    #[arg(long, default_value_t = 3)]
    continents: u32,

    /// Minimum sea gap between continents in global cells.
    #[arg(long, default_value_t = 120)]
    gap: u32,

    /// Number of small scattered islands in open sea.
    #[arg(long, default_value_t = 15)]
    islands: u32,

    /// Mean radius of each small island in global cells.
    #[arg(long, default_value_t = 90)]
    island_radius: u32,

    /// Minimum clearance in global cells between small islands and other land.
    #[arg(long, default_value_t = 150)]
    island_clearance: u32,

    /// Erosion sim resolution (cells per side). 0 = use --res. Lower = faster
    /// but coarser; the result is bilinearly upsampled back to --res.
    #[arg(long, default_value_t = 1024)]
    erosion_res: u32,

    /// Number of erosion sim iterations. 0 = auto = ceil(1.4 · sim_res),
    /// matching dandrino's default.
    #[arg(long, default_value_t = 0)]
    erosion_iter: u32,

    /// Pre-erosion FBM amplitude on land, as a fraction of max_elevation_m.
    /// Higher = more dramatic mountains after erosion.
    #[arg(long, default_value_t = 0.4)]
    relief_amp: f32,

    /// Pre-erosion FBM wavelength in global cells. Sets macro mountain
    /// range spacing.
    #[arg(long, default_value_t = 700.0)]
    relief_wavelength: f32,

    /// Target city count for Phase 5a.
    #[arg(long, default_value_t = 60)]
    settlements: u32,

    /// Minimum spacing between settlements in global cells.
    #[arg(long, default_value_t = 70)]
    settlement_spacing: u32,

    /// Number of largest rivers (by mouth flow) to anchor at the coastal
    /// mouth instead of the inland middle reach. Default is set above any
    /// realistic river count so Phase A defaults to mouth-first for every
    /// river. 0 = original (always inland middle reach).
    #[arg(long, default_value_t = 500)]
    river_mouth_settlements: u32,

    /// Spacing multiplier for Phase-A river picks. Higher = Phase-A
    /// settlements spread further apart, reducing inland clusters of
    /// sibling-river picks at the cost of fewer total Phase-A villages.
    #[arg(long, default_value_t = 2.0)]
    phase_a_spacing_mult: f32,
}

impl GenArgs {
    fn into_config(self) -> WorldGenConfig {
        WorldGenConfig {
            seed: self.seed,
            global_res: self.res,
            sea_ratio: self.sea,
            continent_frequency: 1.0 / (self.wavelength.max(1) as f32),
            continent_octaves: self.octaves.max(1),
            continent_gain: self.gain,
            min_island_cells: self.min_islands,
            min_strait_width_cells: self.min_strait,
            sea_channel_strength: self.channel_strength,
            sea_channel_wavelength: self.channel_wavelength.max(1.0),
            max_isthmus_width_cells: self.max_isthmus,
            continent_seed_count: self.seeds.max(1),
            continent_seed_min_distance_cells: self.seed_distance,
            target_continent_count: self.continents.max(1),
            continent_gap_cells: self.gap,
            small_island_count: self.islands,
            small_island_radius_cells: self.island_radius,
            small_island_min_clearance_cells: self.island_clearance,
            erosion_sim_res: self.erosion_res,
            erosion_iterations: self.erosion_iter,
            initial_relief_amp: self.relief_amp,
            initial_relief_wavelength_cells: self.relief_wavelength.max(1.0),
            settlement_target_count: self.settlements,
            settlement_min_spacing_cells: self.settlement_spacing.max(1),
            settlement_mouth_count: self.river_mouth_settlements,
            settlement_phase_a_spacing_mult: self.phase_a_spacing_mult,
            ..WorldGenConfig::default()
        }
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Generate the low-res global map and dump PNGs for visual inspection.
    Preview {
        #[command(flatten)]
        gen: GenArgs,

        /// Output root. A sub-folder named after the seed is created inside,
        /// so the default puts PNGs at `data/terrain/worldgen_preview/<seed-hex>/`.
        #[arg(long, default_value = "data/terrain/worldgen_preview")]
        out: PathBuf,
    },

    /// Bake per-tile heightmap + splatmap files into a `data/terrain/`-shaped
    /// layout. The default region range covers the full 32×32 km world
    /// (32×32 regions around the origin). Narrow the range explicitly for
    /// faster iteration on a sub-area.
    Bake {
        #[command(flatten)]
        gen: GenArgs,

        /// Output directory (written directly: `out/height/…`, `out/splat/…`,
        /// `out/worldgen.json`, …). No seed subfolder so the layout matches
        /// what the runtime `TerrainIO` expects. PNG previews are emitted
        /// under `out/worldgen_preview/<seed-hex>/`, matching the `preview`
        /// command's default location.
        #[arg(long, default_value = "data/terrain")]
        out: PathBuf,

        /// Minimum region X (inclusive). Region = 16 tiles = 1024 m at 64 m/tile.
        #[arg(long, default_value_t = -16, allow_hyphen_values = true)]
        region_x_min: i32,

        /// Maximum region X (inclusive).
        #[arg(long, default_value_t = 15, allow_hyphen_values = true)]
        region_x_max: i32,

        /// Minimum region Z (inclusive).
        #[arg(long, default_value_t = -16, allow_hyphen_values = true)]
        region_z_min: i32,

        /// Maximum region Z (inclusive).
        #[arg(long, default_value_t = 15, allow_hyphen_values = true)]
        region_z_max: i32,

        /// Only (re)write `water-field/` WFD1 files; leave every other
        /// artifact untouched. Safe against a live world: a full bake
        /// deletes pre-edit heightmap snapshots and rewrites player-edited
        /// tiles, this mode does not. The pipeline is seed-deterministic,
        /// so the water field matches the original bake's terrain.
        #[arg(long)]
        water_field_only: bool,
    },

    /// Dump river segment data influencing a single tile.
    InspectTile {
        #[command(flatten)]
        gen: GenArgs,
        /// Tile X index.
        #[arg(long, allow_hyphen_values = true)]
        tile_x: i32,
        /// Tile Z index.
        #[arg(long, allow_hyphen_values = true)]
        tile_z: i32,
    },

    /// Decompose the baked height at one or more world (x, z) points:
    /// natural surface, nearest river segment params, and carve.
    ProbePoint {
        #[command(flatten)]
        gen: GenArgs,
        /// World coordinates as `x,z` pairs (repeat the flag for multiple points).
        #[arg(
            long = "at",
            value_name = "X,Z",
            required = true,
            allow_hyphen_values = true
        )]
        at: Vec<String>,
    },

    /// Render one region map from existing baked height and splat tiles.
    RenderMapRegion {
        #[arg(long, default_value = "data/terrain")]
        terrain: PathBuf,

        #[arg(long, allow_hyphen_values = true)]
        region_x: i32,

        #[arg(long, allow_hyphen_values = true)]
        region_z: i32,

        #[arg(long)]
        out: PathBuf,
    },

    /// Render one region and its 128/256/512/1024 map pyramid.
    RenderMapPyramid {
        #[arg(long, default_value = "data/terrain")]
        terrain: PathBuf,

        #[arg(long, default_value = "data/terrain")]
        out: PathBuf,

        #[arg(long, allow_hyphen_values = true)]
        region_x: i32,

        #[arg(long, allow_hyphen_values = true)]
        region_z: i32,
    },

    /// Remove baked tree instances that overlap persisted house footprints.
    PruneHouseTrees {
        /// Terrain directory containing `trees/`.
        #[arg(long, default_value = "data/terrain")]
        terrain: PathBuf,

        /// Housing directory containing chunk folders and house JSON files.
        #[arg(long, default_value = "data/housing")]
        housing: PathBuf,

        /// Extra margin around each ground-floor room footprint.
        #[arg(long, default_value_t = 2.0)]
        margin: f32,

        /// Print what would change without writing tree files.
        #[arg(long)]
        dry_run: bool,

        /// Print loaded houses and their ground-floor prune footprints.
        #[arg(long)]
        list_houses: bool,
    },

    /// Re-apply persisted houses to a freshly baked world: flatten terrain to
    /// each house floor, clear grass under ground-floor rooms, and prune
    /// overlapping trees. The offline equivalent of the Map Editor's
    /// "Reinstall", run for every house. Intended to run right after `bake`,
    /// which regenerates (and wipes editor edits from) the tile files.
    ApplyHouses {
        /// Terrain directory containing `height/`, `grass/`, `trees/`.
        #[arg(long, default_value = "data/terrain")]
        terrain: PathBuf,

        /// Housing directory containing chunk folders and house JSON files.
        #[arg(long, default_value = "data/housing")]
        housing: PathBuf,

        /// Smoothstep blend skirt width (m) outside each flattened room.
        #[arg(long, default_value_t = 4.0)]
        blend_radius: f32,

        /// Extra margin (m) around each room for grass clearing.
        #[arg(long, default_value_t = 1.0)]
        grass_margin: f32,

        /// Extra margin (m) around each room for tree pruning.
        #[arg(long, default_value_t = 2.0)]
        tree_margin: f32,

        /// Print what would change without writing any files.
        #[arg(long)]
        dry_run: bool,

        /// Print the loaded houses before applying.
        #[arg(long)]
        list_houses: bool,

        /// Skip the terrain-flatten pass.
        #[arg(long)]
        skip_flatten: bool,

        /// Skip the grass-clear pass.
        #[arg(long)]
        skip_grass: bool,

        /// Skip the tree-prune pass.
        #[arg(long)]
        skip_trees: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Preview { gen, out } => {
            let cfg = gen.into_config();
            preview::run(&cfg, &out)
        }
        Cmd::Bake {
            gen,
            out,
            region_x_min,
            region_x_max,
            region_z_min,
            region_z_max,
            water_field_only,
        } => {
            if region_x_max < region_x_min || region_z_max < region_z_min {
                anyhow::bail!(
                    "invalid region range: x[{},{}] z[{},{}]",
                    region_x_min,
                    region_x_max,
                    region_z_min,
                    region_z_max,
                );
            }
            let cfg = gen.into_config();
            bake::run(
                &cfg,
                &out,
                (region_x_min, region_z_min),
                (region_x_max, region_z_max),
                water_field_only,
            )
        }
        Cmd::InspectTile {
            gen,
            tile_x,
            tile_z,
        } => {
            let cfg = gen.into_config();
            inspect::run(&cfg, tile_x, tile_z)
        }
        Cmd::ProbePoint { gen, at } => {
            let cfg = gen.into_config();
            let mut points = Vec::new();
            for raw in &at {
                let parts: Vec<&str> = raw.split(',').collect();
                if parts.len() != 2 {
                    anyhow::bail!("--at expects 'X,Z', got '{}'", raw);
                }
                let x: f32 = parts[0].trim().parse()?;
                let z: f32 = parts[1].trim().parse()?;
                points.push((x, z));
            }
            inspect::probe(&cfg, &points)
        }
        Cmd::RenderMapRegion {
            terrain,
            region_x,
            region_z,
            out,
        } => map_tile::render_region_to_path(&terrain, region_x, region_z, &out),
        Cmd::RenderMapPyramid {
            terrain,
            out,
            region_x,
            region_z,
        } => map_tile::render_region_pyramid(&terrain, &out, region_x, region_z),
        Cmd::PruneHouseTrees {
            terrain,
            housing,
            margin,
            dry_run,
            list_houses,
        } => prune_house_trees::run(PruneOptions {
            terrain,
            housing,
            margin,
            dry_run,
            list_houses,
        }),
        Cmd::ApplyHouses {
            terrain,
            housing,
            blend_radius,
            grass_margin,
            tree_margin,
            dry_run,
            list_houses,
            skip_flatten,
            skip_grass,
            skip_trees,
        } => apply_houses::run(ApplyOptions {
            terrain,
            housing,
            tree_margin,
            blend_radius,
            grass_margin,
            dry_run,
            list_houses,
            skip_flatten,
            skip_grass,
            skip_trees,
        }),
    }
}
