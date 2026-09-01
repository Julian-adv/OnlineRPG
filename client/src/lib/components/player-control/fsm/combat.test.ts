import { describe, expect, it, vi } from 'vitest'
import type { CombatUpdateResult } from '../../../managers/combatController'
import {
  initMovementState,
  type PlayerState,
  type Position,
} from '../../../utils/movementUtils'
import {
  applyChaseTargetUpdate,
  applyCombatTickOutcome,
  beginAttack,
  ensureAttackState,
  runCombatFrame,
  tickCombat,
  transitionAttackToIdle,
  type CombatControllerLike,
  type CombatOutcomeActions,
  type CombatTickOutcome,
} from './combat'
import { directPathing } from './pathing.fixture'

function actions(): CombatOutcomeActions {
  return {
    stopMovingToIdle: vi.fn(),
    prepareReachedAttackRange: vi.fn(),
    beginAttack: vi.fn(),
    setChasingMovement: vi.fn(),
    showAttackState: vi.fn(),
    sendAttackCycle: vi.fn(),
  }
}

function makeCombatController(result: CombatUpdateResult) {
  return {
    targetMonsterId: 'monster-1',
    update: vi.fn(() => result),
  }
}

function frameController(action: 'none' | 'idle'): CombatControllerLike {
  return {
    targetMonsterId: 'm1',
    update: vi.fn(() => ({ action })),
  }
}

const currentPos = { x: 0, y: 0, z: 0 }
const playerPos: Position = { x: 0, y: 0, z: 0 }
const playerState: PlayerState = {
  state: 'idle',
  speed: 0,
  rotation: 0,
  position: { x: 0, y: 0, z: 0 },
}

describe('applyChaseTargetUpdate', () => {
  it('does nothing when no new target is available', () => {
    const sendPlayerMove = vi.fn()

    const outcome = applyChaseTargetUpdate({
      currentPos,
      chaseGoal: null,
      movementState: null,
      currentSpeed: 0,
      pathing: directPathing(),
      sendPlayerMove,
    })

    expect(outcome.kind).toBe('unchanged')
    expect(sendPlayerMove).not.toHaveBeenCalled()
  })

  it('keeps the live path while the monster mills around its goal', () => {
    const sendPlayerMove = vi.fn()
    const pathing = directPathing()

    const outcome = applyChaseTargetUpdate({
      currentPos,
      newTarget: { x: 1.4, y: 0, z: 1.4 },
      chaseGoal: { x: 1, y: 0, z: 1 },
      movementState: initMovementState(currentPos, { x: 1, y: 0, z: 1 }),
      currentSpeed: 0,
      pathing,
      sendPlayerMove,
    })

    expect(outcome.kind).toBe('unchanged')
    expect(pathing.findPath).not.toHaveBeenCalled()
    expect(sendPlayerMove).not.toHaveBeenCalled()
  })

  it('routes around walls and replaces the server queue with the first leg', () => {
    const sendPlayerMove = vi.fn()
    // A detour: the direct line to (3,4) is walled, so A* returns a corner.
    const pathing = directPathing([
      { x: 3, z: 0, floor: 0 },
      { x: 3, z: 4, floor: 0 },
    ])

    const outcome = applyChaseTargetUpdate({
      currentPos,
      newTarget: { x: 3, y: 0, z: 4 },
      chaseGoal: null,
      movementState: null,
      currentSpeed: 1.25,
      pathing,
      sendPlayerMove,
    })

    expect(outcome.kind).toBe('updated')
    if (outcome.kind !== 'updated') return
    // The player heads for the detour corner, not straight at the monster.
    expect(outcome.movementTarget).toEqual({ x: 3, y: 0, z: 0 })
    expect(outcome.pathWaypoints).toHaveLength(2)
    expect(outcome.chaseGoal).toEqual({ x: 3, y: 0, z: 4 })
    expect(outcome.movementState.currentSpeed).toBe(1.25)
    // append=false: a fresh path replaces the queue rather than detouring
    // through whatever the server was still walking toward.
    expect(sendPlayerMove).toHaveBeenCalledWith(
      { x: 3, y: 0, z: 0 },
      Math.atan2(3, 0),
      false
    )
  })

  it('updates existing movement state in place for chase retargets', () => {
    const sendPlayerMove = vi.fn()
    const movementState = initMovementState(currentPos, { x: 1, y: 0, z: 1 })
    const newTarget = { x: 6, y: 0, z: 8 }

    const outcome = applyChaseTargetUpdate({
      currentPos,
      newTarget,
      chaseGoal: { x: 1, y: 0, z: 1 },
      movementState,
      currentSpeed: 0,
      pathing: directPathing(),
      sendPlayerMove,
    })

    expect(outcome.kind).toBe('updated')
    if (outcome.kind !== 'updated') return
    expect(outcome.movementState).toBe(movementState)
    expect(movementState.targetPos).toEqual(newTarget)
    expect(movementState.totalDistance).toBe(10)
  })

  it('falls back to the monster itself when no path is found', () => {
    const sendPlayerMove = vi.fn()

    const outcome = applyChaseTargetUpdate({
      currentPos,
      newTarget: { x: 3, y: 0, z: 4 },
      chaseGoal: null,
      movementState: null,
      currentSpeed: 0,
      pathing: directPathing([]),
      sendPlayerMove,
    })

    expect(outcome.kind).toBe('updated')
    if (outcome.kind !== 'updated') return
    expect(outcome.pathWaypoints).toEqual([{ x: 3, z: 4, floor: 0 }])
  })
})

