import type {
  CombatUpdateResult,
  MonsterInfo,
} from '../../../managers/combatController'
import {
  initMovementState,
  type MovementState,
  type PlayerState,
  type PlayerStateName,
  type Position,
} from '../../../utils/movementUtils'
import { shortestWrappedDeltaX } from '../../../terrain/world-wrap'
import { PLAYER_ATTACK_RANGE_METERS } from '../../../data/combatTiming'
import {
  buildAttackState,
  buildIdleAfterAttack,
} from '../player-state-builders'
import {
  routeFirstLeg,
  type Pathing,
  type RoutedLeg,
  type SendPlayerMove,
} from './movement-substrate'

// ───────────────────────────────────────────────────────────────────────────
// Chase target update
// ───────────────────────────────────────────────────────────────────────────

/** How far the monster must drift from the goal the live path was routed to
 *  before routing again is worth it. `CombatController` already throttles chase
 *  updates to ~1Hz; this drops the ones where it is only milling about. */
const CHASE_REPATH_DISTANCE = 1.5

interface ApplyChaseTargetInput {
  currentPos: Position
  newTarget?: Position
  /** Monster position the live path was routed to, or null when there is none. */
  chaseGoal: Position | null
  movementState: MovementState | null
  currentSpeed: number
  pathing: Pathing
  sendPlayerMove: SendPlayerMove
}

/** A freshly routed chase leg, ready to install on the moving state. */
export interface ChaseMovement extends RoutedLeg {
  movementState: MovementState
  chaseGoal: Position
}

export type ChaseTargetOutcome =
  | { kind: 'unchanged' }
  | ({ kind: 'updated' } & ChaseMovement)

