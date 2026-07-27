use std::ffi::OsString;
use std::fs::{File, OpenOptions, Permissions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::fs;
use tracing::warn;

use crate::coords;
use crate::defaults;

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Replace a file via same-directory temp + rename. Does not fsync the
/// directory, so entry durability across power loss is not guaranteed.
pub async fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let path = path.to_path_buf();
    let data = data.to_vec();
    tokio::task::spawn_blocking(move || atomic_write_sync(&path, &data))
        .await
        .map_err(std::io::Error::other)?
}

fn atomic_write_sync(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let existing_permissions = match std::fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let (temp_path, mut file) = create_unique_temp_file(path)?;
    let write_result = write_temp(&mut file, data, existing_permissions);
    drop(file);
    commit_or_cleanup(&temp_path, path, write_result)
}

fn write_temp(
    file: &mut File,
    data: &[u8],
    permissions: Option<Permissions>,
) -> std::io::Result<()> {
    file.write_all(data)?;
    if let Some(permissions) = permissions {
        file.set_permissions(permissions)?;
    }
    file.sync_data()
}

fn commit_or_cleanup(
    temp_path: &Path,
    path: &Path,
    write_result: std::io::Result<()>,
) -> std::io::Result<()> {
    let result = write_result.and_then(|_| std::fs::rename(temp_path, path));
    if result.is_err() {
        let _ = std::fs::remove_file(temp_path);
    }
    result
}

fn create_unique_temp_file(path: &Path) -> std::io::Result<(PathBuf, File)> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "target path has no file name",
        )
    })?;

    let attempt = || {
        let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut temp_name = OsString::from(".");
        temp_name.push(file_name);
        temp_name.push(format!(".{}.{counter}.tmp", std::process::id()));
        let candidate = parent.join(temp_name);
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
            .map(|file| (candidate, file))
    };

    match attempt() {
        // A stale temp left by a crashed prior boot can collide once; the next
        // counter value resolves it.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => attempt(),
        result => result,
    }
}

#[cfg(test)]
pub(crate) fn atomic_write_with_injected_failure(
    path: &Path,
    data: &[u8],
    fail_after_bytes: usize,
) -> std::io::Result<()> {
    let (temp_path, mut file) = create_unique_temp_file(path)?;
    let prefix_len = fail_after_bytes.min(data.len());
    let write_result = file
        .write_all(&data[..prefix_len])
        .and_then(|_| Err(std::io::Error::other("injected atomic write failure")));
    drop(file);
    commit_or_cleanup(&temp_path, path, write_result)
}

async fn write_terrain_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    atomic_write(path, data).await
}

pub struct TerrainIO {
    base_dir: PathBuf,
}

impl TerrainIO {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    pub fn base_dir(&self) -> &PathBuf {
        &self.base_dir
    }

