//! Phase 8: offline tree + grass placement baking.
//!
//! Consumes a tile's baked heightmap (65×65 uint16) and splatmap (64×64×4),
//! and emits the per-tile placement binaries the client already knows how to
//! load:
//!
//! * `trees/r±xx_±zz/t_±xxxxx_±zzzzz.bin` — V1 format (magic `TR01`).
//! * `grass/r±xx_±zz/g_±xxxxx_±zzzzz.bin` — V3 quantized format (magic `GR03`).
//!
//! This is a direct port of `client/src/lib/utils/tree-data.ts` and
//! `client/src/lib/utils/grass-data.ts`. Moving placement into the baker lets
//! the offline pipeline emit the full world deterministically rather than
//! relying on the client to regenerate per-tile placements at load time.
//!
//! vegMeta source of truth is the splatmap byte 3, which `tile_bake` already
//! encodes with slope + highland + water-fade density (`short_grass_veg`).
//! Future biome / forest density variations should flow through vegMeta as
//! well so that the runtime and the baker stay aligned on inputs.

use super::tile_bake::{HEIGHT_BIAS, HEIGHT_STEP, TILE_DIM, VERTS_PER_SIDE};
use crate::tree_format::{
    TREE_EXCLUSION_RADIUS, TREE_V1_BYTES_PER_INSTANCE, TREE_V1_HEADER_BYTES, TREE_V1_MAGIC,
    TREE_V1_SCALE,
};

#[inline]
fn decode_height(v: u16) -> f32 {
    v as f32 * HEIGHT_STEP - HEIGHT_BIAS
}

/// Decode a 65×65 uint16 heightmap into f32 meters. Accepts the little-endian
/// byte buffer that `tile_bake::encode_heightmap` writes.
fn decode_heightmap(bytes: &[u8]) -> Vec<f32> {
    debug_assert_eq!(bytes.len(), VERTS_PER_SIDE * VERTS_PER_SIDE * 2);
    let mut out = Vec::with_capacity(VERTS_PER_SIDE * VERTS_PER_SIDE);
    for chunk in bytes.as_chunks::<2>().0 {
        let v = u16::from_le_bytes([chunk[0], chunk[1]]);
        out.push(decode_height(v));
    }
    out
}

/// Bilinear height sample at fractional tile-local coordinates. Matches the
/// client's `sampleHeight`: clamps the input to `[0, TILE_DIM-1]`, then reads
/// the 2×2 neighborhood in the 65×65 vertex grid.
fn sample_height(heights: &[f32], local_x: f32, local_z: f32) -> f32 {
    let td = TILE_DIM as f32;
    let cx = local_x.clamp(0.0, td - 1.0);
    let cz = local_z.clamp(0.0, td - 1.0);
    let ix = cx as usize;
    let iz = cz as usize;
    let fx = cx - ix as f32;
    let fz = cz - iz as f32;
    let ix1 = (ix + 1).min(TILE_DIM);
    let iz1 = (iz + 1).min(TILE_DIM);

    let h00 = heights[iz * VERTS_PER_SIDE + ix];
    let h10 = heights[iz * VERTS_PER_SIDE + ix1];
    let h01 = heights[iz1 * VERTS_PER_SIDE + ix];
    let h11 = heights[iz1 * VERTS_PER_SIDE + ix1];
    let h0 = h00 + (h10 - h00) * fx;
    let h1 = h01 + (h11 - h01) * fx;
    h0 + (h1 - h0) * fz
}

/// 2-channel central-difference slope at cell `(cx, cz)`. Matches the client's
/// `computeSlope` in `tree-data.ts` (edges fall back to the center sample, so
/// the slope is damped at the tile border — accepting a slight asymmetry
/// against neighbor tiles at this scale).
fn compute_slope(heights: &[f32], cx: usize, cz: usize) -> f32 {
    let hc = heights[cz * VERTS_PER_SIDE + cx];
    let hl = if cx > 0 {
        heights[cz * VERTS_PER_SIDE + cx - 1]
    } else {
        hc
    };
    let hr = if cx < TILE_DIM {
        heights[cz * VERTS_PER_SIDE + cx + 1]
    } else {
        hc
    };
    let hu = if cz > 0 {
        heights[(cz - 1) * VERTS_PER_SIDE + cx]
    } else {
        hc
    };
    let hd = if cz < TILE_DIM {
        heights[(cz + 1) * VERTS_PER_SIDE + cx]
    } else {
        hc
    };
    let dx = hr - hl;
    let dz = hd - hu;
    (dx * dx + dz * dz).sqrt() / 2.0
}

