import { describe, expect, it } from 'vitest'
import { staggerRadius, staggerTarget } from './stagger'

describe('staggerRadius', () => {
  it('reads the widest stagger off the unexpired debuffs', () => {
    const now = 1_000
    expect(staggerRadius([], now)).toBe(0)
    expect(staggerRadius([{ id: 'tipsy', until: now + 1 }], now)).toBe(0)
    expect(staggerRadius([{ id: 'wasted', until: now + 1 }], now)).toBe(2.5)
    expect(staggerRadius([{ id: 'wasted', until: now - 1 }], now)).toBe(0)
  })
})

describe('staggerTarget', () => {
  it('lands within the band around the click, keeping height', () => {
    const target = { x: 10, y: 1.3, z: -4 }
    for (let i = 0; i < 50; i++) {
      const p = staggerTarget(target, 2.5)
      const d = Math.hypot(p.x - target.x, p.z - target.z)
      expect(d).toBeGreaterThanOrEqual(1.0 - 1e-6)
      expect(d).toBeLessThanOrEqual(2.5 + 1e-6)
      expect(p.y).toBe(1.3)
    }
  })

  it('is deterministic under a pinned random source', () => {
    const rolls = [0.25, 0.5]
    let i = 0
    const p = staggerTarget({ x: 0, y: 0, z: 0 }, 2, () => rolls[i++])
    // angle π/2 → +z; distance 70% of the radius.
    expect(p.x).toBeCloseTo(0, 6)
    expect(p.z).toBeCloseTo(1.4, 6)
  })
})
