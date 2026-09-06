import { describe, expect, it } from 'vitest'
import { landscapingSamples, type LandscapingStroke } from './landscaping'
import { WORLD_MIN_X } from './world-wrap'

const stroke: LandscapingStroke = {
  start: [31, 16],
  end: [38, 16],
  radius: 2,
  strength: 10,
  palette: 5,
}

describe('landscaping preview permissions', () => {
  it('previews width one as one square or one row, with no visible fringe', () => {
    const brush = {
      ...stroke,
      start: [4.4, 4.4] as [number, number],
      end: null,
      radius: 0.5,
    }
    const core = landscapingSamples(brush, null).filter(
      (s) => !s.fringe && s.weight > 0
    )
    expect(core).toEqual([{ x: 4, z: 4, weight: 1, fringe: false }])
    const road = landscapingSamples(
      { ...brush, end: [10.4, 4.4] },
      null
    ).filter((s) => !s.fringe && s.weight > 0)
    expect(road).toHaveLength(7)
    expect(road.every((s) => s.z === 4 && s.weight === 1)).toBe(true)
    const negative = landscapingSamples(
      { ...brush, start: [-0.5, -0.5] },
      null
    ).filter((s) => !s.fringe && s.weight > 0)
    expect(negative).toEqual([{ x: 0, z: 0, weight: 1, fringe: false }])
  })

  it('shows a full-strength core for a two-cell brush between grid points', () => {
    const samples = landscapingSamples(
      { ...stroke, start: [4.5, 4.5], end: null, radius: 1 },
      null
    )
    for (const [x, z] of [
      [4, 4],
      [4, 5],
      [5, 4],
      [5, 5],
    ]) {
      expect(samples.find((s) => s.x === x && s.z === z)?.weight).toBe(1)
    }
  })

  it('clips normal brushes to owned plots, including the fringe', () => {
    expect(landscapingSamples(stroke, [])).toEqual([])
    const samples = landscapingSamples(stroke, [{ x: 0, z: 0 }])
    expect(samples.length).toBeGreaterThan(0)
    expect(samples.every(({ x }) => x > 0 && x < 32)).toBe(true)
  })

  it('previews the full admin road across estate boundaries', () => {
    const samples = landscapingSamples(stroke, null)
    expect(samples.some(({ x }) => x < 32)).toBe(true)
    expect(samples.some(({ x }) => x > 38)).toBe(true)
  })

  it('keeps unrestricted previews inside the world and rejects long roads', () => {
    const samples = landscapingSamples(
      { ...stroke, start: [16, WORLD_MIN_X], end: null },
      null
    )
    expect(samples.length).toBeGreaterThan(0)
    expect(samples.every(({ z }) => z >= WORLD_MIN_X)).toBe(true)
    expect(landscapingSamples({ ...stroke, end: [1000, 16] }, null)).toEqual([])
  })
})
