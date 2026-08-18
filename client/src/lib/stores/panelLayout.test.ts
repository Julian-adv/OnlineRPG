import { describe, expect, it } from 'vitest'
import { clampPanelPos, panelZ, parseLayout } from './panelLayout'

const SIZE = { width: 364, height: 600 }

describe('parseLayout', () => {
  it('falls back to empty on missing or corrupt storage', () => {
    const empty = { pos: {}, order: [] }
    expect(parseLayout(null)).toEqual(empty)
    expect(parseLayout('{oops')).toEqual(empty)
    expect(parseLayout('"a string"')).toEqual(empty)
  })

  it('drops unknown ids, duplicates and non-numeric positions', () => {
    const raw = JSON.stringify({
      pos: {
        character: { x: 5, y: 6 },
        bogus: { x: 1, y: 2 },
        party: { x: 'a' },
      },
      order: ['party', 'party', 'nope', 'character'],
    })
    expect(parseLayout(raw)).toEqual({
      pos: { character: { x: 5, y: 6 } },
      order: ['party', 'character'],
    })
  })
})

describe('clampPanelPos', () => {
  it('leaves an on-screen position alone', () => {
    expect(clampPanelPos({ x: 100, y: 200 }, SIZE, 1920, 1080)).toEqual({
      x: 100,
      y: 200,
    })
  })

  it('keeps a grabbable sliver on each edge', () => {
    expect(clampPanelPos({ x: -9999, y: 0 }, SIZE, 1920, 1080).x).toBe(
      48 - SIZE.width
    )
    expect(clampPanelPos({ x: 9999, y: 0 }, SIZE, 1920, 1080).x).toBe(1920 - 48)
  })

  it('never lets the header leave the viewport vertically', () => {
    expect(clampPanelPos({ x: 0, y: -50 }, SIZE, 1920, 1080).y).toBe(0)
    expect(clampPanelPos({ x: 0, y: 9999 }, SIZE, 1920, 1080).y).toBe(1080 - 28)
  })

  it('stays non-negative on a viewport shorter than the header', () => {
    expect(clampPanelPos({ x: 0, y: 500 }, SIZE, 320, 20).y).toBe(0)
  })
})

describe('panelZ', () => {
  it('is null until raised, then ranks under the trade windows', () => {
    expect(panelZ([], 'party')).toBeNull()
    expect(panelZ(['party', 'character'], 'party')).toBe(41)
    expect(panelZ(['party', 'character'], 'character')).toBe(42)
    expect(
      panelZ(['party', 'character', 'friends', 'inventory'], 'inventory')
    ).toBeLessThan(45)
  })
})
