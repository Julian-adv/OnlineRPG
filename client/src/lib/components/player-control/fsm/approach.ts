import { shortestWrappedDeltaX } from '../../../terrain/world-wrap'
import {
  positionShortOfTarget,
  type Position,
} from '../../../utils/movementUtils'

// ───────────────────────────────────────────────────────────────────────────
// Click → walk up → act
//
// Every clickable interaction shares one rule: act right away when already
// close enough, otherwise walk into reach and act there. The walk-up arms a
// `PendingApproach` on the moving state, resolved when the walk ends — on
// arrival, and equally on a blocked stop, so a path that falls short still
// interacts. Ranges live in data/approachRanges.ts.
// ───────────────────────────────────────────────────────────────────────────

export interface ApproachSpec {
  position: Position
  /** Act once inside this many metres. */
  range: number
  /** How far short of the target the walk-up stops (0 walks onto it). */
  stopShort: number
}

export interface PendingApproach {
  spec: ApproachSpec
  /** Dungeon depth the target sits on; a mismatch drops the approach. */
  depth: number
  canAct?: (position: Pick<Position, 'x' | 'z'>) => boolean
  act: () => void
}

/** What A* makes of a walk-up goal: a real route, a partial route that only
 *  gets as close as the walls allow, or nothing at all. */
export type RouteQuality = 'found' | 'partial' | 'none'

export type ApproachPlan =
  | { kind: 'act_now' }
  | { kind: 'walk'; target: Position }
  | { kind: 'unreachable' }

export function planApproach(
  from: Pick<Position, 'x' | 'z'>,
  spec: ApproachSpec,
  routeQuality: (target: Position) => RouteQuality,
  canActNow = true
): ApproachPlan {
  const dx = shortestWrappedDeltaX(from.x, spec.position.x)
  const dz = spec.position.z - from.z
  const distance = Math.sqrt(dx * dx + dz * dz)
  if (canActNow && distance <= spec.range) return { kind: 'act_now' }

  const standSpot = positionShortOfTarget(from, spec.position, spec.stopShort)
  if (spec.stopShort > 0 && routeQuality(standSpot) === 'found') {
    return { kind: 'walk', target: standSpot }
  }

  // The stand spot is trigonometry and knows nothing of walls. Aiming at the
  // target instead lets A*'s partial route stop the player beside it, around
  // the wall, rather than against the near face.
  return routeQuality(spec.position) === 'none'
    ? { kind: 'unreachable' }
    : { kind: 'walk', target: spec.position }
}

/** Stopped in reach → fire; stopped short, or on another floor → drop. */
export function resolveApproach(
  pending: PendingApproach,
  player: Pick<Position, 'x' | 'z'>,
  depth: number
): boolean {
  if (depth !== pending.depth) return false
  if (pending.canAct && !pending.canAct(player)) return false
  const { position, range } = pending.spec
  const dx = shortestWrappedDeltaX(player.x, position.x)
  const dz = position.z - player.z
  return dx * dx + dz * dz <= range * range
}
