import type { MonsterInfo } from '../../../managers/combatController'
import type {
  MovementConfig,
  MovementState,
  PlayerStateName,
  Position,
} from '../../../utils/movementUtils'
import {
  runCombatFrame,
  type AttackLineBlocked,
  type CombatControllerLike,
  type CombatOutcomeActions,
} from './combat'
import {
  stepMovementSubstrate,
  type MovementSubstrateOutcome,
  type Pathing,
  type PathWaypoint,
  type SendPlayerMove,
} from './movement-substrate'

// ───────────────────────────────────────────────────────────────────────────
// Terrain Y re-alignment (skipped while seated/interacting)
// ───────────────────────────────────────────────────────────────────────────

interface TerrainSyncPlayer {
  position: {
    x: number
    y: number
    z: number
  }
}

interface SyncTerrainHeightInput {
  player: TerrainSyncPlayer | null
  hasHeightData: (x: number, z: number) => boolean
  sampleHeight: (x: number, z: number) => number
  epsilon?: number
}

/** `position.y` is always the ground (furniture height rides
 *  `interactOffsetY`), so a pose follows the house floor offset too — the
 *  housing layer derives the floor from `y - offset`, and a stale y there
 *  spirals it onto the wrong storey. */
export function syncPlayerTerrainHeight({
  player,
  hasHeightData,
  sampleHeight,
  epsilon = 0.001,
}: SyncTerrainHeightInput): boolean {
  if (!player) return false

  const { x, y, z } = player.position
  if (!hasHeightData(x, z)) return false

  const terrainY = sampleHeight(x, z)
  if (Math.abs(y - terrainY) <= epsilon) return false

  player.position.y = terrainY
  return true
}

// ───────────────────────────────────────────────────────────────────────────
// Movement substrate outcome application
// ───────────────────────────────────────────────────────────────────────────

export interface MovementOutcomeActions {
  stopMovement: () => void
  triggerJumpFeedback: () => void
  setNextWaypoint: (
    currentSpeed: number,
    playerRotation: number,
    movementTarget: Position,
    movementState: MovementState,
    currentWaypointIndex: number
  ) => void
  arrive: (currentSpeed: number, playerRotation: number) => void
  continueMovement: (
    currentSpeed: number,
    playerRotation: number,
    totalDistance: number
  ) => void
}

