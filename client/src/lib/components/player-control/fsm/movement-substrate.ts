import {
  calculateMovementStep,
  initMovementState,
  type MovementConfig,
  type MovementState,
  type Position,
} from '../../../utils/movementUtils'
import { shortestWrappedDeltaX } from '../../../terrain/world-wrap'

export interface PathWaypoint {
  x: number
  z: number
  floor: number
}

/** Network move sender: omitted/false `append` replaces the server's waypoint
 *  queue, true extends it (see PlayerMove in shared messages). */
export type SendPlayerMove = (
  position: Position,
  rotation: number,
  append?: boolean
) => void

/** Everything routing a leg needs, shared by click-to-move and combat chase. */
export interface Pathing {
  currentFloor: number
  getFloorAt: (x: number, z: number, y: number) => number
  findPath: (
    startX: number,
    startZ: number,
    startFloor: number,
    goalX: number,
    goalZ: number,
    goalFloor: number
  ) => { waypoints: PathWaypoint[] }
  waypointHeight: (floor: number, x: number, z: number) => number
}

export interface RoutedLeg {
  pathWaypoints: PathWaypoint[]
  movementTarget: Position
  playerRotation: number
}

/**
 * Route to `goal` and hand the first leg to the server, replacing its queue.
 *
 * The single place a fresh path is built. The server replays every leg it is
 * sent as a straight line, so a goal it cannot walk to directly strands its copy
 * of the player behind geometry the client walked around — routing here, rather
 * than beelining at the goal, is what keeps the two simulations together. The
 * substrate appends the remaining waypoints as each is reached.
 */
export function routeFirstLeg(
  currentPos: Position,
  goal: Position,
  pathing: Pathing,
  sendPlayerMove: SendPlayerMove
): RoutedLeg {
  const goalFloor = pathing.getFloorAt(goal.x, goal.z, goal.y)
  const result = pathing.findPath(
    currentPos.x,
    currentPos.z,
    pathing.currentFloor,
    goal.x,
    goal.z,
    goalFloor
  )
  const pathWaypoints =
    result.waypoints.length > 0
      ? result.waypoints
      : [{ x: goal.x, z: goal.z, floor: goalFloor }]

  const firstWp = pathWaypoints[0]
  const movementTarget: Position = {
    x: firstWp.x,
    y: pathing.waypointHeight(firstWp.floor, firstWp.x, firstWp.z),
    z: firstWp.z,
  }
  const playerRotation = Math.atan2(
    shortestWrappedDeltaX(currentPos.x, movementTarget.x),
    movementTarget.z - currentPos.z
  )
  sendPlayerMove(movementTarget, playerRotation, false)

  return { pathWaypoints, movementTarget, playerRotation }
}

interface MovementSubstrateInput {
  currentPos: Position
  movementTarget: Position
  movementState: MovementState
  pathWaypoints: PathWaypoint[]
  currentWaypointIndex: number
  config: MovementConfig
  deltaTimeSeconds: number
  sampleHeight: (x: number, z: number) => number
  waypointHeight: (floor: number, x: number, z: number) => number
  isMovementBlocked: (
    fromX: number,
    fromZ: number,
    toX: number,
    toZ: number,
    y: number
  ) => boolean
  isUphillTooSteep: (
    x: number,
    z: number,
    y: number,
    dirX: number,
    dirZ: number
  ) => boolean
  setFloorLevel: (floor: number) => void
  writePlayerPosition: (position: Position, rotation: number) => void
  sendPlayerMove: SendPlayerMove
}

/**
 * When a diagonal step is blocked (typically grazing a convex wall corner the
 * 0.3m body radius can't clear), try to keep moving along whichever single axis
 * is still clear. This lets the player slide around corners instead of getting
 * permanently stuck — the pathfinder smooths paths using cell-edge walls only
 * (no radius buffer), so smoothed diagonals can clip corners that continuous
 * collision refuses to cross. Returns the slid position, or null if both axes
 * are blocked (a genuine dead-end).
 */
function resolveWallSlide(
  from: Position,
  to: Position,
  isMovementBlocked: MovementSubstrateInput['isMovementBlocked']
): Position | null {
  const dx = to.x - from.x
  const dz = to.z - from.z
  const EPS = 1e-6

  const xOnlyOk =
    Math.abs(dx) > EPS &&
    !isMovementBlocked(from.x, from.z, from.x + dx, from.z, from.y)
  const zOnlyOk =
    Math.abs(dz) > EPS &&
    !isMovementBlocked(from.x, from.z, from.x, from.z + dz, from.y)

  // When both axes are individually clear (a corner tip blocks only the exact
  // diagonal), keep the axis with the greater progress toward the target.
  const preferX = xOnlyOk && (!zOnlyOk || Math.abs(dx) >= Math.abs(dz))
  if (preferX) return { x: from.x + dx, y: from.y, z: from.z }
  if (zOnlyOk) return { x: from.x, y: from.y, z: from.z + dz }
  return null
}