// ------------------------------------------------------------------
// Mulberry32 RNG, a 1-to-1 port of `createRng` in `simplex-noise.ts`.
// ------------------------------------------------------------------

/// The JS implementation returns an f64 in [0, 1), and downstream comparisons
/// (`rand() < 0.08` etc.) are all performed in f64. We mirror that here so
/// tree/grass placements are deterministic against the same tile seeds.
struct Rng {
    s: u32,
}

impl Rng {
    fn new(seed_i32: i32) -> Self {
        Self { s: seed_i32 as u32 }
    }

    fn next_f64(&mut self) -> f64 {
        self.s = self.s.wrapping_add(0x6d2b_79f5);
        let mut t = (self.s ^ (self.s >> 15)).wrapping_mul(1 | self.s);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        ((t ^ (t >> 14)) as f64) / 4_294_967_296.0
    }
}

/// `((tx * 48271) ^ (tz * 16807)) | 0` — matches `tileSeed` in `tree-data.ts`.
/// The factors fit easily in i32 for any realistic tile range, so `wrapping_mul`
/// matches the `| 0` truncation in JS.
fn tile_seed_trees(tx: i32, tz: i32) -> i32 {
    tx.wrapping_mul(48271) ^ tz.wrapping_mul(16807)
}

/// `((tx * 73856093) ^ (tz * 19349663)) | 0` — matches `tileSeed` in
/// `grass-data.ts`. These factors overflow i32 for |tile| ≳ 30, but JS's
/// `a * b | 0` truncates to the low 32 bits, which is exactly what
/// `wrapping_mul` gives us.
fn tile_seed_grass(tx: i32, tz: i32) -> i32 {
    tx.wrapping_mul(73_856_093) ^ tz.wrapping_mul(19_349_663)
}

// ------------------------------------------------------------------
// Splat vegMeta bands (must match `grass-material.ts`).
// ------------------------------------------------------------------

const SHORT_GRASS_R_MIN: u8 = 230;
const SHORT_GRASS_R_MAX: u8 = 239;
const TALL_GRASS_R_MIN: u8 = 240;
const TALL_GRASS_R_MAX: u8 = 249;

/// Inverse of `short_grass_veg` / `tall_grass_veg` in the splat baker:
/// returns the 0..=9 density encoded in `r_val`. Caller is responsible for
/// having already verified `r_val` falls inside one of the two grass bands.
#[inline]
fn veg_density(r_val: u8) -> u8 {
    if r_val >= TALL_GRASS_R_MIN {
        r_val - TALL_GRASS_R_MIN
    } else {
        r_val - SHORT_GRASS_R_MIN
    }
}

const CHANNELS: usize = 4;
const VEGMETA_OFFSET: usize = 3;

// Tile-world offset: tile n covers x ∈ [n*TILE_DIM - TILE_DIM/2, n*TILE_DIM + TILE_DIM/2).
// Matches `TERRAIN_TILE_SIZE = 64` in the client.
fn tile_min_world(t: i32) -> f32 {
    t as f32 * TILE_DIM as f32 - TILE_DIM as f32 * 0.5
}

// ==================================================================
// Trees (V1 format).
// ==================================================================

/// Per-cell probability that a grass cell (vegMeta 230–249) tries to spawn a
/// tree. Lowered from the client-runtime value of 0.08 because the bake now
/// covers the whole world — the runtime value was tuned when only visited
/// tiles had data and surrounding tiles silently rendered empty.
const TREE_PROBABILITY: f64 = 0.025;

/// Minimum grass density (within either short or tall band) required for a
/// cell to be eligible for tree spawning. The splatmap fades grass density
/// to zero around rivers (and trails off near other features); gating trees
/// here keeps the sparse fringe cells grass-only so trees don't push right
/// up against the bank.
const TREE_MIN_DENSITY: u8 = 4;

/// Exclusion rectangle in world-space meters: `[min_x, min_z, max_x, max_z]`.
/// Currently only used when baking over an already-laid housing footprint —
/// the offline pipeline doesn't generate houses, so the slice is typically
/// empty. Left in place so the baker can be folded into a replat flow later.
pub type ExclusionRect = [f32; 4];

