import animTiming from '../../../../data/player_anim_timing.json'

// Player animation-moment delays are authored in data-src/player_anim_timing.csv;
// this module is the JSON's single import point.
export const PLAYER_ATTACK_IMPACT_DELAY_MS =
  animTiming.player_attack_impact.delayMs
export const PLAYER_ATTACK_DAMAGE_TEXT_DELAY_MS =
  animTiming.player_attack_damage_text.delayMs
// A bow looses much later in its clip than a blade lands in slash1, so a
// ranged attack cannot reuse the melee moments (doc/COMBAT.md 원거리 전투).
export const PLAYER_RANGED_IMPACT_DELAY_MS =
  animTiming.player_ranged_impact.delayMs
/** The gap slash1 leaves between the blade landing and its number, reused by
 *  a shot rather than authored twice — a bow's impact moves with the arrow. */
export const DAMAGE_TEXT_AFTER_IMPACT_MS =
  animTiming.player_attack_damage_text.delayMs -
  animTiming.player_attack_impact.delayMs
// The draw is a fixed-length take, so it is started late enough to finish on
// the loose instead of being stretched to fit.
export const PLAYER_RANGED_DRAW_DELAY_MS = animTiming.player_ranged_draw.delayMs
export const PROP_SWING_IMPACT_MS = animTiming.prop_swing_impact.delayMs
export const SWORD_MISS_DELAY_MS = animTiming.sword_miss.delayMs
// The whoosh (and the splash CAST_MS after it) waits out the cast clip's wind-up.
export const FISHING_CAST_SWING_DELAY_MS = animTiming.fishing_cast_swing.delayMs

// Player melee reach. Shared by the combat controller (chase/attack break-off)
// and the click-to-attack arrival check so the two never drift apart.
export const PLAYER_ATTACK_RANGE_METERS = 2.0

// The server's pickup reach (MAX_PICKUP_DISTANCE, inventory.rs). The click
// walk-up fires a little inside it — see PICKUP_APPROACH in approachRanges.ts.
export const PLAYER_PICKUP_RANGE_METERS = 2.5

export const DEFAULT_MONSTER_ATTACK_IMPACT_DELAY_MS = 0

// Fallback attack cooldown when a monster def is missing the field. Mirrors the
// Rust-side MonsterMovement::default().attack_cooldown_ms.
export const DEFAULT_MONSTER_ATTACK_COOLDOWN_MS = 1500
