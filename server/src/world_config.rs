use crate::terrain::io::TerrainIO;
use onlinerpg_shared::NoSpawnZone;
use serde::Deserialize;
use std::sync::LazyLock;
use tracing::info;

#[derive(Debug, Deserialize)]
pub struct WorldConfig {
    #[serde(rename = "spawnPosition")]
    pub spawn_position: SpawnPosition,
    /// Live ambient monsters one player may own at once, across every type.
    /// Kept small so the field stays sparse now that dungeons carry the dense
    /// fights. The only server-wide bound there is: no global ceiling is
    /// tracked, because this cap times the player count already is one (plus
    /// corpses, which sit outside the cap and expire in 30s). Loosely enforced
    /// on purpose — the count is read before the requests it gates go out.
    #[serde(rename = "maxMonstersPerPlayer")]
    pub max_monsters_per_player: u32,
    /// Monster types that spawn dynamically around players (no fixed zones).
    #[serde(rename = "ambientSpawns", default)]
    pub ambient_spawns: Vec<AmbientSpawnRule>,
    /// Brains tick on the server; off hands them back to owning clients
    /// (doc/SERVER_SIDE_MONSTER_AI.md).
    #[serde(rename = "serverMonsterAi", default = "default_true")]
    pub server_monster_ai: bool,
    #[serde(default)]
    pub pricing: PricingConfig,
}

/// doc/PRICING.md.
#[derive(Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PricingConfig {
    /// A character seen within this many real days counts as active.
    pub active_days: u32,
    /// Wanted growth of per-active-character gold, per game day.
    pub target_daily_growth: f64,
    pub gain: f64,
    pub max_step_per_meeting: f64,
    pub index_min: f64,
    pub index_max: f64,
}

impl PricingConfig {
    pub fn index_min_percent(&self) -> u32 {
        (self.index_min * 100.0).round() as u32
    }
}

impl Default for PricingConfig {
    fn default() -> Self {
        Self {
            active_days: 30,
            target_daily_growth: 0.002,
            gain: 0.5,
            max_step_per_meeting: 0.1,
            index_min: 0.9,
            index_max: 2.0,
        }
    }
}

/// A monster type that spawns dynamically near players, instead of within a
/// hand-authored rectangle. The server picks the position, keyed to distance
/// walked (`ambient_spawn.rs`).
#[derive(Debug, Clone, Deserialize)]
pub struct AmbientSpawnRule {
    #[serde(rename = "monsterType")]
    pub monster_type: String,
}

#[derive(Debug, Deserialize)]
pub struct SpawnPosition {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rotation: f32,
}

impl SpawnPosition {
    /// The spawn's world position (drops `rotation`, which callers apply
    /// separately alongside floor level 0).
    pub fn position(&self) -> crate::types::Position {
        crate::types::Position {
            x: self.x,
            y: self.y,
            z: self.z,
        }
    }
}

fn default_true() -> bool {
    true
}

static WORLD_CONFIG: LazyLock<WorldConfig> = LazyLock::new(|| {
    let data = include_str!("../../data-src/world.json");
    serde_json::from_str(data).expect("Failed to parse world.json")
});

pub fn world_config() -> &'static WorldConfig {
    &WORLD_CONFIG
}

pub fn log_world_config() {
    let cfg = world_config();
    info!(
        "Spawn position: ({}, {}, {}) rotation: {}",
        cfg.spawn_position.x,
        cfg.spawn_position.y,
        cfg.spawn_position.z,
        cfg.spawn_position.rotation
    );
}

/// Load no-spawn zones (towns, safe areas) from all per-region zone files.
/// Monster spawn areas are no longer authored per-region — see `ambientSpawns`
/// in world.json.
///
/// Read or parse failures propagate rather than defaulting to no zones —
/// booting without them would let monsters spawn inside towns.
pub async fn load_no_spawn_zones_from_regions(
    terrain_io: &TerrainIO,
) -> std::io::Result<Vec<NoSpawnZone>> {
    let mut no_spawn_zones = Vec::new();

    for (rx, rz) in terrain_io.list_zone_regions().await? {
        let json = terrain_io.read_zone(rx, rz).await?;

        if let Some(zones) = json.get("noSpawnZones") {
            let parsed =
                serde_json::from_value::<Vec<NoSpawnZone>>(zones.clone()).map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("bad noSpawnZones in r{rx:+03}_{rz:+03}: {e}"),
                    )
                })?;
            no_spawn_zones.extend(parsed);
        }
    }

    info!(
        "Loaded {} no-spawn zones from region files",
        no_spawn_zones.len()
    );
    Ok(no_spawn_zones)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::unique_temp_dir;

    fn write_zone_file(base: &std::path::Path, contents: &str) {
        let path = onlinerpg_terrain::coords::zone_path(base, 0, 0);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[tokio::test]
    async fn missing_zones_dir_loads_empty() {
        let io = TerrainIO::new(unique_temp_dir("nsz_missing"));
        assert!(load_no_spawn_zones_from_regions(&io)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn loads_zones_from_region_file() {
        let base = unique_temp_dir("nsz_ok");
        write_zone_file(
            &base,
            r#"{"noSpawnZones":[{"minX":0.0,"minZ":0.0,"maxX":5.0,"maxZ":5.0}]}"#,
        );
        let zones = load_no_spawn_zones_from_regions(&TerrainIO::new(base.clone()))
            .await
            .unwrap();
        std::fs::remove_dir_all(base).unwrap();
        assert_eq!(zones.len(), 1);
        assert!(zones[0].contains(1.0, 1.0));
    }

    #[tokio::test]
    async fn unlistable_zones_dir_is_an_error() {
        let base = unique_temp_dir("nsz_unlistable");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("zones"), "not a directory").unwrap();
        let result = load_no_spawn_zones_from_regions(&TerrainIO::new(base.clone())).await;
        std::fs::remove_dir_all(base).unwrap();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn misnamed_zone_file_is_an_error() {
        let base = unique_temp_dir("nsz_misnamed");
        let zones = base.join("zones");
        std::fs::create_dir_all(&zones).unwrap();
        std::fs::write(zones.join("r+00-+00.json"), "{}").unwrap();
        let result = load_no_spawn_zones_from_regions(&TerrainIO::new(base.clone())).await;
        std::fs::remove_dir_all(base).unwrap();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn corrupt_zone_file_is_an_error() {
        let base = unique_temp_dir("nsz_corrupt");
        write_zone_file(&base, "{ not json");
        let result = load_no_spawn_zones_from_regions(&TerrainIO::new(base.clone())).await;
        std::fs::remove_dir_all(base).unwrap();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn bad_no_spawn_zone_shape_is_an_error() {
        let base = unique_temp_dir("nsz_bad_shape");
        write_zone_file(&base, r#"{"noSpawnZones":[{"minX":"oops"}]}"#);
        let result = load_no_spawn_zones_from_regions(&TerrainIO::new(base.clone())).await;
        std::fs::remove_dir_all(base).unwrap();
        assert!(result.is_err());
    }
}
