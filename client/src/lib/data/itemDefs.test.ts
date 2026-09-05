import { describe, expect, it } from 'vitest'
import itemDefs, {
  isRangedWeapon,
  isTwoHanded,
  weaponRangeMeters,
  WEAPON_TYPE_LABELS,
  weaponTypeLabel,
  type WeaponType,
} from './itemDefs'
import { PLAYER_ATTACK_RANGE_METERS } from './combatTiming'

describe('weapon types', () => {
  it.each(Object.entries(WEAPON_TYPE_LABELS))(
    'labels %s as %s',
    (weaponType, label) => {
      expect(weaponTypeLabel(weaponType as WeaponType)).toBe(label)
    }
  )

  it('classifies every weapon and only weapons', () => {
    for (const def of Object.values(itemDefs)) {
      expect(def.weaponType !== undefined, def.id).toBe(
        def.category === 'weapon'
      )
      if (def.weaponType) {
        expect(WEAPON_TYPE_LABELS[def.weaponType], def.id).toBeDefined()
      }
    }
  })

  it('distinguishes swords from short swords', () => {
    expect(itemDefs.iron_sword.weaponType).toBe('sword')
    expect(itemDefs.steel_longsword.weaponType).toBe('sword')
    expect(itemDefs.goblin_sword.weaponType).toBe('short_sword')
    expect(itemDefs.small_sword.weaponType).toBe('short_sword')
  })
})

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
