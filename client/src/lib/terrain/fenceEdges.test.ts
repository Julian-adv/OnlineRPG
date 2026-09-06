import { describe, expect, it } from 'vitest'
import {
  fenceKey,
  fenceOnOwnedPlot,
  nearestFenceEdge,
  fenceInReach,
} from './fenceEdges'
import { WORLD_MIN_X, WORLD_MAX_X } from './world-wrap'

describe('fence cell edges', () => {
  it('selects all four sides and shares the same edge with the adjacent cell', () => {
    expect(nearestFenceEdge(2.5, 3.05)).toEqual({ x: 2, z: 3, axis: 'X' })
    expect(nearestFenceEdge(2.95, 3.5)).toEqual({ x: 3, z: 3, axis: 'Z' })
    expect(nearestFenceEdge(2.5, 3.95)).toEqual({ x: 2, z: 4, axis: 'X' })
    expect(nearestFenceEdge(2.05, 3.5)).toEqual({ x: 2, z: 3, axis: 'Z' })
    expect(fenceKey(nearestFenceEdge(2.95, 3.5))).toBe(
      fenceKey(nearestFenceEdge(3.05, 3.5))
    )
    expect(fenceKey(nearestFenceEdge(-2.5, -3.05))).toBe(
      fenceKey(nearestFenceEdge(-2.5, -2.95))
    )
  })

  it('canonicalizes the world seam and measures wrapped reach', () => {
    const edge = nearestFenceEdge(WORLD_MAX_X - 0.05, 1.5)
    expect(edge).toEqual({ x: WORLD_MIN_X, z: 1, axis: 'Z' })
    expect(fenceKey(edge)).toBe(
      fenceKey(nearestFenceEdge(WORLD_MIN_X + 0.05, 1.5))
    )
    expect(fenceInReach(edge, { x: WORLD_MAX_X - 1, z: 1.5 })).toBe(true)
  })

  it('allows all estate boundaries but rejects edges beyond them', () => {
    const plots = [{ x: 0, z: 0 }]
    for (const edge of [
      nearestFenceEdge(0.05, 4.5),
      nearestFenceEdge(31.95, 4.5),
      nearestFenceEdge(4.5, 0.05),
      nearestFenceEdge(4.5, 31.95),
    ]) {
      expect(fenceOnOwnedPlot(edge, plots)).toBe(true)
    }
    expect(fenceOnOwnedPlot(nearestFenceEdge(32.95, 4.5), plots)).toBe(false)
  })
})
