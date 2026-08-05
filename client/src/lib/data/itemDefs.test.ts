import { describe, expect, it } from 'vitest'
import itemDefs, {
  applyBodyCoverage,
  bodyCoveragePercent,
  getItemDef,
  itemBodyCoverage,
  itemCoverageText,
} from './itemDefs'

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

  it('uses one weighted union and ceiling-scales the active profile', () => {
    expect(bodyCoveragePercent(['torso', 'arms', 'legs', 'legs'])).toBe(75)
    expect(
      bodyCoveragePercent(['head', 'torso', 'arms', 'hands', 'legs', 'feet'])
    ).toBe(100)
    expect(applyBodyCoverage({ slash: 3, pierce: 3, blunt: 1 }, 40)).toEqual({
      slash: 2,
      pierce: 2,
      blunt: 1,
    })
    expect(applyBodyCoverage({ slash: 3, pierce: 3, blunt: 1 }, 85)).toEqual({
      slash: 3,
      pierce: 3,
      blunt: 1,
    })
  })

  it('distinguishes defensive armor weight from garment appearance', () => {
    expect(itemCoverageText(getItemDef('leather_armor')!)).toBe(
      'Armor Coverage: Torso (40% weight)'
    )
    expect(itemCoverageText(getItemDef('chain_mail')!)).toBe(
      'Armor Coverage: Torso, Arms, Legs (75% weight)'
    )
    expect(itemCoverageText(getItemDef('brigandine_coat')!)).toBe(
      'Armor Coverage: Torso, Arms (55% weight)'
    )
    expect(itemCoverageText(getItemDef('traveler_robe')!)).toBe(
      'Garment Coverage: Torso, Arms, Legs (not defensive)'
    )
    expect(itemCoverageText(getItemDef('wooden_shield')!)).toBeUndefined()
  })
})
