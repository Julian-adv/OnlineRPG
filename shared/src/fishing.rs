//! Fishing protocol types and tuning constants (design: `doc/FISHING.md`).
//! Shared so the server (authority), the web client (UI), and the
//! agent-client (auto-hook reflex) all read the same shapes and windows.
//! The server owns every timer and roll — clients only render and respond.

use serde::{Deserialize, Serialize};
use std::ops::RangeInclusive;

/// A response to the fish, sent via `ClientMessage::FishingRespond`.
/// `Hook` answers a bite; the other three set the angler's *stance* during
/// the fight — held until replaced, `Hold` releases reel and slack alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FishingAction {
    /// Set the hook when the bobber dips (`ServerMessage::FishingBite`).
    Hook,
    /// Crank line in: closes distance, loads tension.
    Reel,
    /// Pay line out: sheds tension, lets the fish take distance.
    GiveLine,
    /// Neither — let the rod do the talking.
    Hold,
}

/// What the hooked fish is doing, carried in every `FishingFight` update.
/// Broadcast to everyone near the bobber — the "skill" is managing tension
/// against a visible state, not guessing hidden information, which keeps
/// humans (reading gauge and splash) and agent-clients (running
/// `auto_stance` on a human reaction delay) on equal footing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FishState {
    /// Bursting away: tension climbs, drag burns its stamina.
    Running,
    /// Holding position: tension decays, reeling gains real line.
    Resting,
    /// Stamina spent: it can't fight — reel it in to land it.
    Exhausted,
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
        /// Natural 20 on the quality roll, or at/over the species' trophyCm.
        trophy: bool,
    },
    /// Hooked too early, too late, or not at all.
    Escaped,
    /// The angler moved, fought, disconnected, or reeled in deliberately.
    Aborted,
}

/// How far from the player a cast may land (XZ meters).
pub const MAX_CAST_DISTANCE_METERS: f32 = 8.0;

/// Minimum surface−bed depth (meters) for a cast target — skips the paper-thin
/// shoreline fringe. Shared so the client's cast-vs-walk click test cannot
/// drift from the server's water test.
pub const MIN_FISHABLE_DEPTH_M: f32 = 0.1;

/// Casting animation time before the bobber starts waiting.
pub const CAST_MS: u32 = 1_000;

/// Bite wait is uniform in this range, shortened ~2% per fishing level
/// (floored at `WAIT_MIN_MS / 2`).
pub const WAIT_MIN_MS: u32 = 4_000;
pub const WAIT_MAX_MS: u32 = 12_000;

/// How long the bite window stays open. Generous by design: it must fit both
/// human reflexes and an agent-client's network round trip — and the agent
/// deliberately spends `HOOK_REACTION_MS` of it (agent parity).
pub const BITE_WINDOW_MS: u32 = 2_500;

/// How long a person takes to answer the rod, and so how long the
/// agent-client's reflex waits before responding — no edge over a player at
/// the same rod. Both ceilings are load-bearing: the hook must still land
/// inside `BITE_WINDOW_MS + LATENCY_GRACE_MS`, and the stance inside what the
/// fight absorbs (`the_stance_policy_survives_a_human_reaction_delay`).
pub const HOOK_REACTION_MS: RangeInclusive<u64> = 300..=800;
pub const STANCE_REACTION_MS: RangeInclusive<u64> = 250..=350;

/// Slack added server-side to every response deadline so a laggy but
/// in-time click is never punished. Timers live on the server; this is the
/// server forgiving the wire, not trusting the client.
pub const LATENCY_GRACE_MS: u32 = 500;

/// Flotsam's fixed share of the catch table, percent. Skill makes you a
/// better angler, not a tidier river, so junk never thins out with level.
pub const FLOTSAM_SHARE_PCT: u64 = 20;

/// Percent catch-weight growth per fishing level per rarity tier.
/// Multiplicative, so the table's order can never invert.
pub const RARITY_SKILL_BONUS_PCT: u64 = 3;

