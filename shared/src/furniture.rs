//! Solid-furniture collision — the authoritative footprint table + geometry
//! shared by every client that enforces movement:
//! - the browser (via the `passability_*` wasm exports),
//! - the native agent-client (bots), and
//! - the server (later, for validating client movement / catching cheats).
//!
//! Editor-placed furniture is a runtime-loaded analogue of dungeon props: where
//! `dungeon::floor_passability_cells_inner` seals a prop's cell with `EDGE_ALL`
//! at generation time, here we seal a placed furniture piece's footprint cells
//! at load time. Keeping the footprint table and rasterisation in shared Rust
//! (rather than deriving it from meshes at runtime) is what lets all three
//! callers agree on the exact same solid cells — a prerequisite for the server
//! trusting the client's position.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::pathfinding::{build_furniture_passability, FurniturePiece, RuntimePassability};

/// Model-local XZ footprint of an object, in metres (the GLB's world-space XZ
/// bounding box). Deserialised from the embedded footprint table.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OccupancyRect {
    pub min_x: f32,
    pub max_x: f32,
    pub min_z: f32,
    pub max_z: f32,
}

/// Solid-furniture footprint table, generated from the GLBs by
/// `tools/measure-furniture-footprints.mjs` and embedded at compile time. This
/// is the single source shared by the browser (wasm), agent-client and server —
/// no runtime file IO, and it can never drift from the meshes (regenerate + build).
/// To add furniture: mark it `"solid": true` in the object catalog, run the tool,
/// rebuild.
static FOOTPRINTS_JSON: &str = include_str!("../../data/furniture_footprints.json");

fn footprints() -> &'static HashMap<String, OccupancyRect> {
    static TABLE: OnceLock<HashMap<String, OccupancyRect>> = OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str(FOOTPRINTS_JSON).expect("furniture_footprints.json is malformed")
    })
}

/// Vertical extent (m) above a piece's floor within which it blocks. Furniture is
/// a low obstacle (a table/chest is ~1m), NOT a full-height wall — a character is
/// blocked only when they stand near its level. This must stay well under the
/// ~3.1m inter-floor spacing: a player descending a stairwell sweeps Y
/// continuously between floors, so a taller range falsely blocks them where the
/// stair steps pass metres above floor-level furniture. 1.0 comfortably blocks a
/// same-floor player (who stands only ~0.05m above the furniture's base) while
/// freeing anyone more than 1m above it (i.e. up on the stairs).
pub const FURNITURE_BLOCK_HEIGHT: f32 = 1.0;

/// Cache key for one region's furniture.
pub fn region_cache_key(rx: i32, rz: i32) -> String {
    format!("furniture:{rx},{rz}")
}

/// Local footprint of a solid furniture type, or `None` for decorative types
/// (scrolls, coins, wall torches, …) that never block. Looked up in the embedded
/// footprint table.
pub fn solid_occupancy(type_id: &str) -> Option<OccupancyRect> {
    footprints().get(type_id).copied()
}

/// A placed object, reduced to the fields collision needs. Deserialises straight
/// from the region-object JSON and the client's `ObjectPlacement` — unknown
/// fields (`id`, `rotationX`, `text`) are ignored — so both the wasm and the
/// agent-client entry points parse into it with no hand-written mapping.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FurniturePlacement {
    /// Editor object id, unique within its region file.
    #[serde(default)]
    pub id: u32,
    #[serde(rename = "type")]
    pub type_id: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Yaw about the Y axis, in degrees (THREE `rotation.y` convention).
    #[serde(rename = "rotation", default)]
    pub rotation_deg: f32,
    #[serde(default)]
    pub floor_level: u8,
}

/// Fraction of a 1m cell's area the furniture must cover for the cell to be
/// sealed. A cell is blocked when the furniture occupies ≥40% of its floor area,
/// so a piece that only clips a cell shallowly (small overlap area, whichever
/// axis) is ignored. Intuitive "if furniture sits on most of the tile, you can't
/// walk there". Tune here.
const CELL_COVERAGE_THRESHOLD: f32 = 0.40;

