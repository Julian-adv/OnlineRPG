use crate::{
    coords, defaults,
    io::{write_terrain_file, TerrainIO},
};
use onlinerpg_shared::landscaping::{is_cleared, LandscapingTile, CLEARED_BYTES};
use onlinerpg_shared::tree_format::TREE_V1_MAGIC;
use onlinerpg_shared::worldgen::vegetation::GRASS_V3_MAGIC;
use std::io;

const MAGIC: &[u8; 4] = b"LND1";

impl TerrainIO {
    pub async fn read_landscaping_tile(
        &self,
        tx: i32,
        tz: i32,
    ) -> io::Result<Option<LandscapingTile>> {
        let path = coords::landscaping_path(self.base_dir(), tx, tz);
        let data = match tokio::fs::read(path).await {
            Ok(data) => data,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if data.len() != 4 + defaults::SPLATMAP_SIZE + CLEARED_BYTES || &data[..4] != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid landscaping tile",
            ));
        }
        Ok(Some(LandscapingTile {
            tile_x: coords::wrap_tile_x(tx),
            tile_z: tz,
            splat: data[4..4 + defaults::SPLATMAP_SIZE].to_vec(),
            cleared: data[4 + defaults::SPLATMAP_SIZE..].to_vec(),
        }))
    }

    pub async fn write_landscaping_tile(&self, tile: &LandscapingTile) -> io::Result<()> {
        if tile.splat.len() != defaults::SPLATMAP_SIZE || tile.cleared.len() != CLEARED_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid landscaping tile size",
            ));
        }
        let mut data = Vec::with_capacity(4 + defaults::SPLATMAP_SIZE + CLEARED_BYTES);
        data.extend_from_slice(MAGIC);
        data.extend_from_slice(&tile.splat);
        data.extend_from_slice(&tile.cleared);
        write_terrain_file(
            &coords::landscaping_path(self.base_dir(), tile.tile_x, tile.tile_z),
            &data,
        )
        .await
    }
}

pub fn filter_vegetation(data: Vec<u8>, cleared: &[u8]) -> io::Result<Vec<u8>> {
    if cleared.iter().all(|b| *b == 0) {
        return Ok(data);
    }
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "Invalid vegetation tile");
    if data.len() < 12 {
        return Err(invalid());
    }
    let read = |offset| u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
    let types = match read(0) {
        TREE_V1_MAGIC => 2,
        GRASS_V3_MAGIC => 3,
        _ => return Err(invalid()),
    };
    let header = 4 + types * 4;
    if data.len() < header {
        return Err(invalid());
    }
    let counts: Vec<_> = (0..types).map(|i| read(4 + i * 4) as usize).collect();
    let total: usize = counts.iter().sum();
    if data.len() != header + total * 6 {
        return Err(invalid());
    }
    let mut output = data[..header].to_vec();
    let mut offset = header;
    for (kind, count) in counts.into_iter().enumerate() {
        let mut kept = 0u32;
        for _ in 0..count {
            let instance = &data[offset..offset + 6];
            offset += 6;
            let x = u16::from_le_bytes(instance[..2].try_into().unwrap()) as usize * 64 / 65535;
            let z = u16::from_le_bytes(instance[2..4].try_into().unwrap()) as usize * 64 / 65535;
            if x < 64 && z < 64 && is_cleared(cleared, z * 64 + x) {
                continue;
            }
            output.extend_from_slice(instance);
            kept += 1;
        }
        output[4 + kind * 4..8 + kind * 4].copy_from_slice(&kept.to_le_bytes());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use onlinerpg_shared::landscaping::clear_cell;

    fn vegetation(magic: u32, types: usize) -> Vec<u8> {
        let mut bytes = magic.to_le_bytes().to_vec();
        for _ in 0..types {
            bytes.extend_from_slice(&2u32.to_le_bytes());
        }
        for _ in 0..types {
            for position in [100u16, 40000u16] {
                bytes.extend_from_slice(&position.to_le_bytes());
                bytes.extend_from_slice(&position.to_le_bytes());
                bytes.extend_from_slice(&[5, 6]);
            }
        }
        bytes
    }

    #[test]
    fn filters_every_vegetation_type_only_inside_cleared_cells() {
        let mut mask = vec![0; CLEARED_BYTES];
        clear_cell(&mut mask, 0);
        for (magic, types) in [(TREE_V1_MAGIC, 2), (GRASS_V3_MAGIC, 3)] {
            let data = filter_vegetation(vegetation(magic, types), &mask).unwrap();
            assert_eq!(data.len(), 4 + types * 4 + types * 6);
            for kind in 0..types {
                assert_eq!(
                    u32::from_le_bytes(data[4 + kind * 4..8 + kind * 4].try_into().unwrap()),
                    1
                );
            }
        }
    }

    #[tokio::test]
    async fn a_single_atomic_tile_preserves_paint_and_removal_across_reload() {
        let dir = std::env::temp_dir().join(format!("landscaping_tile_{}", std::process::id()));
        let terrain = TerrainIO::new(dir.clone());
        let mut tile = LandscapingTile {
            tile_x: 0,
            tile_z: 0,
            splat: defaults::default_splatmap(),
            cleared: vec![0; CLEARED_BYTES],
        };
        tile.splat[0] = 0x55;
        clear_cell(&mut tile.cleared, 0);
        terrain
            .write_grass(0, 0, &vegetation(GRASS_V3_MAGIC, 3))
            .await
            .unwrap();
        terrain
            .write_trees(0, 0, &vegetation(TREE_V1_MAGIC, 2))
            .await
            .unwrap();
        terrain.write_landscaping_tile(&tile).await.unwrap();
        let restarted = TerrainIO::new(dir.clone());
        assert_eq!(restarted.read_splatmap(0, 0).await.unwrap()[0], 0x55);
        assert_eq!(restarted.read_grass(0, 0).await.unwrap().unwrap().len(), 34);
        assert_eq!(restarted.read_trees(0, 0).await.unwrap().unwrap().len(), 24);
        restarted
            .write_splatmap(0, 0, &defaults::default_splatmap())
            .await
            .unwrap();
        assert_eq!(restarted.read_splatmap(0, 0).await.unwrap()[0], 0);
        assert_eq!(restarted.read_trees(0, 0).await.unwrap().unwrap().len(), 24);
        restarted.delete_region(0, 0).await.unwrap();
        assert!(restarted
            .read_landscaping_tile(0, 0)
            .await
            .unwrap()
            .is_none());
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
}
