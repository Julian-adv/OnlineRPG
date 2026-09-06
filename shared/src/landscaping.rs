use crate::{fence::FencePlot, shortest_world_delta_x, wrap_world_x, WORLD_MAX_X, WORLD_MIN_X};
use serde::{Deserialize, Serialize};

pub const TOOLBOX_ITEM: &str = "landscaping_toolbox";
pub const DEFAULT_PALETTE: [u8; 2] = [0, 5];
pub const PALETTE_ITEMS: [(u8, &str); 7] = [
    (1, "landscaping_palette_sand"),
    (2, "landscaping_palette_red_soil"),
    (3, "landscaping_palette_snow"),
    (4, "landscaping_palette_gravel"),
    (6, "landscaping_palette_pebbles"),
    (7, "landscaping_palette_stone_path"),
    (8, "landscaping_palette_paving"),
];
pub const TILE_CELLS: usize = 64 * 64;
pub const CLEARED_BYTES: usize = TILE_CELLS / 8;
pub const MAX_ROAD_LENGTH: f32 = 362.1;
pub const FRINGE: f32 = 1.5;

pub fn palette_for_item(item: &str) -> Option<u8> {
    PALETTE_ITEMS
        .iter()
        .find(|(_, id)| *id == item)
        .map(|(slot, _)| *slot)
}