export type MovementSubstrateOutcome =
  | { kind: 'blocked' }
  | { kind: 'slope_blocked' }
  | {
      kind: 'continued'
      currentSpeed: number
      playerRotation: number
      totalDistance: number
    }
  | {
      kind: 'next_waypoint'
      currentSpeed: number
      playerRotation: number
      movementTarget: Position
      movementState: MovementState
      currentWaypointIndex: number
    }
  | {
      kind: 'arrived'
      currentSpeed: number
      playerRotation: number
    }

export function stepMovementSubstrate({
  currentPos,
  movementTarget,
  movementState,
  pathWaypoints,
  currentWaypointIndex,
  config,
  deltaTimeSeconds,
  sampleHeight,
  waypointHeight,
  isMovementBlocked,
  isUphillTooSteep,
  setFloorLevel,
  writePlayerPosition,
  sendPlayerMove,
}: MovementSubstrateInput): MovementSubstrateOutcome {
  const result = calculateMovementStep(
    currentPos,
    movementState,
    config,
    deltaTimeSeconds
  )

  movementState.currentSpeed = result.newSpeed
  const currentSpeed = result.newSpeed
  const playerRotation = result.rotation

  if (result.arrived) {
    if (
      isMovementBlocked(
        currentPos.x,
        currentPos.z,
        movementTarget.x,
        movementTarget.z,
        currentPos.y
      )
    ) {
      // Blocked stops replace the server's queue with the stop point so it
      // doesn't keep walking to an already-sent waypoint.
      sendPlayerMove(currentPos, playerRotation)
      return { kind: 'blocked' }
    }

    // Re-sampled: a leg's Y was fixed when it started, and a shaft is one
    // straight leg, so the depth (and the floor's Y) can flip on the way.
    const arrivedPos: Position = {
      x: movementTarget.x,
      y: sampleHeight(movementTarget.x, movementTarget.z),
      z: movementTarget.z,
    }
    writePlayerPosition(arrivedPos, playerRotation)

    const nextWaypointIndex = currentWaypointIndex + 1
    if (nextWaypointIndex < pathWaypoints.length) {
      const nextWp = pathWaypoints[nextWaypointIndex]
      // Pre-set before the leg starts — see MovingStateData.floor.
      setFloorLevel(nextWp.floor)

      const wpPos: Position = {
        x: nextWp.x,
        y: waypointHeight(nextWp.floor, nextWp.x, nextWp.z),
        z: nextWp.z,
      }

      const ndx = shortestWrappedDeltaX(movementTarget.x, wpPos.x)
      const ndz = wpPos.z - movementTarget.z
      const nextRotation = Math.atan2(ndx, ndz)
      const nextMovementState = initMovementState(
        movementTarget,
        wpPos,
        movementState.currentSpeed
      )

      sendPlayerMove(wpPos, nextRotation, true)

      return {
        kind: 'next_waypoint',
        currentSpeed: nextMovementState.currentSpeed,
        playerRotation: nextRotation,
        movementTarget: wpPos,
        movementState: nextMovementState,
        currentWaypointIndex: nextWaypointIndex,
      }
    }

    sendPlayerMove(arrivedPos, playerRotation, true)
    return { kind: 'arrived', currentSpeed, playerRotation }
  }

  let stepPos = result.newPos
  if (
    isMovementBlocked(
      currentPos.x,
      currentPos.z,
      stepPos.x,
      stepPos.z,
      currentPos.y
    )
  ) {
    const slid = resolveWallSlide(currentPos, stepPos, isMovementBlocked)
    if (!slid) {
      sendPlayerMove(currentPos, playerRotation)
      return { kind: 'blocked' }
    }
    stepPos = slid
  }

  const dirX = Math.sin(result.rotation)
  const dirZ = Math.cos(result.rotation)
  if (isUphillTooSteep(currentPos.x, currentPos.z, currentPos.y, dirX, dirZ)) {
    sendPlayerMove(currentPos, playerRotation)
    return { kind: 'slope_blocked' }
  }

  writePlayerPosition(
    {
      x: stepPos.x,
      y: sampleHeight(stepPos.x, stepPos.z),
      z: stepPos.z,
    },
    playerRotation
  )

  return {
    kind: 'continued',
    currentSpeed,
    playerRotation,
    totalDistance: movementState.totalDistance,
  }
}
