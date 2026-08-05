// Common movement calculation utilities shared between local and remote players

import { shortestWrappedDeltaX, unwrapWorldXNear } from '../terrain/world-wrap'

export type MovementMode = 'walk' | 'jog' | 'run'

export interface Position {
  x: number
  y: number
  z: number
}

export interface MovementConfig {
  maxSpeed: number
  acceleration: number
  deceleration: number
  arrivalThreshold: number
}

export interface MovementState {
  currentSpeed: number
  startPos: Position
  targetPos: Position
  totalDistance: number
}

export interface MovementResult {
  newPos: Position
  newSpeed: number
  rotation: number
  arrived: boolean
}

export type PlayerStateName =
  | 'idle'
  | 'moving'
  | 'attack'
  | 'dead'
  | 'interact'
  | 'jump'

export interface PlayerState {
  position: Position
  state: PlayerStateName
  speed: number
  rotation: number
  movementMode?: MovementMode
  attackCounter?: number
  interactionAnim?: string
  interactOffsetY?: number
}

/**
 * Maximum traversable slope in degrees. Anything steeper than this from the
 * player's current position is treated as un-climbable; the movement system
 * blocks the step and triggers a jump animation as feedback.
 */
export const MAX_TRAVERSABLE_SLOPE_DEG = 50

/** Horizontal lookahead distance (meters) used to estimate the slope ahead
 *  of the player. Small enough to be sensitive, large enough to avoid
 *  per-frame noise from heightmap bilinear interpolation. */
export const SLOPE_LOOKAHEAD_DISTANCE = 0.5

const MAX_TRAVERSABLE_SLOPE_TAN = Math.tan(
  (MAX_TRAVERSABLE_SLOPE_DEG * Math.PI) / 180
)

/**
 * Returns true if the upward slope from `currentY` to `forwardY` over
 * `lookaheadDistance` exceeds MAX_TRAVERSABLE_SLOPE_DEG.
 *
 * Only blocks UPHILL motion — descending steep terrain is allowed so the
 * player isn't trapped if they wander to a cliff edge.
 */
export function isSlopeTooSteepUphill(
  currentY: number,
  forwardY: number,
  lookaheadDistance: number = SLOPE_LOOKAHEAD_DISTANCE
): boolean {
  if (lookaheadDistance < 1e-6) return false
  const dy = forwardY - currentY
  if (dy <= 0) return false
  return dy / lookaheadDistance > MAX_TRAVERSABLE_SLOPE_TAN
}

/**
 * Movement-mode policy: sprint runs, torch walks, combat chase runs,
 * otherwise short distances walk and the rest (unknown distance too) jog.
 */
export function getMovementMode(
  distance?: number,
  hasTorch = false,
  sprinting = false,
  inCombat = false
): MovementMode {
  if (sprinting) {
    return 'run'
  }
  if (hasTorch) {
    return 'walk'
  }
  if (inCombat) {
    return 'run'
  }
  if (distance !== undefined && distance <= 3) {
    return 'walk'
  }
  return 'jog'
}

// Default movement configuration
export const DEFAULT_MOVEMENT_CONFIG: MovementConfig = {
  maxSpeed: 3, // units per second
  acceleration: 6, // units per second squared
  deceleration: 6, // units per second squared
  arrivalThreshold: 0.05,
}

// Mirrors shared/src/hunger.rs SPRINT_MOVE_MULT.
export const SPRINT_SPEED_MULT = 1.5

export function scaleMovementConfig(
  config: MovementConfig,
  mult: number
): MovementConfig {
  return {
    ...config,
    maxSpeed: config.maxSpeed * mult,
    acceleration: config.acceleration * mult,
    deceleration: config.deceleration * mult,
  }
}

// Calculate acceleration and deceleration distances based on config
export function getAccelDistance(config: MovementConfig): number {
  return (config.maxSpeed * config.maxSpeed) / (2 * config.acceleration)
}

