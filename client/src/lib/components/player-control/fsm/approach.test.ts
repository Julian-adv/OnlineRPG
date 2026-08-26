import { describe, expect, it } from 'vitest'
import { planApproach, resolveApproach, type PendingApproach } from './approach'

const spec = {
  position: { x: 10, y: 0, z: 0 },
  range: 2,
  stopShort: 1.5,
}

describe('planApproach', () => {
  it('acts on the spot when already in range', () => {
    expect(planApproach({ x: 8.5, z: 0 }, spec)).toEqual({ kind: 'act_now' })
  })

  it('walks and stops short of a solid target', () => {
    const plan = planApproach({ x: 0, z: 0 }, spec)

    expect(plan.kind).toBe('walk')
    if (plan.kind !== 'walk') return
    expect(plan.target.x).toBeCloseTo(8.5)
    expect(plan.target.z).toBeCloseTo(0)
  })

  it('walks onto a target with no clearance (a ground item)', () => {
    const plan = planApproach(
      { x: 0, z: 0 },
      { position: { x: 0, y: 0, z: 6 }, range: 2.2, stopShort: 0 }
    )

    expect(plan).toEqual({ kind: 'walk', target: { x: 0, y: 0, z: 6 } })
  })

  it('walks even from inside range while an interaction still has to exit', () => {
    expect(planApproach({ x: 8.5, z: 0 }, spec, false).kind).toBe('walk')
  })
})

const pending: PendingApproach = { spec, depth: 1, act: () => {} }

describe('resolveApproach', () => {
  it('fires once stopped in range', () => {
    expect(resolveApproach(pending, { x: 8.5, z: 0 }, 1)).toBe(true)
  })

  // A path that stops short — blocked, or routed around — must not leave the
  // action armed to fire on some later, unrelated stop.
  it('drops an approach that stopped out of reach', () => {
    expect(resolveApproach(pending, { x: 5, z: 0 }, 1)).toBe(false)
  })

  it('drops an approach left behind on another dungeon floor', () => {
    expect(resolveApproach(pending, { x: 10, z: 0 }, 2)).toBe(false)
  })
})
