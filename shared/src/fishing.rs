//! Fishing protocol types and tuning constants (design: `doc/FISHING.md`).
//! Shared so the server (authority), the web client (UI), and the
//! agent-client (auto-hook reflex) all read the same shapes and windows.
//! The server owns every timer and roll — clients only render and respond.

use serde::{Deserialize, Serialize};

/// A response to the fish, sent via `ClientMessage::FishingRespond`.
/// One action today; the struggle minigame adds `Reel` / `GiveLine`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FishingAction {
    /// Set the hook when the bobber dips (`ServerMessage::FishingBite`).
    Hook,
}

/// How a fishing session ended, carried by `ServerMessage::FishingEnded`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FishingOutcome {
    /// The fish is in the bag (or on the ground, if the bag was full).
    Caught {
        item_def_id: String,
        /// Rolled length in centimeters — announced, not stored on the item,
        /// so fish stay stackable commodities.
        size_cm: u16,
        /// Exceptional roll (natural 20 on quality, or over the species
        /// threshold). Celebration lands in a later PR; the flag ships now
        /// so the outcome shape doesn't change.
        trophy: bool,
    },
    /// Hooked too early, too late, or not at all.
    Escaped,
    /// The angler moved, fought, disconnected, or reeled in deliberately.
    Aborted,
}

/// How far from the player a cast may land (XZ meters).
pub const MAX_CAST_DISTANCE_METERS: f32 = 8.0;

/// Casting animation time before the bobber starts waiting.
pub const CAST_MS: u32 = 1_000;

/// Bite wait is uniform in this range, shortened ~2% per fishing level
/// (floored at `WAIT_MIN_MS`).
pub const WAIT_MIN_MS: u32 = 4_000;
pub const WAIT_MAX_MS: u32 = 12_000;

/// How long the bite window stays open. Deliberately generous: the same
/// window must be comfortable for a human's reflexes and an agent-client's
/// network round trip (agent parity — no mechanic may need reactions only
/// software can deliver, and none may be too fast for software either).
pub const BITE_WINDOW_MS: u32 = 2_500;

/// Slack added server-side to every response deadline so a laggy but
/// in-time click is never punished. Timers live on the server; this is the
/// server forgiving the wire, not trusting the client.
pub const LATENCY_GRACE_MS: u32 = 500;

/// Skill XP for a catch: `CATCH_XP_PER_RARITY_SQ · rarity²` (rarity 1–5).
pub const CATCH_XP_PER_RARITY_SQ: u64 = 10;

/// Consolation skill XP when a hooked fish escapes.
pub const ESCAPE_XP: u64 = 2;
