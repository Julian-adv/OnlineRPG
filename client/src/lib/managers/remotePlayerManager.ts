import { SvelteMap } from 'svelte/reactivity'
import { get } from 'svelte/store'
import { hmrSingleton } from '../utils/hmr'
import { gameStore } from '../stores/gameStore'
import {
  calculateMovementStep,
  initMovementState,
  getMovementMode,
  hasTargetChanged,
  DEFAULT_MOVEMENT_CONFIG,
  SPRINT_SPEED_MULT,
  scaleMovementConfig,
  type Position,
  type MovementState,
  type MovementConfig,
  type PlayerState,
} from '../utils/movementUtils'
import { entityGroundY } from './entity-ground'
import { FishingAnimationName } from '../types/animations'
import { shortestWrappedDeltaX } from '../terrain/world-wrap'
import type { TerrainHeightManager } from './terrainHeightManager'

// Use the same movement config as local player
const MOVEMENT_CONFIG: MovementConfig = {
  ...DEFAULT_MOVEMENT_CONFIG,
}
const SPRINT_MOVEMENT_CONFIG = scaleMovementConfig(
  MOVEMENT_CONFIG,
  SPRINT_SPEED_MULT
)

/// Far enough that the player went somewhere, rather than the resting flush
/// that lands on the spot they already stopped at.
const PICKUP_CANCEL_DISTANCE = 0.2

function movedFar(from: Position, to: Position): boolean {
  const dx = shortestWrappedDeltaX(from.x, to.x)
  const dz = to.z - from.z
  return dx * dx + dz * dz > PICKUP_CANCEL_DISTANCE * PICKUP_CANCEL_DISTANCE
}

class PlayerStateManager {
  players = new SvelteMap<number, PlayerState>()

  heightManager: TerrainHeightManager | null = null

  // Attack animation duration in seconds (updated from actual animation data)
  attackAnimationDuration = 1.0

  // Remote player movement data (for acceleration/deceleration)
  private movementData = new SvelteMap<number, MovementState>()

  // Server-authoritative movement targets for each remote player.
  private targetPositions = new SvelteMap<number, Position>()

  // Server-authoritative target rotation for each remote player.
  private targetRotations = new SvelteMap<number, number>()
  // Only read inside the update loop — no reactivity needed.
  private targetSprinting = new Map<number, boolean>()

  // Queue for pending attacks when player is still moving
  private attackQueue = new SvelteMap<number, string[]>()

  // Buffered position/rotation received during attack animation (1-slot queue).
  // Applied when the attack ends.
  private pendingMove = new Map<
    number,
    { position: Position; rotation: number; sprinting: boolean }
  >()

  // Timestamp (performance.now()) when each player's attack animation started
  private attackStartTimes = new Map<number, number>()

