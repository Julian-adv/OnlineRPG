//! Default plot grades: rings around settlements and dungeon entrances. Inside
//! `inner` is no-build, `inner..outer` is prime, the rest pioneer. Served when
//! a region has no edited grade file (doc/LAND_SYSTEM.md).

use std::sync::LazyLock;

use onlinerpg_shared::dungeon::entrances;
use onlinerpg_shared::shortest_world_delta_x;
use onlinerpg_terrain::land::{
    plot_origin, GRADE_NOBUILD, GRADE_PIONEER, GRADE_PRIME, PLOT_SIZE, REGION_PLOTS,
};
use serde::Deserialize;

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

fn grade_at(cx: f32, cz: f32) -> u8 {
    let mut grade = GRADE_PIONEER;
    for a in ANCHORS.iter() {
        let dx = shortest_world_delta_x(a.x, cx);
        let d = (dx * dx + (cz - a.z) * (cz - a.z)).sqrt();
        if d < a.inner {
            return GRADE_NOBUILD;
        }
        if d <= a.outer {
            grade = GRADE_PRIME;
        }
    }
    grade
}

pub fn default_grades(rx: i32, rz: i32) -> Vec<u8> {
    let half = PLOT_SIZE as f32 / 2.0;
    (0..REGION_PLOTS)
        .map(|i| {
            let (ox, oz) = plot_origin(rx, rz, i);
            grade_at(ox as f32 + half, oz as f32 + half)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use onlinerpg_terrain::land::plot_addr;

    #[test]
    fn aldermark_core_is_nobuild_and_ring_is_prime() {
        let town = (-1475.2_f32, 4741.6_f32);
        let core = plot_addr(town.0, town.1);
        assert_eq!(default_grades(core.rx, core.rz)[core.index], GRADE_NOBUILD);
        let ring = plot_addr(town.0 + 150.0, town.1);
        assert_eq!(default_grades(ring.rx, ring.rz)[ring.index], GRADE_PRIME);
        let far = plot_addr(town.0 + 600.0, town.1);
        assert_eq!(default_grades(far.rx, far.rz)[far.index], GRADE_PIONEER);
    }
}