pub fn is_landscaping_item(item: &str) -> bool {
    item == TOOLBOX_ITEM || palette_for_item(item).is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LandscapingTool {
    Ground,
    Road,
    Fence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LandscapingStroke {
    pub start: [f32; 2],
    pub end: Option<[f32; 2]>,
    pub radius: u8,
    pub strength: u8,
    pub palette: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LandscapingTile {
    pub tile_x: i32,
    pub tile_z: i32,
    pub splat: Vec<u8>,
    pub cleared: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct BrushSample {
    pub x: i32,
    pub z: i32,
    pub strength: f32,
    pub fringe: bool,
}

pub fn owns_position(plots: &[FencePlot], x: f32, z: f32) -> bool {
    let x = wrap_world_x(x);
    plots.iter().any(|p| {
        x >= p.x as f32 && x < (p.x + 32) as f32 && z >= p.z as f32 && z < (p.z + 32) as f32
    })
}

pub fn owns_sample(plots: &[FencePlot], x: f32, z: f32) -> bool {
    // A splat corner influences all four adjacent ground cells.
    [-0.5, 0.5].iter().all(|dx| {
        [-0.5, 0.5]
            .iter()
            .all(|dz| owns_position(plots, x + dx, z + dz))
    })
}

impl LandscapingStroke {
    pub fn valid(&self) -> bool {
        let in_world = |p: [f32; 2]| {
            p.iter()
                .all(|v| v.is_finite() && (WORLD_MIN_X..WORLD_MAX_X).contains(v))
        };
        if !(1..=10).contains(&self.radius)
            || !(1..=10).contains(&self.strength)
            || self.palette > 8
            || !in_world(self.start)
        {
            return false;
        }
        self.end.is_none_or(|end| {
            in_world(end)
                && shortest_world_delta_x(self.start[0], end[0]).hypot(end[1] - self.start[1])
                    <= MAX_ROAD_LENGTH
        })
    }

    pub fn samples(&self, plots: &[FencePlot]) -> Vec<BrushSample> {
        if !self.valid() {
            return Vec::new();
        }
        let [x1, z1] = self.start;
        let end = self.end.unwrap_or(self.start);
        let dx = shortest_world_delta_x(x1, end[0]);
        let dz = end[1] - z1;
        let length_sq = dx * dx + dz * dz;
        let radius = f32::from(self.radius);
        let outer = radius + FRINGE;
        let mut samples = Vec::new();
        for z in
            ((z1.min(end[1]) - outer).floor() as i32)..=((z1.max(end[1]) + outer).floor() as i32)
        {
            for x in ((x1.min(x1 + dx) - outer).floor() as i32)
                ..=((x1.max(x1 + dx) + outer).floor() as i32)
            {
                let t = if length_sq > 1e-6 {
                    (((x as f32 - x1) * dx + (z as f32 - z1) * dz) / length_sq).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let distance = (x as f32 - (x1 + t * dx)).hypot(z as f32 - (z1 + t * dz));
                if distance > outer || !owns_sample(plots, x as f32, z as f32) {
                    continue;
                }
                let weight = if self.end.is_some() {
                    let t = ((distance - radius * 0.3) / (radius * 0.7)).clamp(0.0, 1.0);
                    1.0 - t * t * (3.0 - 2.0 * t)
                } else {
                    (-distance * distance / (2.0 * (radius / 2.5).powi(2))).exp()
                };
                samples.push(BrushSample {
                    x: wrap_world_x(x as f32) as i32,
                    z,
                    strength: weight * f32::from(self.strength)
                        / if self.end.is_some() { 10.0 } else { 50.0 },
                    fringe: distance > radius,
                });
            }
        }
        samples
    }
}

pub fn paint_cell(cell: &mut [u8], palette: u8, sample: BrushSample) -> bool {
    let before = [cell[0], cell[1], cell[2], cell[3]];
    let (mut primary, mut secondary) = (cell[0] >> 4, cell[0] & 15);
    let blend = f32::from(cell[2]);
    if sample.fringe {
        if primary == palette || secondary == palette {
            return false;
        }
        if primary == secondary || cell[2] <= 25 {
            secondary = palette;
        } else if cell[2] >= 230 {
            primary = palette;
        } else {
            return false;
        }
    } else {
        let strength = sample.strength.clamp(0.0, 1.0);
        if strength <= 0.0 {
            return false;
        }
        cell[2] = if primary == palette {
            (blend * (1.0 - strength)).round() as u8
        } else if secondary == palette {
            (blend + strength * (255.0 - blend)).round() as u8
        } else if cell[2] < 128 {
            secondary = palette;
            (strength * 255.0).round() as u8
        } else {
            primary = palette;
            (255.0 - strength * 255.0).round() as u8
        };
        let dominant = if cell[2] >= 128 { secondary } else { primary };
        if dominant != 0 {
            cell[3] = 0;
        }
        cell[1] = 0;
    }
    cell[0] = primary << 4 | secondary;
    cell != before
}

pub fn is_cleared(mask: &[u8], cell: usize) -> bool {
    cell < TILE_CELLS
        && mask
            .get(cell / 8)
            .is_some_and(|byte| byte & (1 << (cell % 8)) != 0)
}

pub fn clear_cell(mask: &mut [u8], cell: usize) {
    mask[cell / 8] |= 1 << (cell % 8);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(start: [f32; 2], end: Option<[f32; 2]>) -> LandscapingStroke {
        LandscapingStroke {
            start,
            end,
            radius: 3,
            strength: 10,
            palette: 5,
        }
    }

    #[test]
    fn clips_the_whole_road_and_its_fringe_to_owned_plots() {
        let plots = [FencePlot { x: 0, z: 0 }, FencePlot { x: 64, z: 0 }];
        let samples = stroke([16.0, 16.0], Some([80.0, 16.0])).samples(&plots);
        assert!(samples.iter().any(|s| s.x == 16));
        assert!(samples.iter().any(|s| s.x == 80));
        assert!(samples
            .iter()
            .all(|s| (1..32).contains(&s.x) || (65..96).contains(&s.x)));
        assert!(samples.iter().all(|s| (1..32).contains(&s.z)));
    }

    #[test]
    fn corner_blending_cannot_spill_across_the_estate_boundary() {
        let plots = [FencePlot { x: 0, z: 0 }];
        let samples = stroke([0.0, 1.0], None).samples(&plots);
        assert!(!samples.is_empty());
        assert!(samples.iter().all(|s| s.x > 0 && s.z > 0));
        assert!(stroke([-10.0, 1.0], None).samples(&plots).is_empty());
    }

    #[test]
    fn roads_take_the_short_path_across_the_world_seam() {
        let plots = [
            FencePlot {
                x: WORLD_MIN_X as i32,
                z: 0,
            },
            FencePlot {
                x: WORLD_MAX_X as i32 - 32,
                z: 0,
            },
        ];
        let samples =
            stroke([WORLD_MAX_X - 4.0, 16.0], Some([WORLD_MIN_X + 4.0, 16.0])).samples(&plots);
        assert!(samples.iter().any(|s| s.x == WORLD_MIN_X as i32));
        assert!(samples
            .iter()
            .all(|s| s.x < WORLD_MIN_X as i32 + 10 || s.x > WORLD_MAX_X as i32 - 10));
    }

    #[test]
    fn invalid_or_oversized_brushes_have_no_samples() {
        for invalid in [
            stroke([f32::NAN, 0.0], None),
            stroke([0.0, 0.0], Some([1000.0, 0.0])),
            LandscapingStroke {
                radius: 255,
                ..stroke([0.0, 0.0], None)
            },
        ] {
            assert!(!invalid.valid());
            assert!(invalid.samples(&[FencePlot { x: 0, z: 0 }]).is_empty());
        }
    }

    #[test]
    fn painting_meadow_does_not_restore_vegetation() {
        let sample = BrushSample {
            x: 10,
            z: 10,
            strength: 1.0,
            fringe: false,
        };
        let mut cell = [0, 0, 0, 239];
        assert!(paint_cell(&mut cell, 5, sample));
        assert_eq!(cell[3], 0);
        assert!(paint_cell(&mut cell, 0, sample));
        assert_eq!(cell[3], 0);
        assert_eq!(cell[2], 0);
    }
}