/// Placement pass for a single tile. Reads vegMeta from the splatmap and the
/// decoded heightmap, returns the V1-encoded tree binary (empty header +
/// zero instances if the tile produced no trees).
pub fn bake_trees(
    tx: i32,
    tz: i32,
    splatmap: &[u8],
    heightmap_bytes: &[u8],
    exclusion_rects: &[ExclusionRect],
) -> Vec<u8> {
    let heights = decode_heightmap(heightmap_bytes);
    let tile_min_x = tile_min_world(tx);
    let tile_min_z = tile_min_world(tz);
    let mut rng = Rng::new(tile_seed_trees(tx, tz));

    // Two parallel arrays of (local_x, local_z, rotation, scale), one per
    // tree type. Local coords are in tile-space [0, TILE_DIM) so encoding
    // into the u16 position slot is a single multiply.
    let mut tree1: Vec<(f32, f32, f32, f32)> = Vec::new();
    let mut tree2: Vec<(f32, f32, f32, f32)> = Vec::new();

    for cz in 0..TILE_DIM {
        for cx in 0..TILE_DIM {
            let r_val = splatmap[(cz * TILE_DIM + cx) * CHANNELS + VEGMETA_OFFSET];
            if !(SHORT_GRASS_R_MIN..=TALL_GRASS_R_MAX).contains(&r_val) {
                continue;
            }
            if veg_density(r_val) < TREE_MIN_DENSITY {
                continue;
            }
            if rng.next_f64() >= TREE_PROBABILITY {
                continue;
            }
            let slope = compute_slope(&heights, cx, cz);
            if slope > 1.5 {
                continue;
            }

            let local_x = cx as f32 + (rng.next_f64() as f32) * 0.8 + 0.1;
            let local_z = cz as f32 + (rng.next_f64() as f32) * 0.8 + 0.1;
            let world_y = sample_height(&heights, local_x, local_z);
            if world_y < 0.5 {
                continue;
            }

            let rotation = (rng.next_f64() as f32) * std::f32::consts::TAU;
            let is_tree1 = rng.next_f64() < 0.5;
            let slot = if is_tree1 { 0 } else { 1 };
            let (scale_min, scale_range) = TREE_V1_SCALE[slot];
            let scale = scale_min + (rng.next_f64() as f32) * scale_range;

            if !exclusion_rects.is_empty() {
                let world_x = tile_min_x + local_x;
                let world_z = tile_min_z + local_z;
                let r = TREE_EXCLUSION_RADIUS[slot] * scale;
                let mut blocked = false;
                for &[r_min_x, r_min_z, r_max_x, r_max_z] in exclusion_rects {
                    if world_x > r_min_x - r
                        && world_x < r_max_x + r
                        && world_z > r_min_z - r
                        && world_z < r_max_z + r
                    {
                        blocked = true;
                        break;
                    }
                }
                if blocked {
                    continue;
                }
            }

            let bucket = if is_tree1 { &mut tree1 } else { &mut tree2 };
            bucket.push((local_x, local_z, rotation, scale));
        }
    }

    encode_tree_v1(&tree1, &tree2)
}

fn encode_tree_v1(tree1: &[(f32, f32, f32, f32)], tree2: &[(f32, f32, f32, f32)]) -> Vec<u8> {
    let total = tree1.len() + tree2.len();
    let mut out = Vec::with_capacity(TREE_V1_HEADER_BYTES + total * TREE_V1_BYTES_PER_INSTANCE);
    out.extend_from_slice(&TREE_V1_MAGIC.to_le_bytes());
    out.extend_from_slice(&(tree1.len() as u32).to_le_bytes());
    out.extend_from_slice(&(tree2.len() as u32).to_le_bytes());

    let pos_scale = 65535.0 / TILE_DIM as f32;
    let rot_scale = 255.0 / std::f32::consts::TAU;

    for (slot, list) in [tree1, tree2].iter().enumerate() {
        let (scale_min, scale_range) = TREE_V1_SCALE[slot];
        let scale_scale = 255.0 / scale_range;
        for &(lx, lz, rot, scale) in list.iter() {
            let px = (lx * pos_scale).round().clamp(0.0, 65535.0) as u16;
            let pz = (lz * pos_scale).round().clamp(0.0, 65535.0) as u16;
            let r = ((rot * rot_scale).round() as i32) & 0xff;
            let s = ((scale - scale_min) * scale_scale)
                .round()
                .clamp(0.0, 255.0) as u8;
            out.extend_from_slice(&px.to_le_bytes());
            out.extend_from_slice(&pz.to_le_bytes());
            out.push(r as u8);
            out.push(s);
        }
    }

    out
}

