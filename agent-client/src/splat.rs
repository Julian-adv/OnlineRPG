//! Splatmap tiles for the agent: what the ground is made of (road, sand,
//! cliff, river bed...), sampled per world position. The sampler itself is
//! shared with the server (`onlinerpg_terrain::splat`); what the agent adds
//! is the HTTP tile source, for running off the game host.

use std::path::PathBuf;

use onlinerpg_terrain::defaults::SPLATMAP_SIZE;

pub use onlinerpg_terrain::splat::{SplatSampler, SplatTiles};

/// Splat palette indices, from the baker that writes them.
pub use onlinerpg_shared::worldgen::tile_bake::{
    PAL_CLIFF, PAL_PAVING, PAL_RIVER_BED, PAL_ROAD, PAL_SAND, PAL_SNOW, PAL_STONE_PATH,
};

/// Splat tiles over HTTP with a disk cache — the same plumbing as the
/// heightmap twin, via the shared `HttpTiles` source.
pub struct HttpSplatTiles(crate::terrain_http::HttpTiles);

impl HttpSplatTiles {
    pub fn new(base_url: &str, cache_dir: PathBuf) -> Self {
        Self(crate::terrain_http::HttpTiles::new(
            base_url,
            cache_dir,
            "splat",
            "s_",
            SPLATMAP_SIZE,
        ))
    }
}

#[async_trait::async_trait]
impl SplatTiles for HttpSplatTiles {
    async fn read_splat(&self, tx: i32, tz: i32) -> std::io::Result<Vec<u8>> {
        match self.0.read(tx, tz).await? {
            Some(data) => Ok(data),
            // The splat endpoint answers unbaked tiles with a default 200,
            // so a 404 means the URL is wrong — surface it.
            None => Err(std::io::Error::other(format!(
                "splat tile {tx},{tz}: unexpected 404"
            ))),
        }
    }
}
