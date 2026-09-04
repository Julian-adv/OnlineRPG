import { describe, expect, it } from 'vitest'
import { isRangedWeapon, isTwoHanded, weaponRangeMeters } from './itemDefs'
import { PLAYER_ATTACK_RANGE_METERS } from './combatTiming'

describe('weapon reach from items.json', () => {
  it('reads a ranged weapon its declared range', () => {
    expect(weaponRangeMeters('bow')).toBe(10)
    expect(isRangedWeapon('bow')).toBe(true)
    expect(isTwoHanded('bow')).toBe(true)
  })

  it('leaves a weapon with no range at the melee reach', () => {
    expect(weaponRangeMeters('iron_sword')).toBe(PLAYER_ATTACK_RANGE_METERS)
    expect(isRangedWeapon('iron_sword')).toBe(false)
    expect(isTwoHanded('iron_sword')).toBe(false)
  })

  it('falls back to melee for an empty hand or an unknown item', () => {
    expect(weaponRangeMeters(null)).toBe(PLAYER_ATTACK_RANGE_METERS)
    expect(weaponRangeMeters('no_such_item')).toBe(PLAYER_ATTACK_RANGE_METERS)
  })
})
