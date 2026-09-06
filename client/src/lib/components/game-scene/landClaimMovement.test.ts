import { describe, expect, it } from 'vitest'
import { createLandClaimMovementTracker } from './landClaimMovement'
import type { LandClaim } from '../../stores/landClaimStore'

const claim: LandClaim = {
  instance_id: 1,
  tile_x: 0,
  tile_z: 0,
  quadrant: 3,
  status: 'confirm',
}

describe('land preview movement', () => {
  it('refreshes once after movement settles, including within the same plot', () => {
    const update = createLandClaimMovementTracker()
    expect(update(claim, { x: 1, z: 1 }, 1)).toBe(false)
    expect(update(claim, { x: 1, z: 1 }, 1)).toBe(false)
    expect(update(claim, { x: 2, z: 1 }, 0.1)).toBe(false)
    expect(update(claim, { x: 2, z: 1 }, 0.2)).toBe(false)
    expect(update(claim, { x: 3, z: 1 }, 0.1)).toBe(false)
    expect(update(claim, { x: 3, z: 1 }, 0.2)).toBe(false)
    expect(update(claim, { x: 3, z: 1 }, 0.2)).toBe(true)
    expect(update(claim, { x: 3, z: 1 }, 1)).toBe(false)
  })

  it('refreshes a rejected preview after moving and waits for in-flight requests', () => {
    const update = createLandClaimMovementTracker()
    const rejected: LandClaim = { ...claim, status: 'rejected' }
    update(rejected, { x: 1, z: 1 }, 0.1)
    update(rejected, { x: 33, z: 1 }, 0.1)
    expect(update({ ...rejected, refreshing: true }, { x: 33, z: 1 }, 1)).toBe(
      false
    )
    expect(update(rejected, { x: 33, z: 1 }, 0.1)).toBe(true)
  })

  it.each(['pending', 'claimed'] as const)(
    'does not refresh a %s claim',
    (status) => {
      const update = createLandClaimMovementTracker()
      update(claim, { x: 1, z: 1 }, 0.1)
      update(claim, { x: 33, z: 1 }, 0.1)
      expect(update({ ...claim, status }, { x: 33, z: 1 }, 1)).toBe(false)
    }
  )

  it('forgets movement when the preview closes', () => {
    const update = createLandClaimMovementTracker()
    update(claim, { x: 1, z: 1 }, 0.1)
    update(claim, { x: 33, z: 1 }, 0.1)
    expect(update(null, { x: 33, z: 1 }, 1)).toBe(false)
    expect(update(claim, { x: 33, z: 1 }, 1)).toBe(false)
  })
})
