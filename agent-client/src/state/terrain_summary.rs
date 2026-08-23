use super::*;
use crate::geom::PlanarDelta;

/// Sampling geometry, derived from the sight radius so the summary spans
/// exactly what the agent can perceive.
const CELL_M: f32 = 3.0;
const CELLS: i32 = (NPC_SIGHT_RADIUS / CELL_M) as i32 * 2 + 1;
const HALF: i32 = CELLS / 2;

#[derive(Clone, Copy)]
enum Feature {
    Road,
    Building,
    Water,
    Cliff,
    Sand,
    Snow,
}

/// Listing order in the summary; index = enum discriminant.
const FEATURES: [Feature; 6] = [
    Feature::Road,
    Feature::Building,
    Feature::Water,
    Feature::Cliff,
    Feature::Sand,
    Feature::Snow,
];

impl Feature {
    fn name(self) -> &'static str {
        match self {
            Feature::Road => "road",
            Feature::Building => "building",
            Feature::Water => "water",
            Feature::Cliff => "cliff",
            Feature::Sand => "sand",
            Feature::Snow => "snow",
        }
    }
}

/// Sea reads from the heightmap (below sea level 0), rivers from the
/// splat's river-bed palette entry. Plain ground is no feature.
fn surface_feature(surface: Option<u8>, height: Option<f32>) -> Option<Feature> {
    if height.is_some_and(|h| h < 0.0) {
        return Some(Feature::Water);
    }
    match surface? {
        crate::splat::PAL_RIVER_BED => Some(Feature::Water),
        crate::splat::PAL_CLIFF => Some(Feature::Cliff),
        crate::splat::PAL_ROAD | crate::splat::PAL_STONE_PATH | crate::splat::PAL_PAVING => {
            Some(Feature::Road)
        }
        crate::splat::PAL_SAND => Some(Feature::Sand),
        crate::splat::PAL_SNOW => Some(Feature::Snow),
        _ => None,
    }
}

/// A detached surface-terrain summary: position and shared samplers
/// snapshotted from `SharedState` so the tile loads (HTTP on a cache miss)
/// never run under the state lock.
pub struct TerrainSummaryJob {
    px: f32,
    pz: f32,
    py: f32,
    height_sampler: Arc<HeightSampler>,
    splat_sampler: Arc<crate::splat::SplatSampler>,
    world_cache: Arc<std::sync::RwLock<WorldCache>>,
}

impl TerrainSummaryJob {
    /// Buildings and furniture come from the passability cache (sync) and
    /// need no terrain sample.
    async fn feature_at(&self, cx: f32, cz: f32) -> Option<Feature> {
        let blocked = {
            let world = self.world_cache.read().unwrap();
            pathfinding::is_circle_blocked_on_floor(world.passability_cache(), cx, cz, 1.0, 0, None)
        };
        if blocked {
            return Some(Feature::Building);
        }
        let height = self.height_sampler.sample_height(cx, cz).await.ok();
        let surface = self.splat_sampler.dominant_at(cx, cz).await.ok();
        surface_feature(surface, height)
    }

    /// One line naming the nearest of each terrain feature within sight, with
    /// distance and compass direction, plus notable ground-height changes.
    /// Dungeon entrances are already listed in the world state.
    pub async fn render(&self) -> String {
        let (px, pz, py) = (self.px, self.pz, self.py);
        let mut nearest: [Option<PlanarDelta>; FEATURES.len()] = [None; FEATURES.len()];
        for r in 0..CELLS {
            for c in 0..CELLS {
                if r == HALF && c == HALF {
                    continue;
                }
                let cx = px + (c - HALF) as f32 * CELL_M;
                let cz = pz + (r - HALF) as f32 * CELL_M;
                let Some(feature) = self.feature_at(cx, cz).await else {
                    continue;
                };
                let d = PlanarDelta::xz(px, pz, cx, cz);
                let slot = &mut nearest[feature as usize];
                if slot.is_none_or(|best| d.dist < best.dist) {
                    *slot = Some(d);
                }
            }
        }

        let mut parts: Vec<String> = FEATURES
            .iter()
            .zip(nearest)
            .filter_map(|(f, n)| {
                n.map(|d| format!("{} {:.0}m {}", f.name(), d.dist, compass(d.dx, d.dz)))
            })
            .collect();
        if parts.is_empty() {
            parts.push("open ground".to_string());
        }
        let radius = HALF as f32 * CELL_M;
        let mut out = format!("Terrain within {radius:.0}m: {}.", parts.join("; "));

        // Gentle slopes don't read as cliffs; note them so climbs are not a
        // surprise.
        let mut slopes = Vec::new();
        for (label, dx, dz) in [
            ("north", 0.0, -radius),
            ("south", 0.0, radius),
            ("east", radius, 0.0),
            ("west", -radius, 0.0),
        ] {
            if let Ok(h) = self.height_sampler.sample_height(px + dx, pz + dz).await {
                let dh = h - py;
                if dh.abs() >= 2.0 {
                    slopes.push(format!("{label} {dh:+.0}m"));
                }
            }
        }
        if !slopes.is_empty() {
            out.push_str(&format!(
                " Ground height {radius:.0}m out vs you: {}.",
                slopes.join(", ")
            ));
        }
        out.push('\n');
        out
    }
}

impl SharedState {
    /// Snapshot everything the surface-terrain summary needs, so the
    /// expensive tile sampling can run without the state lock. None
    /// underground — the floor layout lines already cover the map.
    pub fn terrain_summary_job(&self) -> Option<TerrainSummaryJob> {
        let p = self.self_player.as_ref()?;
        if self.self_floor_level != 0 {
            return None;
        }
        Some(TerrainSummaryJob {
            px: p.position.x,
            pz: p.position.z,
            py: p.position.y,
            height_sampler: Arc::clone(&self.height_sampler),
            splat_sampler: Arc::clone(&self.splat_sampler),
            world_cache: Arc::clone(&self.world_cache),
        })
    }
}
