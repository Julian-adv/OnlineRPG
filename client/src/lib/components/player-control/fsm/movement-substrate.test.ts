import { describe, expect, it, vi } from 'vitest'
import {
  DEFAULT_MOVEMENT_CONFIG,
  initMovementState,
  type Position,
} from '../../../utils/movementUtils'
import { stepMovementSubstrate, type PathWaypoint } from './movement-substrate'

function makeBaseInput(
  currentPos: Position,
  target: Position,
  waypoints: PathWaypoint[] = [{ x: target.x, z: target.z, floor: 0 }]
) {
  return {
    currentPos,
    movementTarget: target,
    movementState: initMovementState(currentPos, target, 0),
    pathWaypoints: waypoints,
    currentWaypointIndex: 0,
    config: DEFAULT_MOVEMENT_CONFIG,
    deltaTimeSeconds: 0.1,
    sampleHeight: vi.fn((x: number, z: number) => x + z),
    waypointHeight: vi.fn((_f: number, x: number, z: number) => x + z),
    isMovementBlocked: vi.fn(() => false),
    isUphillTooSteep: vi.fn(() => false),
    setFloorLevel: vi.fn(),
    writePlayerPosition: vi.fn(),
    sendPlayerMove: vi.fn(),
  }
}

describe('stepMovementSubstrate', () => {
  it('continues movement and writes the interpolated position', () => {
    const input = makeBaseInput({ x: 0, y: 0, z: 0 }, { x: 10, y: 0, z: 0 })

    const outcome = stepMovementSubstrate(input)

    expect(outcome.kind).toBe('continued')
    expect(input.writePlayerPosition).toHaveBeenCalledWith(
      expect.objectContaining({ x: expect.any(Number), z: 0 }),
      expect.any(Number)
    )
    expect(input.sendPlayerMove).not.toHaveBeenCalled()
  })

  it('arrives at the final waypoint without touching the leg floor', () => {
    const target = { x: 1, y: 0, z: 0 }
    const input = makeBaseInput({ x: 0.99, y: 0, z: 0 }, target, [
      { x: 1, z: 0, floor: 2 },
    ])
    input.deltaTimeSeconds = 1

    const outcome = stepMovementSubstrate(input)

    expect(outcome.kind).toBe('arrived')
    expect(input.setFloorLevel).not.toHaveBeenCalled()
    expect(input.writePlayerPosition).toHaveBeenCalledWith(
      { x: 1, y: 1, z: 0 },
      expect.any(Number)
    )
    expect(input.sendPlayerMove).toHaveBeenCalledWith(
      target,
      expect.any(Number),
      true
    )
  })

  it('initializes the next waypoint without stopping movement', () => {
    const first = { x: 1, y: 0, z: 0 }
    const second = { x: 2, z: 0, floor: 3 }
    const input = makeBaseInput({ x: 0.99, y: 0, z: 0 }, first, [
      { x: 1, z: 0, floor: 2 },
      second,
    ])
    input.deltaTimeSeconds = 1

    const outcome = stepMovementSubstrate(input)

    expect(outcome.kind).toBe('next_waypoint')
    if (outcome.kind !== 'next_waypoint') return
    expect(outcome.currentWaypointIndex).toBe(1)
    expect(outcome.movementTarget).toEqual({ x: 2, y: 2, z: 0 })
    expect(input.setFloorLevel).toHaveBeenCalledExactlyOnceWith(3)
    expect(input.sendPlayerMove).toHaveBeenCalledWith(
      { x: 2, y: 2, z: 0 },
      expect.any(Number),
      true
    )
    // Keyed to the waypoint's own floor: the walker's floor lags a stairwell
    // climb, and the server trusts this Y for collision height.
    expect(input.waypointHeight).toHaveBeenCalledWith(3, 2, 0)
  })

  it('sends the stop position as a replace when a step is blocked', () => {
    const currentPos = { x: 0, y: 0, z: 0 }
    const input = makeBaseInput(currentPos, { x: 10, y: 0, z: 0 })
    input.isMovementBlocked.mockReturnValue(true)

    const outcome = stepMovementSubstrate(input)

    expect(outcome.kind).toBe('blocked')
    expect(input.writePlayerPosition).not.toHaveBeenCalled()
    expect(input.sendPlayerMove).toHaveBeenCalledExactlyOnceWith(
      currentPos,
      expect.any(Number)
    )
  })

  it('sends the stop position when arrival into the target is blocked', () => {
    const currentPos = { x: 0.99, y: 0, z: 0 }
    const input = makeBaseInput(currentPos, { x: 1, y: 0, z: 0 })
    input.deltaTimeSeconds = 1
    input.isMovementBlocked.mockReturnValue(true)

    const outcome = stepMovementSubstrate(input)

    expect(outcome.kind).toBe('blocked')
    expect(input.sendPlayerMove).toHaveBeenCalledExactlyOnceWith(
      currentPos,
      expect.any(Number)
    )
  })

  it('sends the stop position when the slope blocks the step', () => {
    const currentPos = { x: 0, y: 0, z: 0 }
    const input = makeBaseInput(currentPos, { x: 10, y: 0, z: 0 })
    input.isUphillTooSteep.mockReturnValue(true)

    const outcome = stepMovementSubstrate(input)

    expect(outcome.kind).toBe('slope_blocked')
    expect(input.sendPlayerMove).toHaveBeenCalledExactlyOnceWith(
      currentPos,
      expect.any(Number)
    )
  })
})
