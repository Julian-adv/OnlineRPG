//! Tip hat: the hat a performer sets down with the `tip_hat` item so
//! listeners can drop coins in it.

use serde::{Deserialize, Serialize};

/// A tip hat standing near its owner. Wire type (positional array — never
/// reorder fields, append only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TipHat {
    pub id: u64,
    pub owner: crate::entity::PlayerId,
    /// Shown in the tipping dialog, so the tipper needs no roster lookup.
    pub owner_name: String,
    pub position: crate::world::Position,
    /// Owner's yaw when it was set down.
    pub rotation: f32,
    pub floor_level: i8,
}

/// How far the owner may wander before the hat is packed up automatically.
pub const TIP_HAT_LEASH_M: f32 = 5.0;

impl TipHat {
    /// Owner out of leash range, or off this floor entirely.
    pub fn strayed_from(&self, position: &crate::world::Position, floor_level: i8) -> bool {
        self.floor_level != floor_level
            || self.position.dist_xz_sq(position) > TIP_HAT_LEASH_M * TIP_HAT_LEASH_M
    }
}
