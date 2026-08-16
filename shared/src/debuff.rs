//! Timed debuffs (doc/DEBUFF.md). Definitions live in data-src/debuffs.csv;
//! this is only the wire shape.

use serde::{Deserialize, Serialize};

/// One active debuff on the receiver. Wire type (positional array — never
/// reorder fields, append only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveDebuffState {
    pub id: String,
    pub remaining_ms: u64,
}
