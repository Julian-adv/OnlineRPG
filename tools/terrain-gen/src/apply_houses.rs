//! Combined post-bake house replat: flatten terrain to each house floor,
//! clear grass under ground-floor rooms, and prune trees that overlap house
//! footprints — all read from the persisted `data/housing/` store.
//!
//! This is the offline equivalent of the Map Editor's "Reinstall" action
//! (`reinstallSelectedHouse` in `HousingEditorCursor.svelte`), run for every
//! house at once. A full `bake` regenerates the per-tile height/grass/tree
//! files from seed and wipes any editor edits (and deletes the `-original`
//! snapshots), so house-flattened, grass-cleared areas must be re-applied
//! after each bake. Run this right after `bake` to make those areas
//! reproducible across bakes:
//!
//! ```text
//! terrain-gen bake --seed 42
//! terrain-gen apply-houses
//! ```
//!
//! The three passes match the runtime editor:
//! * **Flatten** — every ground-floor room (excluding stairwells) is flattened
//!   to `house.origin.y` with a `blend_radius` smoothstep skirt, matching
//!   `flattenArea`. Cells inside another house's ground-floor footprint are
//!   protected so a blend skirt never dips a neighbour's pad.
//! * **Grass** — grass blades inside each ground-floor room rect (expanded by
//!   `grass_margin`) are removed, matching `roomGrassRect` + `removeGrassInRect`.
//! * **Trees** — reuses `prune_house_trees` with `tree_margin`.
//!
//! Like the editor, the first edit of a tile snapshots the fresh baked file to
//! its `-original` path so the runtime house-delete restore keeps working.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use onlinerpg_shared::{
    housing::{HouseData, RoomData},
    worldgen::{
        tile_bake::{HEIGHT_BIAS, HEIGHT_STEP},
        vegetation::{GRASS_V3_BYTES_PER_INSTANCE, GRASS_V3_HEADER_BYTES, GRASS_V3_MAGIC},
    },
};
use onlinerpg_terrain::{
    coords,
    defaults::{HEIGHTMAP_SIZE, TILE_DIM, VERTS_PER_SIDE},
    trees::TreeExclusionRect,
};

use crate::prune_house_trees::{
    house_tree_rects, is_ground_floor, load_houses, prune_trees, union_tiles,
};

#[derive(Debug)]
pub struct ApplyOptions {
    /// Terrain directory containing `height/`, `grass/`, `trees/`.
    pub terrain: PathBuf,
    /// Housing directory containing chunk folders and house JSON files.
    pub housing: PathBuf,
    /// Extra margin (m) around each ground-floor room for tree pruning.
    pub tree_margin: f32,
    /// Smoothstep blend skirt width (m) outside each flattened room rect.
    pub blend_radius: f32,
    /// Extra margin (m) around each ground-floor room for grass clearing.
    pub grass_margin: f32,
    /// Print what would change without writing any files.
    pub dry_run: bool,
    /// Print the loaded houses before applying.
    pub list_houses: bool,
    /// Skip the terrain-flatten pass.
    pub skip_flatten: bool,
    /// Skip the grass-clear pass.
    pub skip_grass: bool,
    /// Skip the tree-prune pass.
    pub skip_trees: bool,
}

/// World-space AABB: `[min_x, min_z, max_x, max_z]`.
type Rect = [f32; 4];

/// Half a tile in world units — a tile `t` covers `[t*TILE - HALF, t*TILE + HALF)`.
const TILE_HALF: f32 = TILE_DIM as f32 / 2.0;

/// Decode a uint16 heightmap sample to meters (inverse of `encode_height`).
fn decode_height(v: u16) -> f32 {
    v as f32 * HEIGHT_STEP - HEIGHT_BIAS
}

/// Encode meters to a clamped uint16 heightmap sample, matching the baker's
/// `encode_heightmap` and the client's `encodeHeight`.
fn encode_height(m: f32) -> u16 {
    ((m + HEIGHT_BIAS) / HEIGHT_STEP)
        .round()
        .clamp(0.0, 65535.0) as u16
}

/// World-space rect of a ground-floor room (no margin).
fn room_rect(house: &HouseData, room: &RoomData) -> Rect {
    let min_x = house.origin.x + room.local_x as f32;
    let min_z = house.origin.z + room.local_z as f32;
    [
        min_x,
        min_z,
        min_x + room.size_x as f32,
        min_z + room.size_z as f32,
    ]
}

