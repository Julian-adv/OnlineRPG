//! Tiny XZ-plane vector helper used wherever the agent reads `Position`.
//! Y is the height axis and is fixed by the navmesh, so distance and
//! heading are always computed from `(dx, dz)`. Centralising the math
//! keeps callers focused on WHY they care about distance, not HOW it's
//! derived.

use onlinerpg_shared::Position;

/// Planar (X-Z) displacement and Euclidean distance from `from` to `to`.
/// Y is ignored.
#[derive(Clone, Copy)]
pub struct PlanarDelta {
    pub dx: f32,
    pub dz: f32,
    pub dist: f32,
}

impl PlanarDelta {
    pub fn between(from: &Position, to: &Position) -> Self {
        Self::xz(from.x, from.z, to.x, to.z)
    }

    pub fn to_xz(from: &Position, to_x: f32, to_z: f32) -> Self {
        Self::xz(from.x, from.z, to_x, to_z)
    }

    pub fn xz(from_x: f32, from_z: f32, to_x: f32, to_z: f32) -> Self {
        let dx = to_x - from_x;
        let dz = to_z - from_z;
        Self {
            dx,
            dz,
            dist: (dx * dx + dz * dz).sqrt(),
        }
    }

    /// Heading angle (radians) from `from` toward `to`. Matches the
    /// client's `dx.atan2(dz)` convention used to set `PlayerMove.rotation`.
    pub fn rotation(&self) -> f32 {
        self.dx.atan2(self.dz)
    }
}
