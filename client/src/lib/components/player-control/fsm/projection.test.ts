import { describe, expect, it } from 'vitest'
import type { PlayerState } from '../../../utils/movementUtils'
import {
  projectPlayerState,
  shouldEmitProjectedPlayerState,
} from './projection'

const idleState: PlayerState = {
  state: 'idle',
  speed: 0,
  rotation: 0,
  position: { x: 0, y: 0, z: 0 },
}

describe('projectPlayerState', () => {
  it('projects click movement mode from movement distance', () => {
    const state = projectPlayerState({
      currentPosition: { x: 1, y: 2, z: 3 },
      isMoving: true,
      currentSpeed: 2.5,
      playerRotation: 0.25,
      totalDistance: 4,
      hasTorch: false,
      isInCombat: false,
      attackCounter: 0,
      isSprinting: false,
    })

    expect(state).toEqual({
      state: 'moving',
      speed: 2.5,
      rotation: 0.25,
      position: { x: 1, y: 2, z: 3 },
      movementMode: 'jog',
      attackCounter: undefined,
    })
  })

  it('projects combat movement as run with attack counter', () => {
    const state = projectPlayerState({
      currentPosition: { x: 0, y: 0, z: 0 },
      isMoving: true,
      currentSpeed: 3,
      playerRotation: 1,
      hasTorch: false,
      isInCombat: true,
      attackCounter: 7,
      isSprinting: false,
    })

    expect(state.movementMode).toBe('run')
    expect(state.attackCounter).toBe(7)
  })

  it('projects sprinting as run', () => {
    const state = projectPlayerState({
      currentPosition: { x: 0, y: 0, z: 0 },
      isMoving: true,
      currentSpeed: 4.5,
      playerRotation: 0,
      totalDistance: 20,
      hasTorch: false,
      isInCombat: false,
      attackCounter: 0,
      isSprinting: true,
    })

    expect(state.movementMode).toBe('run')
  })
})

describe('shouldEmitProjectedPlayerState', () => {
  it('ignores y-only projection changes', () => {
    const next: PlayerState = {
      ...idleState,
      position: { x: 0, y: 10, z: 0 },
    }

    expect(shouldEmitProjectedPlayerState(idleState, next)).toBe(false)
  })

  it('emits when movement mode changes', () => {
    const next: PlayerState = {
      ...idleState,
      movementMode: 'walk',
    }

    expect(shouldEmitProjectedPlayerState(idleState, next)).toBe(true)
  })
})