// ==================================================================
// Grass (V3 quantized format).
// ==================================================================

/// On-disk grass placement format (magic "GR03"): 16-byte header
/// (`magic`, `shortCount`, `tallCount`, `flowerCount`) + 6 bytes per instance
/// (`u16 px`, `u16 pz`, `u8 rotation`, `u8 scale`). Public so readers (e.g. the
/// offline `apply-houses` grass filter) parse the exact format this writes.
pub const GRASS_V3_MAGIC: u32 = 0x4752_3033; // "GR03"
pub const GRASS_V3_HEADER_BYTES: usize = 16;
pub const GRASS_V3_BYTES_PER_INSTANCE: usize = 6;

const SHORT_SCALE_MIN: f32 = 0.4;
const SHORT_SCALE_RANGE: f32 = 0.3;
const TALL_SCALE_MIN: f32 = 0.5;
const TALL_SCALE_RANGE: f32 = 1.0;
const FLOWER_SCALE_MIN: f32 = 0.42;
const FLOWER_SCALE_RANGE: f32 = 0.18;

// Per-axis blade count inside one splat cell. A cell emits up to N² blades
// (some filtered by the per-density probability). Lowered from the client-
// runtime values (12 / 10) because the bake covers the full world — client
// values were tuned for tiles the user actually visited with the surrounding
// area rendering empty.
const SHORT_BLADES_PER_AXIS: u32 = 8;
const TALL_BLADES_PER_AXIS: u32 = 6;
const BOUNDARY_BLEND_RATIO: f64 = 0.3;

#[derive(Clone, Copy)]
struct VegParams {
    r_min: u8,
    r_max: u8,
    scale_min: f32,
    scale_range: f32,
    blades_per_axis: u32,
}

const SHORT_PARAMS: VegParams = VegParams {
    r_min: SHORT_GRASS_R_MIN,
    r_max: SHORT_GRASS_R_MAX,
    scale_min: SHORT_SCALE_MIN,
    scale_range: SHORT_SCALE_RANGE,
    blades_per_axis: SHORT_BLADES_PER_AXIS,
};
const TALL_PARAMS: VegParams = VegParams {
    r_min: TALL_GRASS_R_MIN,
    r_max: TALL_GRASS_R_MAX,
    scale_min: TALL_SCALE_MIN,
    scale_range: TALL_SCALE_RANGE,
    blades_per_axis: TALL_BLADES_PER_AXIS,
};

const NEIGHBOR_OFFSETS: [(i32, i32); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];

/// A cell borders the other grass type if any 4-neighbor vegMeta falls in
/// that type's R-range. Used to convert a fraction of blades across the
/// border so the short/tall transition doesn't snap at the cell edge.
fn is_boundary_cell(
    splatmap: &[u8],
    cx: usize,
    cz: usize,
    other_r_min: u8,
    other_r_max: u8,
) -> bool {
    for (dx, dz) in NEIGHBOR_OFFSETS {
        let nx = cx as i32 + dx;
        let nz = cz as i32 + dz;
        if nx < 0 || nx >= TILE_DIM as i32 || nz < 0 || nz >= TILE_DIM as i32 {
            continue;
        }
        let r = splatmap[((nz as usize) * TILE_DIM + nx as usize) * CHANNELS + VEGMETA_OFFSET];
        if r >= other_r_min && r <= other_r_max {
            return true;
        }
    }
    false
}

/// One blade per `(own, converted)` pair: the same cell can emit blades
/// classified as either type depending on the per-blade boundary-blend roll.
struct GrassInstances {
    own: Vec<(f32, f32, f32, f32)>,
    converted: Vec<(f32, f32, f32, f32)>,
}

