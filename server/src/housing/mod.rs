pub mod routes;

use onlinerpg_shared::housing::{HouseData, RoomData, RoomType, MAX_FLOOR_LEVEL};
use onlinerpg_shared::world::{WORLD_MAX_X, WORLD_MIN_X};
use onlinerpg_terrain::io::atomic_write;
use std::path::PathBuf;
use tokio::fs;
use tracing::{error, info};

const MIN_ROOM_SIZE: u8 = 3;
const MAX_ROOM_SIZE: u8 = 6;
const MAX_ROOMS: usize = 32;
/// Max room distance from house origin. Kept near `CHUNK_SIZE` so a validated
/// house straddles only a handful of chunks — this is what bounds the
/// `load_neighbors` scan (F-010). See `MAX_NEIGHBOR_CHUNK_SPAN`.
const MAX_ROOM_OFFSET: i32 = 64;

/// Validate house shape and coordinates. Must run before `load_neighbors`,
/// so it bounds every value that feeds the chunk scan (F-010).
pub fn validate_house(house: &HouseData) -> Result<(), String> {
    if house.rooms.is_empty() {
        return Err("House must have at least one room".into());
    }
    if house.rooms.len() > MAX_ROOMS {
        return Err(format!("House must have at most {} rooms", MAX_ROOMS));
    }

    let origin = &house.origin;
    if !origin.is_finite() {
        return Err("House origin must be finite".into());
    }
    // Z shares the world's square extent even though only X wraps
    let in_world = |v: f32| (WORLD_MIN_X..WORLD_MAX_X).contains(&v);
    if !in_world(origin.x) || !in_world(origin.z) {
        return Err("House origin out of world bounds".into());
    }

    for (i, room) in house.rooms.iter().enumerate() {
        if room.local_x.abs() > MAX_ROOM_OFFSET || room.local_z.abs() > MAX_ROOM_OFFSET {
            return Err(format!(
                "Room {} local offset must be within ±{}",
                i, MAX_ROOM_OFFSET
            ));
        }
        // Room size constraints (stairwells allow smaller sizes)
        let min_size = if room.room_type == RoomType::Stairwell {
            1
        } else {
            MIN_ROOM_SIZE
        };
        if room.size_x < min_size || room.size_x > MAX_ROOM_SIZE {
            return Err(format!(
                "Room {} size_x ({}) must be {}-{}",
                i, room.size_x, min_size, MAX_ROOM_SIZE
            ));
        }
        if room.size_z < min_size || room.size_z > MAX_ROOM_SIZE {
            return Err(format!(
                "Room {} size_z ({}) must be {}-{}",
                i, room.size_z, min_size, MAX_ROOM_SIZE
            ));
        }
        if room.wall_height <= 0.0 || room.wall_height > 10.0 {
            return Err(format!(
                "Room {} wall_height ({}) must be 0-10",
                i, room.wall_height
            ));
        }
        if room.floor_level > MAX_FLOOR_LEVEL {
            return Err(format!(
                "Room {} floor_level ({}) must be 0-{}",
                i, room.floor_level, MAX_FLOOR_LEVEL
            ));
        }
        // Wall segment counts must match room dimensions
        let sx = room.size_x as usize;
        let sz = room.size_z as usize;
        if room.wall_north.len() != sx
            || room.wall_south.len() != sx
            || room.wall_east.len() != sz
            || room.wall_west.len() != sz
        {
            return Err(format!(
                "Room {} wall segment counts don't match room size",
                i
            ));
        }
    }

    // Check internal room-room overlap (stairwells may overlap normal rooms)
    for i in 0..house.rooms.len() {
        for j in (i + 1)..house.rooms.len() {
            let a = &house.rooms[i];
            let b = &house.rooms[j];
            if a.room_type == RoomType::Stairwell || b.room_type == RoomType::Stairwell {
                continue;
            }
            if rooms_overlap_world(a, 0.0, 0.0, b, 0.0, 0.0) {
                return Err(format!("Rooms {} and {} overlap", i, j));
            }
        }
    }

    // Validate upper-floor rooms have full floor support from the floor below
    for (i, room) in house.rooms.iter().enumerate() {
        if room.floor_level == 0 {
            continue;
        }
        let support_level = room.floor_level - 1;
        let rx = room.local_x;
        let rz = room.local_z;
        // Check each 1m² cell is supported by a room on the floor below
        for x in rx..(rx + room.size_x as i32) {
            for z in rz..(rz + room.size_z as i32) {
                let supported = house.rooms.iter().any(|other| {
                    other.floor_level == support_level
                        && x >= other.local_x
                        && x < other.local_x + other.size_x as i32
                        && z >= other.local_z
                        && z < other.local_z + other.size_z as i32
                });
                if !supported {
                    return Err(format!(
                        "Room {} ({}F) at ({},{}) has no floor support from {}F",
                        i,
                        room.floor_level + 1,
                        x,
                        z,
                        support_level + 1
                    ));
                }
            }
        }
    }

    Ok(())
}