/// Skill XP for a catch: `CATCH_XP_PER_RARITY_SQ · rarity²` (rarity 1–5).
pub const CATCH_XP_PER_RARITY_SQ: u64 = 10;

/// Consolation skill XP when a fish escapes — hooked and lost, or a bite
/// left to expire.
pub const ESCAPE_XP: u64 = 2;

// --- The fight (after the hook) ----------------------------------------------
// A continuous tug-of-war on the 250 ms server tick. All rates are per
// second; the server integrates them by elapsed time. The fish alternates
// Running/Resting bursts until its stamina is gone, then goes Exhausted;
// only an Exhausted fish reeled inside `CATCH_DISTANCE_M` is landed.
// The line snaps at `TENSION_MAX`; a fight that outlives `FIGHT_TIMEOUT_MS`
// throws the hook — slack-line stalling is not a strategy.

/// The line snaps at or above this tension.
pub const TENSION_MAX: f32 = 100.0;
/// Tension the hook-set itself puts on the line — the fight opens live.
pub const TENSION_INITIAL: f32 = 30.0;
/// Fish pull while Running: `(BASE + PER_RARITY·rarity) · distance factor`,
/// reduced by skill (`SKILL_PULL_RELIEF_PCT`). Deliberately hot: an
/// unmanaged run leaves the safe range in about a second and snaps the line
/// in a few — the gauge demands answers.
pub const TENSION_PULL_BASE_PS: f32 = 20.0;
pub const TENSION_PULL_PER_RARITY_PS: f32 = 2.0;
/// Distance factor `clamp(0.8 + 0.04·distance_m, MIN, MAX)`: a far fish has
/// more line out and pulls harder, a close one barely loads the rod. The MAX
/// clamp guarantees `TENSION_GIVE_RELIEF_PS` always outmuscles the strongest
/// pull, so giving line is never futile.
pub const TENSION_DISTANCE_FACTOR_PER_M: f32 = 0.04;
pub const TENSION_DISTANCE_FACTOR_MIN: f32 = 0.8;
pub const TENSION_DISTANCE_FACTOR_MAX: f32 = 1.3;
/// Natural decay while the fish is Resting or Exhausted.
pub const TENSION_REST_DECAY_PS: f32 = 8.0;
/// Reeling loads the line (an Exhausted fish no longer resists). Higher
/// than the rest decay on purpose: even reeling a Resting fish creeps the
/// gauge up, so the reel can never simply be held.
pub const TENSION_REEL_PS: f32 = 14.0;
/// Giving line sheds tension.
pub const TENSION_GIVE_RELIEF_PS: f32 = 44.0;

/// Fish swim speed while Running.
pub const RUN_SPEED_BASE_MPS: f32 = 1.0;
pub const RUN_SPEED_PER_RARITY_MPS: f32 = 0.1;
/// Extra line a Running fish takes while you give it.
pub const GIVE_LINE_EXTRA_MPS: f32 = 0.6;
/// Reel speed against each fish state, before the skill bonus.
pub const REEL_RESTING_MPS: f32 = 1.6;
pub const REEL_RUNNING_MPS: f32 = 0.6;
pub const REEL_EXHAUSTED_MPS: f32 = 2.5;
/// The fish can never be reeled closer than its session's *line floor*: the
/// waterline measured along the cast ray at cast time (plus this margin), or
/// the rod's own reach when the water starts at the angler's feet — the
/// landed fish is lifted from the water's edge, the float never walks up
/// onto the shore.
pub const MIN_FISH_DISTANCE_M: f32 = 2.0;
pub const WATERLINE_MARGIN_M: f32 = 0.4;
/// Cast-time waterline scan resolution along the player→cast ray.
pub const SHORE_SAMPLE_STEP_M: f32 = 0.5;
/// An Exhausted fish reeled to within this of the line floor is landed.
pub const CATCH_SLACK_M: f32 = 0.3;
/// A fish with stamina left never lets itself be landed: inside this band
/// above the line floor it panics into a fresh Running burst.
pub const PANIC_BAND_M: f32 = 1.0;
/// The fish stays within this radius of the cast point.
pub const FISH_WANDER_RADIUS_M: f32 = 6.0;
/// Per-tick heading blend toward the cast ray on the exhausted reel-in (a
/// per-call fraction, not a per-second rate), so the fish comes home along
/// the line whose waterline the session measured.
pub const EXHAUSTED_STEER_PER_TICK: f32 = 0.15;

