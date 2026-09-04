//! Default plot grades: rings around settlements and dungeon entrances.
//! Inside `inner` is reserved, `inner..outer` is crown, the rest homestead.
//! Served when a region has no edited grade file (doc/LAND_SYSTEM.md).

use std::sync::LazyLock;

use onlinerpg_shared::dungeon::entrances;
use onlinerpg_shared::shortest_world_delta_x;
use onlinerpg_terrain::defaults::TILE_DIM;
use onlinerpg_terrain::land::{plot_origin, LandGrade, PLOT_SIZE, REGION_PLOTS};
use serde::Deserialize;

const REGION_METERS: f32 = 16.0 * TILE_DIM as f32;

#[derive(Deserialize)]
struct MapLabel {
    kind: String,
    x: f32,
    z: f32,
}

struct Anchor {
    x: f32,
    z: f32,
    inner: f32,
    outer: f32,
}

fn ring(kind: &str) -> Option<(f32, f32)> {
    match kind {
        "capital" => Some((180.0, 400.0)),
        "city" => Some((120.0, 320.0)),
        "town" => Some((80.0, 250.0)),
        "entrance" => Some((60.0, 150.0)),
        _ => None,
    }
}

fn anchor(x: f32, z: f32, kind: &str) -> Option<Anchor> {
    ring(kind).map(|(inner, outer)| Anchor { x, z, inner, outer })
}

static ANCHORS: LazyLock<Vec<Anchor>> = LazyLock::new(|| {
    let labels: std::collections::HashMap<String, MapLabel> =
        serde_json::from_str(include_str!("../../data/map_labels.json"))
            .expect("data/map_labels.json");
    labels
        .values()
        .filter_map(|l| anchor(l.x, l.z, &l.kind))
        .chain(
            entrances()
                .iter()
                .filter_map(|d| anchor(d.x, d.z, "entrance")),
        )
        .collect()
});

fn grade_at(anchors: &[&Anchor], cx: f32, cz: f32) -> LandGrade {
    let mut grade = LandGrade::Homestead;
    for a in anchors {
        let dx = shortest_world_delta_x(a.x, cx);
        let dz = cz - a.z;
        let d2 = dx * dx + dz * dz;
        if d2 < a.inner * a.inner {
            return LandGrade::Reserved;
        }
        if d2 <= a.outer * a.outer {
            grade = LandGrade::Crown;
        }
    }
    grade
}

pub fn default_grades(rx: i32, rz: i32) -> Vec<u8> {
    let (ox, oz) = plot_origin(rx, rz, 0);
    let (cx, cz) = (
        ox as f32 + REGION_METERS / 2.0,
        oz as f32 + REGION_METERS / 2.0,
    );
    // Only anchors whose outer ring can reach this region matter.
    let near: Vec<&Anchor> = ANCHORS
        .iter()
        .filter(|a| {
            let reach = REGION_METERS / 2.0 + a.outer;
            shortest_world_delta_x(a.x, cx).abs() <= reach && (a.z - cz).abs() <= reach
        })
        .collect();
    if near.is_empty() {
        return vec![LandGrade::Homestead as u8; REGION_PLOTS];
    }
    let half = PLOT_SIZE as f32 / 2.0;
    (0..REGION_PLOTS)
        .map(|i| {
            let (px, pz) = plot_origin(rx, rz, i);
            grade_at(&near, px as f32 + half, pz as f32 + half) as u8
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use onlinerpg_terrain::land::plot_addr;

    #[test]
    fn aldermark_rings_are_reserved_crown_homestead() {
        let town = (-1475.2_f32, 4741.6_f32);
        let core = plot_addr(town.0, town.1);
        assert_eq!(
            default_grades(core.rx, core.rz)[core.index],
            LandGrade::Reserved as u8
        );
        let ring = plot_addr(town.0 + 150.0, town.1);
        assert_eq!(
            default_grades(ring.rx, ring.rz)[ring.index],
            LandGrade::Crown as u8
        );
        let far = plot_addr(town.0 + 600.0, town.1);
        assert_eq!(
            default_grades(far.rx, far.rz)[far.index],
            LandGrade::Homestead as u8
        );
    }

    #[test]
    fn empty_ocean_region_is_all_homestead() {
        assert!(default_grades(10, -10)
            .iter()
            .all(|&g| g == LandGrade::Homestead as u8));
    }
}
