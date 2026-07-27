//! Per-tile baked water field: the *unified* sea + river surface. One
//! field describes every visible water surface in the tile — river
//! channels, the open sea, and the estuary blend between them — so the
//! runtime renders a single quad per water-bearing tile instead of a sea
//! plane plus a river quad stacked with ad-hoc alpha blending.
//!
//! Relationship to `river_field.rs` (RFD1, kept during the rollout for
//! old clients): the per-pixel *river profile* is computed identically,
//! then folded into the sea with `smoothmax(profile, SEA_LEVEL, k)` so
//! the surface glides into the ocean instead of dead-ending 0.5 m above
//! it. Two extra channels drive the runtime shader blend:
//!
//! * `flow` — downstream direction scaled by an estuary decay (no longer
//!   a unit vector): full speed inland, slowing to a drift as the channel
//!   nears the coastline, zero outside the channel envelope.
//! * `riverness` — 1 inside an inland channel, 0 in open sea. The shader
//!   cross-fades river behavior (flow ripples, river palette) against
//!   sea behavior (Gerstner waves, foam bands, caustics).
//!
//! Format `WFD1` (= Water Field Data, version 1):
//!
//! ```text
//! header (16 bytes):
//!   bytes  0..4   magic    b"WFD1"
//!   bytes  4..6   u16      version (currently 1)
//!   bytes  6..8   u16      grid_x  (== VERTS_PER_SIDE = 65)
//!   bytes  8..10  u16      grid_z  (== VERTS_PER_SIDE = 65)
//!   bytes 10..16  u8[6]    reserved (zero)
//!
//! per-pixel (6 bytes, row-major over 65×65, X then Z):
//!   bytes  0..2   u16      surfaceY — encoded same as heightmap
//!                          (HEIGHT_BIAS / HEIGHT_STEP). Water surface at
//!                          this world XZ: river surface in-channel, sea
//!                          level in open sea, smoothmax blend at the
//!                          estuary. On land the river profile collapses
//!                          below the local bed (see river_field.rs), so
//!                          after smoothmax the value is ≥ SEA_LEVEL but
//!                          still under the terrain → depth ≤ 0 → hidden.
//!   byte   2      i8       flowX — downstream flow × 127, estuary-scaled
//!   byte   3      i8       flowZ — downstream flow × 127, estuary-scaled
//!   byte   4      u8       riverness × 255
//!   byte   5      u8       turbulence × 255 — whitewater intensity. Baked
//!                          from three world-space churn sources: slope
//!                          breaks along the flow (the foaming toe where a
//!                          steep reach flattens out), sharp bends (piling
//!                          into the outer bank), and sparse bank-bump
//!                          noise pockets. Zero off-channel and through
//!                          the estuary. Was reserved-zero before
//!                          2026-07-12; old files simply render no river
//!                          foam.
//! ```
//!
//! Cross-tile consistency: same contract as RFD1 — both tiles touching a
//! seam see the same segment list (filtered with the global
//! `river_margin`), so identical world-XZ pixels produce identical
//! records regardless of which tile owns the segment. Sea-only tiles
//! (no river segments in range) get **no file**; the runtime synthesizes
//! a flat `SEA_LEVEL` field with zero flow and zero riverness, which is
//! exactly what this bake would emit there.

use super::super::global_map::GlobalMap;
use super::super::noise::{fbm2, smoothstep, PerlinNoise};
use super::super::vector_features::{
    min_distance_to_segments, project_point_to_segment, segments_near_tile_wrap_x, RiverSegment,
    Segment,
};
use super::constants::{
    HEIGHT_BIAS, HEIGHT_STEP, RIVER_CARVE_TAPER_EXTRA_M, RIVER_CARVE_TAPER_MIN_M,
    RIVER_DEPTH_OFFSET_M, RIVER_OFF_CHANNEL_SAFETY_M, VERTS_PER_SIDE,
};
use super::context::BakeContext;
use super::heightmap::{lerp, sample_carved_bed};

pub const WATER_FIELD_BIN_MAGIC: &[u8; 4] = b"WFD1";
pub const WATER_FIELD_BIN_VERSION: u16 = 1;
pub const WATER_FIELD_HEADER_SIZE: usize = 16;
pub const WATER_FIELD_PIXEL_SIZE: usize = 6;
const WATER_FIELD_PAYLOAD_SIZE: usize = VERTS_PER_SIDE * VERTS_PER_SIDE * WATER_FIELD_PIXEL_SIZE;
pub const WATER_FIELD_TOTAL_SIZE: usize = WATER_FIELD_HEADER_SIZE + WATER_FIELD_PAYLOAD_SIZE;