export function applyMovementSubstrateOutcome(
  outcome: MovementSubstrateOutcome,
  actions: MovementOutcomeActions
) {
  switch (outcome.kind) {
    case 'blocked':
      actions.stopMovement()
      return

    case 'slope_blocked':
      actions.stopMovement()
      actions.triggerJumpFeedback()
      return

    case 'next_waypoint':
      actions.setNextWaypoint(
        outcome.currentSpeed,
        outcome.playerRotation,
        outcome.movementTarget,
        outcome.movementState,
        outcome.currentWaypointIndex
      )
      return

    case 'arrived':
      actions.arrive(outcome.currentSpeed, outcome.playerRotation)
      return

    case 'continued':
      actions.continueMovement(
        outcome.currentSpeed,
        outcome.playerRotation,
        outcome.totalDistance
      )
      return

    default: {
      const _exhaustive: never = outcome
      return _exhaustive
    }
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Movement frame (single substrate step + outcome application)
// ───────────────────────────────────────────────────────────────────────────

interface RunMovementFrameInput {
  currentPos: Position
  movementTarget: Position
  movementState: MovementState
  pathWaypoints: PathWaypoint[]
  currentWaypointIndex: number
  config: MovementConfig
  deltaTimeSeconds: number
  sampleHeight: (x: number, z: number) => number
  waypointHeight: (floor: number, x: number, z: number) => number
  isMovementBlocked: (
    fromX: number,
    fromZ: number,
    toX: number,
    toZ: number,
    y: number
  ) => boolean
  isUphillTooSteep: (
    x: number,
    z: number,
    y: number,
    dirX: number,
    dirZ: number
  ) => boolean
  setFloorLevel: (floor: number) => void
  writePlayerPosition: (position: Position, rotation: number) => void
  sendPlayerMove: SendPlayerMove
  actions: MovementOutcomeActions
}

export function runMovementFrame(input: RunMovementFrameInput) {
  const outcome = stepMovementSubstrate(input)
  applyMovementSubstrateOutcome(outcome, input.actions)
}

// ───────────────────────────────────────────────────────────────────────────
// Per-frame player movement tick (death/respawn → terrain → combat → movement)
// ───────────────────────────────────────────────────────────────────────────

interface MovementTickPlayer {
  health: number
  position: Position
}

interface RunPlayerMovementTickInput {
  deltaTime: number
  currentPlayer: MovementTickPlayer | null
  playerStateName: PlayerStateName
  isMoving: boolean
  currentSpeed: number
  movementTarget: Position | null
  movementState: MovementState | null
  pathWaypoints: PathWaypoint[]
  currentWaypointIndex: number
  chaseGoal: Position | null
  config: MovementConfig
  isInCombat: boolean
  combatController: CombatControllerLike
  cooldownMs: number
  /** Equipped weapon's reach; the chase breaks off here. */
  attackRange: number
  chasePathing: Pathing
  getMonsterInfo: (monsterId: string) => MonsterInfo | undefined
  findMonsterPosition: (monsterId: string) => Position | undefined
  attackLineBlocked: AttackLineBlocked
  sampleHeight: (x: number, z: number) => number
  waypointHeight: (floor: number, x: number, z: number) => number
  hasHeightData: (x: number, z: number) => boolean
  isMovementBlocked: (
    fromX: number,
    fromZ: number,
    toX: number,
    toZ: number,
    y: number
  ) => boolean
  isUphillTooSteep: (
    x: number,
    z: number,
    y: number,
    dirX: number,
    dirZ: number
  ) => boolean
  setFloorLevel: (floor: number) => void
  writePlayerPosition: (position: Position, rotation: number) => void
  sendPlayerMove: SendPlayerMove
  actions: {
    transitionToDead: () => void
    transitionToRespawned: () => void
    resetStoppedSpeed: () => void
    combat: CombatOutcomeActions
    movement: MovementOutcomeActions
  }
}

export function runPlayerMovementTick({
  deltaTime,
  currentPlayer,
  playerStateName,
  isMoving,
  currentSpeed,
  movementTarget,
  movementState,
  pathWaypoints,
  currentWaypointIndex,
  chaseGoal,
  config,
  isInCombat,
  combatController,
  cooldownMs,
  attackRange,
  chasePathing,
  getMonsterInfo,
  findMonsterPosition,
  attackLineBlocked,
  sampleHeight,
  waypointHeight,
  hasHeightData,
  isMovementBlocked,
  isUphillTooSteep,
  setFloorLevel,
  writePlayerPosition,
  sendPlayerMove,
  actions,
}: RunPlayerMovementTickInput) {
  if (currentPlayer && currentPlayer.health <= 0) {
    actions.transitionToDead()
    return
  }
  if (currentPlayer && currentPlayer.health > 0 && playerStateName === 'dead') {
    actions.transitionToRespawned()
    return
  }

  syncPlayerTerrainHeight({
    player: currentPlayer,
    hasHeightData,
    sampleHeight,
  })

  const combatApplication = runCombatFrame({
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
    attackRange,
    pathing: chasePathing,
    getMonsterInfo,
    findMonsterPosition,
    attackLineBlocked,
    sendPlayerMove,
    actions: actions.combat,
  })

  // A re-routed chase rewrote the moving state's target and path; the locals
  // destructured above are now stale, so let the next frame walk the new one.
  if (combatApplication.kind === 'handled') return

  if (!isMoving || !movementTarget || !currentPlayer || !movementState) {
    if (currentSpeed > 0) {
      actions.resetStoppedSpeed()
    }
    return
  }

  runMovementFrame({
    currentPos: {
      x: currentPlayer.position.x,
      y: currentPlayer.position.y,
      z: currentPlayer.position.z,
    },
    movementTarget,
    movementState,
    pathWaypoints,
    currentWaypointIndex,
    config,
    deltaTimeSeconds: deltaTime / 1000,
    sampleHeight,
    waypointHeight,
    isMovementBlocked,
    isUphillTooSteep,
    setFloorLevel,
    writePlayerPosition,
    sendPlayerMove,
    actions: actions.movement,
  })
}
