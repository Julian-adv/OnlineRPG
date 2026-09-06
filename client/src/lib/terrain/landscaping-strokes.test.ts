import { describe, expect, it } from 'vitest'
import { LandscapingStrokes } from './landscaping-strokes'
import type { LandscapingStroke } from './landscaping'
import { WORLD_MIN_X, WORLD_MAX_X } from './world-wrap'

function dab(x: number, z = 4.5): LandscapingStroke {
  return { start: [x, z], end: null, radius: 1, strength: 10, palette: 5 }
}

describe('landscaping strokes', () => {
  it('joins successive drag positions instead of leaving gaps between dabs', () => {
    const strokes = new LandscapingStrokes()
    strokes.begin(dab(4.5))
    expect(strokes.take()).toEqual(dab(4.5))
    strokes.move(dab(10.5))
    expect(strokes.take()).toEqual({ ...dab(4.5), end: [10.5, 4.5] })
    strokes.move(dab(16.5))
    expect(strokes.take()).toEqual({ ...dab(10.5), end: [16.5, 4.5] })
  })

  it('keeps the final segment when the mouse is released before a response', () => {
    const strokes = new LandscapingStrokes()
    strokes.begin(dab(4.5))
    strokes.take()
    strokes.move(dab(10.5))
    strokes.finish()
    expect(strokes.take()).toEqual({ ...dab(4.5), end: [10.5, 4.5] })
    expect(strokes.take()).toBeNull()
  })

  it('preserves rapid separate clicks without painting a line between them', () => {
    const strokes = new LandscapingStrokes()
    for (const x of [4.5, 10.5, 16.5]) {
      strokes.begin(dab(x))
      strokes.finish()
    }
    for (const x of [4.5, 10.5, 16.5]) expect(strokes.take()).toEqual(dab(x))
    expect(strokes.take()).toBeNull()
  })

  it('retains a road submitted while the preceding edit is pending', () => {
    const strokes = new LandscapingStrokes()
    const road = { ...dab(4.5), end: [16.5, 4.5] as [number, number] }
    strokes.begin(dab(0.5))
    strokes.finish()
    strokes.addRoad(road)
    expect(strokes.take()).toEqual(dab(0.5))
    expect(strokes.take()).toEqual(road)
  })

  it('continues painting while held still and clears drafts on mode changes', () => {
    const strokes = new LandscapingStrokes()
    strokes.begin(dab(4.5))
    strokes.take()
    expect(strokes.take()).toEqual(dab(4.5))
    strokes.move(dab(10.5))
    strokes.clear()
    expect(strokes.take()).toBeNull()
  })

  it('connects across the world seam but does not bridge teleports', () => {
    const strokes = new LandscapingStrokes()
    strokes.begin(dab(WORLD_MAX_X - 2))
    strokes.take()
    strokes.move(dab(WORLD_MIN_X + 2))
    expect(strokes.take()?.end).toEqual([WORLD_MIN_X + 2, 4.5])
    strokes.move(dab(0.5))
    expect(strokes.take()).toEqual(dab(0.5))
  })
})