/// Sea surface elevation (m). The whole worldgen pipeline treats 0 as sea
/// level (heightmap bias, carve floors, coast extraction).
const SEA_LEVEL_M: f32 = 0.0;
/// Longitudinal estuary hand-off, keyed on the distance from the channel
/// centerline (the pixel's projection point) to the nearest coastline:
/// sea-like within `NEAR`, ramping to fully river beyond `FAR`. An
/// earlier gate keyed on the carved bed's elevation (sea level → +1.5 m)
/// instead, but lowland rivers carve their bed to ~sea level for
/// kilometers, so the whole plains course baked out as estuary
/// (riverness 0, flow 0.3) and the runtime shader couldn't tell a
/// mid-plains reach from the mouth.
const ESTUARY_COAST_NEAR_M: f32 = 32.0;
/// See `ESTUARY_COAST_NEAR_M` — full river speed beyond this distance.
const ESTUARY_COAST_FAR_M: f32 = 160.0;
/// Smooth-max radius (m) for folding the river profile into the sea
/// plane. Within ±k of the crossover the two surfaces blend with C1
/// continuity instead of a hard `max()` kink.
const SURFACE_SMOOTHMAX_K_M: f32 = 0.3;
/// Residual flow speed at the mouth (fraction of full speed) — water at
/// the estuary still drifts seaward instead of freezing. Matches the old
/// runtime `flowSpeed = mix(0.3, 1.0, …)` ramp.
const ESTUARY_MIN_FLOW: f32 = 0.3;

/// Whitewater: distance (m) of the carved-bed probes up/downstream of the
/// centerline projection. Sets the length scale a "slope break" is judged
/// over — a rapid shorter than this reads as one steep step. Probes are
/// re-projected onto the channel centerline before sampling: on a meander
/// the straight-line probe cuts the corner into the bank, and the bank's
/// rise read as a fake steep grade that painted every lowland bend white.
const TURB_PROBE_M: f32 = 6.0;
/// Slope-break window on `slope_up − slope_down` (m/m). A reach dropping
/// ≥ TURB_JUMP_HI more per meter upstream than downstream churns the toe
/// fully white (hydraulic jump); below TURB_JUMP_LO it stays glassy.
const TURB_JUMP_LO: f32 = 0.035;
const TURB_JUMP_HI: f32 = 0.13;
/// Steep-run churn window (m/m) on the steeper of the two probe slopes —
/// the rapid itself carries some white even before it flattens out.
const TURB_RAPID_LO: f32 = 0.06;
const TURB_RAPID_HI: f32 = 0.18;
const TURB_RAPID_GAIN: f32 = 0.45;
/// Bend churn: flow direction is re-sampled this far downstream and the
/// turn's |sin| windowed. Lowland meanders rotate the weighted flow a
/// little everywhere — the onset must sit above that background drift or
/// every snaking reach paints white; only genuinely tight corners pass.
const TURB_BEND_PROBE_M: f32 = 7.0;
const TURB_BEND_LO: f32 = 0.30;
const TURB_BEND_HI: f32 = 0.70;
/// Bank-bump pockets: world-space fBm gated to a near-bank band. The
/// frequency puts pockets ~10 m apart so most of a smooth bank stays
/// clean.
const TURB_BANK_NOISE_FREQ: f32 = 0.09;
const TURB_BANK_GAIN: f32 = 0.7;
/// Seed salt for the bank-bump noise ("WATB") — decorrelates it from
/// every other worldgen noise derived from the same world seed.
const TURB_NOISE_SALT: u64 = 0x5741_5442;

/// Slope-break (hydraulic-jump) churn: full white when a steep upstream
/// grade flattens out by ≥ TURB_JUMP_HI m/m over the probe distance.
#[inline]
fn slope_break_term(slope_up: f32, slope_down: f32) -> f32 {
    smoothstep(TURB_JUMP_LO, TURB_JUMP_HI, slope_up - slope_down)
}

/// Bend churn from the |sin| of the flow-direction turn across the
/// downstream probe.
#[inline]
fn bend_term(turn_sin: f32) -> f32 {
    smoothstep(TURB_BEND_LO, TURB_BEND_HI, turn_sin)
}

