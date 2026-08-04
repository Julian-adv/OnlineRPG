import { describe, it, expect } from 'vitest'
import {
  getMovementMode,
  getAccelDistance,
  getDecelDistance,
  calculateMovementStep,
  initMovementState,
  hasTargetChanged,
  DEFAULT_MOVEMENT_CONFIG,
  type MovementConfig,
  type Position,
} from './movementUtils'
import { WORLD_MAX_X, WORLD_MIN_X } from '../terrain/world-wrap'

describe('getMovementMode', () => {
  it('returns walk for short distance without torch', () => {
    expect(getMovementMode(0)).toBe('walk')
    expect(getMovementMode(1.5)).toBe('walk')
    expect(getMovementMode(3)).toBe('walk')
  })

  it('returns jog for every non-short distance without torch', () => {
    expect(getMovementMode(3.01)).toBe('jog')
    expect(getMovementMode(5)).toBe('jog')
    expect(getMovementMode(8)).toBe('jog')
    expect(getMovementMode(8.01)).toBe('jog')
    expect(getMovementMode(100)).toBe('jog')
  })

  it('skips jog when carrying a torch', () => {
    expect(getMovementMode(3, true)).toBe('walk')
    expect(getMovementMode(5, true)).toBe('run')
    expect(getMovementMode(8, true)).toBe('run')
    expect(getMovementMode(100, true)).toBe('run')
  })
})

describe('getAccelDistance / getDecelDistance', () => {
  it('matches kinematic formula v^2 / (2a) for defaults', () => {
    // maxSpeed=3, accel=6 → 9 / 12 = 0.75
    expect(getAccelDistance(DEFAULT_MOVEMENT_CONFIG)).toBeCloseTo(0.75)
    expect(getDecelDistance(DEFAULT_MOVEMENT_CONFIG)).toBeCloseTo(0.75)
  })

  it('scales with maxSpeed squared', () => {
    const cfg: MovementConfig = {
      maxSpeed: 6,
      acceleration: 6,
      deceleration: 6,
      arrivalThreshold: 0.05,
    }
    // 36 / 12 = 3
    expect(getAccelDistance(cfg)).toBeCloseTo(3)
  })

  it('uses separate accel/decel values', () => {
    const cfg: MovementConfig = {
      maxSpeed: 4,
      acceleration: 2,
      deceleration: 8,
      arrivalThreshold: 0.05,
    }
    expect(getAccelDistance(cfg)).toBeCloseTo(16 / 4) // 4
    expect(getDecelDistance(cfg)).toBeCloseTo(16 / 16) // 1
  })
})

describe('initMovementState', () => {
  it('computes total XZ distance ignoring Y', () => {
    const state = initMovementState(
      { x: 0, y: 100, z: 0 },
      { x: 3, y: 200, z: 4 }
    )
    expect(state.totalDistance).toBeCloseTo(5)
  })

  it('defaults currentSpeed to 0', () => {
    const state = initMovementState({ x: 0, y: 0, z: 0 }, { x: 1, y: 0, z: 0 })
    expect(state.currentSpeed).toBe(0)
  })

  it('preserves provided currentSpeed', () => {
    const state = initMovementState(
      { x: 0, y: 0, z: 0 },
      { x: 1, y: 0, z: 0 },
      2.5
    )
    expect(state.currentSpeed).toBe(2.5)
  })

  it('copies start and target (no aliasing)', () => {
    const start: Position = { x: 1, y: 2, z: 3 }
    const target: Position = { x: 4, y: 5, z: 6 }
    const state = initMovementState(start, target)
    start.x = 999
    target.x = 999
    expect(state.startPos.x).toBe(1)
    expect(state.targetPos.x).toBe(4)
  })

  it('measures the short distance across the X seam', () => {
    const state = initMovementState(
      { x: WORLD_MAX_X - 1, y: 0, z: 0 },
      { x: WORLD_MIN_X + 1, y: 0, z: 0 }
    )
    expect(state.totalDistance).toBe(2)
  })
})