/// World 1m grid cells a placement's rotated footprint seals: every cell the
/// furniture covers by at least [`CELL_COVERAGE_THRESHOLD`] of its area. Falls
/// back to the single most-covered cell so a small piece still seals one.
///
/// The occupancy rect is treated as an oriented box (its four world corners),
/// rotated by the placement yaw — world = (lx·c + lz·s, −lx·s + lz·c), matching
/// the browser's `objectFootprint.rotatedRectAabb`. The overlap is the exact
/// clipped-polygon area, so the result is angle-independent and matches across
/// client, agent-client and server.
pub fn footprint_cells(occ: &OccupancyRect, x: f32, z: f32, rotation_deg: f32) -> Vec<(i32, i32)> {
    let rot = rotation_deg.to_radians();
    let c = rot.cos();
    let s = rot.sin();
    let world = |lx: f32, lz: f32| (x + (lx * c + lz * s), z + (-lx * s + lz * c));
    let corners = [
        world(occ.min_x, occ.min_z),
        world(occ.max_x, occ.min_z),
        world(occ.max_x, occ.max_z),
        world(occ.min_x, occ.max_z),
    ];

    // Candidate cell range = the corners' world-space AABB.
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for &(wx, wz) in &corners {
        min_x = min_x.min(wx);
        max_x = max_x.max(wx);
        min_z = min_z.min(wz);
        max_z = max_z.max(wz);
    }

    let mut cells = Vec::new();
    let mut best_cell = (x.floor() as i32, z.floor() as i32);
    let mut best_area = 0.0;
    for cz in (min_z.floor() as i32)..=(max_z.floor() as i32) {
        for cx in (min_x.floor() as i32)..=(max_x.floor() as i32) {
            let area = cell_overlap_area(&corners, cx, cz);
            if area > best_area {
                best_area = area;
                best_cell = (cx, cz);
            }
            if area >= CELL_COVERAGE_THRESHOLD {
                cells.push((cx, cz));
            }
        }
    }
    if cells.is_empty() {
        cells.push(best_cell);
    }
    cells
}

/// Area (in `0..=1`, since the cell area is 1) of the furniture OBB clipped to
/// the unit cell `[cx, cx+1] × [cz, cz+1]` — the exact Sutherland–Hodgman
/// intersection polygon's shoelace area. `0` when disjoint.
fn cell_overlap_area(corners: &[(f32, f32); 4], cx: i32, cz: i32) -> f32 {
    let x0 = cx as f32;
    let x1 = x0 + 1.0;
    let z0 = cz as f32;
    let z1 = z0 + 1.0;

    // (axis: 0=x/1=z, keep_ge, bound) for each cell edge.
    let edges = [
        (0u8, true, x0),
        (0, false, x1),
        (1, true, z0),
        (1, false, z1),
    ];

    let mut poly: Vec<(f32, f32)> = corners.to_vec();
    for (axis, keep_ge, bound) in edges {
        if poly.len() < 3 {
            return 0.0;
        }
        let coord = |p: (f32, f32)| if axis == 0 { p.0 } else { p.1 };
        let inside = |p: (f32, f32)| {
            if keep_ge {
                coord(p) >= bound
            } else {
                coord(p) <= bound
            }
        };
        let intersect = |a: (f32, f32), b: (f32, f32)| {
            let (ac, bc) = (coord(a), coord(b));
            let t = (bound - ac) / (bc - ac);
            (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1))
        };
        let mut out: Vec<(f32, f32)> = Vec::with_capacity(poly.len() + 1);
        for i in 0..poly.len() {
            let prev = poly[(i + poly.len() - 1) % poly.len()];
            let cur = poly[i];
            let (prev_in, cur_in) = (inside(prev), inside(cur));
            if cur_in {
                if !prev_in {
                    out.push(intersect(prev, cur));
                }
                out.push(cur);
            } else if prev_in {
                out.push(intersect(prev, cur));
            }
        }
        poly = out;
    }

    if poly.len() < 3 {
        return 0.0;
    }
    // Shoelace in the cell's LOCAL frame (subtract x0/z0). The area is
    // translation-invariant, but the clipped polygon sits at world coordinates
    // that can be thousands of metres out (a village far from the origin); the
    // raw `ax*bz - bx*az` products would then be ~1e7 and f32 catastrophic
    // cancellation garbages a sub-1m² area. Shifting to [0,1]-local keeps the
    // products O(1) and the area exact.
    let mut area = 0.0;
    for i in 0..poly.len() {
        let (ax, az) = poly[i];
        let (bx, bz) = poly[(i + 1) % poly.len()];
        area += (ax - x0) * (bz - z0) - (bx - x0) * (az - z0);
    }
    (area * 0.5).abs()
}

/// Whether an object type is solid furniture (has a collision footprint).
pub fn is_solid(type_id: &str) -> bool {
    solid_occupancy(type_id).is_some()
}

/// Resolve a region's placements to solid furniture pieces (one per solid
/// placement, with its footprint cells). Non-solid types are skipped.
pub fn furniture_pieces(placements: &[FurniturePlacement]) -> Vec<FurniturePiece> {
    let mut pieces = Vec::new();
    for p in placements {
        let Some(occ) = solid_occupancy(&p.type_id) else {
            continue;
        };
        let cells = footprint_cells(&occ, p.x, p.z, p.rotation_deg);
        pieces.push(FurniturePiece {
            cells,
            floor_level: p.floor_level,
            y_base: p.y,
            wall_height: FURNITURE_BLOCK_HEIGHT,
        });
    }
    pieces
}

