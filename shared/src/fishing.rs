//! Fishing protocol types and tuning constants (design: `doc/FISHING.md`).
//! Shared so the server (authority), the web client (UI), and the
//! agent-client (auto-hook reflex) all read the same shapes and windows.
//! The server owns every timer and roll — clients only render and respond.

use serde::{Deserialize, Serialize};

/// A response to the fish, sent via `ClientMessage::FishingRespond`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FishingAction {
    /// Set the hook when the bobber dips (`ServerMessage::FishingBite`).
    Hook,
    /// Pull in line — correct while the fish is `Tiring`.
    Reel,
    /// Yield line — correct while the fish is `Pulling`.
    GiveLine,
}

/// What the hooked fish is doing this struggle round, announced in
/// `ServerMessage::FishingStruggleRound`. The broadcast tells everyone the
/// state — the "skill" is answering correctly in time, not guessing hidden
/// information, which is what keeps humans and agent-clients on equal
/// footing (agents auto-answer; humans read the prompt).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FishState {
    /// The fish runs — give line or the tension spikes.
    Pulling,
    /// The fish tires — reel or lose your progress window.
    Tiring,
}

impl FishState {
    /// The action that answers this state.
    pub fn correct_action(&self) -> FishingAction {
        match self {
            FishState::Pulling => FishingAction::GiveLine,
            FishState::Tiring => FishingAction::Reel,
        }
    }
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

// --- Struggle (the fight after the hook) ------------------------------------

/// Struggle rounds for a rarity tier: commons fight briefly, legends fight
/// long. Rarity 1 → 3 rounds … rarity 5 → 7.
pub const STRUGGLE_BASE_ROUNDS: u32 = 2;

/// Response window per round: `BASE − 150·(rarity−1) + 60·skill`, clamped to
/// `[MIN, BASE]`. Generous on purpose — same agent-parity reasoning as the
/// bite window.
pub const STRUGGLE_BASE_WINDOW_MS: u32 = 3_000;
pub const STRUGGLE_MIN_WINDOW_MS: u32 = 1_800;

/// Tension meter: starts at 0; the fish escapes at or above `TENSION_MAX`.
pub const TENSION_MAX: u32 = 100;
/// A correct response relaxes the line.
pub const TENSION_CORRECT_RELIEF: u32 = 10;
/// A wrong or missed response: `BASE + PER_RARITY · rarity`.
pub const TENSION_MISS_BASE: u32 = 30;
pub const TENSION_MISS_PER_RARITY: u32 = 5;

/// Rounds a fish of this rarity fights for.
pub fn struggle_rounds(rarity: u32) -> u32 {
    STRUGGLE_BASE_ROUNDS + rarity.max(1)
}

/// Response window for one struggle round.
pub fn struggle_window_ms(rarity: u32, skill_level: u32) -> u32 {
    let tightened = STRUGGLE_BASE_WINDOW_MS.saturating_sub(150 * rarity.saturating_sub(1));
    (tightened + 60 * skill_level).clamp(STRUGGLE_MIN_WINDOW_MS, STRUGGLE_BASE_WINDOW_MS)
}

/// Tension added by a wrong or missed response. Rarity 0 (junk flotsam)
/// fights like a common fish — the `max(1)` clamp (same as `struggle_rounds`)
/// keeps the invariant that all-wrong play always tops out the tension meter
/// within the round count (3 × 35 = 105 ≥ 100).
pub fn tension_miss_penalty(rarity: u32) -> u32 {
    TENSION_MISS_BASE + TENSION_MISS_PER_RARITY * rarity.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struggle_scales_with_rarity_and_skill() {
        assert_eq!(struggle_rounds(1), 3);
        assert_eq!(struggle_rounds(5), 7);
        // Common fish, novice: full window. Legend, novice: tightened.
        assert_eq!(struggle_window_ms(1, 0), 3_000);
        assert_eq!(struggle_window_ms(5, 0), 2_400);
        // Skill widens but never past the base, and the floor holds.
        assert_eq!(struggle_window_ms(5, 10), 3_000);
        assert!(struggle_window_ms(5, 0) >= STRUGGLE_MIN_WINDOW_MS);
        // Escape math: a legend punishes misses hardest; junk (rarity 0)
        // clamps up to the common-fish penalty so misses can still snap
        // the line within its 3 rounds.
        assert_eq!(tension_miss_penalty(0), 35);
        assert_eq!(tension_miss_penalty(1), 35);
        assert_eq!(tension_miss_penalty(5), 55);
        assert!(
            struggle_rounds(0) * tension_miss_penalty(0) >= TENSION_MAX,
            "all-wrong play must always escape, even on junk"
        );
    }

    #[test]
    fn correct_actions_answer_states() {
        assert_eq!(FishState::Pulling.correct_action(), FishingAction::GiveLine);
        assert_eq!(FishState::Tiring.correct_action(), FishingAction::Reel);
    }
}
