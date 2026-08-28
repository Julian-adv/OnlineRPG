import { existsSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { equipBgCandidates } from './equipBackground'

const PUBLIC_DIR = join(__dirname, '..', '..', '..', 'public')

const REACHABLE_COMBOS = [
  ['knight', 'male'],
  ['knight', 'female'],
  ['barbarian', 'male'],
  ['barbarian', 'female'],
  ['rogue', 'male'],
  ['rogue', 'female'],
  ['caveman', 'male'],
  ['caveman', 'female'],
  ['valkyrie', 'female'],
  ['ranger', 'male'],
  ['priest', 'male'],
  ['priest', 'female'],
  ['bard', 'female'],
] as const

describe('equipBgCandidates', () => {
  it('prefers own-gender art, then opposite gender, then the default', () => {
    expect(equipBgCandidates('knight', 'male')).toEqual([
      '/character_concepts/knight.png',
      '/character_concepts/female_knight.png',
      '/character_concepts/female_priest.png',
    ])
    expect(equipBgCandidates('bard', 'male')).toEqual([
      '/character_concepts/bard.png',
      '/character_concepts/female_bard.png',
      '/character_concepts/female_priest.png',
    ])
    expect(equipBgCandidates('ranger', 'female')).toEqual([
      '/character_concepts/female_ranger.png',
      '/character_concepts/ranger.png',
      '/character_concepts/female_priest.png',
    ])
  })

  it('uses irregular filenames for cavewoman and valkyrie', () => {
    expect(equipBgCandidates('caveman', 'female')[0]).toBe(
      '/character_concepts/cavewoman.png'
    )
    expect(equipBgCandidates('caveman', 'male')[0]).toBe(
      '/character_concepts/caveman.png'
    )
    expect(equipBgCandidates('valkyrie', 'female')).toEqual([
      '/character_concepts/valkyrie.jpg',
      '/character_concepts/female_priest.png',
    ])
  })

  it('always includes the default and never repeats a candidate', () => {
    const classes = [
      'knight',
      'barbarian',
      'rogue',
      'caveman',
      'valkyrie',
      'ranger',
      'priest',
      'bard',
      'merchant',
      'guard',
      'samurai',
    ]
    for (const characterClass of classes) {
      for (const gender of ['male', 'female'] as const) {
        const candidates = equipBgCandidates(characterClass, gender)
        expect(candidates).toContain('/character_concepts/female_priest.png')
        expect(new Set(candidates).size).toBe(candidates.length)
      }
    }
  })

  it('falls back to own art before the male art for the female priest', () => {
    expect(equipBgCandidates('priest', 'female')).toEqual([
      '/character_concepts/female_priest.png',
      '/character_concepts/priest.png',
    ])
  })

  it('resolves every creatable class/gender combo to a real file first try', () => {
    for (const [characterClass, gender] of REACHABLE_COMBOS) {
      const [first] = equipBgCandidates(characterClass, gender)
      expect(existsSync(join(PUBLIC_DIR, first)), first).toBe(true)
    }
  })
})