fn compute_grass_instances(
    params: VegParams,
    other: VegParams,
    tx: i32,
    tz: i32,
    splatmap: &[u8],
    heights: &[f32],
) -> GrassInstances {
    let bpa = params.blades_per_axis;
    let step = 1.0 / bpa as f32;
    let density_range = (params.r_max - params.r_min) as f32;
    let mut rng = Rng::new(tile_seed_grass(tx, tz) ^ params.r_min as i32);

    let mut own: Vec<(f32, f32, f32, f32)> = Vec::new();
    let mut converted: Vec<(f32, f32, f32, f32)> = Vec::new();

    for cz in 0..TILE_DIM {
        for cx in 0..TILE_DIM {
            let r_val = splatmap[(cz * TILE_DIM + cx) * CHANNELS + VEGMETA_OFFSET];
            if r_val < params.r_min || r_val > params.r_max {
                continue;
            }
            let density = if density_range > 0.0 {
                (r_val - params.r_min) as f32 / density_range
            } else {
                1.0
            };
            let boundary = is_boundary_cell(splatmap, cx, cz, other.r_min, other.r_max);

            for _dz in 0..bpa {
                for _dx in 0..bpa {
                    let local_x = cx as f32 + (_dx as f32) * step + (rng.next_f64() as f32) * step;
                    let local_z = cz as f32 + (_dz as f32) * step + (rng.next_f64() as f32) * step;
                    if rng.next_f64() >= density as f64 {
                        continue;
                    }
                    let world_y = sample_height(heights, local_x, local_z);
                    if world_y < 0.05 {
                        continue;
                    }

                    let rotation = (rng.next_f64() as f32) * std::f32::consts::TAU;
                    let is_converted = boundary && rng.next_f64() < BOUNDARY_BLEND_RATIO;
                    let (scale_min, scale_range) = if is_converted {
                        (other.scale_min, other.scale_range)
                    } else {
                        (params.scale_min, params.scale_range)
                    };
                    let scale = scale_min + (rng.next_f64() as f32) * scale_range;

                    let bucket = if is_converted {
                        &mut converted
                    } else {
                        &mut own
                    };
                    bucket.push((local_x, local_z, rotation, scale));
                }
            }
        }
    }

    GrassInstances { own, converted }
}

fn compute_flower_instances(
    tx: i32,
    tz: i32,
    splatmap: &[u8],
    heights: &[f32],
) -> Vec<(f32, f32, f32, f32)> {
    let mut rng = Rng::new(tile_seed_grass(tx, tz) ^ 0xf10e);
    let density_range = (SHORT_GRASS_R_MAX - SHORT_GRASS_R_MIN) as f32;

    let mut out: Vec<(f32, f32, f32, f32)> = Vec::new();

    for cz in 0..TILE_DIM {
        for cx in 0..TILE_DIM {
            let r_val = splatmap[(cz * TILE_DIM + cx) * CHANNELS + VEGMETA_OFFSET];
            if !(SHORT_GRASS_R_MIN..=SHORT_GRASS_R_MAX).contains(&r_val) {
                continue;
            }

            // Higher grass density → fewer flowers. t=0 sparse → ~40%, t=1 dense → ~5%.
            let t = (r_val - SHORT_GRASS_R_MIN) as f32 / density_range;
            let flower_prob = 0.4f64 * (0.125f64).powf(t as f64);

            // One flower chance per cell, matching the client so
            // tile-seed → flower set is deterministic.
            let local_x = cx as f32 + 0.5 + (rng.next_f64() as f32 - 0.5) * 0.8;
            let local_z = cz as f32 + 0.5 + (rng.next_f64() as f32 - 0.5) * 0.8;
            if rng.next_f64() >= flower_prob {
                continue;
            }

            let world_y = sample_height(heights, local_x, local_z);
            if world_y < 0.05 {
                continue;
            }
            let rotation = (rng.next_f64() as f32) * std::f32::consts::TAU;
            let scale = FLOWER_SCALE_MIN + (rng.next_f64() as f32) * FLOWER_SCALE_RANGE;
            out.push((local_x, local_z, rotation, scale));
        }
    }

    out
}

/// Run short + tall + flower placement for a tile and serialize to V3.
pub fn bake_grass(tx: i32, tz: i32, splatmap: &[u8], heightmap_bytes: &[u8]) -> Vec<u8> {
    let heights = decode_heightmap(heightmap_bytes);

    let short = compute_grass_instances(SHORT_PARAMS, TALL_PARAMS, tx, tz, splatmap, &heights);
    let tall = compute_grass_instances(TALL_PARAMS, SHORT_PARAMS, tx, tz, splatmap, &heights);
    // Cross-swap converted buckets: a short-cell blade that rolled converted
    // renders as tall, and vice versa. Keeps the transition band looking like
    // a gradual mix rather than a hard swap.
    let mut short_list = short.own;
    short_list.extend(tall.converted);
    let mut tall_list = tall.own;
    tall_list.extend(short.converted);
    let flowers = compute_flower_instances(tx, tz, splatmap, &heights);

    encode_grass_v3(&short_list, &tall_list, &flowers)
}

