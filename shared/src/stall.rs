//! Merchant stall: the table of wares a merchant lays out with `/lay_stall`
//! and packs up with `/pack_stall` (or by logging out).

use serde::{Deserialize, Serialize};

/// A laid-out stall visible to nearby players. Wire type (positional array —
/// never reorder fields, append only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stall {
    pub id: u64,
    pub owner: crate::entity::PlayerId,
    pub position: crate::world::Position,
    /// Owner's yaw when laid out, so the long side faces them.
    pub rotation: f32,
    pub floor_level: i8,
}