export function applyChaseTargetUpdate({
  currentPos,
  newTarget,
  chaseGoal,
  movementState,
  currentSpeed,
  pathing,
  sendPlayerMove,
}: ApplyChaseTargetInput): ChaseTargetOutcome {
  if (!newTarget) return { kind: 'unchanged' }

  if (
    chaseGoal &&
    Math.abs(shortestWrappedDeltaX(chaseGoal.x, newTarget.x)) <=
      CHASE_REPATH_DISTANCE &&
    Math.abs(chaseGoal.z - newTarget.z) <= CHASE_REPATH_DISTANCE
  ) {
    return { kind: 'unchanged' }
  }

  const leg = routeFirstLeg(currentPos, newTarget, pathing, sendPlayerMove)

  // Unlike a click, chase retargets a live integrator rather than starting one.
  const start = { x: currentPos.x, y: currentPos.y, z: currentPos.z }
  const nextMovementState =
    movementState ?? initMovementState(start, leg.movementTarget, currentSpeed)
  if (movementState) {
    const dx = shortestWrappedDeltaX(currentPos.x, leg.movementTarget.x)
    const dz = leg.movementTarget.z - currentPos.z
    nextMovementState.targetPos = { ...leg.movementTarget }
    nextMovementState.totalDistance = Math.sqrt(dx * dx + dz * dz)
    nextMovementState.startPos = start
  }

  return {
    kind: 'updated',
    ...leg,
    movementState: nextMovementState,
    chaseGoal: { ...newTarget },
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Combat tick
// ───────────────────────────────────────────────────────────────────────────

export interface CombatControllerLike {
  targetMonsterId: string | null
  update(
    deltaTime: number,
    playerPos: Position,
    monsterInfo: MonsterInfo | undefined,
    monsterObjPos: Position | undefined,
    isMoving: boolean,
    cooldownMs: number,
    currentPlayerState: string,
    lineBlocked: boolean
  ): CombatUpdateResult
}

function withinAttackRange(from: Position, to: Position): boolean {
  const dx = shortestWrappedDeltaX(from.x, to.x)
  const dz = to.z - from.z
  return dx * dx + dz * dz <= PLAYER_ATTACK_RANGE_METERS ** 2
}

/** Whether a wall stands between the two points on `floor`. */
export type AttackLineBlocked = (
  from: Position,
  to: Position,
  floor: number
) => boolean

export interface TickCombatInput {
  combatController: CombatControllerLike
  deltaTime: number
  playerPos: Position
  playerStateName: PlayerStateName
  isMoving: boolean
  currentSpeed: number
  chaseGoal: Position | null
  movementState: MovementState | null
  cooldownMs: number
  pathing: Pathing
  getMonsterInfo: (monsterId: string) => MonsterInfo | undefined
  findMonsterPosition: (monsterId: string) => Position | undefined
  attackLineBlocked: AttackLineBlocked
  sendPlayerMove: SendPlayerMove
}

export type CombatTickOutcome =
  | { kind: 'none' }
  | { kind: 'idle' }
  | { kind: 'reached_attack_range'; monsterId: string }
  | { kind: 'chasing_unchanged' }
  | ({ kind: 'chasing_updated' } & ChaseMovement)
  | { kind: 'attacking'; playerRotation: number }
  | {
      kind: 'attack_cycle'
      monsterId: string
      playerRotation: number
    }

export function tickCombat({
  combatController,
  deltaTime,
  playerPos,
  playerStateName,
  isMoving,
  currentSpeed,
  chaseGoal,
  movementState,
  cooldownMs,
  pathing,
  getMonsterInfo,
  findMonsterPosition,
  attackLineBlocked,
  sendPlayerMove,
}: TickCombatInput): CombatTickOutcome {
  const targetId = combatController.targetMonsterId
  if (!targetId) return { kind: 'none' }

  // Only a target already within reach can be walled off in a way the swing
  // decision reads — the wasm-side wall scan stays off the approach frames.
  const monsterPos = findMonsterPosition(targetId)
  const lineBlocked =
    !!monsterPos &&
    withinAttackRange(playerPos, monsterPos) &&
    attackLineBlocked(playerPos, monsterPos, pathing.currentFloor)
  const result = combatController.update(
    deltaTime,
    playerPos,
    getMonsterInfo(targetId),
    monsterPos,
    isMoving,
    cooldownMs,
    playerStateName,
    lineBlocked
  )

  switch (result.action) {
    case 'none':
      return { kind: 'none' }
    case 'idle':
      return { kind: 'idle' }
    case 'reached_attack_range':
      return { kind: 'reached_attack_range', monsterId: targetId }
    case 'chasing': {
      const chase = applyChaseTargetUpdate({
        currentPos: playerPos,
        newTarget: result.newTarget,
        chaseGoal,
        movementState,
        currentSpeed,
        pathing,
        sendPlayerMove,
      })

      if (chase.kind === 'unchanged') return { kind: 'chasing_unchanged' }
      return { ...chase, kind: 'chasing_updated' }
    }
    case 'attacking':
      return { kind: 'attacking', playerRotation: result.rotation }
    case 'attack_cycle':
      return {
        kind: 'attack_cycle',
        monsterId: result.monsterId,
        playerRotation: result.rotation,
      }
    default: {
      const _exhaustive: never = result
      return _exhaustive
    }
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Combat tick outcome application
// ───────────────────────────────────────────────────────────────────────────

export type CombatOutcomeApplication =
  | { kind: 'continue_movement' }
  | { kind: 'handled' }

export interface CombatOutcomeActions {
  stopMovingToIdle: () => void
  prepareReachedAttackRange: () => void
  beginAttack: (monsterId: string) => void
  setChasingMovement: (chase: ChaseMovement) => void
  showAttackState: (playerRotation: number) => void
  sendAttackCycle: (monsterId: string, playerRotation: number) => void
}

export function applyCombatTickOutcome(
  outcome: CombatTickOutcome,
  actions: CombatOutcomeActions
): CombatOutcomeApplication {
  switch (outcome.kind) {
    case 'idle':
      actions.stopMovingToIdle()
      return { kind: 'handled' }

    case 'reached_attack_range':
      actions.prepareReachedAttackRange()
      actions.beginAttack(outcome.monsterId)
      return { kind: 'handled' }

    case 'chasing_updated': {
      const { kind: _kind, ...chase } = outcome
      actions.setChasingMovement(chase)
      // The caller's movement locals still describe the path we just replaced,
      // so skip this frame's step rather than walk a stale waypoint list.
      return { kind: 'handled' }
    }

    case 'chasing_unchanged':
    case 'none':
      return { kind: 'continue_movement' }

    case 'attacking':
      actions.showAttackState(outcome.playerRotation)
      return { kind: 'handled' }

    case 'attack_cycle':
      actions.sendAttackCycle(outcome.monsterId, outcome.playerRotation)
      return { kind: 'handled' }

    default: {
      const _exhaustive: never = outcome
      return _exhaustive
    }
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Combat frame (combat sub-step of the movement tick)
// ───────────────────────────────────────────────────────────────────────────

interface CombatFramePlayer {
  position: Position
}

interface RunCombatFrameInput {
  isInCombat: boolean
  combatController: CombatControllerLike
  deltaTime: number
  currentPlayer: CombatFramePlayer | null
  playerStateName: PlayerStateName
  isMoving: boolean
  currentSpeed: number
  chaseGoal: Position | null
  movementState: MovementState | null
  cooldownMs: number
  pathing: Pathing
  getMonsterInfo: (monsterId: string) => MonsterInfo | undefined
  findMonsterPosition: (monsterId: string) => Position | undefined
  attackLineBlocked: AttackLineBlocked
  sendPlayerMove: SendPlayerMove
  actions: CombatOutcomeActions
}

export function runCombatFrame({
  isInCombat,
  combatController,
  deltaTime,
  currentPlayer,
  playerStateName,
  isMoving,
  currentSpeed,
  chaseGoal,
  movementState,
  cooldownMs,
  pathing,
  getMonsterInfo,
  findMonsterPosition,
  attackLineBlocked,
  sendPlayerMove,
  actions,
}: RunCombatFrameInput): CombatOutcomeApplication {
  if (!isInCombat || !currentPlayer) return { kind: 'continue_movement' }

  const combat = tickCombat({
    combatController,
    deltaTime,
    playerPos: {
      x: currentPlayer.position.x,
      y: currentPlayer.position.y,
      z: currentPlayer.position.z,
    },
    playerStateName,
    isMoving,
    currentSpeed,
    chaseGoal,
    movementState,
    cooldownMs,
    pathing,
    getMonsterInfo,
    findMonsterPosition,
    attackLineBlocked,
    sendPlayerMove,
  })

  return applyCombatTickOutcome(combat, actions)
}

// ───────────────────────────────────────────────────────────────────────────
// Attack state transitions
// ───────────────────────────────────────────────────────────────────────────

export interface AttackTargetInfo {
  state?: string
  isDeadPending?: boolean
}

interface BeginAttackInput {
  monsterId: string
  monsterInfo: AttackTargetInfo | undefined
  currentPosition: Position | null
  playerRotation: number
  previousPlayerState: PlayerState
  lastSentPosition: Position | null
  /** Starts combat and returns the counter for this swing. */
  beginCombat: (monsterId: string, inRange: boolean) => number
  sendPlayerMove: (position: Position, rotation: number) => void
  sendPlayerAttack: (monsterId: string) => void
}

export type BeginAttackOutcome =
  | { kind: 'ignored_unattackable_target' }
  | {
      kind: 'started'
      nextPlayerState: PlayerState
    }

export function beginAttack({
  monsterId,
  monsterInfo,
  currentPosition,
  playerRotation,
  previousPlayerState,
  lastSentPosition,
  beginCombat,
  sendPlayerMove,
  sendPlayerAttack,
}: BeginAttackInput): BeginAttackOutcome {
  // No local data means the monster is gone (removed/desynced): a swing at it
  // can only be rejected, so treat it like a dead target.
  if (
    !monsterInfo ||
    monsterInfo.state === 'dead' ||
    monsterInfo.isDeadPending
  ) {
    return { kind: 'ignored_unattackable_target' }
  }

  const attackCounter = beginCombat(monsterId, true)

  if (currentPosition) {
    const shouldSendMove =
      !lastSentPosition ||
      Math.abs(currentPosition.x - lastSentPosition.x) > 0.01 ||
      Math.abs(currentPosition.z - lastSentPosition.z) > 0.01 ||
      Math.abs(playerRotation - previousPlayerState.rotation) > 0.01

    if (shouldSendMove) {
      sendPlayerMove(currentPosition, playerRotation)
    }
  }

  sendPlayerAttack(monsterId)

  return {
    kind: 'started',
    nextPlayerState: buildAttackState(
      previousPlayerState,
      playerRotation,
      attackCounter
    ),
  }
}

export type AttackToIdleTransition =
  | { kind: 'ignored' }
  | { kind: 'idle'; nextPlayerState: PlayerState }

export function transitionAttackToIdle(
  previousPlayerState: PlayerState
): AttackToIdleTransition {
  if (previousPlayerState.state !== 'attack') return { kind: 'ignored' }
  return {
    kind: 'idle',
    nextPlayerState: buildIdleAfterAttack(previousPlayerState),
  }
}

export type EnsureAttackStateOutcome =
  | { kind: 'ignored' }
  | { kind: 'attack'; nextPlayerState: PlayerState }

export function ensureAttackState(
  previousPlayerState: PlayerState,
  playerRotation: number,
  attackCounter: number
): EnsureAttackStateOutcome {
  if (previousPlayerState.state === 'attack') return { kind: 'ignored' }
  return {
    kind: 'attack',
    nextPlayerState: buildAttackState(
      previousPlayerState,
      playerRotation,
      attackCounter
    ),
  }
}