/// Check overlap against already-loaded neighboring houses.
pub fn validate_house_neighbors(house: &HouseData, neighbors: &[HouseData]) -> Result<(), String> {
    for neighbor in neighbors {
        if neighbor.id == house.id {
            continue;
        }
        for (i, room) in house.rooms.iter().enumerate() {
            for other in neighbor.rooms.iter() {
                if rooms_overlap_world(
                    room,
                    house.origin.x,
                    house.origin.z,
                    other,
                    neighbor.origin.x,
                    neighbor.origin.z,
                ) {
                    return Err(format!("Room {} overlaps with a neighboring house", i));
                }
            }
        }
    }
    Ok(())
}

fn rooms_overlap_world(
    a: &RoomData,
    a_ox: f32,
    a_oz: f32,
    b: &RoomData,
    b_ox: f32,
    b_oz: f32,
) -> bool {
    let ax = a_ox + a.local_x as f32;
    let az = a_oz + a.local_z as f32;
    let bx = b_ox + b.local_x as f32;
    let bz = b_oz + b.local_z as f32;
    ax < bx + b.size_x as f32
        && ax + a.size_x as f32 > bx
        && az < bz + b.size_z as f32
        && az + a.size_z as f32 > bz
        && a.floor_level == b.floor_level
}

/// File-based housing storage, organized by terrain chunk.
///
/// Layout: `{base_dir}/{chunk_x}_{chunk_z}/{house_id}.json`
///
/// Chunk coordinates are derived from house origin using CHUNK_SIZE.
#[derive(Clone)]
pub struct HousingIO {
    base_dir: PathBuf,
}

/// Chunk size in world units — matches terrain tile size.
pub(crate) const CHUNK_SIZE: f32 = 64.0;

/// Per-axis chunk-scan cap for `load_neighbors`, derived from the room
/// offset/size caps so it tracks them automatically. A validated house reaches
/// at most `MAX_ROOM_OFFSET + MAX_ROOM_SIZE` from its origin, spanning only a
/// few chunks; `load_neighbors` enforces this directly (defense-in-depth for
/// F-010) so the scan stays bounded even if it is ever reached without
/// `validate_house`.
pub(crate) const MAX_NEIGHBOR_CHUNK_SPAN: i32 =
    2 * (MAX_ROOM_OFFSET + MAX_ROOM_SIZE as i32) / CHUNK_SIZE as i32 + 4;

fn chunk_prefix(cx: i32, cz: i32) -> String {
    format!("r{:+03}_{:+03}", cx, cz)
}

/// Generate the next house ID from already-loaded houses: `r{cx}_{cz}_{n}`
pub fn next_house_id(cx: i32, cz: i32, existing: &[HouseData]) -> String {
    let prefix = format!("{}_", chunk_prefix(cx, cz));
    let max_n = existing
        .iter()
        .filter_map(|h| h.id.strip_prefix(&prefix)?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    format!("{}{}", prefix, max_n + 1)
}

/// Reject IDs that could escape the housing directory when used as a filename.
/// Real IDs are `r{cx}_{cz}_{n}`, so only alphanumerics, `_`, `+`, `-` are allowed;
/// forbidding `.` and `/` blocks path traversal (`..`) and extension injection.
pub fn is_valid_house_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-'))
}

/// Parse chunk coordinates from a house ID (e.g. `r-24_+07_1` → Some((-24, 7)))
pub fn parse_chunk_from_id(id: &str) -> Option<(i32, i32)> {
    let s = id.strip_prefix('r')?;
    let mut parts = s.splitn(3, '_');
    let cx = parts.next()?.parse::<i32>().ok()?;
    let cz = parts.next()?.parse::<i32>().ok()?;
    Some((cx, cz))
}

