import { describe, it, expect } from 'vitest'
import { computeChaseAim } from './chase-aim'

const at = (x: number, z: number, y = 0) => ({ x, y, z })
const open = () => false
const walled = () => true

describe('computeChaseAim', () => {
  it('aims at the engage ring around the live target', () => {
    // Leg target where the server last clipped the chase; player has since
    // moved on to z=13.
    const aim = computeChaseAim(at(0, 0), at(0, 10.5), at(0, 13), 1.15, open)
    expect(aim).toBeDefined()
    expect(aim!.x).toBeCloseTo(0)
    expect(aim!.z).toBeCloseTo(13 - 1.15, 3)
  })

  it('stands still once inside the engage ring', () => {
    const aim = computeChaseAim(at(0, 12), at(0, 11), at(0, 13), 1.15, walled)
    expect(aim).toEqual(at(0, 12))
  })

  it('keeps the leg target when the straight line is walled off', () => {
    const aim = computeChaseAim(at(0, 0), at(0, 2), at(0, 3), 1.15, walled)
    expect(aim).toBeUndefined()
  })

  it('keeps the leg target on an implausible shift (teleported target)', () => {
    const aim = computeChaseAim(at(0, 0), at(0, 2), at(0, 60), 1.15, open)
    expect(aim).toBeUndefined()
  })
})
