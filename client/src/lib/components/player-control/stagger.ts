import { debuffStaggerM } from '../../data/debuffPresentation'
import type { ActiveDebuff } from '../../stores/debuffStore'
import type { Position } from '../../utils/movementUtils'

/** The widest stagger any active debuff carries, 0 when walking straight. */
export function staggerRadius(debuffs: ActiveDebuff[], now: number): number {
  return debuffs.reduce(
    (max, d) => (d.until > now ? Math.max(max, debuffStaggerM(d.id)) : max),
    0
  )
}

/** A click target pushed off in a random direction, between 40% and 100%
 *  of `radius`, so a staggering walker weaves toward where they meant to
 *  go. `random` is `Math.random` unless a test pins it. */
export function staggerTarget(
  target: Position,
  radius: number,
  random: () => number = Math.random
): Position {
  const angle = random() * Math.PI * 2
  const dist = radius * (0.4 + random() * 0.6)
  return {
    x: target.x + Math.cos(angle) * dist,
    y: target.y,
    z: target.z + Math.sin(angle) * dist,
  }
}
