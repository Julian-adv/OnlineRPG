import { existsSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { equipBgCandidates, equipBgFilter } from './equipBackground'
import { PLAYER_CLASSES, getAvailableGenders } from './modelPaths'

const PUBLIC_DIR = join(__dirname, '..', '..', '..', 'public')
const DEFAULT = '/character_concepts/female_priest.webp'

const REACHABLE_COMBOS = PLAYER_CLASSES.flatMap((cls) =>
  getAvailableGenders(cls).map((gender) => [cls, gender] as const)
)

describe('equipBgCandidates', () => {
  it('prefers own-gender art, then opposite gender, then the default', () => {
    expect(equipBgCandidates('knight', 'male')).toEqual([
      '/character_concepts/knight.webp',
      '/character_concepts/female_knight.webp',
      DEFAULT,
    ])
    expect(equipBgCandidates('ranger', 'female')).toEqual([
      '/character_concepts/female_ranger.webp',
      '/character_concepts/ranger.webp',
      DEFAULT,
    ])
  })

  it('uses irregular filenames for cavewoman and valkyrie', () => {
    expect(equipBgCandidates('caveman', 'female')[0]).toBe(
      '/character_concepts/cavewoman.webp'
    )
    expect(equipBgCandidates('valkyrie', 'female')).toEqual([
      '/character_concepts/valkyrie.webp',
      DEFAULT,
    ])
  })

  it('always includes the default and never repeats a candidate', () => {
    for (const cls of [...PLAYER_CLASSES, 'merchant', 'guard', 'samurai']) {
      for (const gender of ['male', 'female'] as const) {
        const candidates = equipBgCandidates(cls, gender)
        expect(candidates).toContain(DEFAULT)
        expect(new Set(candidates).size).toBe(candidates.length)
      }
    }
  })

  it('resolves every creatable class/gender combo to a real file first try', () => {
    expect(REACHABLE_COMBOS.length).toBe(13)
    for (const [cls, gender] of REACHABLE_COMBOS) {
      const [first] = equipBgCandidates(cls, gender)
      expect(existsSync(join(PUBLIC_DIR, first)), first).toBe(true)
    }
  })
})

describe('equipBgFilter', () => {
  it('darkens only the mapped art', () => {
    expect(equipBgFilter('/character_concepts/barbarian.webp')).toBe(
      'brightness(0.7)'
    )
    expect(equipBgFilter('/character_concepts/knight.webp')).toBeUndefined()
    expect(equipBgFilter(undefined)).toBeUndefined()
  })
})