/// Polynomial smooth maximum: exact `max(a, b)` beyond ±k of the
/// crossover, C1-continuous blend (bulging up to k/4 above both inputs)
/// inside it.
#[inline]
fn smoothmax(a: f32, b: f32, k: f32) -> f32 {
    let h = ((a - b) / k * 0.5 + 0.5).clamp(0.0, 1.0);
    b + (a - b) * h + k * h * (1.0 - h)
}

/// Bake the per-tile unified water field. Returns `None` when the tile
/// carries no river segments — the runtime treats a missing file as
/// "flat sea at `SEA_LEVEL`, no flow, riverness 0" (and skips the tile
/// entirely when the heightmap has no sub-sea vertex either).
pub fn bake_water_field(
    map: &GlobalMap,
    ctx: &BakeContext,
    heights: &[f32],
    tile_origin_x: f32,
    tile_origin_z: f32,
    river_segs: &[RiverSegment],
) -> Option<Vec<u8>> {
    if river_segs.is_empty() {
        return None;
    }

    // Unit tangent per segment — used by every pixel's flow accumulation,
    // so amortize the sqrt over the tile instead of paying it per pixel.
    // Zero-length segments produce (0, 0) which the weighting loop skips.
    let seg_tangents: Vec<(f32, f32)> = river_segs
        .iter()
        .map(|s| {
            let dx = s.bx - s.ax;
            let dz = s.bz - s.az;
            let len_sq = dx * dx + dz * dz;
            if len_sq < 1e-6 {
                (0.0, 0.0)
            } else {
                let inv = 1.0 / len_sq.sqrt();
                (dx * inv, dz * inv)
            }
        })
        .collect();

    // Coast segments for the estuary gate. Filter margin: a pixel's
    // centerline projection sits within `river_margin_m()` of the tile
    // (beyond that the radial envelope is 0 and the gate is moot), and a
    // coast segment farther than `FAR` from the projection can't change
    // the saturated gate — so both tiles at a seam see every segment
    // that matters, keeping the field seam-consistent.
    let tile_max_x = tile_origin_x + (VERTS_PER_SIDE as f32 - 1.0);
    let tile_max_z = tile_origin_z + (VERTS_PER_SIDE as f32 - 1.0);
    let coast_segs = segments_near_tile_wrap_x(
        &ctx.coasts_world,
        tile_origin_x,
        tile_origin_z,
        tile_max_x,
        tile_max_z,
        ESTUARY_COAST_FAR_M + super::river_margin_m(),
        map.config.world_size_m as f32,
    );

    // Bank-bump noise is seeded from the world seed alone (world-space
    // coordinates, no tile inputs), so seam tiles sample identical values.
    let turb_noise = PerlinNoise::new(map.config.seed ^ TURB_NOISE_SALT);

    let mut out = Vec::with_capacity(WATER_FIELD_TOTAL_SIZE);
    out.extend_from_slice(WATER_FIELD_BIN_MAGIC);
    out.extend_from_slice(&WATER_FIELD_BIN_VERSION.to_le_bytes());
    out.extend_from_slice(&(VERTS_PER_SIDE as u16).to_le_bytes());
    out.extend_from_slice(&(VERTS_PER_SIDE as u16).to_le_bytes());
    out.extend_from_slice(&[0u8; 6]);

    for j in 0..VERTS_PER_SIDE {
        for i in 0..VERTS_PER_SIDE {
            let wx = tile_origin_x + i as f32;
            let wz = tile_origin_z + j as f32;
            let bed_y = heights[j * VERTS_PER_SIDE + i];
            let px = compute_pixel(
                wx,
                wz,
                bed_y,
                map,
                ctx,
                river_segs,
                &seg_tangents,
                &coast_segs,
                &turb_noise,
            );
            let v = ((px.surface_y + HEIGHT_BIAS) / HEIGHT_STEP)
                .round()
                .clamp(0.0, 65535.0) as u16;
            out.extend_from_slice(&v.to_le_bytes());
            out.push(encode_unit(px.flow_x) as u8);
            out.push(encode_unit(px.flow_z) as u8);
            out.push((px.riverness.clamp(0.0, 1.0) * 255.0).round() as u8);
            out.push((px.turbulence.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    Some(out)
}

#[inline]
fn encode_unit(v: f32) -> i8 {
    (v.clamp(-1.0, 1.0) * 127.0).round().clamp(-127.0, 127.0) as i8
}

struct WaterPixel {
    surface_y: f32,
    flow_x: f32,
    flow_z: f32,
    riverness: f32,
    turbulence: f32,
}

/// Single-pass query that returns both the inverse-distance-weighted flow
/// direction (averaged across all segments with weight `1/(d² + 1)`) and
/// the nearest segment's `(idx, t)` for surface elevation. Near a Voronoi
/// boundary two segments have comparable weights so the blended direction
/// crosses smoothly; away from boundaries the squared falloff makes the
/// nearest segment dominate. Avoids a separate post-smoothing pass.
fn weighted_flow_and_nearest(
    px: f32,
    pz: f32,
    segs: &[RiverSegment],
    tangents: &[(f32, f32)],
) -> Option<(f32, f32, usize, f32, f32)> {
    if segs.is_empty() {
        return None;
    }
    let mut sx = 0.0f32;
    let mut sz = 0.0f32;
    let mut w_total = 0.0f32;
    let mut best_sq = f32::INFINITY;
    let mut best_idx = 0usize;
    let mut best_t = 0.0f32;
    for (i, s) in segs.iter().enumerate() {
        let (d_sq, t) = project_point_to_segment(px, pz, s.ax, s.az, s.bx, s.bz);
        if d_sq < best_sq {
            best_sq = d_sq;
            best_idx = i;
            best_t = t;
        }
        let (tx, tz) = tangents[i];
        if tx == 0.0 && tz == 0.0 {
            continue;
        }
        let w = 1.0 / (d_sq + 1.0);
        sx += tx * w;
        sz += tz * w;
        w_total += w;
    }
    let best_d = best_sq.sqrt();
    if w_total < 1e-6 {
        return Some((0.0, 0.0, best_idx, best_t, best_d));
    }
    let fx = sx / w_total;
    let fz = sz / w_total;
    let mag = (fx * fx + fz * fz).sqrt();
    let (fx, fz) = if mag > 1e-4 {
        (fx / mag, fz / mag)
    } else {
        // Cancellation (opposing tangents balanced) — fall back to the
        // dominant segment so the pixel still carries a meaningful flow
        // direction instead of stalling the shader's ripple/scroll.
        tangents[best_idx]
    };
    Some((fx, fz, best_idx, best_t, best_d))
}

/// Compute the field record for one pixel.
///
/// The river *profile* by perpendicular distance `dist` is identical to
/// `river_field::compute_pixel` (see the envelope walkthrough there):
/// channel plateau at `bed_at_proj + RIVER_DEPTH_OFFSET_M`, bank fade to
/// the local bed over `(half_width, bank_end]`, then the off-channel
/// safety collapse to `bed − RIVER_OFF_CHANNEL_SAFETY_M`.
///
/// The unified surface then folds the profile into the sea:
/// `smoothmax(profile, SEA_LEVEL, k)`. In open sea the profile sits far
/// below `SEA_LEVEL` → surface is exactly sea level. Inland the profile
/// dominates. At the estuary — where mouth distributaries carve the bed
/// to sub-sea floors so the profile descends through sea level — the
/// smoothmax carries the surface into the ocean with no kink.
///
/// `riverness` fades on two axes: radially, from 1 inside the channel to
/// 0 at `safety_end` (so the invisible off-channel zone reads as sea if
/// terrain edits ever expose it); and longitudinally via the estuary
/// gate — the final coast-adjacent reach of a river behaves like sea
/// even inside the channel envelope. Flow uses the same radial envelope
/// and only decelerates (never zeroes) along the estuary gate, keeping a
/// seaward drift at the mouth.
#[allow(clippy::too_many_arguments)]
fn compute_pixel(
    wx: f32,
    wz: f32,
    bed_y_pixel: f32,
    map: &GlobalMap,
    ctx: &BakeContext,
    river_segs: &[RiverSegment],
    seg_tangents: &[(f32, f32)],
    coast_segs: &[Segment],
    turb_noise: &PerlinNoise,
) -> WaterPixel {
    let Some((flow_x, flow_z, idx, t, dist)) =
        weighted_flow_and_nearest(wx, wz, river_segs, seg_tangents)
    else {
        return WaterPixel {
            surface_y: smoothmax(
                bed_y_pixel - RIVER_OFF_CHANNEL_SAFETY_M,
                SEA_LEVEL_M,
                SURFACE_SMOOTHMAX_K_M,
            ),
            flow_x: 0.0,
            flow_z: 0.0,
            riverness: 0.0,
            turbulence: 0.0,
        };
    };
    let seg = &river_segs[idx];

    // Surface = carved bed at the centerline projection + runtime offset.
    // Re-evaluated via the global elevation pipeline so the value is
    // independent of which tile owns the projection — in a delta wedge
    // the projection can fall outside the tile being baked.
    let proj_x = lerp(seg.ax, seg.bx, t);
    let proj_z = lerp(seg.az, seg.bz, t);
    let bed_at_proj = sample_carved_bed(map, ctx, proj_x, proj_z, river_segs, coast_segs);
    let flow_norm = lerp(seg.flow_norm_a, seg.flow_norm_b, t);
    let width = lerp(seg.width_a, seg.width_b, t);
    let half_width = width * 0.5;
    let taper = RIVER_CARVE_TAPER_MIN_M + RIVER_CARVE_TAPER_EXTRA_M * flow_norm;
    let surface_full = bed_at_proj + RIVER_DEPTH_OFFSET_M;
    let bank_end = half_width + taper;
    let safety_end = bank_end + taper;
    let profile = if dist <= bank_end {
        let s = 1.0 - smoothstep(half_width, bank_end, dist);
        bed_y_pixel + (surface_full - bed_y_pixel) * s
    } else {
        let s = smoothstep(bank_end, safety_end, dist);
        bed_y_pixel - RIVER_OFF_CHANNEL_SAFETY_M * s
    };
    let surface_y = smoothmax(profile, SEA_LEVEL_M, SURFACE_SMOOTHMAX_K_M);

    // Radial envelope: 1 across the visible channel, fading to 0 at the
    // outer safety edge. Slightly wider than the visible bank fade so
    // wave damping ramps in before the bank alpha edge, not on it.
    let radial = 1.0 - smoothstep(half_width, safety_end, dist);
    if radial <= 0.0 {
        // Off-envelope the radial factor zeroes riverness and flow no
        // matter what the gate says — skip the coast-distance scan, which
        // would otherwise run for every off-channel pixel in the tile.
        return WaterPixel {
            surface_y,
            flow_x: 0.0,
            flow_z: 0.0,
            riverness: 0.0,
            turbulence: 0.0,
        };
    }

    // Longitudinal handoff to the sea, keyed on the centerline's distance
    // to the coastline: full sea within NEAR of the coast, full river
    // beyond FAR. Evaluated at the projection (not the pixel) so a whole
    // channel cross-section shares one gate value.
    let coast_dist = min_distance_to_segments(proj_x, proj_z, coast_segs);
    let estuary_gate = smoothstep(ESTUARY_COAST_NEAR_M, ESTUARY_COAST_FAR_M, coast_dist);
    let riverness = radial * estuary_gate;
    let flow_speed = radial * (ESTUARY_MIN_FLOW + (1.0 - ESTUARY_MIN_FLOW) * estuary_gate);

    // ── Whitewater turbulence ──
    // All three churn sources are pure world-space queries (global bed,
    // segment list, world-seeded noise), so seam tiles bake identical
    // bytes. Skipped where nothing could show: past the bank fade
    // (`dist ≥ bank_end`) and through the estuary (calm drift) — the
    // skip also saves the two extra carved-bed probes per pixel.
    let turbulence = if estuary_gate > 0.0 && dist < bank_end {
        // 1. Slope break along the flow: step up/downstream from the
        //    projection, re-project onto the centerline, and read the
        //    carved bed THERE — the true channel gradient. A steep
        //    upstream grade meeting a gentle downstream one is the
        //    foaming toe of a rapid; the steep run itself carries a
        //    weaker churn. (Sampling at the raw stepped point instead
        //    reads the bank on every meander — see TURB_PROBE_M.)
        let bed_along =
            |dx: f32, dz: f32| -> f32 {
                weighted_flow_and_nearest(proj_x + dx, proj_z + dz, river_segs, seg_tangents)
                    .map_or(bed_at_proj, |(_, _, i2, t2, _)| {
                        let s2 = &river_segs[i2];
                        sample_carved_bed(
                            map,
                            ctx,
                            lerp(s2.ax, s2.bx, t2),
                            lerp(s2.az, s2.bz, t2),
                            river_segs,
                            coast_segs,
                        )
                    })
            };
        let up_bed = bed_along(-flow_x * TURB_PROBE_M, -flow_z * TURB_PROBE_M);
        let down_bed = bed_along(flow_x * TURB_PROBE_M, flow_z * TURB_PROBE_M);
        let slope_up = (up_bed - bed_at_proj) / TURB_PROBE_M;
        let slope_down = (bed_at_proj - down_bed) / TURB_PROBE_M;
        let jump = slope_break_term(slope_up, slope_down);
        let rapid =
            smoothstep(TURB_RAPID_LO, TURB_RAPID_HI, slope_up.max(slope_down)) * TURB_RAPID_GAIN;

        // 2. Sharp bend: how far the weighted flow direction rotates over
        //    the downstream probe, thrown mostly at the outer bank (the
        //    side the current piles into).
        let bend = weighted_flow_and_nearest(
            proj_x + flow_x * TURB_BEND_PROBE_M,
            proj_z + flow_z * TURB_BEND_PROBE_M,
            river_segs,
            seg_tangents,
        )
        .map_or(0.0, |(fx2, fz2, ..)| {
            let turn = flow_x * fz2 - flow_z * fx2;
            let side = flow_x * (wz - proj_z) - flow_z * (wx - proj_x);
            let outer = 0.5 - 0.5 * (side / half_width.max(1e-3)).clamp(-1.0, 1.0) * turn.signum();
            bend_term(turn.abs()) * (0.45 + 0.55 * outer)
        });

        // 3. Bank bumps: sparse noise pockets in a near-bank band where an
        //    uneven bank chops the passing current.
        let bank_band = smoothstep(0.55 * half_width, 0.9 * half_width, dist)
            * (1.0 - smoothstep(0.95 * half_width, bank_end, dist));
        let bumps = fbm2(
            turb_noise,
            wx * TURB_BANK_NOISE_FREQ,
            wz * TURB_BANK_NOISE_FREQ,
            2,
            2.0,
            0.5,
        );
        let bank = bank_band * smoothstep(0.30, 0.55, bumps) * TURB_BANK_GAIN;

        // Core churn fades over the bank ramp like the visible water does;
        // the estuary gate calms the whole mouth reach.
        let core_env = 1.0 - smoothstep(half_width, bank_end, dist);
        (((jump + rapid + bend).min(1.0) * core_env + bank).min(1.0)) * estuary_gate
    } else {
        0.0
    };

    WaterPixel {
        surface_y,
        flow_x: flow_x * flow_speed,
        flow_z: flow_z * flow_speed,
        riverness,
        turbulence,
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::vector_features::RiverSegment;
    use super::*;

    fn small_test_ctx() -> (
        crate::worldgen::global_map::GlobalMap,
        crate::worldgen::tile_bake::BakeContext,
    ) {
        // Tiny world keeps the test fast.
        let cfg = crate::worldgen::config::WorldGenConfig {
            seed: 7,
            world_size_m: 256,
            global_res: 32,
            ..Default::default()
        };
        let mut map = crate::worldgen::continent::generate_continent_mask(&cfg);
        crate::worldgen::elevation::generate_elevation(&mut map);
        let rm = crate::worldgen::rivers::compute_flow(&map);
        let net = crate::worldgen::roads::compute_roads(&map, &[], &rm);
        let coast =
            crate::worldgen::coasts::extract_coasts(&map.land_mask, map.config.global_res as usize);
        let ctx = BakeContext::new(&map, &rm, &net, &coast);
        (map, ctx)
    }

    fn pixel(bin: &[u8], i: usize, j: usize) -> (f32, i8, i8, f32, f32) {
        let off = WATER_FIELD_HEADER_SIZE + (j * VERTS_PER_SIDE + i) * WATER_FIELD_PIXEL_SIZE;
        let s = u16::from_le_bytes([bin[off], bin[off + 1]]);
        (
            s as f32 * HEIGHT_STEP - HEIGHT_BIAS,
            bin[off + 2] as i8,
            bin[off + 3] as i8,
            bin[off + 4] as f32 / 255.0,
            bin[off + 5] as f32 / 255.0,
        )
    }

    #[test]
    fn turbulence_terms_respond_to_slope_break_and_bend() {
        // Uniform grade — the water keeps its speed, no churn.
        assert_eq!(slope_break_term(0.2, 0.2), 0.0);
        // Steep upstream flattening out — the foaming toe of a rapid.
        assert_eq!(slope_break_term(0.2, 0.0), 1.0);
        // Gentle-to-steep (the lip of a drop) must NOT foam — whitewater
        // belongs at the bottom of the descent, not its start.
        assert_eq!(slope_break_term(0.0, 0.2), 0.0);
        // Straight flow — no bend churn; a hard turn — full churn.
        assert_eq!(bend_term(0.0), 0.0);
        assert_eq!(bend_term(0.9), 1.0);
    }

    #[test]
    fn empty_segments_returns_none() {
        let (map, ctx) = small_test_ctx();
        let heights = vec![0.0f32; VERTS_PER_SIDE * VERTS_PER_SIDE];
        let bin = bake_water_field(&map, &ctx, &heights, 0.0, 0.0, &[]);
        assert!(bin.is_none());
    }

    #[test]
    fn smoothmax_matches_max_outside_blend_radius() {
        // Exact max() beyond ±k, continuous bump (≤ k/4) at the crossover.
        assert_eq!(smoothmax(5.0, 0.0, 0.3), 5.0);
        assert_eq!(smoothmax(-5.0, 0.0, 0.3), 0.0);
        let at_crossover = smoothmax(0.0, 0.0, 0.3);
        assert!(at_crossover > 0.0 && at_crossover <= 0.3 * 0.25 + 1e-6);
    }

    #[test]
    fn binary_size_matches_layout() {
        // Pin the on-disk layout — runtime decoders hard-code these offsets
        // and any drift would silently corrupt every loader.
        let (map, ctx) = small_test_ctx();
        let heights = vec![5.0f32; VERTS_PER_SIDE * VERTS_PER_SIDE];
        let segs = vec![RiverSegment {
            ax: -10.0,
            az: 0.0,
            bx: 10.0,
            bz: 0.0,
            flow_norm_a: 0.5,
            flow_norm_b: 0.5,
            width_a: 4.0,
            width_b: 4.0,
            bed_floor_a: 0.0,
            bed_floor_b: 0.0,
        }];
        let bin = bake_water_field(&map, &ctx, &heights, -32.0, -32.0, &segs)
            .expect("non-empty segments produce a file");
        assert_eq!(bin.len(), WATER_FIELD_TOTAL_SIZE);
        assert_eq!(&bin[0..4], WATER_FIELD_BIN_MAGIC);
        assert_eq!(
            u16::from_le_bytes([bin[4], bin[5]]),
            WATER_FIELD_BIN_VERSION
        );
        assert_eq!(u16::from_le_bytes([bin[6], bin[7]]), VERTS_PER_SIDE as u16);
        assert_eq!(u16::from_le_bytes([bin[8], bin[9]]), VERTS_PER_SIDE as u16);
    }

    #[test]
    fn surface_collapses_to_sea_floor_beyond_carve_envelope() {
        // Same envelope walkthrough as the RFD1 test, with the WFD1 twist:
        // the collapsed off-channel profile (`bed − SAFETY`) is folded
        // through smoothmax, so on land at bed 5 m the far surface sits at
        // `smoothmax(0, SEA_LEVEL, k)` — the sea plane with the crossover
        // bump — instead of dropping to bed − 5.
        let (map, ctx) = small_test_ctx();
        let heights = vec![5.0f32; VERTS_PER_SIDE * VERTS_PER_SIDE];
        let segs = vec![RiverSegment {
            ax: -32.0,
            az: 0.0,
            bx: 32.0,
            bz: 0.0,
            flow_norm_a: 0.5,
            flow_norm_b: 0.5,
            width_a: 4.0,
            width_b: 4.0,
            bed_floor_a: 0.0,
            bed_floor_b: 0.0,
        }];
        let bin = bake_water_field(&map, &ctx, &heights, -32.0, -32.0, &segs)
            .expect("segment present, file is written");

        // half_width = 2.0, taper = 3.0 + 7.0*0.5 = 6.5.
        // bank_end = 8.5, safety_end = 15.
        let (on_axis, _, _, r_axis, _) = pixel(&bin, 32, 32);
        let (inside_half_width, ..) = pixel(&bin, 32, 33);
        let (safety_ramp, ..) = pixel(&bin, 32, 43); // dist = 11
        let (far, fx_far, fz_far, r_far, t_far) = pixel(&bin, 32, 60); // dist = 28

        assert!(
            (on_axis - inside_half_width).abs() < 0.01,
            "surface stays flat inside the channel: on_axis={on_axis}, inside={inside_half_width}"
        );
        // Off-channel profile values below sea level clamp to the sea
        // plane (bed 5 → profile ≤ 5 − safety·s < 0 at these distances
        // needs s > 5/5; at dist 11 s≈0.34 → profile ≈ 3.3, still land-
        // side, so the ramp is visible; far collapses to sea level).
        assert!(
            safety_ramp < 5.0 - 0.1,
            "safety ramp pulls the surface below the local bed, got {safety_ramp}"
        );
        let far_expected = smoothmax(5.0 - RIVER_OFF_CHANNEL_SAFETY_M, 0.0, SURFACE_SMOOTHMAX_K_M);
        assert!(
            (far - far_expected).abs() < 0.1,
            "far surface folds into the sea plane ({far_expected}m), got {far}"
        );

        // Flow now dies with the radial envelope instead of propagating
        // across the whole tile: past safety_end it must be zero, and the
        // riverness scalar with it.
        assert_eq!((fx_far, fz_far), (0, 0), "flow decays to zero off-channel");
        assert!(r_far < 0.01, "riverness is zero off-channel, got {r_far}");
        assert!(t_far < 0.01, "turbulence is zero off-channel, got {t_far}");

        // On-axis riverness/flow depend on the estuary gate, which reads
        // the coastline distance at the centerline projection — a
        // property of the generated test world, not of the inputs above.
        // Derive the expectation from the same query the bake used.
        let coast_segs = segments_near_tile_wrap_x(
            &ctx.coasts_world,
            -32.0,
            -32.0,
            32.0,
            32.0,
            ESTUARY_COAST_FAR_M + crate::worldgen::tile_bake::river_margin_m(),
            map.config.world_size_m as f32,
        );
        let coast_dist = min_distance_to_segments(0.0, 0.0, &coast_segs);
        let gate = smoothstep(ESTUARY_COAST_NEAR_M, ESTUARY_COAST_FAR_M, coast_dist);
        assert!(
            (r_axis - gate).abs() < 0.01,
            "on-axis riverness equals the estuary gate ({gate}), got {r_axis}"
        );
        let (_, fx_axis, _, _, _) = pixel(&bin, 32, 32);
        let flow_expected = (127.0 * (ESTUARY_MIN_FLOW + (1.0 - ESTUARY_MIN_FLOW) * gate)).round();
        assert!(
            (fx_axis as f32 - flow_expected).abs() <= 2.0,
            "on-axis flowX carries the gated speed ({flow_expected}), got {fx_axis}"
        );
    }

    #[test]
    fn surface_continuous_across_tile_boundary() {
        // Two adjacent tiles baked with the same river polyline must
        // emit byte-identical records on their shared edge. Wide
        // segments (`width_a/b = 50` ≫ natural max) force
        // `mouth_fan_bed_floor` into its fan-drop branch — the path
        // where the pre-fix bake diverged between in-tile bilinear and
        // out-of-tile fallback.
        let (map, ctx) = small_test_ctx();
        let heights = vec![5.0f32; VERTS_PER_SIDE * VERTS_PER_SIDE];
        let segs = vec![
            RiverSegment {
                ax: 33.0,
                az: -30.0,
                bx: 33.0,
                bz: 0.0,
                flow_norm_a: 0.8,
                flow_norm_b: 0.8,
                width_a: 50.0,
                width_b: 50.0,
                bed_floor_a: 0.0,
                bed_floor_b: 0.0,
            },
            RiverSegment {
                ax: 33.0,
                az: 0.0,
                bx: 33.0,
                bz: 30.0,
                flow_norm_a: 0.8,
                flow_norm_b: 0.8,
                width_a: 50.0,
                width_b: 50.0,
                bed_floor_a: 0.0,
                bed_floor_b: 0.0,
            },
        ];
        let bin_a = bake_water_field(&map, &ctx, &heights, -32.0, -32.0, &segs)
            .expect("non-empty segments produce a file");
        let bin_b = bake_water_field(&map, &ctx, &heights, 32.0, -32.0, &segs)
            .expect("non-empty segments produce a file");
        let last_col = VERTS_PER_SIDE - 1;
        for j in 0..VERTS_PER_SIDE {
            let a_off =
                WATER_FIELD_HEADER_SIZE + (j * VERTS_PER_SIDE + last_col) * WATER_FIELD_PIXEL_SIZE;
            let b_off = WATER_FIELD_HEADER_SIZE + (j * VERTS_PER_SIDE) * WATER_FIELD_PIXEL_SIZE;
            assert_eq!(
                &bin_a[a_off..a_off + WATER_FIELD_PIXEL_SIZE],
                &bin_b[b_off..b_off + WATER_FIELD_PIXEL_SIZE],
                "tile A right edge != tile B left edge at j={j}"
            );
        }
    }
}