/// Build a passability entry (one `EDGE_ALL`-sealed grid per solid piece) for a
/// region's placements, ready to insert into the shared passability cache.
/// Returns `None` when nothing solid is present (caller should drop the entry).
pub fn build_furniture_passability_for_placements(
    placements: &[FurniturePlacement],
) -> Option<RuntimePassability> {
    build_furniture_passability(&furniture_pieces(placements))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_table_seals_only_its_cell() {
        // Table (1.6m wide × ~1m deep) centred on a cell covers its own cell fully
        // but each X neighbour only ~0.29 of its area (0.3 wide × 0.97 deep) —
        // below the 40% area threshold, so they stay open. Just the core seals.
        let occ = solid_occupancy("table").unwrap();
        let cells = footprint_cells(&occ, 0.5, 0.5, 0.0);
        assert_eq!(cells, vec![(0, 0)], "cells: {cells:?}");
    }

    #[test]
    fn full_x_shallow_z_neighbour_stays_open() {
        // A table shifted toward +Z spans the z=1 row's full width (X = 100%) but
        // only clips it ~29% deep, so its overlap AREA there is ~0.29 < 40% and
        // the z=1 cell stays walkable — while the core z=0 cell (much larger
        // overlap area) still seals. (A pure X-overlap rule wrongly blocked z=1.)
        let occ = solid_occupancy("table").unwrap();
        let cells = footprint_cells(&occ, 0.5, 0.8, 0.0);
        assert!(
            !cells.contains(&(0, 1)),
            "shallow-Z neighbour must stay open: {cells:?}"
        );
        assert!(cells.contains(&(0, 0)), "core cell must seal: {cells:?}");
    }

    #[test]
    fn shallow_corner_clip_stays_open() {
        // A 0.5m block sitting on a cell corner clips each of the 4 touching
        // cells by only 0.25×0.25 = 0.0625 area (well under 30%), so none is
        // sealed by the rule — only the fallback single cell remains.
        let block = OccupancyRect {
            min_x: -0.25,
            max_x: 0.25,
            min_z: -0.25,
            max_z: 0.25,
        };
        let cells = footprint_cells(&block, 1.0, 1.0, 0.0);
        assert_eq!(cells.len(), 1, "cells: {cells:?}");
    }

    #[test]
    fn ninety_degree_rotation_swaps_axes() {
        // A plank long in Z, narrow in X: unrotated it spans more cells in Z;
        // rotated 90° it spans more in X.
        let plank = OccupancyRect {
            min_x: -0.4,
            max_x: 0.4,
            min_z: -1.5,
            max_z: 1.5,
        };
        let span = |cells: &[(i32, i32)]| {
            let xs: Vec<i32> = cells.iter().map(|c| c.0).collect();
            let zs: Vec<i32> = cells.iter().map(|c| c.1).collect();
            (
                xs.iter().max().unwrap() - xs.iter().min().unwrap(),
                zs.iter().max().unwrap() - zs.iter().min().unwrap(),
            )
        };

        let (xu, zu) = span(&footprint_cells(&plank, 0.5, 0.5, 0.0));
        assert!(zu > xu, "unrotated plank should be longer in Z");

        let (xr, zr) = span(&footprint_cells(&plank, 0.5, 0.5, 90.0));
        assert!(xr > zr, "rotated plank should be longer in X");
    }

    #[test]
    fn footprint_stable_far_from_origin() {
        // Regression: the f32 shoelace (`ax*bz - bx*az`) over world coordinates
        // thousands of metres out (a village ~5km from the origin) suffered
        // catastrophic cancellation and over-sealed cells. A table placed there
        // must still seal only its core cell, exactly like the same table near
        // the origin. (This exact placement once wrongly sealed 3 cells.)
        let occ = solid_occupancy("table").unwrap();
        let cells = footprint_cells(&occ, -1468.4, 4732.6, 0.0);
        assert_eq!(
            cells,
            vec![(-1469, 4732)],
            "far table over-sealed: {cells:?}"
        );
    }

    #[test]
    fn placement_deserialises_from_region_json() {
        // Both the wasm path and the agent-client parse this wire shape straight
        // into FurniturePlacement — pin the field renames and the ignored extras.
        let json = r#"{"floorLevel":1,"id":9,"rotation":90.0,"rotationX":5,
                       "type":"table","x":-1.5,"y":0.7,"z":4.2,"text":"hi"}"#;
        let p: FurniturePlacement = serde_json::from_str(json).unwrap();
        assert_eq!(p.type_id, "table");
        assert_eq!(p.rotation_deg, 90.0);
        assert_eq!(p.floor_level, 1);
        assert_eq!(p.x, -1.5);
    }

    #[test]
    fn non_solid_type_has_no_footprint() {
        assert!(solid_occupancy("scroll").is_none());
        assert!(solid_occupancy("coin_pile").is_none());
    }

    #[test]
    fn sub_metre_piece_seals_at_least_one_cell() {
        // A small ammo box away from any cell centre still seals its origin cell.
        let occ = solid_occupancy("ammobox").unwrap();
        let cells = footprint_cells(&occ, 10.05, 20.05, 0.0);
        assert!(!cells.is_empty());
    }
}