  // Move remote players toward their target positions with acceleration/deceleration
  update(deltaTime: number) {
    const dt = deltaTime / 1000 // Convert to seconds
    const now = performance.now()

    // Check for attack animations that have finished
    this.attackStartTimes.forEach((startTime, playerId) => {
      const elapsed = (now - startTime) / 1000
      if (elapsed < this.attackAnimationDuration) return

      this.attackStartTimes.delete(playerId)

      const p = this.players.get(playerId)
      if (!p || p.state !== 'attack') return

      const pending = this.pendingMove.get(playerId)
      this.pendingMove.delete(playerId)

      if (pending) {
        // Apply buffered position/rotation from move received during attack
        this.targetPositions.set(playerId, pending.position)
        this.targetRotations.set(playerId, pending.rotation)
        this.targetSprinting.set(playerId, pending.sprinting)
        this.players.set(playerId, {
          ...p,
          state: 'idle',
          rotation: pending.rotation,
        })
      } else {
        this.players.set(playerId, {
          ...p,
          state: 'idle',
        })
      }
    })

    // Snapshot other-player store state once for the frame (torch lookup below).
    const otherPlayers = get(gameStore).otherPlayers

    // Update players
    this.targetPositions.forEach((targetPos, playerId) => {
      // Get current interpolated position or initialize from player position
      const currentPlayer = this.players.get(playerId)
      if (!currentPlayer) return

      // Skip movement update if player is attacking, dead, or interacting
      if (
        currentPlayer?.state === 'attack' ||
        currentPlayer?.state === 'dead' ||
        currentPlayer?.state === 'interact'
      ) {
        return
      }

      const currentPos = currentPlayer.position

      // Get or initialize movement data
      let movement = this.movementData.get(playerId)
      const targetChanged = hasTargetChanged(movement, targetPos)

      if (targetChanged) {
        // New target - initialize movement from current position
        movement = initMovementState(
          currentPos,
          targetPos,
          movement?.currentSpeed ?? 0
        )
        this.movementData.set(playerId, movement)
      }

      // movement is guaranteed to be defined after above block
      if (!movement) return

      // Calculate movement step
      const sprinting = this.targetSprinting.get(playerId) ?? false
      const movementConfig = sprinting
        ? SPRINT_MOVEMENT_CONFIG
        : MOVEMENT_CONFIG
      const result = calculateMovementStep(
        currentPos,
        movement,
        movementConfig,
        dt
      )

      // calculateMovementStep only advances XZ and carries Y over, and the
      // move protocol has no per-waypoint Y, so the ground has to be
      // resampled here. Without it a remote keeps the Y it entered the floor
      // with, which reads as sinking through dungeon and house stairs.
      result.newPos.y = entityGroundY(
        this.heightManager,
        otherPlayers.get(playerId)?.floorLevel ?? 0,
        result.newPos.x,
        result.newPos.z,
        currentPos.y
      )

      // Update movement state
      movement.currentSpeed = result.newSpeed
      this.movementData.set(playerId, movement)

      // Update player
      // Since we skip movement update if player is already attacking,
      // currentState will just be based on whether they arrived
      const currentState = result.arrived ? 'idle' : 'moving'

      if (result.arrived) {
        // Use server-authoritative rotation on arrival (handles face-only packets)
        const targetRotation = this.targetRotations.get(playerId)
        this.players.set(playerId, {
          position: result.newPos,
          state: currentState,
          speed: 0,
          rotation:
            targetRotation ?? currentPlayer?.rotation ?? result.rotation,
          movementMode: undefined,
        })

        // Check for queued attacks upon arrival
        const queue = this.attackQueue.get(playerId)
        if (queue && queue.length > 0) {
          console.log(
            `[RemotePlayerManager] Executing queued attack for ${playerId} upon arrival`
          )
          queue.shift() // Consume one attack
          if (queue.length === 0) {
            this.attackQueue.delete(playerId)
          } else {
            this.attackQueue.set(playerId, queue)
          }
          this.executeAttack(playerId)
        }
      } else {
        const hasTorch = otherPlayers.get(playerId)?.torchOn ?? false
        const movementMode = getMovementMode(
          movement.totalDistance,
          hasTorch,
          sprinting
        )

        this.players.set(playerId, {
          position: result.newPos,
          state: currentState,
          speed: result.newSpeed,
          rotation: result.rotation,
          movementMode,
        })
      }
    })
  }

  // Initialize remote player state with position and rotation
  initPlayer(playerId: number, position: Position, rotation: number) {
    this.targetPositions.set(playerId, { ...position })
    this.targetSprinting.delete(playerId)
    this.players.set(playerId, {
      position: { ...position },
      state: 'idle',
      speed: 0,
      rotation,
    })
  }

  // Clean up data for players that have left
  removePlayer(playerId: number) {
    this.players.delete(playerId)
    this.movementData.delete(playerId)
    this.targetPositions.delete(playerId)
    this.targetRotations.delete(playerId)
    this.targetSprinting.delete(playerId)
    this.attackQueue.delete(playerId)
    this.pendingMove.delete(playerId)
    this.attackStartTimes.delete(playerId)
  }

  // Reset all data
  reset() {
    this.players.clear()
    this.movementData.clear()
    this.targetPositions.clear()
    this.targetRotations.clear()
    this.targetSprinting.clear()
    this.attackQueue.clear()
    this.pendingMove.clear()
    this.attackStartTimes.clear()
  }

  handleDead(playerId: number) {
    const player = this.players.get(playerId)
    if (!player) return

    this.attackStartTimes.delete(playerId)
    this.players.set(playerId, {
      ...player,
      state: 'dead',
      speed: 0,
    })
  }

