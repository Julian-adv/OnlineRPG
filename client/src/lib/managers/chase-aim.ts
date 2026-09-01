import { shortestWrappedDeltaX } from '../terrain/world-wrap'
import { positionShortOfTarget, type Position } from '../utils/movementUtils'

// Extrapolation bound, not a wall proxy (walls go through `isBlocked`): a
// live target much farther than the synced leg target means a teleport or a
// desynced view, where walking the straight line is worse than the sync.
const LIVE_AIM_MAX_SHIFT_METERS = 4.0

/** Where a chasing monster should walk given its target's live position:
 *  the engage ring around the target, the monster's own position once inside
 *  the ring, or `undefined` to keep the synced leg target after an implausible
 *  shift. */
export function computeChaseAim(
  monsterPos: Position,
  legTarget: Position,
  live: Position,
  stopRange: number
): Position | undefined {
  const dx = shortestWrappedDeltaX(monsterPos.x, live.x)
  const dz = live.z - monsterPos.z
  const dist = Math.hypot(dx, dz)
  if (dist <= stopRange) return monsterPos
  const aim = positionShortOfTarget(monsterPos, live, stopRange)
  const shiftX = shortestWrappedDeltaX(legTarget.x, aim.x)
  const shiftZ = aim.z - legTarget.z
  if (
    shiftX * shiftX + shiftZ * shiftZ >
    LIVE_AIM_MAX_SHIFT_METERS * LIVE_AIM_MAX_SHIFT_METERS
  ) {
    return undefined
  }
  return aim
}