fn point_in_rect([min_x, min_z, max_x, max_z]: Rect, x: f32, z: f32) -> bool {
    x >= min_x && x <= max_x && z >= min_z && z <= max_z
}

fn point_in_any(rects: &[Rect], x: f32, z: f32) -> bool {
    rects.iter().any(|&r| point_in_rect(r, x, z))
}

fn tile_min_world(t: i32) -> f32 {
    t as f32 * TILE_DIM as f32 - TILE_HALF
}

// ===================================================================
// Height flatten
// ===================================================================

/// Lazily-loaded, in-memory heightmap editor. Loads each affected tile once,
/// applies every room's flatten pass to the shared working copy (so a later
/// room's blend reads the earlier room's result, like the editor), then writes
/// the dirty tiles — snapshotting the pre-edit file to `-original` on the way.
struct HeightEditor<'a> {
    terrain: &'a Path,
    /// Working copy: decoded uint16 samples per tile.
    cache: HashMap<(i32, i32), Vec<u16>>,
    /// Tiles with no heightmap file — skipped (nothing to flatten there).
    missing: HashSet<(i32, i32)>,
    dirty: BTreeSet<(i32, i32)>,
}

impl<'a> HeightEditor<'a> {
    fn new(terrain: &'a Path) -> Self {
        Self {
            terrain,
            cache: HashMap::new(),
            missing: HashSet::new(),
            dirty: BTreeSet::new(),
        }
    }

    /// Ensure a tile is loaded. Returns `false` if the tile has no file.
    fn load(&mut self, tx: i32, tz: i32) -> Result<bool> {
        let key = (tx, tz);
        if self.cache.contains_key(&key) {
            return Ok(true);
        }
        if self.missing.contains(&key) {
            return Ok(false);
        }
        let path = coords::heightmap_path(self.terrain, tx, tz);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.missing.insert(key);
                return Ok(false);
            }
            Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
        };
        if bytes.len() != HEIGHTMAP_SIZE {
            bail!(
                "heightmap {} has wrong size {} (expected {})",
                path.display(),
                bytes.len(),
                HEIGHTMAP_SIZE
            );
        }
        let heights: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        self.cache.insert(key, heights);
        Ok(true)
    }

    /// Flatten one room rect to `target`, with a smoothstep blend skirt of
    /// `blend` metres, skipping cells inside any `protected` rect. Ported
    /// vertex-for-vertex from `flattenArea` in `terrain-height-brushes.ts`.
    fn flatten_room(
        &mut self,
        rect: Rect,
        target: f32,
        blend: f32,
        protected: &[Rect],
    ) -> Result<()> {
        let [min_x, min_z, max_x, max_z] = rect;
        let exp_min_x = min_x - blend;
        let exp_min_z = min_z - blend;
        let exp_max_x = max_x + blend;
        let exp_max_z = max_z + blend;

        let min_tx = coords::world_to_tile(exp_min_x);
        let max_tx = coords::world_to_tile(exp_max_x);
        let min_tz = coords::world_to_tile(exp_min_z);
        let max_tz = coords::world_to_tile(exp_max_z);

        let target_encoded = encode_height(target);
        let verts = VERTS_PER_SIDE as i32;

        for tz in min_tz..=max_tz {
            for tx in min_tx..=max_tx {
                if !self.load(tx, tz)? {
                    continue;
                }
                let tile_min_x = tile_min_world(tx);
                let tile_min_z = tile_min_world(tz);

                let start_cx = ((exp_min_x - tile_min_x).floor() as i32).max(0);
                let end_cx = ((exp_max_x - tile_min_x).floor() as i32).min(verts - 1);
                let start_cz = ((exp_min_z - tile_min_z).floor() as i32).max(0);
                let end_cz = ((exp_max_z - tile_min_z).floor() as i32).min(verts - 1);

                let data = self.cache.get_mut(&(tx, tz)).expect("tile loaded above");
                let mut touched = false;

                for cz in start_cz..=end_cz {
                    for cx in start_cx..=end_cx {
                        let world_cx = tile_min_x + cx as f32;
                        let world_cz = tile_min_z + cz as f32;

                        if point_in_any(protected, world_cx, world_cz) {
                            continue;
                        }

                        // Distance from the rect edges (0 inside).
                        let dx = (min_x - world_cx).max(0.0).max(world_cx - max_x);
                        let dz = (min_z - world_cz).max(0.0).max(world_cz - max_z);
                        let dist = (dx * dx + dz * dz).sqrt();

                        let idx = cz as usize * VERTS_PER_SIDE + cx as usize;

                        if dist <= 0.0 {
                            data[idx] = target_encoded;
                            touched = true;
                        } else if dist < blend {
                            let t = dist / blend;
                            let b = 1.0 - t * t * (3.0 - 2.0 * t);
                            let cur = decode_height(data[idx]);
                            data[idx] = encode_height(cur + (target - cur) * b);
                            touched = true;
                        }
                    }
                }

                if touched {
                    self.dirty.insert((tx, tz));
                }
            }
        }
        Ok(())
    }

    /// Write every dirty tile back, snapshotting the pre-edit bytes to the
    /// `-original` path first (only if one isn't already present). Returns the
    /// number of tiles written.
    fn write_all(&self, dry_run: bool) -> Result<usize> {
        for &(tx, tz) in &self.dirty {
            let heights = &self.cache[&(tx, tz)];
            let mut bytes = Vec::with_capacity(HEIGHTMAP_SIZE);
            for &v in heights {
                bytes.extend_from_slice(&v.to_le_bytes());
            }

            if !dry_run {
                // Snapshot the pre-edit file to `-original` before overwriting
                // it — `hpath` still holds the untouched baked bytes here.
                let hpath = coords::heightmap_path(self.terrain, tx, tz);
                let orig_path = coords::original_heightmap_path(self.terrain, tx, tz);
                if !orig_path.exists() {
                    if let Some(parent) = orig_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::copy(&hpath, &orig_path)
                        .with_context(|| format!("snapshot {}", orig_path.display()))?;
                }
                fs::write(&hpath, &bytes).with_context(|| format!("write {}", hpath.display()))?;
            }
        }
        Ok(self.dirty.len())
    }
}