/// Stamina pool: `BASE + PER_RARITY·rarity` (junk clamps to rarity 1, as the
/// old round count did).
pub const STAMINA_BASE: f32 = 38.0;
pub const STAMINA_PER_RARITY: f32 = 12.0;
/// Drag burns stamina only while Running and only under real tension —
/// a slack line never tires the fish.
pub const STAMINA_DRAIN_MIN_TENSION: f32 = 20.0;
pub const STAMINA_DRAIN_BASE_PS: f32 = 2.0;
/// Scaled by `(tension / 100)²`: the square keeps timid mid-band play slow
/// and pays real progress only for riding the gauge near the top.
pub const STAMINA_DRAIN_TENSION_PS: f32 = 12.0;
/// Resting on a slack line (below the drain threshold) recovers stamina.
pub const STAMINA_RECOVER_PS: f32 = 4.0;

/// Behavior burst durations (uniform rolls); rarer fish run a little
/// longer. Runs dominate the fight — the rests are breathers, not the norm.
pub const RUN_MIN_MS: u32 = 2_000;
pub const RUN_MAX_MS: u32 = 3_500;
pub const RUN_MAX_PER_RARITY_MS: u32 = 150;
pub const REST_MIN_MS: u32 = 800;
pub const REST_MAX_MS: u32 = 2_000;

/// Skill: drag control. Tension gained from the fish's pull drops this
/// percent per fishing level (capped), and reel speed gains the same.
pub const SKILL_PULL_RELIEF_PCT: f32 = 1.0;
pub const SKILL_PULL_RELIEF_CAP_PCT: f32 = 30.0;
pub const SKILL_REEL_BONUS_PCT: f32 = 1.0;

/// A fight that outlives this throws the hook (`Escaped`).
pub const FIGHT_TIMEOUT_MS: u32 = 60_000;

/// Stamina pool for a rarity tier.
pub fn stamina_max(rarity: u32) -> f32 {
    STAMINA_BASE + STAMINA_PER_RARITY * rarity.max(1) as f32
}

/// Tension per second the fish adds while Running.
pub fn fish_pull_ps(rarity: u32, distance_m: f32, skill_level: u32) -> f32 {
    let factor = (TENSION_DISTANCE_FACTOR_MIN + TENSION_DISTANCE_FACTOR_PER_M * distance_m)
        .clamp(TENSION_DISTANCE_FACTOR_MIN, TENSION_DISTANCE_FACTOR_MAX);
    let relief =
        1.0 - (SKILL_PULL_RELIEF_PCT * skill_level as f32).min(SKILL_PULL_RELIEF_CAP_PCT) / 100.0;
    (TENSION_PULL_BASE_PS + TENSION_PULL_PER_RARITY_PS * rarity.max(1) as f32) * factor * relief
}

/// Stamina per second the drag burns while the fish runs under tension.
pub fn stamina_drain_ps(tension: f32) -> f32 {
    let t = (tension / 100.0).clamp(0.0, 1.0);
    STAMINA_DRAIN_BASE_PS + STAMINA_DRAIN_TENSION_PS * t * t
}

/// Meters per second a reeling angler takes back against this fish state.
pub fn reel_speed_mps(state: FishState, skill_level: u32) -> f32 {
    let base = match state {
        FishState::Running => REEL_RUNNING_MPS,
        FishState::Resting => REEL_RESTING_MPS,
        FishState::Exhausted => REEL_EXHAUSTED_MPS,
    };
    base * (1.0 + SKILL_REEL_BONUS_PCT * skill_level as f32 / 100.0)
}

