import type { LandClaim } from '../../stores/landClaimStore'

export function createLandClaimMovementTracker() {
  let instanceId: number | null = null
  let lastX = 0
  let lastZ = 0
  let stillSeconds = 0
  let moved = false

  return (
    claim: LandClaim | null,
    position: { x: number; z: number } | null,
    delta: number
  ): boolean => {
    if (
      !claim ||
      !position ||
      claim.status === 'pending' ||
      claim.status === 'claimed'
    ) {
      instanceId = null
      return false
    }
    if (instanceId !== claim.instance_id) {
      instanceId = claim.instance_id
      lastX = position.x
      lastZ = position.z
      stillSeconds = 0
      moved = false
      return false
    }
    if (position.x !== lastX || position.z !== lastZ) {
      lastX = position.x
      lastZ = position.z
      stillSeconds = 0
      moved = true
      return false
    }
    stillSeconds += delta
    if (!moved || stillSeconds < 0.35 || claim.refreshing) return false
    moved = false
    return true
  }
}