/// Flatten every house's ground-floor rooms to its floor. Returns tiles written.
fn flatten_houses(
    terrain: &Path,
    houses: &[HouseData],
    blend: f32,
    dry_run: bool,
) -> Result<usize> {
    let mut editor = HeightEditor::new(terrain);

    for (i, house) in houses.iter().enumerate() {
        // Protect every OTHER house's ground-floor footprint so this house's
        // blend skirt never lowers a neighbour's flattened pad. Mirrors the
        // editor's `buildGroundFloorRects(ph => ph.id === house.id)` exclude.
        let mut protected: Vec<Rect> = Vec::new();
        for (j, other) in houses.iter().enumerate() {
            if j == i {
                continue;
            }
            for room in other.rooms.iter().filter(|r| is_ground_floor(r)) {
                protected.push(room_rect(other, room));
            }
        }

        for room in house.rooms.iter().filter(|r| is_ground_floor(r)) {
            editor.flatten_room(room_rect(house, room), house.origin.y, blend, &protected)?;
        }
    }

    editor.write_all(dry_run)
}

// ===================================================================
// Grass clear
// ===================================================================

fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

/// Drop V3 grass instances whose world position falls inside any `rects`.
/// Returns the rewritten buffer and the number of blades removed.
fn filter_grass_v3_in_rects(
    tx: i32,
    tz: i32,
    data: &[u8],
    rects: &[Rect],
) -> Result<(Vec<u8>, usize)> {
    if data.len() < GRASS_V3_HEADER_BYTES {
        bail!("grass data header is truncated");
    }
    let magic = read_u32_le(data, 0);
    if magic != GRASS_V3_MAGIC {
        bail!("unsupported grass data magic 0x{magic:08x}");
    }
    let counts = [
        read_u32_le(data, 4) as usize,
        read_u32_le(data, 8) as usize,
        read_u32_le(data, 12) as usize,
    ];
    let total: usize = counts.iter().sum();
    let expected = GRASS_V3_HEADER_BYTES + total * GRASS_V3_BYTES_PER_INSTANCE;
    if data.len() != expected {
        bail!(
            "grass data length {} != expected {} (counts {:?})",
            data.len(),
            expected,
            counts
        );
    }

    let tile_min_x = tile_min_world(tx);
    let tile_min_z = tile_min_world(tz);
    // Inverse of `encode_grass_v3`'s `pos_scale = 65535 / TILE_DIM`.
    let inv_pos_scale = TILE_DIM as f32 / 65535.0;

    let mut kept_counts = [0u32; 3];
    let mut body: Vec<u8> = Vec::with_capacity(data.len() - GRASS_V3_HEADER_BYTES);
    let mut removed = 0usize;
    let mut offset = GRASS_V3_HEADER_BYTES;

    for (bucket, &count) in counts.iter().enumerate() {
        for _ in 0..count {
            let inst = &data[offset..offset + GRASS_V3_BYTES_PER_INSTANCE];
            offset += GRASS_V3_BYTES_PER_INSTANCE;
            let px = u16::from_le_bytes([inst[0], inst[1]]) as f32;
            let pz = u16::from_le_bytes([inst[2], inst[3]]) as f32;
            let world_x = tile_min_x + px * inv_pos_scale;
            let world_z = tile_min_z + pz * inv_pos_scale;

            if point_in_any(rects, world_x, world_z) {
                removed += 1;
            } else {
                kept_counts[bucket] += 1;
                body.extend_from_slice(inst);
            }
        }
    }

    let mut out = Vec::with_capacity(GRASS_V3_HEADER_BYTES + body.len());
    out.extend_from_slice(&GRASS_V3_MAGIC.to_le_bytes());
    for c in kept_counts {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out.extend_from_slice(&body);
    Ok((out, removed))
}

/// World-space grass rect of a ground-floor room, expanded by `margin` on all
/// sides (matches `roomGrassRect`).
fn room_grass_rect(house: &HouseData, room: &RoomData, margin: f32) -> Rect {
    let [min_x, min_z, max_x, max_z] = room_rect(house, room);
    [
        min_x - margin,
        min_z - margin,
        max_x + margin,
        max_z + margin,
    ]
}

/// Clear grass under every house's ground-floor rooms. Returns
/// `(changed_tiles, blades_removed)`.
fn clear_grass(
    terrain: &Path,
    houses: &[HouseData],
    margin: f32,
    dry_run: bool,
) -> Result<(BTreeSet<(i32, i32)>, usize)> {
    let mut rects: Vec<Rect> = Vec::new();
    for house in houses {
        for room in house.rooms.iter().filter(|r| is_ground_floor(r)) {
            rects.push(room_grass_rect(house, room, margin));
        }
    }

    let mut changed = BTreeSet::new();
    let mut removed_total = 0usize;

    // `union_tiles` yields nothing for empty `rects`, so no houses → no-op.
    for (tx, tz) in union_tiles(&rects) {
        let path = coords::grass_path(terrain, tx, tz);
        let data = match fs::read(&path) {
            Ok(d) => d,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
        };

        let (filtered, removed) = filter_grass_v3_in_rects(tx, tz, &data, &rects)
            .with_context(|| format!("filter {}", path.display()))?;
        if removed == 0 {
            continue;
        }

        if !dry_run {
            let orig_path = coords::original_grass_path(terrain, tx, tz);
            if !orig_path.exists() {
                if let Some(parent) = orig_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&orig_path, &data)
                    .with_context(|| format!("write {}", orig_path.display()))?;
            }
            fs::write(&path, &filtered).with_context(|| format!("write {}", path.display()))?;
        }

        removed_total += removed;
        changed.insert((tx, tz));
    }

    Ok((changed, removed_total))
}

