use serde::{Deserialize, Serialize};

use crate::Position;

/// Highest housing floor level (0 = ground floor). Housing thus occupies
/// passability floor indices `0..=MAX_FLOOR_LEVEL`; dungeon depths start
/// just above this range (see `dungeon::DUNGEON_FLOOR_INDEX_BASE`), so the
/// two systems can never collide in floor-keyed collision queries. Raising
/// this is the single knob that grows housing — the dungeon base follows
/// automatically.
pub const MAX_FLOOR_LEVEL: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum RoomType {
    #[serde(rename = "normal")]
    #[default]
    Normal,
    #[serde(rename = "stairwell")]
    Stairwell,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum RoofType {
    #[serde(rename = "flat")]
    #[default]
    Flat,
    #[serde(rename = "gabled")]
    Gabled,
    #[serde(rename = "steep")]
    Steep,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum RoofRidgeDir {
    #[serde(rename = "auto")]
    #[default]
    Auto,
    #[serde(rename = "x")]
    X,
    #[serde(rename = "z")]
    Z,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WallDirection {
    #[serde(rename = "north")]
    North,
    #[serde(rename = "south")]
    South,
    #[serde(rename = "east")]
    East,
    #[serde(rename = "west")]
    West,
}

impl WallDirection {
    pub const ALL: [WallDirection; 4] = [
        WallDirection::North,
        WallDirection::South,
        WallDirection::East,
        WallDirection::West,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum WallVariant {
    #[serde(rename = "solid")]
    Solid,
    #[serde(rename = "door")]
    WithDoor,
    #[serde(rename = "window")]
    WithWindow,
    #[serde(rename = "open")]
    Open,
    #[serde(rename = "double-door")]
    WithDoubleDoor,
}

impl WallVariant {
    /// Segments players can toggle open/closed (doors and window shutters).
    pub fn is_openable(self) -> bool {
        matches!(
            self,
            WallVariant::WithDoor | WallVariant::WithDoubleDoor | WallVariant::WithWindow
        )
    }

    pub fn is_door(self) -> bool {
        matches!(self, WallVariant::WithDoor | WallVariant::WithDoubleDoor)
    }
}

/// Partner of a double-door half: consecutive halves pair from the run start;
/// a trailing odd one has none.
pub fn double_door_partner(segs: &[WallConfig], i: usize) -> Option<usize> {
    if segs.get(i)?.variant != WallVariant::WithDoubleDoor {
        return None;
    }
    let mut r = i;
    while r > 0 && segs[r - 1].variant == WallVariant::WithDoubleDoor {
        r -= 1;
    }
    let partner = if (i - r).is_multiple_of(2) {
        i + 1
    } else {
        i - 1
    };
    (segs.get(partner)?.variant == WallVariant::WithDoubleDoor).then_some(partner)
}

/// The other half of a double door as `(room_index, segment_index)`: same wall, else adjacent room.
pub fn door_partner(
    rooms: &[RoomData],
    room_index: usize,
    dir: WallDirection,
    i: usize,
) -> Option<(usize, usize)> {
    double_door_partner(rooms.get(room_index)?.wall(dir), i)
        .map(|p| (room_index, p))
        .or_else(|| cross_room_door_partner(rooms, room_index, dir, i))
}

fn wall_line_coord(room: &RoomData, dir: WallDirection) -> i32 {
    match dir {
        WallDirection::North => room.local_z,
        WallDirection::South => room.local_z + room.size_z as i32,
        WallDirection::East => room.local_x + room.size_x as i32,
        WallDirection::West => room.local_x,
    }
}

/// Cross-room partner `(room_index, segment_index)` of a lone double-door
/// half at a wall end: the adjacent same-floor room's collinear wall segment
/// touching it, when that one is a lone double-door half too.
pub fn cross_room_door_partner(
    rooms: &[RoomData],
    room_index: usize,
    dir: WallDirection,
    i: usize,
) -> Option<(usize, usize)> {
    let room = rooms.get(room_index)?;
    let segs = room.wall(dir);
    let is_lone = |segs: &[WallConfig], i: usize| {
        segs.get(i).map(|s| s.variant) == Some(WallVariant::WithDoubleDoor)
            && double_door_partner(segs, i).is_none()
    };
    if segs.is_empty() || !is_lone(segs, i) || (i != 0 && i != segs.len() - 1) {
        return None;
    }
    let is_ns = matches!(dir, WallDirection::North | WallDirection::South);
    let (a0, a_len) = if is_ns {
        (room.local_x, room.size_x as i32)
    } else {
        (room.local_z, room.size_z as i32)
    };
    let edge = if i == 0 { a0 } else { a0 + a_len };
    let line = wall_line_coord(room, dir);
    for (ri, o) in rooms.iter().enumerate() {
        if ri == room_index
            || o.floor_level != room.floor_level
            || o.room_type == RoomType::Stairwell
        {
            continue;
        }
        if wall_line_coord(o, dir) != line {
            continue;
        }
        let (oa0, oa_len) = if is_ns {
            (o.local_x, o.size_x as i32)
        } else {
            (o.local_z, o.size_z as i32)
        };
        if (if i == 0 { oa0 + oa_len } else { oa0 }) != edge {
            continue;
        }
        let o_segs = o.wall(dir);
        if o_segs.is_empty() {
            continue;
        }
        let oi = if i == 0 { o_segs.len() - 1 } else { 0 };
        if is_lone(o_segs, oi) {
            return Some((ri, oi));
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WallConfig {
    pub variant: WallVariant,
    pub texture: u8,
    #[serde(default)]
    pub is_open: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomData {
    #[serde(default)]
    pub room_type: RoomType,
    #[serde(default)]
    pub roof_type: RoofType,
    #[serde(default)]
    pub roof_ridge_dir: RoofRidgeDir,
    /// Stairwell ascends in reverse direction (180°/270° rotation)
    #[serde(default)]
    pub stair_reversed: bool,
    pub local_x: i32,
    pub local_z: i32,
    pub size_x: u8,
    pub size_z: u8,
    pub floor_level: u8,
    pub floor_texture: u8,
    pub roof_texture: u8,
    pub wall_height: f32,
    /// 1m segments: north wall (length = size_x)
    pub wall_north: Vec<WallConfig>,
    /// 1m segments: south wall (length = size_x)
    pub wall_south: Vec<WallConfig>,
    /// 1m segments: east wall (length = size_z)
    pub wall_east: Vec<WallConfig>,
    /// 1m segments: west wall (length = size_z)
    pub wall_west: Vec<WallConfig>,
}

impl RoomData {
    pub fn wall(&self, dir: WallDirection) -> &[WallConfig] {
        match dir {
            WallDirection::North => &self.wall_north,
            WallDirection::South => &self.wall_south,
            WallDirection::East => &self.wall_east,
            WallDirection::West => &self.wall_west,
        }
    }

    pub fn wall_mut(&mut self, dir: WallDirection) -> &mut [WallConfig] {
        match dir {
            WallDirection::North => &mut self.wall_north,
            WallDirection::South => &mut self.wall_south,
            WallDirection::East => &mut self.wall_east,
            WallDirection::West => &mut self.wall_west,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PassabilityGrid {
    pub floor_level: u8,
    pub origin_x: i32,
    pub origin_z: i32,
    pub width: u8,
    pub depth: u8,
    /// Packed edge bits per cell (N=1, E=2, S=4, W=8). Length = width * depth.
    pub cells: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HouseData {
    pub id: String,
    pub owner_id: String,
    pub origin: Position,
    pub rooms: Vec<RoomData>,
    #[serde(default)]
    pub passability: Vec<PassabilityGrid>,
}
