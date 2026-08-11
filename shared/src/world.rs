//! World primitives: 3D position, axis-aligned no-spawn rectangles, and
//! the in-game calendar/clock value the server broadcasts. Tiny but
//! shared by virtually every other type, so they live in one place that
//! has no dependencies on the rest of the crate.

use serde::{Deserialize, Serialize};

/// East-west circumference of the baked world, in meters.
pub const WORLD_WIDTH_X: f32 = 32_768.0;
/// West edge of the first baked terrain tile. Tile -256 is centered at
/// -16,384 and extends another half tile west.
pub const WORLD_MIN_X: f32 = -16_416.0;
/// East edge of the last baked terrain tile. This edge is the same periodic
/// location as `WORLD_MIN_X` and therefore belongs to the wrapped interval's
/// exclusive end.
pub const WORLD_MAX_X: f32 = WORLD_MIN_X + WORLD_WIDTH_X;

/// Normalize a world X coordinate into the terrain's canonical baked range.
#[inline]
pub fn wrap_world_x(x: f32) -> f32 {
    (x - WORLD_MIN_X).rem_euclid(WORLD_WIDTH_X) + WORLD_MIN_X
}

/// Shortest signed X offset from `from_x` to `to_x` on the cylindrical world.
#[inline]
pub fn shortest_world_delta_x(from_x: f32, to_x: f32) -> f32 {
    let raw_delta = to_x - from_x;
    let half_width = WORLD_WIDTH_X * 0.5;
    if raw_delta >= -half_width && raw_delta < half_width {
        raw_delta
    } else {
        (raw_delta + half_width).rem_euclid(WORLD_WIDTH_X) - half_width
    }
}

/// Yaw facing the ground-plane offset `(dx, dz)` — the game's rotation
/// convention (0 faces +Z). `None` for a zero offset, so callers keep
/// their current facing instead of snapping to +Z.
#[inline]
pub fn bearing_xz(dx: f32, dz: f32) -> Option<f32> {
    if dx == 0.0 && dz == 0.0 {
        None
    } else {
        Some(dx.atan2(dz))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Position {
    /// Return this position with X normalized across the cylindrical world
    /// seam. Y and Z are unchanged.
    pub fn wrapped_x(mut self) -> Self {
        self.x = wrap_world_x(self.x);
        self
    }

    /// True when every component is a finite number (no NaN/±∞).
    pub fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// Squared shortest-periodic distance in the X-Z ground plane, ignoring
    /// height. X wraps around the world; Z remains bounded.
    pub fn dist_xz_sq(&self, other: &Position) -> f32 {
        let dx = shortest_world_delta_x(self.x, other.x);
        let dz = self.z - other.z;
        dx * dx + dz * dz
    }

    /// Yaw from `self` toward `other`, wrap-aware like `dist_xz_sq`.
    /// `None` when the two share a ground position.
    pub fn bearing_xz_to(&self, other: &Position) -> Option<f32> {
        bearing_xz(shortest_world_delta_x(self.x, other.x), other.z - self.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_x_wraps_at_baked_terrain_edges() {
        assert_eq!(wrap_world_x(WORLD_MIN_X), WORLD_MIN_X);
        assert_eq!(wrap_world_x(WORLD_MAX_X), WORLD_MIN_X);
        assert_eq!(wrap_world_x(WORLD_MAX_X + 0.25), WORLD_MIN_X + 0.25);
        assert_eq!(wrap_world_x(WORLD_MIN_X - 0.25), WORLD_MAX_X - 0.25);
    }

    #[test]
    fn world_x_distance_uses_short_path_across_seam() {
        assert_eq!(
            shortest_world_delta_x(WORLD_MAX_X - 1.0, WORLD_MIN_X + 1.0),
            2.0
        );
        assert_eq!(
            shortest_world_delta_x(WORLD_MIN_X + 1.0, WORLD_MAX_X - 1.0),
            -2.0
        );

        let east = Position {
            x: WORLD_MAX_X - 1.0,
            y: 0.0,
            z: 4.0,
        };
        let west = Position {
            x: WORLD_MIN_X + 1.0,
            y: 99.0,
            z: 7.0,
        };
        assert_eq!(east.dist_xz_sq(&west), 13.0);

        // Bearing takes the short way across the seam too: 2 m east, 3 m north.
        assert_eq!(east.bearing_xz_to(&west), Some(2.0f32.atan2(3.0)));
        assert_eq!(east.bearing_xz_to(&east), None);
    }
}

/// Distance (game units) within which agent (NPC) clients perceive nearby
/// humans and monsters: the agent-client surfaces only entities within it to
/// the LLM, and the server applies it to NPC gameplay checks (e.g. deal
/// offers). Event *delivery* uses the wider EVENT_DELIVERY_RADIUS.
pub const NPC_SIGHT_RADIUS: f32 = 27.0;

/// Server AOI for gameplay event delivery, and the client's dungeon
/// registration / door-resync boundary (exposed to TS via
/// dungeon_constants()): the farthest world point visible in a fullscreen
/// browser spanning dual 4K monitors.
pub const EVENT_DELIVERY_RADIUS: f32 = 43.0;

/// Agent connections must receive everything they perceive.
const _: () = assert!(EVENT_DELIVERY_RADIUS >= NPC_SIGHT_RADIUS);

/// Player walk speed in units/sec. Client prediction, agent-client walks and
/// the server's authoritative movement simulation must all agree on this.
pub const PLAYER_MOVE_SPEED: f32 = 3.0;

/// Longest move target (or appended leg) the server accepts; the farthest
/// in-view click is ~42m. Farther targets are refused and snapped back, so
/// clients sending long forced moves must split them into shorter legs.
pub const MAX_MOVE_TARGET_DISTANCE: f32 = 60.0;

/// Axis-aligned rectangular zone where monsters must not spawn (e.g. towns).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoSpawnZone {
    pub min_x: f32,
    pub min_z: f32,
    pub max_x: f32,
    pub max_z: f32,
}

impl NoSpawnZone {
    pub fn contains(&self, x: f32, z: f32) -> bool {
        x >= self.min_x && x <= self.max_x && z >= self.min_z && z <= self.max_z
    }

    /// Like `contains`, but with the rectangle expanded by `margin` on all
    /// sides — used to keep spawns clear of the area *around* a town too.
    pub fn contains_with_margin(&self, x: f32, z: f32, margin: f32) -> bool {
        x >= self.min_x - margin
            && x <= self.max_x + margin
            && z >= self.min_z - margin
            && z <= self.max_z + margin
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameDateTime {
    pub year: u32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

impl std::fmt::Display for GameDateTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute
        )
    }
}