// ===================================================================
// Driver
// ===================================================================

fn print_house_list(houses: &[HouseData]) {
    println!("Houses:");
    for house in houses {
        let ground = house.rooms.iter().filter(|r| is_ground_floor(r)).count();
        println!(
            "  {} origin=({:.1}, {:.3}, {:.1}) rooms={} groundFloor={}",
            house.id,
            house.origin.x,
            house.origin.y,
            house.origin.z,
            house.rooms.len(),
            ground
        );
    }
}

pub fn run(options: ApplyOptions) -> Result<()> {
    let houses = load_houses(&options.housing)?;

    if options.list_houses {
        print_house_list(&houses);
    }

    let flatten_tiles = if options.skip_flatten {
        0
    } else {
        flatten_houses(
            &options.terrain,
            &houses,
            options.blend_radius,
            options.dry_run,
        )?
    };

    let (grass_tiles, grass_removed) = if options.skip_grass {
        (BTreeSet::new(), 0)
    } else {
        clear_grass(
            &options.terrain,
            &houses,
            options.grass_margin,
            options.dry_run,
        )?
    };

    let (tree_tiles, trees_removed) = if options.skip_trees {
        (BTreeSet::new(), 0)
    } else {
        let mut rects: Vec<TreeExclusionRect> = Vec::new();
        for house in &houses {
            rects.extend(house_tree_rects(house, options.tree_margin));
        }
        let outcome = prune_trees(&options.terrain, &rects, options.dry_run)?;
        (outcome.changed_tiles, outcome.trees_removed)
    };

    let mode = if options.dry_run {
        "Dry-run"
    } else {
        "Applied"
    };
    println!(
        "{mode} apply-houses: {} house(s)\n  \
         flatten: {} tile(s) written\n  \
         grass:   {} tile(s) changed, {} blade(s) removed\n  \
         trees:   {} tile(s) changed, {} tree(s) removed",
        houses.len(),
        flatten_tiles,
        grass_tiles.len(),
        grass_removed,
        tree_tiles.len(),
        trees_removed,
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a V3 grass buffer from per-bucket instance positions (local
    /// tile-space metres), mirroring `encode_grass_v3`'s quantization.
    fn encode_grass(buckets: [&[(f32, f32)]; 3]) -> Vec<u8> {
        let pos_scale = 65535.0 / TILE_DIM as f32;
        let mut out = Vec::new();
        out.extend_from_slice(&GRASS_V3_MAGIC.to_le_bytes());
        for b in buckets {
            out.extend_from_slice(&(b.len() as u32).to_le_bytes());
        }
        for b in buckets {
            for &(lx, lz) in b {
                let px = (lx * pos_scale).round().clamp(0.0, 65535.0) as u16;
                let pz = (lz * pos_scale).round().clamp(0.0, 65535.0) as u16;
                out.extend_from_slice(&px.to_le_bytes());
                out.extend_from_slice(&pz.to_le_bytes());
                out.push(0); // rotation
                out.push(0); // scale
            }
        }
        out
    }

    #[test]
    fn height_encode_decode_roundtrip() {
        assert_eq!(encode_height(0.0), 10000);
        assert_eq!(encode_height(-200.0), 6000);
        assert!((decode_height(encode_height(12.5)) - 12.5).abs() < 0.05);
    }

    #[test]
    fn grass_filter_removes_only_instances_inside_rect() {
        // Tile (0,0) covers world x/z in [-32, 32). Local 32m → world ~0.
        // Instance A at local (32,32) ≈ world (0,0)   → inside rect
        // Instance B at local (52,32) ≈ world (20,0)  → outside rect
        let data = encode_grass([&[(32.0, 32.0)], &[(52.0, 32.0)], &[]]);
        let rect: Rect = [-5.0, -5.0, 5.0, 5.0];

        let (out, removed) = filter_grass_v3_in_rects(0, 0, &data, &[rect]).unwrap();
        assert_eq!(removed, 1);

        // Header counts: short dropped to 0, tall kept 1, flower 0.
        assert_eq!(read_u32_le(&out, 0), GRASS_V3_MAGIC);
        assert_eq!(read_u32_le(&out, 4), 0);
        assert_eq!(read_u32_le(&out, 8), 1);
        assert_eq!(read_u32_le(&out, 12), 0);
        assert_eq!(
            out.len(),
            GRASS_V3_HEADER_BYTES + GRASS_V3_BYTES_PER_INSTANCE
        );
    }

    #[test]
    fn grass_filter_noop_when_rect_misses() {
        let data = encode_grass([&[(10.0, 10.0)], &[], &[]]);
        // Rect far from the single blade at world ~(-22,-22).
        let rect: Rect = [100.0, 100.0, 110.0, 110.0];
        let (out, removed) = filter_grass_v3_in_rects(0, 0, &data, &[rect]).unwrap();
        assert_eq!(removed, 0);
        assert_eq!(out, data);
    }

    #[test]
    fn grass_filter_rejects_bad_magic() {
        let mut data = encode_grass([&[(1.0, 1.0)], &[], &[]]);
        data[0] = 0xff;
        assert!(filter_grass_v3_in_rects(0, 0, &data, &[[0.0, 0.0, 1.0, 1.0]]).is_err());
    }
}