describe('tickCombat', () => {
  it('returns none when there is no combat target', () => {
    const controller = {
      targetMonsterId: null,
      update: vi.fn(),
    }

    const outcome = tickCombat({
      combatController: controller,
      deltaTime: 16,
      playerPos,
      playerStateName: 'idle',
      isMoving: false,
      currentSpeed: 0,
      chaseGoal: null,
      movementState: null,
      cooldownMs: 1500,
      pathing: directPathing(),
      getMonsterInfo: vi.fn(),
      attackLineBlocked: () => false,
      findMonsterPosition: vi.fn(),
      sendPlayerMove: vi.fn(),
    })

    expect(outcome.kind).toBe('none')
    expect(controller.update).not.toHaveBeenCalled()
  })

  it('maps attack cycles with rotation', () => {
    const controller = makeCombatController({
      action: 'attack_cycle',
      monsterId: 'monster-1',
      rotation: 1.25,
    })

    const outcome = tickCombat({
      combatController: controller,
      deltaTime: 16,
      playerPos,
      playerStateName: 'attack',
      isMoving: false,
      currentSpeed: 0,
      chaseGoal: null,
      movementState: null,
      cooldownMs: 1500,
      pathing: directPathing(),
      getMonsterInfo: vi.fn(() => ({ state: 'idle' })),
      attackLineBlocked: () => false,
      findMonsterPosition: vi.fn(() => ({ x: 1, y: 0, z: 0 })),
      sendPlayerMove: vi.fn(),
    })

    expect(outcome).toEqual({
      kind: 'attack_cycle',
      monsterId: 'monster-1',
      playerRotation: 1.25,
    })
  })

  it('updates chase movement when combat provides a new target', () => {
    const controller = makeCombatController({
      action: 'chasing',
      newTarget: { x: 3, y: 0, z: 4 },
    })
    const sendPlayerMove = vi.fn()

    const outcome = tickCombat({
      combatController: controller,
      deltaTime: 16,
      playerPos,
      playerStateName: 'moving',
      isMoving: true,
      currentSpeed: 0.5,
      chaseGoal: null,
      movementState: null,
      cooldownMs: 1500,
      pathing: directPathing(),
      getMonsterInfo: vi.fn(() => ({ state: 'idle' })),
      attackLineBlocked: () => false,
      findMonsterPosition: vi.fn(() => ({ x: 3, y: 0, z: 4 })),
      sendPlayerMove,
    })

    expect(outcome.kind).toBe('chasing_updated')
    expect(sendPlayerMove).toHaveBeenCalledWith(
      { x: 3, y: 0, z: 4 },
      Math.atan2(3, 4),
      false
    )
  })
})