fn encode_grass_v3(
    short_list: &[(f32, f32, f32, f32)],
    tall_list: &[(f32, f32, f32, f32)],
    flowers: &[(f32, f32, f32, f32)],
) -> Vec<u8> {
    let total = short_list.len() + tall_list.len() + flowers.len();
    let mut out = Vec::with_capacity(GRASS_V3_HEADER_BYTES + total * GRASS_V3_BYTES_PER_INSTANCE);
    out.extend_from_slice(&GRASS_V3_MAGIC.to_le_bytes());
    out.extend_from_slice(&(short_list.len() as u32).to_le_bytes());
    out.extend_from_slice(&(tall_list.len() as u32).to_le_bytes());
    out.extend_from_slice(&(flowers.len() as u32).to_le_bytes());

    let pos_scale = 65535.0 / TILE_DIM as f32;
    let rot_scale = 255.0 / std::f32::consts::TAU;

    #[allow(clippy::type_complexity)]
    let buckets: [(&[(f32, f32, f32, f32)], (f32, f32)); 3] = [
        (short_list, (SHORT_SCALE_MIN, SHORT_SCALE_RANGE)),
        (tall_list, (TALL_SCALE_MIN, TALL_SCALE_RANGE)),
        (flowers, (FLOWER_SCALE_MIN, FLOWER_SCALE_RANGE)),
    ];

    for (list, (scale_min, scale_range)) in buckets {
        let scale_scale = 255.0 / scale_range;
        for &(lx, lz, rot, scale) in list.iter() {
            let px = (lx * pos_scale).round().clamp(0.0, 65535.0) as u16;
            let pz = (lz * pos_scale).round().clamp(0.0, 65535.0) as u16;
            let r = ((rot * rot_scale).round() as i32) & 0xff;
            let s = ((scale - scale_min) * scale_scale)
                .round()
                .clamp(0.0, 255.0) as u8;
            out.extend_from_slice(&px.to_le_bytes());
            out.extend_from_slice(&pz.to_le_bytes());
            out.push(r as u8);
            out.push(s);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_heights(meters: f32) -> Vec<u8> {
        let encoded = ((meters + HEIGHT_BIAS) / HEIGHT_STEP).round() as u16;
        let mut out = Vec::with_capacity(VERTS_PER_SIDE * VERTS_PER_SIDE * 2);
        for _ in 0..VERTS_PER_SIDE * VERTS_PER_SIDE {
            out.extend_from_slice(&encoded.to_le_bytes());
        }
        out
    }

    fn splat_with_veg(veg: u8) -> Vec<u8> {
        let mut out = vec![0u8; TILE_DIM * TILE_DIM * 4];
        for cz in 0..TILE_DIM {
            for cx in 0..TILE_DIM {
                out[(cz * TILE_DIM + cx) * 4 + VEGMETA_OFFSET] = veg;
            }
        }
        out
    }

    #[test]
    fn tree_header_is_v1() {
        let h = flat_heights(5.0);
        let s = splat_with_veg(235);
        let bin = bake_trees(0, 0, &s, &h, &[]);
        assert_eq!(&bin[0..4], &TREE_V1_MAGIC.to_le_bytes());
        // 12-byte header; rest must be a multiple of 6 (quantized tree struct).
        assert!(bin.len() >= TREE_V1_HEADER_BYTES);
        let body = bin.len() - TREE_V1_HEADER_BYTES;
        assert_eq!(body % TREE_V1_BYTES_PER_INSTANCE, 0);
    }

    #[test]
    fn grass_header_is_v3() {
        let h = flat_heights(5.0);
        let s = splat_with_veg(239); // dense short grass
        let bin = bake_grass(0, 0, &s, &h);
        assert_eq!(&bin[0..4], &GRASS_V3_MAGIC.to_le_bytes());
        let body = bin.len() - GRASS_V3_HEADER_BYTES;
        assert_eq!(body % GRASS_V3_BYTES_PER_INSTANCE, 0);
    }

    #[test]
    fn trees_skip_underwater_cells() {
        // Height below the 0.5 m floor → every cell rejected after the
        // sample, so tree count is zero even though vegMeta says grass.
        let h = flat_heights(0.2);
        let s = splat_with_veg(235);
        let bin = bake_trees(0, 0, &s, &h, &[]);
        let c1 = u32::from_le_bytes([bin[4], bin[5], bin[6], bin[7]]);
        let c2 = u32::from_le_bytes([bin[8], bin[9], bin[10], bin[11]]);
        assert_eq!(c1 + c2, 0);
    }

    #[test]
    fn grass_skips_underwater_cells() {
        let h = flat_heights(0.02);
        let s = splat_with_veg(239);
        let bin = bake_grass(0, 0, &s, &h);
        let c_short = u32::from_le_bytes([bin[4], bin[5], bin[6], bin[7]]);
        let c_tall = u32::from_le_bytes([bin[8], bin[9], bin[10], bin[11]]);
        let c_flower = u32::from_le_bytes([bin[12], bin[13], bin[14], bin[15]]);
        assert_eq!(c_short + c_tall + c_flower, 0);
    }

    #[test]
    fn trees_skip_low_density_cells() {
        // Sparse grass cells (density < TREE_MIN_DENSITY) must not spawn
        // trees. Without the threshold, river-edge fade cells (density 1-3)
        // would still pick up trees at the global probability.
        let h = flat_heights(5.0);
        let s = splat_with_veg(231); // short grass density 1
        let bin = bake_trees(0, 0, &s, &h, &[]);
        let c1 = u32::from_le_bytes([bin[4], bin[5], bin[6], bin[7]]);
        let c2 = u32::from_le_bytes([bin[8], bin[9], bin[10], bin[11]]);
        assert_eq!(c1 + c2, 0, "trees must skip density-1 short grass cells");

        let s_tall = splat_with_veg(241); // tall grass density 1
        let bin_t = bake_trees(0, 0, &s_tall, &h, &[]);
        let t1 = u32::from_le_bytes([bin_t[4], bin_t[5], bin_t[6], bin_t[7]]);
        let t2 = u32::from_le_bytes([bin_t[8], bin_t[9], bin_t[10], bin_t[11]]);
        assert_eq!(t1 + t2, 0, "trees must skip density-1 tall grass cells");
    }

    #[test]
    fn tree_min_density_boundary_is_exact() {
        // density 3 (just below) → no trees; density 4 (== TREE_MIN_DENSITY)
        // → trees can spawn. With 4096 cells and 0.025 probability the
        // expected count at density 4 is ~100, so a non-zero result is a
        // robust signal.
        let h = flat_heights(5.0);

        let just_below = splat_with_veg(233); // short density 3
        let bin_lo = bake_trees(0, 0, &just_below, &h, &[]);
        let c_lo = u32::from_le_bytes([bin_lo[4], bin_lo[5], bin_lo[6], bin_lo[7]])
            + u32::from_le_bytes([bin_lo[8], bin_lo[9], bin_lo[10], bin_lo[11]]);
        assert_eq!(c_lo, 0, "density 3 (TREE_MIN_DENSITY - 1) must skip trees");

        let at_threshold = splat_with_veg(234); // short density 4
        let bin_hi = bake_trees(0, 0, &at_threshold, &h, &[]);
        let c_hi = u32::from_le_bytes([bin_hi[4], bin_hi[5], bin_hi[6], bin_hi[7]])
            + u32::from_le_bytes([bin_hi[8], bin_hi[9], bin_hi[10], bin_hi[11]]);
        assert!(
            c_hi > 0,
            "density 4 (== TREE_MIN_DENSITY) must allow trees, got {c_hi}"
        );
    }

    #[test]
    fn veg_density_round_trips_encoders() {
        // veg_density is the inverse of short_grass_veg / tall_grass_veg in
        // the splat baker. Drift between the two would cause the tree
        // density gate to silently misclassify cells.
        for d in 0..=9u8 {
            assert_eq!(veg_density(SHORT_GRASS_R_MIN + d), d);
            assert_eq!(veg_density(TALL_GRASS_R_MIN + d), d);
        }
    }

    #[test]
    fn deterministic_for_same_tile() {
        let h = flat_heights(5.0);
        let s = splat_with_veg(235);
        let a = bake_trees(3, -2, &s, &h, &[]);
        let b = bake_trees(3, -2, &s, &h, &[]);
        assert_eq!(a, b);
        let g1 = bake_grass(3, -2, &s, &h);
        let g2 = bake_grass(3, -2, &s, &h);
        assert_eq!(g1, g2);
    }
}
