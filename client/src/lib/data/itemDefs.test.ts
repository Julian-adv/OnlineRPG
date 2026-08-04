import { describe, expect, it } from 'vitest'
import itemDefs, { getItemDef, itemBodyCoverage } from './itemDefs'

describe('garment body coverage metadata', () => {
  it('keeps anatomical coverage independent from construction and slot', () => {
    for (const id of ['leather_helmet', 'iron_helmet', 'plate_helmet']) {
      expect(itemBodyCoverage(getItemDef(id)!)).toEqual(['head'])
    }
    for (const id of ['leather_armor', 'breastplate']) {
      expect(itemBodyCoverage(getItemDef(id)!)).toEqual(['torso'])
    }
    for (const id of ['chain_mail', 'traveler_robe', 'padded_battle_robe']) {
      expect(itemBodyCoverage(getItemDef(id)!)).toEqual([
        'torso',
        'arms',
        'legs',
      ])
    }
    expect(itemBodyCoverage(getItemDef('brigandine_coat')!)).toEqual([
      'torso',
      'arms',
    ])
  })

  it('requires coverage on garments without assigning it to held gear', () => {
    for (const def of Object.values(itemDefs)) {
      const isGarment =
        def.equipmentKind === 'clothing' || def.equipmentKind === 'body_armor'
      expect(itemBodyCoverage(def).length > 0, def.id).toBe(isGarment)
    }
  })
})
