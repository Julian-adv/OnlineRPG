import { shortestWrappedDeltaX, wrapWorldX } from '../../../terrain/world-wrap'
import type { Position } from '../../../utils/movementUtils'

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
  act: () => void
}

export type ApproachPlan =
  | { kind: 'act_now' }
  | { kind: 'walk'; target: Position }

export function planApproach(
  from: Pick<Position, 'x' | 'z'>,
  spec: ApproachSpec,
  canActNow = true
): ApproachPlan {
  const dx = shortestWrappedDeltaX(from.x, spec.position.x)
  const dz = spec.position.z - from.z
  const distance = Math.sqrt(dx * dx + dz * dz)
  if (canActNow && distance <= spec.range) return { kind: 'act_now' }

  const walked =
    distance > 0 ? 1 - Math.min(spec.stopShort, distance) / distance : 0
  return {
    kind: 'walk',
    target: {
      x: wrapWorldX(from.x + dx * walked),
      y: spec.position.y,
      z: from.z + dz * walked,
    },
  }
}

/** Stopped in reach → fire; stopped short, or on another floor → drop. */
export function resolveApproach(
  pending: PendingApproach,
  player: Pick<Position, 'x' | 'z'>,
  depth: number
): boolean {
  if (depth !== pending.depth) return false
  const { position, range } = pending.spec
  const dx = shortestWrappedDeltaX(player.x, position.x)
  const dz = position.z - player.z
  return dx * dx + dz * dz <= range * range
}