    pub async fn read_heightmap(&self, tx: i32, tz: i32) -> std::io::Result<Vec<u8>> {
        let path = coords::heightmap_path(&self.base_dir, tx, tz);
        match fs::read(&path).await {
            Ok(data) if data.len() == defaults::HEIGHTMAP_SIZE => Ok(data),
            Ok(data) => {
                warn!(
                    "Heightmap {:?} has wrong size {} (expected {}), returning default",
                    path,
                    data.len(),
                    defaults::HEIGHTMAP_SIZE
                );
                Ok(defaults::default_heightmap())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(defaults::default_heightmap()),
            Err(e) => Err(e),
        }
    }

    pub async fn write_heightmap(&self, tx: i32, tz: i32, data: &[u8]) -> std::io::Result<()> {
        if data.len() != defaults::HEIGHTMAP_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Heightmap: expected {} bytes, got {}",
                    defaults::HEIGHTMAP_SIZE,
                    data.len()
                ),
            ));
        }
        let path = coords::heightmap_path(&self.base_dir, tx, tz);
        write_terrain_file(&path, data).await
    }

    pub async fn read_splatmap(&self, tx: i32, tz: i32) -> std::io::Result<Vec<u8>> {
        let path = coords::splatmap_path(&self.base_dir, tx, tz);
        match fs::read(&path).await {
            Ok(data) if data.len() == defaults::SPLATMAP_SIZE => Ok(data),
            Ok(data) => {
                warn!(
                    "Splatmap {:?} has wrong size {} (expected {}), returning default",
                    path,
                    data.len(),
                    defaults::SPLATMAP_SIZE
                );
                Ok(defaults::default_splatmap())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(defaults::default_splatmap()),
            Err(e) => Err(e),
        }
    }

    pub async fn write_splatmap(&self, tx: i32, tz: i32, data: &[u8]) -> std::io::Result<()> {
        if data.len() != defaults::SPLATMAP_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Splatmap: expected {} bytes, got {}",
                    defaults::SPLATMAP_SIZE,
                    data.len()
                ),
            ));
        }
        let path = coords::splatmap_path(&self.base_dir, tx, tz);
        write_terrain_file(&path, data).await
    }

    pub async fn read_minimap(&self, rx: i32, rz: i32) -> std::io::Result<Option<Vec<u8>>> {
        let path = coords::minimap_path(&self.base_dir, rx, rz);
        match fs::read(&path).await {
            Ok(data) => Ok(Some(data)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn write_minimap(&self, rx: i32, rz: i32, data: &[u8]) -> std::io::Result<()> {
        let path = coords::minimap_path(&self.base_dir, rx, rz);
        write_terrain_file(&path, data).await
    }

    /// Read pre-computed grass placement data (variable-length binary).
    /// Returns None if the file does not exist.
    pub async fn read_grass(&self, tx: i32, tz: i32) -> std::io::Result<Option<Vec<u8>>> {
        let path = coords::grass_path(&self.base_dir, tx, tz);
        match fs::read(&path).await {
            Ok(data) => Ok(Some(data)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Write pre-computed grass placement data (variable-length binary).
    pub async fn write_grass(&self, tx: i32, tz: i32, data: &[u8]) -> std::io::Result<()> {
        let path = coords::grass_path(&self.base_dir, tx, tz);
        write_terrain_file(&path, data).await
    }

    /// Read original (pre-housing) heightmap. Returns None if not found.
    pub async fn read_original_heightmap(
        &self,
        tx: i32,
        tz: i32,
    ) -> std::io::Result<Option<Vec<u8>>> {
        let path = coords::original_heightmap_path(&self.base_dir, tx, tz);
        match fs::read(&path).await {
            Ok(data) if data.len() == defaults::HEIGHTMAP_SIZE => Ok(Some(data)),
            Ok(data) => {
                warn!(
                    "Original heightmap {:?} has wrong size {} (expected {}), ignoring",
                    path,
                    data.len(),
                    defaults::HEIGHTMAP_SIZE
                );
                Ok(None)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Write original (pre-housing) heightmap.
    pub async fn write_original_heightmap(
        &self,
        tx: i32,
        tz: i32,
        data: &[u8],
    ) -> std::io::Result<()> {
        if data.len() != defaults::HEIGHTMAP_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Original heightmap: expected {} bytes, got {}",
                    defaults::HEIGHTMAP_SIZE,
                    data.len()
                ),
            ));
        }
        let path = coords::original_heightmap_path(&self.base_dir, tx, tz);
        write_terrain_file(&path, data).await
    }

    /// Read pre-computed tree placement data (variable-length binary).
    /// Returns None if the file does not exist.
    pub async fn read_trees(&self, tx: i32, tz: i32) -> std::io::Result<Option<Vec<u8>>> {
        let path = coords::tree_path(&self.base_dir, tx, tz);
        match fs::read(&path).await {
            Ok(data) => Ok(Some(data)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Write pre-computed tree placement data (variable-length binary).
    pub async fn write_trees(&self, tx: i32, tz: i32, data: &[u8]) -> std::io::Result<()> {
        let path = coords::tree_path(&self.base_dir, tx, tz);
        write_terrain_file(&path, data).await
    }

    /// Read per-tile river-field data (pixel-aligned surfaceY + flowDir,
    /// format `RFD1` — see `shared/src/worldgen/tile_bake/river_field.rs`).
    /// Returns None when the offline baker did not produce a file for this
    /// tile (no nearby river segments in the tile's filter window).
    pub async fn read_river_field(&self, tx: i32, tz: i32) -> std::io::Result<Option<Vec<u8>>> {
        let path = coords::river_field_path(&self.base_dir, tx, tz);
        match fs::read(&path).await {
            Ok(data) => Ok(Some(data)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Read per-tile unified water-field data (surfaceY + flow +
    /// riverness, format `WFD1` — see
    /// `shared/src/worldgen/tile_bake/water_field.rs`). Returns None when
    /// the offline baker did not produce a file for this tile (no nearby
    /// river segments — the client synthesizes a flat sea field).
    pub async fn read_water_field(&self, tx: i32, tz: i32) -> std::io::Result<Option<Vec<u8>>> {
        let path = coords::water_field_path(&self.base_dir, tx, tz);
        match fs::read(&path).await {
            Ok(data) => Ok(Some(data)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Read original (pre-housing) grass placement data. Returns None if not found.
    pub async fn read_original_grass(&self, tx: i32, tz: i32) -> std::io::Result<Option<Vec<u8>>> {
        let path = coords::original_grass_path(&self.base_dir, tx, tz);
        match fs::read(&path).await {
            Ok(data) => Ok(Some(data)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Write original (pre-housing) grass placement data.
    pub async fn write_original_grass(&self, tx: i32, tz: i32, data: &[u8]) -> std::io::Result<()> {
        let path = coords::original_grass_path(&self.base_dir, tx, tz);
        write_terrain_file(&path, data).await
    }

    /// Copy current heightmap → original heightmap if original doesn't exist yet.
    /// No-op if original already exists. Returns true if a copy was made.
    pub async fn ensure_original_heightmap(&self, tx: i32, tz: i32) -> std::io::Result<bool> {
        let orig_path = coords::original_heightmap_path(&self.base_dir, tx, tz);
        match fs::metadata(&orig_path).await {
            Ok(_) => return Ok(false), // already exists
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let data = self.read_heightmap(tx, tz).await?;
        self.write_original_heightmap(tx, tz, &data).await?;
        Ok(true)
    }

    /// Copy current grass → original grass if original doesn't exist yet.
    /// No-op if original already exists. Returns true if a copy was made.
    pub async fn ensure_original_grass(&self, tx: i32, tz: i32) -> std::io::Result<bool> {
        let orig_path = coords::original_grass_path(&self.base_dir, tx, tz);
        match fs::metadata(&orig_path).await {
            Ok(_) => return Ok(false), // already exists
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let data = match self.read_grass(tx, tz).await? {
            Some(d) => d,
            None => return Ok(false), // no grass data to snapshot
        };
        self.write_original_grass(tx, tz, &data).await?;
        Ok(true)
    }

    pub async fn delete_region(&self, rx: i32, rz: i32) -> std::io::Result<()> {
        let height_dir = coords::height_region_dir(&self.base_dir, rx, rz);
        let splat_dir = coords::splat_region_dir(&self.base_dir, rx, rz);
        let grass_dir = coords::grass_region_dir(&self.base_dir, rx, rz);
        let tree_dir = coords::tree_region_dir(&self.base_dir, rx, rz);
        let orig_height_dir = coords::original_height_region_dir(&self.base_dir, rx, rz);
        let orig_grass_dir = coords::original_grass_region_dir(&self.base_dir, rx, rz);
        let minimap_file = coords::minimap_path(&self.base_dir, rx, rz);

        for dir in [
            &height_dir,
            &splat_dir,
            &grass_dir,
            &tree_dir,
            &orig_height_dir,
            &orig_grass_dir,
        ] {
            match fs::remove_dir_all(dir).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        match fs::remove_file(&minimap_file).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(())
    }

    /// List all region coordinates that have zone files.
    pub async fn list_zone_regions(&self) -> std::io::Result<Vec<(i32, i32)>> {
        let zones_dir = self.base_dir.join("zones");
        let mut entries = match fs::read_dir(&zones_dir).await {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        let mut regions = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // Parse "r+00_+00.json" pattern
            if let Some(stem) = name.strip_suffix(".json") {
                if let Some(rest) = stem.strip_prefix('r') {
                    if let Some((rx_str, rz_str)) = rest.split_once('_') {
                        if let (Ok(rx), Ok(rz)) = (rx_str.parse::<i32>(), rz_str.parse::<i32>()) {
                            regions.push((rx, rz));
                        }
                    }
                }
            }
        }
        Ok(regions)
    }

    /// Read zone data for a region. Returns empty JSON object if file not found.
    pub async fn read_zone(&self, rx: i32, rz: i32) -> std::io::Result<serde_json::Value> {
        let path = coords::zone_path(&self.base_dir, rx, rz);
        match fs::read_to_string(&path).await {
            Ok(json_str) => serde_json::from_str(&json_str)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(serde_json::Value::Object(Default::default()))
            }
            Err(e) => Err(e),
        }
    }

    /// Write zone data for a region.
    pub async fn write_zone(
        &self,
        rx: i32,
        rz: i32,
        json: &serde_json::Value,
    ) -> std::io::Result<()> {
        let path = coords::zone_path(&self.base_dir, rx, rz);
        let json_str = serde_json::to_string_pretty(json)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        write_terrain_file(&path, json_str.as_bytes()).await
    }

    /// Read object data for a region. Returns empty JSON object if file not found.
    pub async fn read_object(&self, rx: i32, rz: i32) -> std::io::Result<serde_json::Value> {
        let path = coords::object_path(&self.base_dir, rx, rz);
        match fs::read_to_string(&path).await {
            Ok(json_str) => serde_json::from_str(&json_str)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(serde_json::Value::Object(Default::default()))
            }
            Err(e) => Err(e),
        }
    }

    /// Write object data for a region.
    pub async fn write_object(
        &self,
        rx: i32,
        rz: i32,
        json: &serde_json::Value,
    ) -> std::io::Result<()> {
        let path = coords::object_path(&self.base_dir, rx, rz);
        let json_str = serde_json::to_string_pretty(json)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        write_terrain_file(&path, json_str.as_bytes()).await
    }
}