/// The sound tension-management policy, shared so the agent-client's reflex
/// and the server tests play the same game a practiced human does: reel an
/// exhausted fish, shed tension when it's high, take line when it's safe.
/// A run is never cranked against: `TENSION_REEL_PS` stacks on the pull, and
/// a human-speed hand cannot back off in time.
pub fn auto_stance(state: FishState, tension_pct: u32) -> FishingAction {
    match state {
        FishState::Exhausted => FishingAction::Reel,
        FishState::Running if tension_pct >= 45 => FishingAction::GiveLine,
        FishState::Running => FishingAction::Hold,
        _ if tension_pct >= 75 => FishingAction::GiveLine,
        _ if tension_pct <= 45 => FishingAction::Reel,
        _ => FishingAction::Hold,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn giving_line_always_beats_the_strongest_pull() {
        // Rarity 5, max distance factor, no skill relief: the worst case.
        let worst = fish_pull_ps(5, 1_000.0, 0);
        assert!(
            worst < TENSION_GIVE_RELIEF_PS,
            "give-line must always shed tension, worst pull {worst}"
        );
    }

    #[test]
    fn pull_scales_with_rarity_distance_and_skill() {
        assert!(fish_pull_ps(5, 5.0, 0) > fish_pull_ps(1, 5.0, 0));
        assert!(fish_pull_ps(1, 12.0, 0) > fish_pull_ps(1, 1.0, 0));
        assert!(fish_pull_ps(1, 5.0, 30) < fish_pull_ps(1, 5.0, 0));
        // Junk fights like a common fish.
        assert_eq!(fish_pull_ps(0, 5.0, 0), fish_pull_ps(1, 5.0, 0));
        assert_eq!(stamina_max(0), stamina_max(1));
    }

    #[test]
    fn auto_stance_manages_the_band() {
        assert_eq!(
            auto_stance(FishState::Exhausted, 90),
            FishingAction::Reel,
            "an exhausted fish is reeled no matter the tension"
        );
        assert_eq!(auto_stance(FishState::Running, 80), FishingAction::GiveLine);
        assert_eq!(auto_stance(FishState::Resting, 10), FishingAction::Reel);
        assert_eq!(
            auto_stance(FishState::Running, 50),
            FishingAction::GiveLine,
            "a run is answered with line, not held to the top of the band"
        );
        assert_eq!(
            auto_stance(FishState::Running, 20),
            FishingAction::Hold,
            "a slack line during a run needs nothing"
        );
        assert_eq!(
            auto_stance(FishState::Resting, 60),
            FishingAction::Hold,
            "a resting fish sheds tension on its own"
        );
    }

    // Constant on purpose: this test locks the tuning invariant.
    #[allow(clippy::assertions_on_constants)]
    #[test]
    fn the_distance_bands_nest_correctly() {
        // Catch slack < panic band: reeling to the line floor lands an
        // exhausted fish, while a lively one panics before it gets there.
        assert!(CATCH_SLACK_M < PANIC_BAND_M);
        assert!(WATERLINE_MARGIN_M < MIN_FISH_DISTANCE_M);
    }

    #[test]
    fn drag_pays_off_quadratically() {
        // Riding the top of the gauge must clearly out-tire timid play.
        assert!(stamina_drain_ps(90.0) > stamina_drain_ps(50.0) * 1.8);
        assert_eq!(stamina_drain_ps(0.0), STAMINA_DRAIN_BASE_PS);
    }

    #[test]
    fn reeling_is_fastest_against_an_exhausted_fish() {
        assert!(reel_speed_mps(FishState::Exhausted, 0) > reel_speed_mps(FishState::Resting, 0));
        assert!(reel_speed_mps(FishState::Resting, 0) > reel_speed_mps(FishState::Running, 0));
        assert!(reel_speed_mps(FishState::Resting, 10) > reel_speed_mps(FishState::Resting, 0));
    }
}
