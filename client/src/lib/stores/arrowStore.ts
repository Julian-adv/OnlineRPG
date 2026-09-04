import { writable } from 'svelte/store'

/** One arrow in flight. The shot is already resolved when this is created —
 *  the server answered `PlayerAttacked` before the bow even released — so the
 *  arrow is a tracer, not a simulation. `hit` is what it has to look like:
 *  a hit steers onto the target so the two agree, a miss flies straight past.
 *
 *  Keyed by shooter, and one per shooter is enough: the attack interval
 *  (1380 ms) is longer than the longest flight (10 m at ARROW_SPEED). */
export interface ArrowShot {
  monsterId: string
  hit: boolean
  /** Where it left the bow. */
  from: { x: number; y: number; z: number }
  /** The target as it stood at release — the bearing a miss keeps flying. */
  to: { x: number; y: number; z: number }
  flightMs: number
  /** `performance.now()` at release, so a late frame lands it in one step
   *  rather than replaying the flight. */
  launchedAt: number
  /** The round spent, which decides the model drawn. */
  ammoItemDefId?: string | null
}

/** Metres per second. Fixed rather than a fixed flight time: the delay it
 *  makes visible is the point, so it has to grow with distance. */
export const ARROW_SPEED_MPS = 30

let arrows = new Map<number, ArrowShot>()
export const arrowsInFlight = writable<Map<number, ArrowShot>>(arrows)

export function flightMsFor(distance: number): number {
  return (distance / ARROW_SPEED_MPS) * 1000
}

export function launchArrow(playerId: number, shot: ArrowShot) {
  arrows = new Map(arrows)
  arrows.set(playerId, shot)
  arrowsInFlight.set(arrows)
}

export function landArrow(playerId: number) {
  if (!arrows.has(playerId)) return
  arrows = new Map(arrows)
  arrows.delete(playerId)
  arrowsInFlight.set(arrows)
}

/** A shot waiting for its bow position. `monsterManager` knows the combat
 *  facts but not where the bow is in the world; `GameScene` holds the player
 *  models. The request is made at the release moment, so the scene samples
 *  the bow where the draw actually ends rather than where it began. */
export interface ArrowRequest {
  playerId: number
  monsterId: string
  hit: boolean
  flightMs: number
  /** The round spent, which decides the model drawn. */
  ammoItemDefId?: string | null
}

let pending: ArrowRequest[] = []

export function requestArrow(request: ArrowRequest) {
  pending.push(request)
}

/** Drained once per frame by the scene; empties even if a request cannot be
 *  fulfilled, so a bow that never resolved does not queue up forever. */
export function takeArrowRequests(): ArrowRequest[] {
  if (pending.length === 0) return pending
  const taken = pending
  pending = []
  return taken
}

/** Nothing survives leaving the world — a shot mid-flight when the scene goes
 *  would otherwise reappear on the next one. */
export function clearArrows() {
  pending = []
  if (arrows.size === 0) return
  arrows = new Map()
  arrowsInFlight.set(arrows)
}
