import { describe, expect, it, vi } from 'vitest'
import type { Position } from '../../../utils/movementUtils'
import {
  decideMoveRequest,
  runMoveRequest,
  startClickMovement,
  type MoveRequestActions,
} from './move-request'

const baseInput = {
  pickupAfterArrival: null,
  currentPlayerHealth: 10,
  interactionExit: 'none' as const,
  hasCurrentPlayer: true,
  isMoving: false,
  hasKeyboardInput: false,
}

describe('decideMoveRequest', () => {
  it('clears pending pickup only for ordinary movement requests', () => {
    expect(decideMoveRequest(baseInput).clearPendingPickupAfterMove).toBe(true)
    expect(
      decideMoveRequest({
        ...baseInput,
        pickupAfterArrival: 3,
      }).clearPendingPickupAfterMove
    ).toBe(false)
  })

  it('ignores dead players before interaction exit handling', () => {
    expect(
      decideMoveRequest({
        ...baseInput,
        currentPlayerHealth: 0,
        interactionExit: 'pickup',
      })
    ).toEqual({
      kind: 'ignored',
      clearPendingPickupAfterMove: true,
    })
  })

  it('preserves pickup immediate retry and object delayed stand-up decisions', () => {
    expect(
      decideMoveRequest({
        ...baseInput,
        interactionExit: 'pickup',
      }).kind
    ).toBe('exit_pickup_and_retry')

    expect(
      decideMoveRequest({
        ...baseInput,
        interactionExit: 'object',
      }).kind
    ).toBe('exit_object_and_delay')
  })

  it('allows replacing click movement but blocks keyboard contention', () => {
    expect(
      decideMoveRequest({
        ...baseInput,
        isMoving: true,
      }).kind
    ).toBe('start')

    expect(
      decideMoveRequest({
        ...baseInput,
        isMoving: true,
        hasKeyboardInput: true,
      }).kind
    ).toBe('ignored')
  })

  it('requires a current player to start movement', () => {
    expect(
      decideMoveRequest({
        ...baseInput,
        hasCurrentPlayer: false,
        currentPlayerHealth: null,
      }).kind
    ).toBe('ignored')
  })
})

const currentPos: Position = { x: 0, y: 0, z: 0 }
const clickPosition: Position = { x: 4, y: 0, z: 5 }

describe('startClickMovement', () => {
  it('uses pathfinding waypoints when available', () => {
    const sendPlayerMove = vi.fn()

    const started = startClickMovement({
      currentPos,
      clickPosition,
      initialSpeed: 0,
      pickupAfterArrival: null,
      currentFloor: 0,
      getFloorAt: vi.fn(() => 1),
      findPath: vi.fn(() => ({
        waypoints: [{ x: 2, z: 3, floor: 1 }],
      })),
      waypointHeight: vi.fn((_f: number, x: number, z: number) => x + z),
      sendPlayerMove,
    })

    expect(started.pathWaypoints).toEqual([{ x: 2, z: 3, floor: 1 }])
    expect(started.movementTarget).toEqual({ x: 2, y: 5, z: 3 })
    expect(started.pendingPickupAfterMoveInstanceId).toBeNull()
    expect(sendPlayerMove).toHaveBeenCalledWith(
      { x: 2, y: 5, z: 3 },
      expect.any(Number),
      false
    )
  })

  // The sent Y is the server's authoritative collision height. Deriving it
  // from the walker's current floor put a climber's waypoint a storey low and
  // sealed them under furniture on the floor below, so it must key off the
  // waypoint's own floor.
  it("resolves the waypoint's height on the waypoint's floor, not the walker's", () => {
    const waypointHeight = vi.fn(() => 7)

    startClickMovement({
      currentPos,
      clickPosition,
      initialSpeed: 0,
      pickupAfterArrival: null,
      currentFloor: 0,
      getFloorAt: vi.fn(() => 1),
      findPath: vi.fn(() => ({ waypoints: [{ x: 2, z: 3, floor: 1 }] })),
      waypointHeight,
      sendPlayerMove: vi.fn(),
    })

    expect(waypointHeight).toHaveBeenCalledWith(1, 2, 3)
  })

  it('falls back to a direct waypoint when pathfinding returns no path', () => {
    const sendPlayerMove = vi.fn()

    const started = startClickMovement({
      currentPos,
      clickPosition,
      initialSpeed: 0,
      pickupAfterArrival: 42,
      currentFloor: 0,
      getFloorAt: vi.fn(() => 2),
      findPath: vi.fn(() => ({ waypoints: [] })),
      waypointHeight: vi.fn((_f: number, x: number, z: number) => x + z),
      sendPlayerMove,
    })

    expect(started.pathWaypoints).toEqual([{ x: 4, z: 5, floor: 2 }])
    expect(started.movementTarget).toEqual({ x: 4, y: 9, z: 5 })
    expect(started.pendingPickupAfterMoveInstanceId).toBe(42)
  })

  it('carries active movement speed into a replacement path', () => {
    const started = startClickMovement({
      currentPos,
      clickPosition,
      initialSpeed: 2.25,
      pickupAfterArrival: null,
      currentFloor: 0,
      getFloorAt: () => 0,
      findPath: () => ({ waypoints: [] }),
      waypointHeight: () => 0,
      sendPlayerMove: vi.fn(),
    })

    expect(started.movementState.currentSpeed).toBe(2.25)
  })
})