describe('applyCombatTickOutcome', () => {
  it('handles idle by stopping movement', () => {
    const a = actions()

    expect(applyCombatTickOutcome({ kind: 'idle' }, a)).toEqual({
      kind: 'handled',
    })
    expect(a.stopMovingToIdle).toHaveBeenCalledOnce()
  })

  it('handles reached attack range by preparing and starting attack', () => {
    const a = actions()

    expect(
      applyCombatTickOutcome(
        { kind: 'reached_attack_range', monsterId: 'm1' },
        a
      )
    ).toEqual({ kind: 'handled' })

    expect(a.prepareReachedAttackRange).toHaveBeenCalledOnce()
    expect(a.beginAttack).toHaveBeenCalledWith('m1')
  })

  it('installs the re-routed chase path and skips the stale frame', () => {
    const a = actions()
    const movementTarget = { x: 1, y: 2, z: 3 }
    const movementState = {
      currentSpeed: 1,
      startPos: { x: 0, y: 0, z: 0 },
      targetPos: movementTarget,
      totalDistance: 10,
    }
    const pathWaypoints = [{ x: 1, z: 3, floor: 0 }]
    const chaseGoal = { x: 9, y: 0, z: 9 }

    // 'handled', not 'continue_movement': the caller's waypoint locals describe
    // the path this outcome just replaced.
    expect(
      applyCombatTickOutcome(
        {
          kind: 'chasing_updated',
          pathWaypoints,
          movementTarget,
          movementState,
          playerRotation: 0.5,
          chaseGoal,
        },
        a
      )
    ).toEqual({ kind: 'handled' })

    expect(a.setChasingMovement).toHaveBeenCalledWith({
      pathWaypoints,
      movementTarget,
      movementState,
      playerRotation: 0.5,
      chaseGoal,
    })
  })

  it('continues movement for no-op combat outcomes', () => {
    const a = actions()

    for (const outcome of [
      { kind: 'none' },
      { kind: 'chasing_unchanged' },
    ] satisfies CombatTickOutcome[]) {
      expect(applyCombatTickOutcome(outcome, a)).toEqual({
        kind: 'continue_movement',
      })
    }
  })

  it('handles attack animation and attack cycle outcomes', () => {
    const a = actions()

    expect(
      applyCombatTickOutcome({ kind: 'attacking', playerRotation: 1 }, a)
    ).toEqual({ kind: 'handled' })
    expect(
      applyCombatTickOutcome(
        { kind: 'attack_cycle', monsterId: 'm1', playerRotation: 2 },
        a
      )
    ).toEqual({ kind: 'handled' })

    expect(a.showAttackState).toHaveBeenCalledWith(1)
    expect(a.sendAttackCycle).toHaveBeenCalledWith('m1', 2)
  })
})

describe('runCombatFrame', () => {
  it('continues movement when combat is inactive or player is missing', () => {
    const a = actions()

    expect(
      runCombatFrame({
        isInCombat: false,
        combatController: frameController('idle'),
        deltaTime: 16,
        currentPlayer: { position: { x: 0, y: 0, z: 0 } },
        playerStateName: 'moving',
        isMoving: true,
        currentSpeed: 1,
        chaseGoal: null,
        movementState: null,
        cooldownMs: 1500,
        pathing: directPathing(),
        getMonsterInfo: vi.fn(),
        attackLineBlocked: () => false,
        findMonsterPosition: vi.fn(),
        sendPlayerMove: vi.fn(),
        actions: a,
      })
    ).toEqual({ kind: 'continue_movement' })

    expect(a.stopMovingToIdle).not.toHaveBeenCalled()
  })

  it('ticks combat and applies handled outcomes', () => {
    const a = actions()

    expect(
      runCombatFrame({
        isInCombat: true,
        combatController: frameController('idle'),
        deltaTime: 16,
        currentPlayer: { position: { x: 1, y: 2, z: 3 } },
        playerStateName: 'moving',
        isMoving: true,
        currentSpeed: 1,
        chaseGoal: null,
        movementState: null,
        cooldownMs: 1500,
        pathing: directPathing(),
        getMonsterInfo: vi.fn(),
        attackLineBlocked: () => false,
        findMonsterPosition: vi.fn(),
        sendPlayerMove: vi.fn(),
        actions: a,
      })
    ).toEqual({ kind: 'handled' })

    expect(a.stopMovingToIdle).toHaveBeenCalledOnce()
  })
})

