import animTiming from '../../../../data/player_anim_timing.json'
import { getItemDef } from './itemDefs'
import { AnimationIndex } from '../types/animations'
import type { SkillId } from '../network/networkTypes'
import {
  weapon_skill_attack_cooldown_ms,
  weapon_skill_melee_range,
} from '../wasm/onlinerpg_shared'

// Player animation-moment delays are authored in data-src/player_anim_timing.csv;
// this module is the JSON's single import point.
export const PLAYER_ATTACK_IMPACT_DELAY_MS =
  animTiming.player_attack_impact.delayMs
export const PLAYER_ATTACK_DAMAGE_TEXT_DELAY_MS =
  animTiming.player_attack_damage_text.delayMs
export const PROP_SWING_IMPACT_MS = animTiming.prop_swing_impact.delayMs
export const SWORD_MISS_DELAY_MS = animTiming.sword_miss.delayMs
export const SPEAR_ATTACK_IMPACT_DELAY_MS =
  animTiming.spear_attack_impact.delayMs
export const SPEAR_ATTACK_DAMAGE_TEXT_DELAY_MS =
  animTiming.spear_attack_damage_text.delayMs
export const SPEAR_MISS_DELAY_MS = animTiming.spear_miss.delayMs
// The whoosh (and the splash CAST_MS after it) waits out the cast clip's wind-up.
export const FISHING_CAST_SWING_DELAY_MS = animTiming.fishing_cast_swing.delayMs

// Player melee reach. Shared by the combat controller (chase/attack break-off)
// and the click-to-attack arrival check so the two never drift apart.
export const PLAYER_ATTACK_RANGE_METERS = 2.0
export const PLAYER_ATTACK_COOLDOWN_MS = 1533

export interface PlayerWeaponCombatProfile {
  weaponSkill?: SkillId
  animationIndex: AnimationIndex
  rangeMeters: number
  cooldownMs: number
  impactDelayMs: number
  damageTextDelayMs: number
  missDelayMs: number
}

export function getPlayerWeaponCombatProfile(
  itemDefId?: string | null
): PlayerWeaponCombatProfile {
  const weaponSkill = itemDefId
    ? getItemDef(itemDefId)?.weaponSkill
    : undefined
  const rangeMeters = weaponSkill
    ? weapon_skill_melee_range(weaponSkill)
    : PLAYER_ATTACK_RANGE_METERS
  const cooldownMs = weaponSkill
    ? weapon_skill_attack_cooldown_ms(weaponSkill)
    : PLAYER_ATTACK_COOLDOWN_MS

  if (weaponSkill === 'spear') {
    return {
      weaponSkill,
      animationIndex: AnimationIndex.SLASH3,
      rangeMeters,
      cooldownMs,
      impactDelayMs: SPEAR_ATTACK_IMPACT_DELAY_MS,
      damageTextDelayMs: SPEAR_ATTACK_DAMAGE_TEXT_DELAY_MS,
      missDelayMs: SPEAR_MISS_DELAY_MS,
    }
  }

  return {
    weaponSkill,
    animationIndex: AnimationIndex.SLASH1,
    rangeMeters,
    cooldownMs,
    impactDelayMs: PLAYER_ATTACK_IMPACT_DELAY_MS,
    damageTextDelayMs: PLAYER_ATTACK_DAMAGE_TEXT_DELAY_MS,
    missDelayMs: SWORD_MISS_DELAY_MS,
  }
}

// Range within which a click picks an item up directly; beyond it the player
// walks over first. Mirrors the server's MAX_PICKUP_DISTANCE (inventory.rs).
export const PLAYER_PICKUP_RANGE_METERS = 2.5

export const DEFAULT_MONSTER_ATTACK_IMPACT_DELAY_MS = 0

// Fallback attack cooldown when a monster def is missing the field. Mirrors the
// Rust-side MonsterMovement::default().attack_cooldown_ms.
export const DEFAULT_MONSTER_ATTACK_COOLDOWN_MS = 1500
