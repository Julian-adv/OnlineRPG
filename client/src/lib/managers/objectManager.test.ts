import { describe, expect, it } from 'vitest'
import { objectManager } from './objectManager'

const chair = (id: number, x: number, z: number) => ({
  id,
  type: 'chair',
  x,
  y: 1.3,
  z,
  rotation: 0,
})

describe('seat lookup', () => {
  // Two chairs at one table sit closer to each other than a sitter's
  // stored position does to either, so the server's placement id must win.
  ;(objectManager as unknown as { cache: Map<string, unknown> }).cache.set(
    'test',
    { placements: [chair(42, -1451.7, 4750.3), chair(40, -1450.0, 4751.5)] }
  )

  it('takes the placement the server named over the nearest one', () => {
    expect(
      objectManager.findNearestPlacement('chair', -1449.0, 4753.4, 42)?.id
    ).toBe(42)
    expect(
      objectManager.findNearestPlacement('chair', -1451.7, 4750.3, 40)?.id
    ).toBe(40)
  })

  it('falls back to distance without an id', () => {
    expect(
      objectManager.findNearestPlacement('chair', -1449.0, 4753.4)?.id
    ).toBe(40)
    expect(
      objectManager.findNearestPlacement('chair', -1449.0, 4753.4, null)?.id
    ).toBe(40)
    expect(
      objectManager.findNearestPlacement('bed', -1449.0, 4753.4, 42)
    ).toBeNull()
  })
})
