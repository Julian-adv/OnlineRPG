import { describe, expect, it, vi } from 'vitest'
import { AnimationIndex } from '../types/animations'
import { getPlayerWeaponCombatProfile } from './combatTiming'

vi.mock('../wasm/onlinerpg_shared', () => ({
  weapon_skill_melee_range: (skill: string) => (skill === 'spear' ? 3 : 2),
  weapon_skill_attack_cooldown_ms: (skill: string) =>
    skill === 'spear' ? 2467 : 1533,
}))

describe('player weapon combat profiles', () => {
  it('maps Spear to its dedicated clip, reach, cadence, and impact timing', () => {
    expect(getPlayerWeaponCombatProfile('spear')).toEqual({
      weaponSkill: 'spear',
      animationIndex: AnimationIndex.SLASH3,
      rangeMeters: 3,
      cooldownMs: 2467,
      impactDelayMs: 1060,
      damageTextDelayMs: 1250,
      missDelayMs: 1060,
    })
  })

  it('keeps swords and daggers on the existing slash1 profile', () => {
    for (const itemDefId of ['iron_sword', 'dagger']) {
      const profile = getPlayerWeaponCombatProfile(itemDefId)
      expect(profile.animationIndex).toBe(AnimationIndex.SLASH1)
      expect(profile.rangeMeters).toBe(2)
      expect(profile.cooldownMs).toBe(1533)
      expect(profile.impactDelayMs).toBe(540)
    }
  })
})
