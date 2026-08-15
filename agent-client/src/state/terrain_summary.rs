use super::*;

/// A detached surface-terrain grid render: position and shared samplers
/// snapshotted from `SharedState` so the tile loads (HTTP on a cache miss)
/// never run under the state lock.
pub struct TerrainGridJob {
    px: f32,
    pz: f32,
    py: f32,
    height_sampler: Arc<HeightSampler>,
    splat_sampler: Arc<crate::splat::SplatSampler>,
    world_cache: Arc<std::sync::RwLock<WorldCache>>,
}

impl TerrainGridJob {
    pub async fn render(&self) -> String {
        const CELLS: i32 = GRID_CELLS;
        const CELL_M: f32 = GRID_CELL_M;
        const HALF: i32 = GRID_HALF;

        let (px, pz, py) = (self.px, self.pz, self.py);
        // Height and surface type per cell center (async tile loads).
        let mut heights = vec![None; (CELLS * CELLS) as usize];
        let mut surfaces = vec![None; (CELLS * CELLS) as usize];
        for r in 0..CELLS {
            let cz = pz + (r - HALF) as f32 * CELL_M;
            for c in 0..CELLS {
                let cx = px + (c - HALF) as f32 * CELL_M;
                let i = (r * CELLS + c) as usize;
                heights[i] = self.height_sampler.sample_height(cx, cz).await.ok();
                surfaces[i] = self.splat_sampler.primary_at(cx, cz).await.ok();
            }
        }

        let mut grid: Vec<Vec<char>> = (0..CELLS)
            .map(|r| {
                (0..CELLS)
                    .map(|c| {
                        let i = (r * CELLS + c) as usize;
                        ground_char(surfaces[i], heights[i])
                    })
                    .collect()
            })
            .collect();

        // Buildings and furniture from the passability cache (sync).
        {
            let world = self.world_cache.read().unwrap();
            let cache = world.passability_cache();
            for r in 0..CELLS {
                let cz = pz + (r - HALF) as f32 * CELL_M;
                for c in 0..CELLS {
                    let cx = px + (c - HALF) as f32 * CELL_M;
                    if pathfinding::is_circle_blocked_on_floor(cache, cx, cz, 1.0, 0, None) {
                        grid[r as usize][c as usize] = '#';
                    }
                }
            }
            // Dungeon entrances.
            for d in world.all_dungeons() {
                overlay(&mut grid, px, pz, d.entrance.x, d.entrance.z, 'D');
            }
        }

        // Terrain and fixed map objects only — players, monsters and NPCs
        // live in the entity lists and [Sighted] events, with exact
        // coordinates there. Mixing them in would go stale within a turn.
        grid[HALF as usize][HALF as usize] = '@';

        // Row labels carry exact z, the header carries the x span, so the
        // agent can map any cell to world coordinates without arithmetic
        // guesswork.
        let west_x = px - HALF as f32 * CELL_M;
        let east_x = px + HALF as f32 * CELL_M;
        let mut out = format!(
            "Map: surface, you at ({px:.0}, {pz:.0}) — {size}x{size}m, {cell:.0}m per cell, \
             north up. Columns left to right: x={west:.0} to x={east:.0} (+{cell:.0} per \
             column). Row labels are that row's z.\n",
            size = CELLS * CELL_M as i32,
            cell = CELL_M,
            west = west_x,
            east = east_x,
            px = px,
            pz = pz,
        );
        for (r, row) in grid.iter().enumerate() {
            let cz = pz + (r as i32 - HALF) as f32 * CELL_M;
            out.push_str(&format!("z={:<6.0}", cz));
            for ch in row {
                out.push(' ');
                out.push(*ch);
            }
            out.push('\n');
        }
        out.push_str(
            "(. ground  R road  s sand  ~ water  ^ cliff  * snow  # building  \
             D dungeon entrance  @ you; characters and items are in the lists \
             above, not on this map)\n",
        );

        // Gentle slopes don't show in the glyphs; summarize them so climbs
        // are not a surprise. Cliff cells already read as ^.
        let h_at = |r: i32, c: i32| heights[(r * CELLS + c) as usize];
        let mut slopes = Vec::new();
        for (label, r, c) in [
            ("north", 0, HALF),
            ("south", CELLS - 1, HALF),
            ("east", HALF, CELLS - 1),
            ("west", HALF, 0),
        ] {
            if let Some(h) = h_at(r, c) {
                let dh = h - py;
                if dh.abs() >= 2.0 {
                    slopes.push(format!("{label} {dh:+.0}m"));
                }
            }
        }
        if !slopes.is_empty() {
            out.push_str(&format!(
                "Ground height at the map edge vs you: {}.\n",
                slopes.join(", ")
            ));
        }
        out
    }
}

impl SharedState {
    /// Snapshot everything the surface-terrain grid render needs, so the
    /// expensive tile sampling can run without the state lock. None
    /// underground — the floor layout lines already cover the map.
    pub fn terrain_grid_job(&self) -> Option<TerrainGridJob> {
        let p = self.self_player.as_ref()?;
        if self.self_floor_level != 0 {
            return None;
        }
        Some(TerrainGridJob {
            px: p.position.x,
            pz: p.position.z,
            py: p.position.y,
            height_sampler: Arc::clone(&self.height_sampler),
            splat_sampler: Arc::clone(&self.splat_sampler),
            world_cache: Arc::clone(&self.world_cache),
        })
    }
}