export function getDecelDistance(config: MovementConfig): number {
  return (config.maxSpeed * config.maxSpeed) / (2 * config.deceleration)
}

/**
 * Calculate the next movement step with acceleration/deceleration
 *
 * @param currentPos - Current position
 * @param movement - Movement state (start, target, total distance, current speed)
 * @param config - Movement configuration
 * @param deltaTimeSeconds - Time delta in seconds
 * @returns Movement result with new position, speed, rotation, and arrival status
 */
export function calculateMovementStep(
  currentPos: Position,
  movement: MovementState,
  config: MovementConfig,
  deltaTimeSeconds: number
): MovementResult {
  const { targetPos, totalDistance } = movement
  const accelDistance = getAccelDistance(config)
  const decelDistance = getDecelDistance(config)

  // Calculate remaining distance on XZ plane (Y is handled by terrain)
  const dx = shortestWrappedDeltaX(currentPos.x, targetPos.x)
  const dz = targetPos.z - currentPos.z
  const remainingDistance = Math.sqrt(dx * dx + dz * dz)

  // Check if arrived
  if (remainingDistance <= config.arrivalThreshold) {
    return {
      newPos: {
        x: unwrapWorldXNear(currentPos.x, targetPos.x),
        y: currentPos.y,
        z: targetPos.z,
      },
      newSpeed: 0,
      rotation: Math.atan2(dx, dz),
      arrived: true,
    }
  }

  // Calculate traveled distance
  const traveledDistance = totalDistance - remainingDistance

  // Determine speed based on phase (acceleration, cruise, deceleration)
  let newSpeed = movement.currentSpeed
  if (traveledDistance < accelDistance) {
    // Acceleration phase
    newSpeed = Math.min(
      newSpeed + config.acceleration * deltaTimeSeconds,
      config.maxSpeed
    )
  } else if (remainingDistance > decelDistance) {
    // Cruise phase
    newSpeed = config.maxSpeed
  } else {
    // Deceleration phase
    newSpeed = Math.max(newSpeed - config.deceleration * deltaTimeSeconds, 0)
  }

  // Calculate rotation (direction of movement)
  const rotation = Math.atan2(dx, dz)

  // Move at current speed
  const moveDistance = newSpeed * deltaTimeSeconds

  let newPos: Position
  if (moveDistance >= remainingDistance || newSpeed <= 0.001) {
    // Arrived at destination
    newPos = {
      x: unwrapWorldXNear(currentPos.x, targetPos.x),
      y: currentPos.y,
      z: targetPos.z,
    }
    return {
      newPos,
      newSpeed: 0,
      rotation,
      arrived: true,
    }
  } else {
    // Continue moving on XZ plane
    const dirX = dx / remainingDistance
    const dirZ = dz / remainingDistance
    newPos = {
      x: currentPos.x + dirX * moveDistance,
      y: currentPos.y,
      z: currentPos.z + dirZ * moveDistance,
    }
  }

  return {
    newPos,
    newSpeed,
    rotation,
    arrived: false,
  }
}

/**
 * Initialize movement state from current position to target
 *
 * @param currentPos - Current position
 * @param targetPos - Target position
 * @param currentSpeed - Current movement speed (default 0)
 * @returns Movement state
 */
export function initMovementState(
  currentPos: Position,
  targetPos: Position,
  currentSpeed: number = 0
): MovementState {
  const dx = shortestWrappedDeltaX(currentPos.x, targetPos.x)
  const dz = targetPos.z - currentPos.z
  const totalDistance = Math.sqrt(dx * dx + dz * dz)

  return {
    currentSpeed,
    startPos: { ...currentPos },
    targetPos: { ...targetPos },
    totalDistance,
  }
}

/**
 * Check if target position has changed
 */
export function hasTargetChanged(
  movement: MovementState | undefined,
  newTarget: Position
): boolean {
  if (!movement) return true
  return (
    movement.targetPos.x !== newTarget.x ||
    movement.targetPos.y !== newTarget.y ||
    movement.targetPos.z !== newTarget.z
  )
}