function actions(): MoveRequestActions {
  return {
    clearPendingPickupAfterMove: vi.fn(),
    exitPickupAndRetry: vi.fn(),
    exitObjectAndDelay: vi.fn(),
    applyStartedMovement: vi.fn(),
  }
}

const deps = {
  initialSpeed: 0,
  currentFloor: 0,
  getFloorAt: () => 0,
  findPath: () => ({ waypoints: [] }),
  waypointHeight: () => 0,
  sendPlayerMove: vi.fn(),
}

describe('runMoveRequest', () => {
  it('routes pickup and object interaction exits before starting movement', () => {
    const pickupActions = actions()
    runMoveRequest({
      clickPosition: { x: 1, y: 0, z: 0 },
      pickupAfterArrival: null,
      currentPlayer: { health: 10, position: { x: 0, y: 0, z: 0 } },
      interactionExit: 'pickup',
      isMoving: false,
      hasKeyboardInput: false,
      actions: pickupActions,
      ...deps,
    })

    expect(pickupActions.exitPickupAndRetry).toHaveBeenCalledOnce()
    expect(pickupActions.applyStartedMovement).not.toHaveBeenCalled()

    const objectActions = actions()
    runMoveRequest({
      clickPosition: { x: 1, y: 0, z: 0 },
      pickupAfterArrival: null,
      currentPlayer: { health: 10, position: { x: 0, y: 0, z: 0 } },
      interactionExit: 'object',
      isMoving: false,
      hasKeyboardInput: false,
      actions: objectActions,
      ...deps,
    })

    expect(objectActions.exitObjectAndDelay).toHaveBeenCalledOnce()
    expect(objectActions.applyStartedMovement).not.toHaveBeenCalled()
  })

  it('starts movement when the request is allowed', () => {
    const a = actions()
    runMoveRequest({
      clickPosition: { x: 1, y: 0, z: 0 },
      pickupAfterArrival: 7,
      currentPlayer: { health: 10, position: { x: 0, y: 0, z: 0 } },
      interactionExit: 'none',
      isMoving: false,
      hasKeyboardInput: false,
      actions: a,
      ...deps,
    })

    expect(a.clearPendingPickupAfterMove).not.toHaveBeenCalled()
    expect(a.applyStartedMovement).toHaveBeenCalledWith(
      expect.objectContaining({
        movementState: expect.objectContaining({ currentSpeed: 0 }),
      })
    )
  })

  it('preserves momentum when an active click path is replaced', () => {
    const a = actions()
    runMoveRequest({
      ...deps,
      clickPosition: { x: 2, y: 0, z: 1 },
      initialSpeed: 2.4,
      pickupAfterArrival: null,
      currentPlayer: { health: 10, position: { x: 1, y: 0, z: 0 } },
      interactionExit: 'none',
      isMoving: true,
      hasKeyboardInput: false,
      actions: a,
    })

    expect(a.applyStartedMovement).toHaveBeenCalledWith(
      expect.objectContaining({
        movementState: expect.objectContaining({ currentSpeed: 2.4 }),
      })
    )
    expect(deps.sendPlayerMove).toHaveBeenLastCalledWith(
      { x: 2, y: 0, z: 1 },
      expect.any(Number),
      false
    )
  })

  it('clears pending pickup and ignores blocked requests', () => {
    const a = actions()
    runMoveRequest({
      clickPosition: { x: 1, y: 0, z: 0 },
      pickupAfterArrival: null,
      currentPlayer: { health: 0, position: { x: 0, y: 0, z: 0 } },
      interactionExit: 'none',
      isMoving: false,
      hasKeyboardInput: false,
      actions: a,
      ...deps,
    })

    expect(a.clearPendingPickupAfterMove).toHaveBeenCalledOnce()
    expect(a.applyStartedMovement).not.toHaveBeenCalled()
  })
})
