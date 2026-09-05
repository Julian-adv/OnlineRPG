import { describe, expect, it } from 'vitest'
import { planApproach, resolveApproach, type PendingApproach } from './approach'

const spec = {
  position: { x: 10, y: 0, z: 0 },
  range: 2,
  stopShort: 1.5,
}

const routable = () => 'found' as const

describe('planApproach', () => {
  it('acts on the spot when already in range', () => {
    expect(planApproach({ x: 8.5, z: 0 }, spec, routable)).toEqual({
      kind: 'act_now',
    })
  })

  it('walks and stops short of a solid target', () => {
    const plan = planApproach({ x: 0, z: 0 }, spec, routable)

    expect(plan.kind).toBe('walk')
    if (plan.kind !== 'walk') return
    expect(plan.target.x).toBeCloseTo(8.5)
    expect(plan.target.z).toBeCloseTo(0)
  })

  it('walks onto a target with no clearance (a ground item)', () => {
    const plan = planApproach(
      { x: 0, z: 0 },
      { position: { x: 0, y: 0, z: 6 }, range: 2.2, stopShort: 0 },
      routable
    )

    expect(plan).toEqual({ kind: 'walk', target: { x: 0, y: 0, z: 6 } })
  })

  it('walks even from inside range while an interaction still has to exit', () => {
    expect(planApproach({ x: 8.5, z: 0 }, spec, routable, false).kind).toBe(
      'walk'
    )
  })

  // A stand spot inside the wall the target sits behind gets only a partial
  // route, which walks the player into that wall. Aiming at the target routes
  // them around it instead.
  it('re-aims at the target when the stand spot is walled off', () => {
    const plan = planApproach({ x: 0, z: 0 }, spec, () => 'partial')

    expect(plan).toEqual({ kind: 'walk', target: spec.position })
  })

  it('keeps the stand spot when it routes cleanly', () => {
    const quality = (target: { x: number }) =>
      target.x === spec.position.x ? ('partial' as const) : ('found' as const)

    const plan = planApproach({ x: 0, z: 0 }, spec, quality)

    expect(plan.kind).toBe('walk')
    if (plan.kind !== 'walk') return
    expect(plan.target.x).toBeCloseTo(8.5)
  })

  it('refuses to move when nothing about the target routes at all', () => {
    expect(planApproach({ x: 0, z: 0 }, spec, () => 'none')).toEqual({
      kind: 'unreachable',
    })
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

  it('drops an in-range approach when an obstacle still blocks the action', () => {
    expect(
      resolveApproach({ ...pending, canAct: () => false }, { x: 8.5, z: 0 }, 1)
    ).toBe(false)
  })
})
