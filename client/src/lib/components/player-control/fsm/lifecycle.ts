import type {
  MovementState,
  PlayerState,
  Position,
} from '../../../utils/movementUtils'
import {
  buildDeadState,
  buildJumpState,
  buildRespawnedState,
} from '../player-state-builders'
import type { PathWaypoint } from './movement-substrate'

// ───────────────────────────────────────────────────────────────────────────
// Runtime state reset helpers
// ───────────────────────────────────────────────────────────────────────────

export interface ControlRuntimeState {
  isMoving: boolean
  movementTarget: Position | null
  movementState: MovementState | null
  currentSpeed: number
  pathWaypoints: PathWaypoint[]
  currentWaypointIndex: number
}

export function resetMovementRuntimeState(): ControlRuntimeState {
  return {
    isMoving: false,
    movementTarget: null,
    movementState: null,
    currentSpeed: 0,
    pathWaypoints: [],
    currentWaypointIndex: 0,
  }
}

export function resetRespawnRuntimeState(): ControlRuntimeState & {
  playerRotation: number
} {
  return {
    ...resetMovementRuntimeState(),
    playerRotation: 0,
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Jump feedback (transient steep-slope jump animation with cooldown)
// ───────────────────────────────────────────────────────────────────────────

export interface JumpFeedbackRuntime {
  lastJumpFeedbackAt: number
}

export type JumpFeedbackTransition =
  | { kind: 'cooldown'; runtime: JumpFeedbackRuntime }
  | {
      kind: 'started'
      runtime: JumpFeedbackRuntime
      nextPlayerState: PlayerState
    }

interface BeginJumpFeedbackInput {
  previousPlayerState: PlayerState
  now: number
  lastJumpFeedbackAt: number
  cooldownMs: number
}

export function beginJumpFeedback({
  previousPlayerState,
  now,
  lastJumpFeedbackAt,
  cooldownMs,
}: BeginJumpFeedbackInput): JumpFeedbackTransition {
  if (now - lastJumpFeedbackAt < cooldownMs) {
    return {
      kind: 'cooldown',
      runtime: { lastJumpFeedbackAt },
    }
  }

  return {
    kind: 'started',
    runtime: { lastJumpFeedbackAt: now },
    nextPlayerState: buildJumpState(previousPlayerState),
  }
}

export function shouldFinishJumpFeedback(playerState: PlayerState): boolean {
  return playerState.state === 'jump'
}

// ───────────────────────────────────────────────────────────────────────────
// Death / respawn transitions
// ───────────────────────────────────────────────────────────────────────────

export type DeadTransitionOutcome =
  | { kind: 'ignored_already_dead' }
  | {
      kind: 'dead'
      runtime: ControlRuntimeState
      nextPlayerState: PlayerState
    }

export function transitionToDeadState(
  previousPlayerState: PlayerState
): DeadTransitionOutcome {
  if (previousPlayerState.state === 'dead') {
    return { kind: 'ignored_already_dead' }
  }

  return {
    kind: 'dead',
    runtime: resetMovementRuntimeState(),
    nextPlayerState: buildDeadState(previousPlayerState),
  }
}

export function transitionToRespawnedState(
  previousPlayerState: PlayerState,
  position: Position
): {
  runtime: ControlRuntimeState & { playerRotation: number }
  nextPlayerState: PlayerState
} {
  const runtime = resetRespawnRuntimeState()
  return {
    runtime,
    nextPlayerState: buildRespawnedState(
      previousPlayerState,
      position,
      runtime.playerRotation
    ),
  }
}
