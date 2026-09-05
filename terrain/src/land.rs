//! Land plots: tile quadrants, 32 m cells whose edges fall on multiples of 32
//! (tiles are centered on multiples of 64). One grade byte per plot, 1,024
//! per region (doc/LAND_SYSTEM.md).

use crate::coords::{tile_to_region, world_to_tile, wrap_tile_x};
use crate::defaults::TILE_DIM;

pub const PLOT_SIZE: i32 = TILE_DIM as i32 / 2;
pub const REGION_PLOTS: usize = 16 * 16 * 4;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandGrade {
    Reserved = 0,
    Homestead = 1,
    Crown = 2,
}

impl TryFrom<u8> for LandGrade {
    type Error = u8;

    fn try_from(byte: u8) -> Result<Self, u8> {
        match byte {
            0 => Ok(Self::Reserved),
            1 => Ok(Self::Homestead),
            2 => Ok(Self::Crown),
            other => Err(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlotAddr {
    pub rx: i32,
    pub rz: i32,
    pub index: usize,
}

/// Tile index and quadrant bit along one axis.
fn tile_and_quadrant(world: f32) -> (i32, i32) {
    let tile = world_to_tile(world);
    let col = (world / PLOT_SIZE as f32).floor() as i32;
    (tile, col + 1 - 2 * tile)
}

pub fn plot_addr(x: f32, z: f32) -> PlotAddr {
    let (tx, qx) = tile_and_quadrant(x);
    let (tz, qz) = tile_and_quadrant(z);
    let tx = wrap_tile_x(tx);
    let rx = tile_to_region(tx);
    let rz = tile_to_region(tz);
    let lx = tx - rx * 16;
    let lz = tz - rz * 16;
    PlotAddr {
        rx,
        rz,
        index: ((lz * 16 + lx) * 4 + qz * 2 + qx) as usize,
    }
}

/// World-space min corner of a plot.
pub fn plot_origin(rx: i32, rz: i32, index: usize) -> (i32, i32) {
    let q = (index % 4) as i32;
    let tile = (index / 4) as i32;
    let tx = rx * 16 + tile % 16;
    let tz = rz * 16 + tile / 16;
    let tile_dim = TILE_DIM as i32;
    (
        tx * tile_dim - PLOT_SIZE + (q % 2) * PLOT_SIZE,
        tz * tile_dim - PLOT_SIZE + (q / 2) * PLOT_SIZE,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_round_trip_through_origin() {
        for &(x, z) in &[
            (-33.0, -33.0),
            (-32.0, -32.0),
            (-1.0, 0.0),
            (0.0, 0.0),
            (31.9, 31.9),
            (32.0, 32.0),
            (1023.0, -1025.0),
            (1024.0, -1024.0),
        ] {
            let a = plot_addr(x, z);
            let (ox, oz) = plot_origin(a.rx, a.rz, a.index);
            assert!(
                ox as f32 <= x && x < (ox + PLOT_SIZE) as f32,
                "{x} -> {a:?} origin {ox}"
            );
            assert!(
                oz as f32 <= z && z < (oz + PLOT_SIZE) as f32,
                "{z} -> {a:?} origin {oz}"
            );
        }
    }

    #[test]
    fn quadrants_split_tile_zero() {
        assert_eq!(plot_addr(-1.0, -1.0).index, 0);
        assert_eq!(plot_addr(1.0, -1.0).index, 1);
        assert_eq!(plot_addr(-1.0, 1.0).index, 2);
        assert_eq!(plot_addr(1.0, 1.0).index, 3);
        assert_eq!(plot_addr(32.0, -1.0).index, 4);
    }

    #[test]
    fn x_wraps_into_canonical_regions() {
        let a = plot_addr(-16.0 * 1024.0 - 33.0, 0.0);
        assert_eq!(a.rx, 15);
        assert_eq!(plot_addr(16.0 * 1024.0 - 32.0, 0.0).rx, -16);
    }
}