describe('beginAttack', () => {
  function runBeginAttack(
    overrides: Partial<Parameters<typeof beginAttack>[0]>
  ) {
    const calls = {
      beginCombat: vi.fn(() => 1),
      sendPlayerMove: vi.fn(),
      sendPlayerAttack: vi.fn(),
    }
    const result = beginAttack({
      monsterId: 'm1',
      monsterInfo: { state: 'idle' },
      currentPosition: { x: 1, y: 0, z: 2 },
      playerRotation: 0.5,
      previousPlayerState: playerState,
      lastSentPosition: null,
      ...calls,
      ...overrides,
    })
    return { ...calls, result }
  }

  it('ignores dead targets', () => {
    const { beginCombat, result } = runBeginAttack({
      monsterInfo: { state: 'dead' },
    })

    expect(result.kind).toBe('ignored_unattackable_target')
    expect(beginCombat).not.toHaveBeenCalled()
  })

  it('ignores targets with no local data', () => {
    const { beginCombat, sendPlayerAttack, result } = runBeginAttack({
      monsterInfo: undefined,
    })

    expect(result.kind).toBe('ignored_unattackable_target')
    expect(beginCombat).not.toHaveBeenCalled()
    expect(sendPlayerAttack).not.toHaveBeenCalled()
  })

  it('starts combat, syncs position, sends attack, and returns attack state', () => {
    const currentPosition = { x: 1, y: 0, z: 2 }
    const { beginCombat, sendPlayerMove, sendPlayerAttack, result } =
      runBeginAttack({ currentPosition })

    expect(beginCombat).toHaveBeenCalledWith('m1', true)
    expect(sendPlayerMove).toHaveBeenCalledWith(currentPosition, 0.5)
    expect(sendPlayerAttack).toHaveBeenCalledWith('m1')
    expect(result).toEqual({
      kind: 'started',
      nextPlayerState: {
        ...playerState,
        state: 'attack',
        rotation: 0.5,
        attackCounter: 1,
      },
    })
  })

  it('skips position sync when position and facing are unchanged', () => {
    const { sendPlayerMove } = runBeginAttack({
      currentPosition: { x: 1, y: 10, z: 2 },
      playerRotation: playerState.rotation,
      lastSentPosition: { x: 1, y: 0, z: 2 },
    })

    expect(sendPlayerMove).not.toHaveBeenCalled()
  })

  it('syncs the new facing even when the position is unchanged', () => {
    const currentPosition = { x: 1, y: 10, z: 2 }
    const { sendPlayerMove } = runBeginAttack({
      currentPosition,
      playerRotation: 1.5,
      lastSentPosition: { x: 1, y: 0, z: 2 },
    })

    expect(sendPlayerMove).toHaveBeenCalledWith(currentPosition, 1.5)
  })
})

describe('transitionAttackToIdle', () => {
  it('ignores non-attack states', () => {
    expect(transitionAttackToIdle(playerState)).toEqual({
      kind: 'ignored',
    })
  })

  it('builds idle state after attack', () => {
    const attackState: PlayerState = {
      ...playerState,
      state: 'attack',
      attackCounter: 2,
    }

    expect(transitionAttackToIdle(attackState)).toEqual({
      kind: 'idle',
      nextPlayerState: {
        ...attackState,
        state: 'idle',
        speed: 0,
        attackCounter: 0,
      },
    })
  })
})

describe('ensureAttackState', () => {
  it('ignores already attacking states', () => {
    expect(
      ensureAttackState({ ...playerState, state: 'attack' }, 1, 3)
    ).toEqual({
      kind: 'ignored',
    })
  })

  it('builds attack state when not already attacking', () => {
    expect(ensureAttackState(playerState, 1.25, 3)).toEqual({
      kind: 'attack',
      nextPlayerState: {
        ...playerState,
        state: 'attack',
        rotation: 1.25,
        attackCounter: 3,
      },
    })
  })
})