pub fn world_to_chunk(x: f32, z: f32) -> (i32, i32) {
    (
        (x / CHUNK_SIZE).floor() as i32,
        (z / CHUNK_SIZE).floor() as i32,
    )
}

impl HousingIO {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn chunk_dir(&self, cx: i32, cz: i32) -> PathBuf {
        self.base_dir.join(chunk_prefix(cx, cz))
    }

    /// Build the on-disk path for a house. Rejects ids that could escape the
    /// housing directory, so every fs op funnels its validation through here —
    /// including callers that skip the route-level guard (e.g. `toggle_door`).
    fn house_path(&self, cx: i32, cz: i32, house_id: &str) -> std::io::Result<PathBuf> {
        if !is_valid_house_id(house_id) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid house id",
            ));
        }
        Ok(self.chunk_dir(cx, cz).join(format!("{}.json", house_id)))
    }

    /// Read all houses in a chunk.
    pub async fn read_chunk(&self, cx: i32, cz: i32) -> std::io::Result<Vec<HouseData>> {
        Self::read_house_dir(self.chunk_dir(cx, cz)).await
    }

    /// Every house across all chunk directories (boot-time cache build).
    pub async fn read_all_houses(&self) -> Vec<HouseData> {
        let mut entries = match fs::read_dir(&self.base_dir).await {
            Ok(e) => e,
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    error!("Failed to read housing base dir: {}", e);
                }
                return Vec::new();
            }
        };

        let mut houses = Vec::new();
        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(e) => {
                    error!("Failed to enumerate housing base dir: {}", e);
                    break;
                }
            };
            let is_dir = match entry.file_type().await {
                Ok(file_type) => file_type.is_dir(),
                Err(e) => {
                    error!("Failed to inspect housing entry {:?}: {}", entry.path(), e);
                    continue;
                }
            };
            if !is_dir {
                continue;
            }
            match Self::read_house_dir(entry.path()).await {
                Ok(mut chunk) => houses.append(&mut chunk),
                Err(e) => error!("Failed to read housing dir {:?}: {}", entry.path(), e),
            }
        }
        houses
    }

    async fn read_house_dir(dir: PathBuf) -> std::io::Result<Vec<HouseData>> {
        let mut entries = match fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e),
        };

        let mut houses = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                match fs::read_to_string(&path).await {
                    Ok(content) => match serde_json::from_str::<HouseData>(&content) {
                        Ok(house) => houses.push(house),
                        Err(e) => error!("Failed to parse house file {:?}: {}", path, e),
                    },
                    Err(e) => error!("Failed to read house file {:?}: {}", path, e),
                }
            }
        }

        Ok(houses)
    }

    /// Save a house to disk. Creates chunk directory if needed.
    pub async fn write_house(&self, house: &HouseData) -> std::io::Result<()> {
        let (cx, cz) = world_to_chunk(house.origin.x, house.origin.z);
        let path = self.house_path(cx, cz, &house.id)?;
        let dir = self.chunk_dir(cx, cz);
        fs::create_dir_all(&dir).await?;

        let json = serde_json::to_string_pretty(house)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        atomic_write(&path, json.as_bytes()).await?;
        info!("Saved house {} to {:?}", house.id, path);
        Ok(())
    }

    /// Delete a house from disk. Returns true if the file existed.
    pub async fn delete_house(&self, house_id: &str, cx: i32, cz: i32) -> std::io::Result<bool> {
        let path = self.house_path(cx, cz, house_id)?;
        match fs::remove_file(&path).await {
            Ok(()) => {
                info!("Deleted house {} from {:?}", house_id, path);
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Find and read a house by ID. Parses chunk coords from the ID for O(1) lookup.
    pub async fn find_house(&self, house_id: &str) -> std::io::Result<Option<HouseData>> {
        if let Some((cx, cz)) = parse_chunk_from_id(house_id) {
            let path = self.house_path(cx, cz, house_id)?;
            match fs::read_to_string(&path).await {
                Ok(content) => match serde_json::from_str::<HouseData>(&content) {
                    Ok(house) => return Ok(Some(house)),
                    Err(e) => error!("Failed to parse house {:?}: {}", path, e),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
pub(crate) mod test_fixtures {
    use onlinerpg_shared::housing::{HouseData, RoomData, WallConfig, WallVariant};
    use onlinerpg_shared::Position;

    fn walls(n: usize) -> Vec<WallConfig> {
        vec![
            WallConfig {
                variant: WallVariant::Solid,
                texture: 0,
                is_open: false,
            };
            n
        ]
    }

    pub(crate) fn room_at(local_x: i32, local_z: i32) -> RoomData {
        RoomData {
            room_type: Default::default(),
            roof_type: Default::default(),
            roof_ridge_dir: Default::default(),
            stair_reversed: false,
            local_x,
            local_z,
            size_x: 3,
            size_z: 3,
            floor_level: 0,
            floor_texture: 0,
            roof_texture: 0,
            wall_height: 3.0,
            wall_north: walls(3),
            wall_south: walls(3),
            wall_east: walls(3),
            wall_west: walls(3),
        }
    }

    pub(crate) fn house_at(x: f32, z: f32, rooms: Vec<RoomData>) -> HouseData {
        HouseData {
            id: "r+00_+00_1".into(),
            owner_id: "test".into(),
            origin: Position { x, y: 0.0, z },
            rooms,
            passability: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::{house_at, room_at};
    use super::{is_valid_house_id, validate_house, HousingIO, MAX_ROOMS, MAX_ROOM_OFFSET};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn accepts_valid_house() {
        assert!(validate_house(&house_at(10.0, 10.0, vec![room_at(0, 0)])).is_ok());
    }

    #[test]
    fn rejects_room_offset_out_of_bounds() {
        let too_far = MAX_ROOM_OFFSET + 1;
        assert!(validate_house(&house_at(10.0, 10.0, vec![room_at(too_far, 0)])).is_err());
        assert!(validate_house(&house_at(10.0, 10.0, vec![room_at(0, -too_far)])).is_err());
        assert!(validate_house(&house_at(10.0, 10.0, vec![room_at(i32::MAX, i32::MIN)])).is_err());
    }

    #[test]
    fn rejects_origin_out_of_world() {
        assert!(validate_house(&house_at(1e9, 0.0, vec![room_at(0, 0)])).is_err());
        assert!(validate_house(&house_at(0.0, -1e9, vec![room_at(0, 0)])).is_err());
        assert!(validate_house(&house_at(f32::NAN, 0.0, vec![room_at(0, 0)])).is_err());
        assert!(validate_house(&house_at(f32::INFINITY, 0.0, vec![room_at(0, 0)])).is_err());
    }

    #[test]
    fn rejects_too_many_rooms() {
        let rooms = (0..=MAX_ROOMS as i32)
            .map(|i| room_at(0, i * 4 - 64))
            .collect();
        assert!(validate_house(&house_at(10.0, 10.0, rooms)).is_err());
    }

    #[test]
    fn accepts_real_ids() {
        assert!(is_valid_house_id("r-24_+07_1"));
        assert!(is_valid_house_id("r+00_+00_12"));
    }

    #[test]
    fn rejects_traversal_and_separators() {
        assert!(!is_valid_house_id(""));
        assert!(!is_valid_house_id(".."));
        assert!(!is_valid_house_id("../../etc/passwd"));
        assert!(!is_valid_house_id("foo/bar"));
        assert!(!is_valid_house_id("foo.json"));
        assert!(!is_valid_house_id(&"a".repeat(65)));
    }

    #[test]
    fn house_path_rejects_traversal() {
        let io = HousingIO::new(PathBuf::from("/base"));
        assert!(io.house_path(0, 0, "r+00_+00_1").is_ok());
        assert!(io.house_path(0, 0, "../../etc/passwd").is_err());
    }

    #[tokio::test]
    async fn write_house_persists_readable_json() {
        let dir = unique_temp_dir("housing_write");
        let io = HousingIO::new(dir.clone());
        let house = house_at(10.0, 10.0, vec![room_at(0, 0)]);

        io.write_house(&house).await.unwrap();

        let stored = io.find_house(&house.id).await.unwrap().unwrap();
        assert_eq!(stored.id, house.id);
        assert_eq!(stored.rooms.len(), 1);
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let counter = TEST_TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "_onlinerpg_{name}_{}_{}",
            std::process::id(),
            counter
        ))
    }
}
