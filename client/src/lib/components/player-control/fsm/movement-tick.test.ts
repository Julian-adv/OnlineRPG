import { describe, expect, it, vi } from 'vitest'
import {
  applyMovementSubstrateOutcome,
  runMovementFrame,
  runPlayerMovementTick,
  syncPlayerTerrainHeight,
  type MovementOutcomeActions,
} from './movement-tick'
import { directPathing } from './pathing.fixture'

function actions(): MovementOutcomeActions {
  return {
    stopMovement: vi.fn(),
    triggerJumpFeedback: vi.fn(),
    setNextWaypoint: vi.fn(),
    arrive: vi.fn(),
    continueMovement: vi.fn(),
  }
}

describe('syncPlayerTerrainHeight', () => {
  it('keeps non-interaction players aligned with terrain height', () => {
    const player = { position: { x: 1, y: 2, z: 3 } }

    const changed = syncPlayerTerrainHeight({
      playerStateName: 'moving',
      player,
      hasHeightData: () => true,
      sampleHeight: () => 4,
    })

    expect(changed).toBe(true)
    expect(player.position.y).toBe(4)
  })

  it('does not touch object or pickup interaction height', () => {
    const sampleHeight = vi.fn(() => 4)
    const player = { position: { x: 1, y: 2, z: 3 } }

    const changed = syncPlayerTerrainHeight({
      playerStateName: 'interact',
      player,
      hasHeightData: () => true,
      sampleHeight,
    })

    expect(changed).toBe(false)
    expect(sampleHeight).not.toHaveBeenCalled()
    expect(player.position.y).toBe(2)
  })

  it('does nothing without height data or meaningful y drift', () => {
    const player = { position: { x: 1, y: 2, z: 3 } }

    expect(
      syncPlayerTerrainHeight({
        playerStateName: 'idle',
        player,
        hasHeightData: () => false,
        sampleHeight: () => 4,
      })
    ).toBe(false)

    expect(
      syncPlayerTerrainHeight({
        playerStateName: 'idle',
        player,
        hasHeightData: () => true,
        sampleHeight: () => 2.0005,
      })
    ).toBe(false)
    expect(player.position.y).toBe(2)
  })
})

describe('applyMovementSubstrateOutcome', () => {
  it('stops movement on blocked outcomes', () => {
    const a = actions()

    applyMovementSubstrateOutcome({ kind: 'blocked' }, a)

    expect(a.stopMovement).toHaveBeenCalledOnce()
    expect(a.triggerJumpFeedback).not.toHaveBeenCalled()
  })

  it('stops movement and triggers jump feedback on slope blocks', () => {
    const a = actions()

    applyMovementSubstrateOutcome({ kind: 'slope_blocked' }, a)

    expect(a.stopMovement).toHaveBeenCalledOnce()
    expect(a.triggerJumpFeedback).toHaveBeenCalledOnce()
  })

  it('applies next waypoint state', () => {
    const a = actions()
    const movementTarget = { x: 1, y: 2, z: 3 }
    const movementState = {
      currentSpeed: 4,
      startPos: { x: 0, y: 0, z: 0 },
      targetPos: movementTarget,
      totalDistance: 5,
    }

    applyMovementSubstrateOutcome(
      {
        kind: 'next_waypoint',
        currentSpeed: 4,
        playerRotation: 0.5,
        movementTarget,
        movementState,
        currentWaypointIndex: 2,
      },
      a
    )

    expect(a.setNextWaypoint).toHaveBeenCalledWith(
      4,
      0.5,
      movementTarget,
      movementState,
      2
    )
  })

  it('routes arrival and continued outcomes', () => {
    const a = actions()

    applyMovementSubstrateOutcome(
      { kind: 'arrived', currentSpeed: 0, playerRotation: 1 },
      a
    )
    applyMovementSubstrateOutcome(
      {
        kind: 'continued',
        currentSpeed: 2,
        playerRotation: 3,
        totalDistance: 4,
      },
      a
    )

    expect(a.arrive).toHaveBeenCalledWith(0, 1)
    expect(a.continueMovement).toHaveBeenCalledWith(2, 3, 4)
  })
})

