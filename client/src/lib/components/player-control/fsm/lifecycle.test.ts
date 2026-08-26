import { describe, expect, it } from 'vitest'
import type { PlayerState } from '../../../utils/movementUtils'
import {
  beginJumpFeedback,
  resetMovementRuntimeState,
  resetRespawnRuntimeState,
  shouldFinishJumpFeedback,
  transitionToDeadState,
  transitionToRespawnedState,
} from './lifecycle'

describe('transition reset helpers', () => {
  it('resets movement runtime state', () => {
    expect(resetMovementRuntimeState()).toEqual({
      isMoving: false,
      movementTarget: null,
      movementState: null,
      currentSpeed: 0,
      pathWaypoints: [],
      currentWaypointIndex: 0,
    })
  })

  it('resets respawn runtime state including rotation', () => {
    expect(resetRespawnRuntimeState()).toEqual({
      isMoving: false,
      movementTarget: null,
      movementState: null,
      currentSpeed: 0,
      pathWaypoints: [],
      currentWaypointIndex: 0,
      playerRotation: 0,
    })
  })
})

const idleState: PlayerState = {
  state: 'idle',
  speed: 0,
  rotation: 0,
  position: { x: 1, y: 2, z: 3 },
}

describe('beginJumpFeedback', () => {
  it('blocks feedback while the cooldown is active', () => {
    expect(
      beginJumpFeedback({
        previousPlayerState: idleState,
        now: 1200,
        lastJumpFeedbackAt: 1000,
        cooldownMs: 1000,
      })
    ).toEqual({
      kind: 'cooldown',
      runtime: { lastJumpFeedbackAt: 1000 },
    })
  })

  it('starts jump feedback and updates the cooldown timestamp', () => {
    const transition = beginJumpFeedback({
      previousPlayerState: idleState,
      now: 2500,
      lastJumpFeedbackAt: 1000,
      cooldownMs: 1000,
    })

    expect(transition.kind).toBe('started')
    if (transition.kind !== 'started') return
    expect(transition.runtime.lastJumpFeedbackAt).toBe(2500)
    expect(transition.nextPlayerState.state).toBe('jump')
    expect(transition.nextPlayerState.position).toEqual(idleState.position)
  })
})

describe('shouldFinishJumpFeedback', () => {
  it('only finishes when the player is still in jump feedback state', () => {
    expect(shouldFinishJumpFeedback({ ...idleState, state: 'jump' })).toBe(true)
    expect(shouldFinishJumpFeedback(idleState)).toBe(false)
  })
})

const movingState: PlayerState = {
  state: 'moving',
  speed: 3,
  rotation: 1,
  position: { x: 1, y: 2, z: 3 },
  movementMode: 'run',
  attackCounter: 4,
}

describe('transitionToDeadState', () => {
  it('ignores repeated dead transitions', () => {
    expect(transitionToDeadState({ ...movingState, state: 'dead' })).toEqual({
      kind: 'ignored_already_dead',
    })
  })

  it('returns dead player state and reset runtime', () => {
    const result = transitionToDeadState(movingState)

    expect(result.kind).toBe('dead')
    if (result.kind !== 'dead') return
    expect(result.runtime.isMoving).toBe(false)
    expect(result.runtime.movementTarget).toBeNull()
    expect(result.runtime.currentSpeed).toBe(0)
    expect(result.nextPlayerState).toEqual({
      ...movingState,
      state: 'dead',
      speed: 0,
      movementMode: undefined,
    })
  })
})

describe('transitionToRespawnedState', () => {
  it('returns idle respawn state and reset runtime', () => {
    const position = { x: 10, y: 0, z: 20 }

    const result = transitionToRespawnedState(movingState, position)

    expect(result.runtime.isMoving).toBe(false)
    expect(result.runtime.playerRotation).toBe(0)
    expect(result.nextPlayerState).toEqual({
      ...movingState,
      state: 'idle',
      speed: 0,
      rotation: 0,
      movementMode: undefined,
      attackCounter: 0,
      position,
    })
  })
})
