import { describe, expect, it } from 'vitest'
import { clampPanelPos, panelZ, parsePositions } from './panelLayout'

const SIZE = { width: 364, height: 600 }

describe('parsePositions', () => {
  it('falls back to empty on missing or corrupt storage', () => {
    expect(parsePositions(null)).toEqual({})
    expect(parsePositions('{oops')).toEqual({})
    expect(parsePositions('"a string"')).toEqual({})
  })

  it('drops unknown ids and non-numeric positions, ignores a legacy order', () => {
    const raw = JSON.stringify({
      pos: {
        character: { x: 5, y: 6 },
        bogus: { x: 1, y: 2 },
        party: { x: 'a' },
      },
      order: ['party', 'character'],
    })
    expect(parsePositions(raw)).toEqual({ character: { x: 5, y: 6 } })
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

  it('keeps the sliver clear of the header buttons on the left edge', () => {
    expect(clampPanelPos({ x: -9999, y: 0 }, SIZE, 1920, 1080, 60).x).toBe(
      48 + 60 - SIZE.width
    )
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
  it('ranks panels bottom-up under the consent toasts', () => {
    expect(panelZ(['party', 'character'], 'party')).toBe(40)
    expect(panelZ(['party', 'character'], 'character')).toBe(41)
    expect(
      panelZ(['party', 'character', 'friends', 'inventory'], 'inventory')
    ).toBeLessThan(44)
  })
})
