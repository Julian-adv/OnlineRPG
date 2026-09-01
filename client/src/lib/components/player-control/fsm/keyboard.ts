import type { MovementConfig, Position } from '../../../utils/movementUtils'
import { shortestWrappedDeltaX } from '../../../terrain/world-wrap'
import type { InteractionExitKind } from './interaction'
import type { SendPlayerMove } from './movement-substrate'

// ───────────────────────────────────────────────────────────────────────────
// Throttled move sender (server needs ~5Hz waypoints, not 3cm per-frame steps)
// ───────────────────────────────────────────────────────────────────────────

/** Distance between network samples; ≈7 sends/s at walk speed, matching the
 *  server's 5Hz movement tick. */
const KEYBOARD_SEND_INTERVAL = 0.5

export interface KeyboardMoveSender {
  /** Per-frame position after a successful step. Sends a replace on the first
   *  step of a session, then appends a path sample every send interval. */
  step(position: Position, rotation: number): void
  /** Send the resting position once when the session ends (keys released or
   *  step blocked) so the server converges on the exact stop point. */
  flush(position: Position): void
  /** Drop the session without sending (another mover owns the queue). */
  reset(): void
}

export function createKeyboardMoveSender(
  send: SendPlayerMove
): KeyboardMoveSender {
  let lastSent: Position | null = null
  let lastRotation = 0
  return {
    step(position, rotation) {
      lastRotation = rotation
      if (lastSent === null) {
        send(position, rotation)
        lastSent = { ...position }
        return
      }
      const dx = shortestWrappedDeltaX(lastSent.x, position.x)
      const dz = position.z - lastSent.z
      if (
        dx * dx + dz * dz >=
        KEYBOARD_SEND_INTERVAL * KEYBOARD_SEND_INTERVAL
      ) {
        send(position, rotation, undefined, true)
        lastSent = { ...position }
      }
    },
    flush(position) {
      if (lastSent === null) return
      if (position.x !== lastSent.x || position.z !== lastSent.z) {
        send(position, lastRotation, undefined, true)
      }
      lastSent = null
    },
    reset() {
      lastSent = null
    },
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Tap-step tracker (a short tap completes one clean walking step)
// ───────────────────────────────────────────────────────────────────────────

/** Total distance of one tap step; a session shorter than this glides the
 *  rest of the way after release. */
export const KEYBOARD_TAP_STEP = 0.5

export interface KeyboardTapTracker {
  /** Feed every pressed frame to record the session start and direction. */
  track(position: Position, direction: KeyboardDirection | null): void
  /** End the session. Returns a glide target (XZ) when the session moved but
   *  stayed under the step distance; null otherwise. */
  release(position: Position | null): { x: number; z: number } | null
}

export function createKeyboardTapTracker(): KeyboardTapTracker {
  let start: Position | null = null
  let lastDir: KeyboardDirection | null = null
  return {
    track(position, direction) {
      if (start === null) start = { ...position }
      if (direction) lastDir = direction
    },
    release(position) {
      const s = start
      const d = lastDir
      start = null
      lastDir = null
      if (!s || !d || !position) return null
      const dx = shortestWrappedDeltaX(s.x, position.x)
      const dz = position.z - s.z
      const moved = Math.hypot(dx, dz)
      // No actual step (blocked at a wall, interaction-exit tap): don't
      // conjure movement the player never started.
      if (moved <= 1e-3) return null
      const remaining = KEYBOARD_TAP_STEP - moved
      if (remaining <= 0.01) return null
      const len = Math.hypot(d.x, d.z) || 1
      return {
        x: position.x + (d.x / len) * remaining,
        z: position.z + (d.z / len) * remaining,
      }
    },
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Keyboard movement integrator (delta-time step with accel ramp, no waypoints)
// ───────────────────────────────────────────────────────────────────────────

export interface KeyboardDirection {
  x: number
  z: number
}

// Session-scoped speed ramp so keyboard starts match the click-move
// acceleration curve instead of snapping to full speed.
export interface KeyboardSpeedRamp {
  advance(config: MovementConfig, deltaTimeSeconds: number): number
  reset(): void
}

export function createKeyboardSpeedRamp(): KeyboardSpeedRamp {
  let speed = 0
  return {
    advance(config, deltaTimeSeconds) {
      speed = Math.min(
        speed + config.acceleration * deltaTimeSeconds,
        config.maxSpeed
      )
      return speed
    },
    reset() {
      speed = 0
    },
  }
}

interface KeyboardMovementInput {
  currentPos: Position
  direction: KeyboardDirection
  config: MovementConfig
  deltaTimeSeconds: number
  speedRamp: KeyboardSpeedRamp
  sampleHeight: (x: number, z: number) => number
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
  writePlayerPosition: (position: Position, rotation: number) => void
  sendPlayerMove: (position: Position, rotation: number) => void
}

export type KeyboardMovementOutcome =
  | { kind: 'blocked' }
  | { kind: 'slope_blocked' }
  | {
      kind: 'moved'
      currentSpeed: number
      playerRotation: number
    }

export function applyKeyboardMovement({
  currentPos,
  direction,
  config,
  deltaTimeSeconds,
  speedRamp,
  sampleHeight,
  isMovementBlocked,
  isUphillTooSteep,
  writePlayerPosition,
  sendPlayerMove,
}: KeyboardMovementInput): KeyboardMovementOutcome {
  // Clamp tab-switch delta spikes so one frame can't teleport the player.
  const dt = Math.min(deltaTimeSeconds, 0.1)
  const currentSpeed = speedRamp.advance(config, dt)
  const speed = currentSpeed * dt
  const newX = currentPos.x + direction.x * speed
  const newZ = currentPos.z + direction.z * speed

  if (isMovementBlocked(currentPos.x, currentPos.z, newX, newZ, currentPos.y)) {
    return { kind: 'blocked' }
  }

  if (
    isUphillTooSteep(
      currentPos.x,
      currentPos.z,
      currentPos.y,
      direction.x,
      direction.z
    )
  ) {
    return { kind: 'slope_blocked' }
  }

  const groundY = sampleHeight(newX, newZ)
  const playerRotation = Math.atan2(direction.x, direction.z)
  const position = { x: newX, y: groundY, z: newZ }

  writePlayerPosition(position, playerRotation)
  sendPlayerMove(position, playerRotation)

  return {
    kind: 'moved',
    currentSpeed,
    playerRotation,
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Keyboard movement outcome application
// ───────────────────────────────────────────────────────────────────────────

export interface KeyboardMovementOutcomeActions {
  stopMovement: () => void
  triggerJumpFeedback: () => void
  setMoved: (currentSpeed: number, playerRotation: number) => void
}

export type KeyboardMovementOutcomeApplication =
  | { kind: 'handled' }
  | { kind: 'moved' }

export function applyKeyboardMovementOutcome(
  outcome: KeyboardMovementOutcome,
  actions: KeyboardMovementOutcomeActions
): KeyboardMovementOutcomeApplication {
  switch (outcome.kind) {
    case 'blocked':
      actions.stopMovement()
      return { kind: 'handled' }

    case 'slope_blocked':
      actions.stopMovement()
      actions.triggerJumpFeedback()
      return { kind: 'handled' }

    case 'moved':
      actions.setMoved(outcome.currentSpeed, outcome.playerRotation)
      return { kind: 'moved' }

    default: {
      const _exhaustive: never = outcome
      return _exhaustive
    }
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Keyboard frame (per-frame WASD step with click-move / combat preemption)
// ───────────────────────────────────────────────────────────────────────────

interface KeyboardFramePlayer {
  position: Position
}

export interface KeyboardFrameActions extends KeyboardMovementOutcomeActions {
  exitPickupInteraction: () => void
  exitObjectInteraction: () => void
  clearClickMovement: () => void
  cancelCombat: () => void
  markMoving: () => void
  setKeyboardIdleRuntime: () => void
  emitKeyboardPlayerState: () => void
  /** Start a click-move to `target` to finish a tap step. */
  requestMove: (target: { x: number; z: number }) => void
}

interface RunKeyboardFrameInput {
  currentPlayer: KeyboardFramePlayer | null
  hasKeysPressed: boolean
  isKeyboardMoving: boolean
  interactionExit: InteractionExitKind
  hasMovementTarget: boolean
  isInCombat: boolean
  direction: KeyboardDirection | null
  config: MovementConfig
  deltaTimeSeconds: number
  sampleHeight: (x: number, z: number) => number
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
  writePlayerPosition: (position: Position, rotation: number) => void
  moveSender: KeyboardMoveSender
  tapTracker: KeyboardTapTracker
  speedRamp: KeyboardSpeedRamp
  actions: KeyboardFrameActions
}

export function runKeyboardFrame({
  currentPlayer,
  hasKeysPressed,
  isKeyboardMoving,
  interactionExit,
  hasMovementTarget,
  isInCombat,
  direction,
  config,
  deltaTimeSeconds,
  sampleHeight,
  isMovementBlocked,
  isUphillTooSteep,
  writePlayerPosition,
  moveSender,
  tapTracker,
  speedRamp,
  actions,
}: RunKeyboardFrameInput) {
  if (!currentPlayer || !hasKeysPressed) {
    const tapTarget = tapTracker.release(currentPlayer?.position ?? null)
    speedRamp.reset()
    // Session over: a click-path or combat chase owns the movement queue now
    // (their replace supersedes us), so hand off without sending.
    if (!currentPlayer || hasMovementTarget || isInCombat) {
      moveSender.reset()
      return
    }
    if (tapTarget) {
      // A short tap finishes one clean step via the click-move pipeline; its
      // replace send supersedes any pending sample.
      moveSender.reset()
      actions.requestMove(tapTarget)
    } else {
      moveSender.flush(currentPlayer.position)
      // Long-hold release has no glide arrival to settle the machine, so it
      // must leave keyboard_moving itself or the run animation loops forever.
      if (isKeyboardMoving) {
        actions.setKeyboardIdleRuntime()
        actions.emitKeyboardPlayerState()
      }
    }
    return
  }

  tapTracker.track(currentPlayer.position, direction)

  if (interactionExit !== 'none') {
    if (interactionExit === 'pickup') {
      actions.exitPickupInteraction()
    } else {
      actions.exitObjectInteraction()
    }
  }

  if (hasMovementTarget) {
    actions.clearClickMovement()
    actions.cancelCombat()
  }

  if (isInCombat) {
    actions.cancelCombat()
  }

  if (direction) {
    const outcome = applyKeyboardMovement({
      currentPos: {
        x: currentPlayer.position.x,
        y: currentPlayer.position.y,
        z: currentPlayer.position.z,
      },
      direction,
      config,
      deltaTimeSeconds,
      speedRamp,
      sampleHeight,
      isMovementBlocked,
      isUphillTooSteep,
      writePlayerPosition: (position, rotation) => {
        writePlayerPosition(position, rotation)
        actions.markMoving()
      },
      sendPlayerMove: moveSender.step,
    })

    const keyboardApplication = applyKeyboardMovementOutcome(outcome, actions)
    if (keyboardApplication.kind === 'handled') {
      // Blocked against a wall or slope: sync the stop point so the server
      // doesn't keep walking to a stale sample. Reset the ramp too, or holding
      // into an obstacle charges it to full and the next clear step launches.
      speedRamp.reset()
      moveSender.flush(currentPlayer.position)
      return
    }
  } else {
    speedRamp.reset()
    actions.setKeyboardIdleRuntime()
  }

  actions.emitKeyboardPlayerState()
}