describe('runMovementFrame', () => {
  it('steps movement substrate and applies the outcome', () => {
    const a = actions()

    runMovementFrame({
      currentPos: { x: 0, y: 0, z: 0 },
      movementTarget: { x: 0.01, y: 0, z: 0 },
      movementState: {
        currentSpeed: 0,
        startPos: { x: 0, y: 0, z: 0 },
        targetPos: { x: 0.01, y: 0, z: 0 },
        totalDistance: 0.01,
      },
      pathWaypoints: [],
      currentWaypointIndex: 0,
      config: {
        maxSpeed: 3,
        acceleration: 6,
        deceleration: 6,
        arrivalThreshold: 0.05,
      },
      deltaTimeSeconds: 0.016,
      sampleHeight: () => 0,
      waypointHeight: () => 0,
      isMovementBlocked: () => false,
      isUphillTooSteep: () => false,
      setFloorLevel: vi.fn(),
      writePlayerPosition: vi.fn(),
      sendPlayerMove: vi.fn(),
      actions: a,
    })

    expect(a.arrive).toHaveBeenCalledOnce()
  })
})

function baseInput() {
  return {
    deltaTime: 16,
    currentPlayer: { health: 10, position: { x: 0, y: 0, z: 0 } },
    playerStateName: 'idle' as const,
    isMoving: false,
    currentSpeed: 0,
    movementTarget: null,
    movementState: null,
    pathWaypoints: [],
    currentWaypointIndex: 0,
    config: {
      maxSpeed: 3,
      acceleration: 6,
      deceleration: 6,
      arrivalThreshold: 0.05,
    },
    isInCombat: false,
    combatController: {
      targetMonsterId: null,
      update: vi.fn(),
    },
    cooldownMs: 1500,
    chaseGoal: null,
    chasePathing: directPathing(),
    getMonsterInfo: vi.fn(),
    findMonsterPosition: vi.fn(),
    sampleHeight: () => 0,
    waypointHeight: () => 0,
    hasHeightData: () => true,
    isMovementBlocked: () => false,
    isUphillTooSteep: () => false,
    setFloorLevel: vi.fn(),
    writePlayerPosition: vi.fn(),
    sendPlayerMove: vi.fn(),
    actions: {
      transitionToDead: vi.fn(),
      transitionToRespawned: vi.fn(),
      resetStoppedSpeed: vi.fn(),
      combat: {
        stopMovingToIdle: vi.fn(),
        prepareReachedAttackRange: vi.fn(),
        beginAttack: vi.fn(),
        setChasingMovement: vi.fn(),
        showAttackState: vi.fn(),
        sendAttackCycle: vi.fn(),
      },
      movement: {
        stopMovement: vi.fn(),
        triggerJumpFeedback: vi.fn(),
        setNextWaypoint: vi.fn(),
        arrive: vi.fn(),
        continueMovement: vi.fn(),
      },
    },
  }
}

describe('runPlayerMovementTick', () => {
  it('transitions dead players before terrain or combat processing', () => {
    const input = {
      ...baseInput(),
      currentPlayer: { health: 0, position: { x: 0, y: 3, z: 0 } },
    }

    runPlayerMovementTick(input)

    expect(input.actions.transitionToDead).toHaveBeenCalledOnce()
    expect(input.currentPlayer.position.y).toBe(3)
  })

  it('recovers respawned players if the control state is still dead', () => {
    const input = {
      ...baseInput(),
      playerStateName: 'dead' as const,
      currentPlayer: { health: 10, position: { x: 0, y: 3, z: 0 } },
    }

    runPlayerMovementTick(input)

    expect(input.actions.transitionToRespawned).toHaveBeenCalledOnce()
    expect(input.actions.resetStoppedSpeed).not.toHaveBeenCalled()
    expect(input.currentPlayer.position.y).toBe(3)
  })

  it('resets stale speed when there is no active movement', () => {
    const input = {
      ...baseInput(),
      currentSpeed: 1,
    }

    runPlayerMovementTick(input)

    expect(input.actions.resetStoppedSpeed).toHaveBeenCalledOnce()
  })

  it('runs movement when runtime movement is active', () => {
    const input = {
      ...baseInput(),
      isMoving: true,
      movementTarget: { x: 0.01, y: 0, z: 0 },
      movementState: {
        currentSpeed: 0,
        startPos: { x: 0, y: 0, z: 0 },
        targetPos: { x: 0.01, y: 0, z: 0 },
        totalDistance: 0.01,
      },
    }

    runPlayerMovementTick(input)

    expect(input.actions.movement.arrive).toHaveBeenCalledOnce()
  })
})
