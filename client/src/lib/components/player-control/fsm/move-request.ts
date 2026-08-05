import {
  initMovementState,
  type MovementState,
  type Position,
} from '../../../utils/movementUtils'
import type { InteractionExitKind } from './interaction'
import {
  routeFirstLeg,
  type Pathing,
  type PathWaypoint,
  type SendPlayerMove,
} from './movement-substrate'

// ───────────────────────────────────────────────────────────────────────────
// Move-request decision (click → start / exit-interaction / ignore)
// ───────────────────────────────────────────────────────────────────────────

export type MoveRequestDecision =
  | {
      kind: 'ignored'
      clearPendingPickupAfterMove: boolean
    }
  | {
      kind: 'exit_pickup_and_retry'
      clearPendingPickupAfterMove: boolean
    }
  | {
      kind: 'exit_object_and_delay'
      clearPendingPickupAfterMove: boolean
    }
  | {
      kind: 'start'
      clearPendingPickupAfterMove: boolean
    }

interface DecideMoveRequestInput {
  pickupAfterArrival: number | null
  currentPlayerHealth: number | null
  interactionExit: InteractionExitKind
  hasCurrentPlayer: boolean
  isMoving: boolean
  hasKeyboardInput: boolean
}

export function decideMoveRequest({
  pickupAfterArrival,
  currentPlayerHealth,
  interactionExit,
  hasCurrentPlayer,
  isMoving,
  hasKeyboardInput,
}: DecideMoveRequestInput): MoveRequestDecision {
  const clearPendingPickupAfterMove = pickupAfterArrival === null

  if (currentPlayerHealth !== null && currentPlayerHealth <= 0) {
    return { kind: 'ignored', clearPendingPickupAfterMove }
  }

  if (interactionExit === 'pickup') {
    return { kind: 'exit_pickup_and_retry', clearPendingPickupAfterMove }
  }

  if (interactionExit === 'object') {
    return { kind: 'exit_object_and_delay', clearPendingPickupAfterMove }
  }

  if (!hasCurrentPlayer || isMoving || hasKeyboardInput) {
    if (hasCurrentPlayer && isMoving && !hasKeyboardInput) {
      return { kind: 'start', clearPendingPickupAfterMove }
    }
    return { kind: 'ignored', clearPendingPickupAfterMove }
  }

  return { kind: 'start', clearPendingPickupAfterMove }
}

// ───────────────────────────────────────────────────────────────────────────
// Path-based click movement initialization
// ───────────────────────────────────────────────────────────────────────────

interface StartClickMovementInput extends Pathing {
  currentPos: Position
  clickPosition: Position
  initialSpeed: number
  pickupAfterArrival: number | null
  sendPlayerMove: SendPlayerMove
}

export interface StartedClickMovement {
  pathWaypoints: PathWaypoint[]
  currentWaypointIndex: number
  movementState: MovementState
  movementTarget: Position
  playerRotation: number
  pendingPickupAfterMoveInstanceId: number | null
}

export function startClickMovement({
  currentPos,
  clickPosition,
  initialSpeed,
  pickupAfterArrival,
  sendPlayerMove,
  ...pathing
}: StartClickMovementInput): StartedClickMovement {
  const leg = routeFirstLeg(currentPos, clickPosition, pathing, sendPlayerMove)
  return {
    ...leg,
    currentWaypointIndex: 0,
    movementState: initMovementState(
      currentPos,
      leg.movementTarget,
      initialSpeed
    ),
    pendingPickupAfterMoveInstanceId: pickupAfterArrival,
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Full move-request flow (decision + click movement start)
// ───────────────────────────────────────────────────────────────────────────

interface MoveRequestPlayer {
  health: number
  position: Position
}

export interface MoveRequestActions {
  clearPendingPickupAfterMove: () => void
  exitPickupAndRetry: () => void
  exitObjectAndDelay: () => void
  applyStartedMovement: (started: StartedClickMovement) => void
}

interface RunMoveRequestInput extends Pathing {
  clickPosition: Position
  initialSpeed: number
  pickupAfterArrival: number | null
  currentPlayer: MoveRequestPlayer | null
  interactionExit: InteractionExitKind
  isMoving: boolean
  hasKeyboardInput: boolean
  sendPlayerMove: SendPlayerMove
  actions: MoveRequestActions
}

export function runMoveRequest({
  clickPosition,
  initialSpeed,
  pickupAfterArrival,
  currentPlayer,
  interactionExit,
  isMoving,
  hasKeyboardInput,
  currentFloor,
  getFloorAt,
  findPath,
  waypointHeight,
  sendPlayerMove,
  actions,
}: RunMoveRequestInput) {
  const decision = decideMoveRequest({
    pickupAfterArrival,
    currentPlayerHealth: currentPlayer?.health ?? null,
    interactionExit,
    hasCurrentPlayer: currentPlayer !== null,
    isMoving,
    hasKeyboardInput,
  })

  if (decision.clearPendingPickupAfterMove) {
    actions.clearPendingPickupAfterMove()
  }

  switch (decision.kind) {
    case 'ignored':
      return
    case 'exit_pickup_and_retry':
      actions.exitPickupAndRetry()
      return
    case 'exit_object_and_delay':
      actions.exitObjectAndDelay()
      return
    case 'start':
      break
  }

  if (!currentPlayer) return

  actions.applyStartedMovement(
    startClickMovement({
      currentPos: {
        x: currentPlayer.position.x,
        y: currentPlayer.position.y,
        z: currentPlayer.position.z,
      },
      clickPosition,
      initialSpeed,
      pickupAfterArrival,
      currentFloor,
      getFloorAt,
      findPath,
      waypointHeight,
      sendPlayerMove,
    })
  )
}
