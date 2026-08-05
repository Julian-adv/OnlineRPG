import {
  getMovementMode,
  type PlayerState,
  type Position,
} from '../../../utils/movementUtils'

export interface PlayerStateProjectionInput {
  currentPosition: Position
  isMoving: boolean
  currentSpeed: number
  playerRotation: number
  totalDistance?: number
  hasTorch: boolean
  isInCombat: boolean
  attackCounter: number
  isSprinting: boolean
}

export function projectPlayerState({
  currentPosition,
  isMoving,
  currentSpeed,
  playerRotation,
  totalDistance,
  hasTorch,
  isInCombat,
  attackCounter,
  isSprinting,
}: PlayerStateProjectionInput): PlayerState {
  const movementMode = isMoving
    ? getMovementMode(totalDistance, hasTorch, isSprinting, isInCombat)
    : undefined

  return {
    state: isMoving ? 'moving' : 'idle',
    speed: currentSpeed,
    rotation: playerRotation,
    position: currentPosition,
    movementMode,
    attackCounter: isInCombat ? attackCounter : undefined,
  }
}

export function shouldEmitProjectedPlayerState(
  previousState: PlayerState,
  nextState: PlayerState
): boolean {
  return (
    nextState.state !== previousState.state ||
    Math.abs(nextState.speed - previousState.speed) > 0.01 ||
    nextState.rotation !== previousState.rotation ||
    Math.abs(nextState.position.x - previousState.position.x) > 0.01 ||
    Math.abs(nextState.position.z - previousState.position.z) > 0.01 ||
    nextState.movementMode !== previousState.movementMode ||
    nextState.attackCounter !== previousState.attackCounter
  )
}
