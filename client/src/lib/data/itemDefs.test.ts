import { describe, expect, it } from 'vitest'
import itemDefs, {
  WEAPON_TYPE_LABELS,
  weaponTypeLabel,
  type WeaponType,
} from './itemDefs'

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