describe('hasTargetChanged', () => {
  it('returns true when movement is undefined', () => {
    expect(hasTargetChanged(undefined, { x: 0, y: 0, z: 0 })).toBe(true)
  })

  it('returns false when target matches exactly', () => {
    const state = initMovementState({ x: 0, y: 0, z: 0 }, { x: 1, y: 2, z: 3 })
    expect(hasTargetChanged(state, { x: 1, y: 2, z: 3 })).toBe(false)
  })

  it('returns true when any axis differs', () => {
    const state = initMovementState({ x: 0, y: 0, z: 0 }, { x: 1, y: 2, z: 3 })
    expect(hasTargetChanged(state, { x: 1.0001, y: 2, z: 3 })).toBe(true)
    expect(hasTargetChanged(state, { x: 1, y: 2.0001, z: 3 })).toBe(true)
    expect(hasTargetChanged(state, { x: 1, y: 2, z: 3.0001 })).toBe(true)
  })
})

describe('calculateMovementStep', () => {
  const cfg = DEFAULT_MOVEMENT_CONFIG

  it('snaps to target and returns arrived when within threshold', () => {
    const target: Position = { x: 10, y: 0, z: 10 }
    const state = initMovementState({ x: 0, y: 0, z: 0 }, target)
    const currentPos: Position = { x: 10, y: 5, z: 10.02 } // within 0.05
    const result = calculateMovementStep(currentPos, state, cfg, 0.016)
    expect(result.arrived).toBe(true)
    expect(result.newSpeed).toBe(0)
    expect(result.newPos).toEqual({ x: 10, y: 5, z: 10 })
  })

  it('accelerates from rest during accel phase', () => {
    const target: Position = { x: 100, y: 0, z: 0 }
    const state = initMovementState({ x: 0, y: 0, z: 0 }, target)
    const dt = 0.1
    const result = calculateMovementStep({ x: 0, y: 0, z: 0 }, state, cfg, dt)
    // newSpeed = 0 + accel * dt = 6 * 0.1 = 0.6
    expect(result.newSpeed).toBeCloseTo(0.6)
    expect(result.arrived).toBe(false)
  })

  it('caps acceleration at maxSpeed', () => {
    const target: Position = { x: 100, y: 0, z: 0 }
    const state = initMovementState({ x: 0, y: 0, z: 0 }, target)
    state.currentSpeed = 2.9
    const result = calculateMovementStep(
      { x: 0.1, y: 0, z: 0 },
      state,
      cfg,
      0.1
    )
    expect(result.newSpeed).toBe(cfg.maxSpeed)
  })

  it('holds maxSpeed during cruise phase', () => {
    const target: Position = { x: 100, y: 0, z: 0 }
    const state = initMovementState({ x: 0, y: 0, z: 0 }, target)
    state.currentSpeed = cfg.maxSpeed
    // Past accel distance (0.75), far from decel zone
    const result = calculateMovementStep({ x: 50, y: 0, z: 0 }, state, cfg, 0.1)
    expect(result.newSpeed).toBe(cfg.maxSpeed)
  })

  it('decelerates when within decel distance of target', () => {
    const target: Position = { x: 5, y: 0, z: 0 }
    const state = initMovementState({ x: 0, y: 0, z: 0 }, target)
    state.currentSpeed = cfg.maxSpeed
    // 0.5 remaining < decel distance 0.75
    const result = calculateMovementStep(
      { x: 4.5, y: 0, z: 0 },
      state,
      cfg,
      0.1
    )
    expect(result.newSpeed).toBeLessThan(cfg.maxSpeed)
    expect(result.newSpeed).toBeCloseTo(cfg.maxSpeed - cfg.deceleration * 0.1)
  })

  it('does not let deceleration go below 0', () => {
    const target: Position = { x: 5, y: 0, z: 0 }
    const state = initMovementState({ x: 0, y: 0, z: 0 }, target)
    state.currentSpeed = 0.1
    const result = calculateMovementStep(
      { x: 4.9, y: 0, z: 0 },
      state,
      cfg,
      1.0
    )
    // Would undercut to negative; clamped to 0 and arrives
    expect(result.newSpeed).toBe(0)
    expect(result.arrived).toBe(true)
  })

  it('rotates toward target using atan2(dx, dz)', () => {
    const target: Position = { x: 0, y: 0, z: 10 } // +Z
    const state = initMovementState({ x: 0, y: 0, z: 0 }, target)
    const result = calculateMovementStep({ x: 0, y: 0, z: 0 }, state, cfg, 0.1)
    expect(result.rotation).toBeCloseTo(0) // atan2(0, 10) = 0

    const target2: Position = { x: 10, y: 0, z: 0 } // +X
    const state2 = initMovementState({ x: 0, y: 0, z: 0 }, target2)
    const result2 = calculateMovementStep(
      { x: 0, y: 0, z: 0 },
      state2,
      cfg,
      0.1
    )
    expect(result2.rotation).toBeCloseTo(Math.PI / 2) // atan2(10, 0)
  })

  it('moves along XZ direction proportional to speed*dt', () => {
    const target: Position = { x: 100, y: 0, z: 0 }
    const state = initMovementState({ x: 0, y: 0, z: 0 }, target)
    state.currentSpeed = 3
    const result = calculateMovementStep({ x: 10, y: 5, z: 0 }, state, cfg, 0.1)
    // moveDistance = 3 * 0.1 = 0.3, all along +X
    expect(result.newPos.x).toBeCloseTo(10.3)
    expect(result.newPos.z).toBeCloseTo(0)
    expect(result.newPos.y).toBe(5) // Y preserved
  })

  it('continues through the X seam instead of reversing across the world', () => {
    const start: Position = { x: WORLD_MAX_X - 1, y: 5, z: 0 }
    const target: Position = { x: WORLD_MIN_X + 1, y: 0, z: 0 }
    const state = initMovementState(start, target)
    state.currentSpeed = cfg.maxSpeed

    const result = calculateMovementStep(start, state, cfg, 0.1)

    expect(result.arrived).toBe(false)
    expect(result.newPos.x).toBeCloseTo(start.x + 0.3)
    expect(result.rotation).toBeCloseTo(Math.PI / 2)
  })

  it('preserves Y from currentPos (terrain handles Y)', () => {
    const target: Position = { x: 10, y: 999, z: 0 }
    const state = initMovementState({ x: 0, y: 0, z: 0 }, target)
    state.currentSpeed = 2
    const result = calculateMovementStep({ x: 1, y: 42, z: 0 }, state, cfg, 0.1)
    expect(result.newPos.y).toBe(42)
  })

  it('snaps to target when moveDistance exceeds remaining', () => {
    const target: Position = { x: 1, y: 0, z: 0 }
    const state = initMovementState({ x: 0, y: 0, z: 0 }, target)
    state.currentSpeed = cfg.maxSpeed
    // 0.1 remaining, moveDistance = 3*1 = 3 >> 0.1
    const result = calculateMovementStep(
      { x: 0.9, y: 0, z: 0 },
      state,
      cfg,
      1.0
    )
    expect(result.arrived).toBe(true)
    expect(result.newPos).toEqual({ x: 1, y: 0, z: 0 })
    expect(result.newSpeed).toBe(0)
  })

  it('moves along normalized direction on diagonals', () => {
    // Place us past accel zone (traveled > 0.75) and far from decel zone,
    // so we cruise at maxSpeed.
    const start: Position = { x: 0, y: 0, z: 0 }
    const target: Position = { x: 30, y: 0, z: 40 } // distance 50
    const state = initMovementState(start, target)
    state.currentSpeed = cfg.maxSpeed
    // currentPos such that remaining = 40, traveled = 10 (past accel, far from decel)
    const result = calculateMovementStep(
      { x: 30 * 0.2, y: 0, z: 40 * 0.2 },
      state,
      cfg,
      0.1
    )
    // moveDistance = 3 * 0.1 = 0.3 along dir (0.6, 0.8)
    expect(result.newPos.x).toBeCloseTo(30 * 0.2 + 0.3 * 0.6)
    expect(result.newPos.z).toBeCloseTo(40 * 0.2 + 0.3 * 0.8)
  })
})
