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
  | { kind: 'ignored' }
  | { kind: 'exit_pickup_and_retry' }
  | { kind: 'exit_object_and_delay' }
  | { kind: 'start' }

interface DecideMoveRequestInput {
  currentPlayerHealth: number | null
  interactionExit: InteractionExitKind
  hasCurrentPlayer: boolean
  isMoving: boolean
  hasKeyboardInput: boolean
}

export function decideMoveRequest({
  currentPlayerHealth,
  interactionExit,
  hasCurrentPlayer,
  isMoving,
  hasKeyboardInput,
}: DecideMoveRequestInput): MoveRequestDecision {
  if (currentPlayerHealth !== null && currentPlayerHealth <= 0) {
    return { kind: 'ignored' }
  }

  if (interactionExit === 'pickup') return { kind: 'exit_pickup_and_retry' }
  if (interactionExit === 'object') return { kind: 'exit_object_and_delay' }

  if (!hasCurrentPlayer || isMoving || hasKeyboardInput) {
    if (hasCurrentPlayer && isMoving && !hasKeyboardInput) {
      return { kind: 'start' }
    }
    return { kind: 'ignored' }
  }

  return { kind: 'start' }
}

// ───────────────────────────────────────────────────────────────────────────
// Path-based click movement initialization
// ───────────────────────────────────────────────────────────────────────────

interface StartClickMovementInput extends Pathing {
  currentPos: Position
  clickPosition: Position
  sendPlayerMove: SendPlayerMove
  /** Carry the current speed so a mid-run redirect doesn't restart at 0. */
  startSpeed: number
}

export interface StartedClickMovement {
  pathWaypoints: PathWaypoint[]
  currentWaypointIndex: number
  movementState: MovementState
  movementTarget: Position
  playerRotation: number
}

export function startClickMovement({
  currentPos,
  clickPosition,
  sendPlayerMove,
  startSpeed,
  ...pathing
}: StartClickMovementInput): StartedClickMovement {
  const leg = routeFirstLeg(currentPos, clickPosition, pathing, sendPlayerMove)
  return {
    ...leg,
    currentWaypointIndex: 0,
    movementState: initMovementState(
      currentPos,
      leg.movementTarget,
      startSpeed
    ),
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
  exitPickupAndRetry: () => void
  exitObjectAndDelay: () => void
  applyStartedMovement: (started: StartedClickMovement) => void
}

interface RunMoveRequestInput extends Pathing {
  clickPosition: Position
  currentPlayer: MoveRequestPlayer | null
  interactionExit: InteractionExitKind
  isMoving: boolean
  hasKeyboardInput: boolean
  sendPlayerMove: SendPlayerMove
  startSpeed: number
  actions: MoveRequestActions
}

export function runMoveRequest({
  clickPosition,
  currentPlayer,
  interactionExit,
  isMoving,
  hasKeyboardInput,
  currentFloor,
  getFloorAt,
  findPath,
  waypointHeight,
  sendPlayerMove,
  startSpeed,
  actions,
}: RunMoveRequestInput) {
  const decision = decideMoveRequest({
    currentPlayerHealth: currentPlayer?.health ?? null,
    interactionExit,
    hasCurrentPlayer: currentPlayer !== null,
    isMoving,
    hasKeyboardInput,
  })

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
      currentFloor,
      getFloorAt,
      findPath,
      waypointHeight,
      sendPlayerMove,
      startSpeed,
    })
  )
}