  handleRespawn(playerId: number, position: Position, rotation: number) {
    this.movementData.delete(playerId)
    this.attackQueue.delete(playerId)
    this.attackStartTimes.delete(playerId)
    this.targetPositions.set(playerId, { ...position })
    this.targetSprinting.delete(playerId)
    this.players.set(playerId, {
      position: { ...position },
      state: 'idle',
      speed: 0,
      rotation,
    })
  }

  teleportPlayer(playerId: number, position: Position, rotation: number) {
    this.targetPositions.set(playerId, { ...position })
    this.targetSprinting.delete(playerId)
    this.movementData.delete(playerId)
    this.players.set(playerId, {
      position: { ...position },
      state: 'idle',
      speed: 0,
      rotation,
    })
  }

  handleInteraction(
    playerId: number,
    anim: string,
    offsetY: number,
    position?: Position,
    rotation?: number
  ) {
    const player = this.players.get(playerId)
    if (!player) return

    const newState: PlayerState = {
      ...player,
      state: 'interact',
      speed: 0,
      interactionAnim: anim,
      interactOffsetY: offsetY,
    }
    if (position) {
      newState.position = { ...position }
      this.targetPositions.set(playerId, { ...position })
    }
    if (rotation !== undefined) {
      newState.rotation = rotation
      this.targetRotations.set(playerId, rotation)
    }
    this.players.set(playerId, newState)
  }

  /** A remote's one-shot interact clip ended. A finished fishing cast hands
   *  over to the looping fishing idle; anything else drops back to idle. */
  handleInteractionFinished(playerId: number) {
    const player = this.players.get(playerId)
    if (player?.state !== 'interact') return
    if (player.interactionAnim === FishingAnimationName.CAST) {
      this.handleInteraction(playerId, FishingAnimationName.IDLE, 0)
    } else {
      this.handleStopInteraction(playerId)
    }
  }

  handleStopInteraction(playerId: number) {
    const player = this.players.get(playerId)
    if (!player || player.state !== 'interact') return

    this.players.set(playerId, {
      ...player,
      state: 'idle',
      interactionAnim: undefined,
      interactOffsetY: undefined,
    })
  }

  handleAttack(playerId: number) {
    const player = this.players.get(playerId)
    if (!player) return

    const movement = this.movementData.get(playerId)
    const isMoving = movement && movement.currentSpeed > 0.01

    if (isMoving) {
      // Still moving - queue the attack
      console.log(`[RemotePlayerManager] Queueing attack for ${playerId}`)
      const queue = this.attackQueue.get(playerId) || []
      queue.push('attack') // Currently monsterId isn't stored in PlayerState, so just queue an 'attack' event
      this.attackQueue.set(playerId, queue)
    } else {
      // Not moving - execute immediately
      this.executeAttack(playerId)
    }
  }

  setTargetPosition(
    playerId: number,
    targetPosition: Position,
    rotation: number,
    sprinting = false
  ) {
    const player = this.players.get(playerId)

    // During attack animation, buffer the move for after the animation ends.
    if (player?.state === 'attack') {
      this.pendingMove.set(playerId, {
        position: { ...targetPosition },
        rotation,
        sprinting,
      })
      return
    }

    // The picker's own client drops the crouch the moment it moves, so a move
    // that actually goes somewhere ends it here too — otherwise the remote
    // stays frozen for the rest of a clip the picker already abandoned. Held
    // poses (bench, forge) are left to their StopInteraction. The distance
    // gate ignores the resting flush sent when the walk to the item ends.
    if (
      player?.state === 'interact' &&
      player.interactionAnim === 'pickup' &&
      movedFar(player.position, targetPosition)
    ) {
      this.handleStopInteraction(playerId)
    }

    this.targetPositions.set(playerId, { ...targetPosition })
    this.targetRotations.set(playerId, rotation)
    this.targetSprinting.set(playerId, sprinting)
  }

  private executeAttack(playerId: number) {
    const player = this.players.get(playerId)
    if (!player) return

    // Set state to attack and record start time for update() to check
    this.players.set(playerId, {
      ...player,
      state: 'attack',
      attackCounter: (player.attackCounter ?? 0) + 1,
    })
    this.attackStartTimes.set(playerId, performance.now())
  }
}

export const remotePlayerManager = hmrSingleton(
  'remotePlayerManager',
  () => new PlayerStateManager()
)
