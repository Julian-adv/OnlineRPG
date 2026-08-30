import type { ClickIntent } from '../../../managers/inputHandler'
import type { WallDirection } from '../../../utils/house-geometry'
import type {
  MovementState,
  PlayerState,
  Position,
} from '../../../utils/movementUtils'
import {
  buildIdleAfterInteract,
  buildInteractState,
  buildPickupState,
} from '../player-state-builders'

// ───────────────────────────────────────────────────────────────────────────
// Object interaction position math
// ───────────────────────────────────────────────────────────────────────────

export interface InteractionOffset {
  x?: number
  y?: number
  z?: number
}

export function getObjectInteractionEntryPosition(
  position: Position,
  offset?: InteractionOffset
): Pick<Position, 'x' | 'z'> {
  return {
    x: position.x + (offset?.x ?? 0),
    z: position.z + (offset?.z ?? 0),
  }
}

export function getObjectInteractionExitPosition(
  currentPosition: Position,
  rotation: number,
  footDistance = 0.7
): Pick<Position, 'x' | 'z'> {
  return {
    x: currentPosition.x + Math.sin(rotation) * footDistance,
    z: currentPosition.z + Math.cos(rotation) * footDistance,
  }
}

/** Where to stand after leaving an object: one cell to the side (the side
 *  nearer `toward`, the click destination, first), then one cell ahead — a
 *  chair usually faces a table, and stepping out into its solid cell left
 *  the player wedged on the passability grid. Straight ahead when every
 *  candidate is blocked. */
export function pickObjectExitPosition(
  currentPosition: Position,
  rotation: number,
  isBlocked: (x: number, z: number) => boolean,
  toward?: Pick<Position, 'x' | 'z'>
): Pick<Position, 'x' | 'z'> {
  const at = (r: number) =>
    getObjectInteractionExitPosition(currentPosition, r, 1.0)
  const left = at(rotation + Math.PI / 2)
  const right = at(rotation - Math.PI / 2)
  const forward = at(rotation)
  const dist2 = (p: Pick<Position, 'x' | 'z'>) =>
    toward ? (toward.x - p.x) ** 2 + (toward.z - p.z) ** 2 : 0
  const sides = dist2(left) <= dist2(right) ? [left, right] : [right, left]
  return [...sides, forward].find((c) => !isBlocked(c.x, c.z)) ?? forward
}

interface ObjectInteractionPlayer {
  position: Position
}

export function applyObjectInteractionPosition(
  player: ObjectInteractionPlayer,
  position: Pick<Position, 'x' | 'z'>,
  height: {
    hasHeightData: (x: number, z: number) => boolean
    sampleHeight: (x: number, z: number) => number
  }
) {
  player.position.x = position.x
  player.position.z = position.z
  if (height.hasHeightData(position.x, position.z)) {
    player.position.y = height.sampleHeight(position.x, position.z)
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Object interaction enter/exit transitions
// ───────────────────────────────────────────────────────────────────────────

type InteractIntent = Extract<ClickIntent, { type: 'interact_object' }>

interface BeginObjectInteractionInput {
  intent: InteractIntent
  previousPlayerState: PlayerState
  cancelCombat: () => void
}

export interface BeginObjectInteractionOutcome {
  isMoving: false
  movementTarget: Position | null
  playerRotation: number
  nextPlayerState: PlayerState
  entryPosition: Pick<Position, 'x' | 'z'>
}

export function beginObjectInteraction({
  intent,
  previousPlayerState,
  cancelCombat,
}: BeginObjectInteractionInput): BeginObjectInteractionOutcome {
  cancelCombat()

  return {
    isMoving: false,
    movementTarget: null,
    playerRotation: intent.rotation,
    nextPlayerState: buildInteractState(
      previousPlayerState,
      intent.position,
      intent.rotation,
      intent.interaction,
      intent.interactOffset?.y ?? 0
    ),
    entryPosition: getObjectInteractionEntryPosition(
      intent.position,
      intent.interactOffset
    ),
  }
}

export function exitObjectInteraction(
  previousPlayerState: PlayerState
): PlayerState {
  return buildIdleAfterInteract(previousPlayerState)
}

// ───────────────────────────────────────────────────────────────────────────
// Pickup enter transition
// ───────────────────────────────────────────────────────────────────────────

interface BeginPickupInput {
  instanceId: number
  previousPlayerState: PlayerState
  hasGroundItem: (instanceId: number) => boolean
  beginPickup: (instanceId: number) => void
  cancelCombat: () => void
}

export type BeginPickupOutcome =
  | { kind: 'ignored' }
  | {
      kind: 'started'
      pendingPickupInstanceId: number
      isMoving: false
      movementTarget: Position | null
      movementState: MovementState | null
      currentSpeed: 0
      nextPlayerState: PlayerState
    }

export function beginPickupInteraction({
  instanceId,
  previousPlayerState,
  hasGroundItem,
  beginPickup,
  cancelCombat,
}: BeginPickupInput): BeginPickupOutcome {
  if (previousPlayerState.state === 'dead') return { kind: 'ignored' }
  if (!hasGroundItem(instanceId)) return { kind: 'ignored' }

  beginPickup(instanceId)
  cancelCombat()

  return {
    kind: 'started',
    pendingPickupInstanceId: instanceId,
    isMoving: false,
    movementTarget: null,
    movementState: null,
    currentSpeed: 0,
    nextPlayerState: buildPickupState(previousPlayerState),
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Pickup grab / finish / exit
// ───────────────────────────────────────────────────────────────────────────

export function shouldFinishPendingPickup(
  pendingPickupInstanceId: number | null,
  playerState: PlayerState
): boolean {
  return (
    pendingPickupInstanceId !== null &&
    (playerState.state !== 'interact' ||
      playerState.interactionAnim !== 'pickup')
  )
}

export function finishPendingPickup(
  pendingPickupInstanceId: number | null,
  finishPickup: (instanceId: number) => void
): number | null {
  if (pendingPickupInstanceId === null) return null
  finishPickup(pendingPickupInstanceId)
  return null
}

interface PickupGrabActions {
  setInHand: (instanceId: number) => void
  remove: (instanceId: number) => void
  sendPickupItem: (instanceId: number) => void
}

export function handlePickupGrab(
  pendingPickupInstanceId: number | null,
  actions: PickupGrabActions
): void {
  if (pendingPickupInstanceId === null) return
  actions.setInHand(pendingPickupInstanceId)
  if (pendingPickupInstanceId < 0) {
    actions.remove(pendingPickupInstanceId)
    return
  }
  actions.sendPickupItem(pendingPickupInstanceId)
}

export type ExitPickupInteractionOutcome =
  | { kind: 'ignored' }
  | { kind: 'exited'; nextPlayerState: PlayerState }

export function exitPickupInteraction(
  previousPlayerState: PlayerState
): ExitPickupInteractionOutcome {
  if (
    previousPlayerState.state !== 'interact' ||
    previousPlayerState.interactionAnim !== 'pickup'
  ) {
    return { kind: 'ignored' }
  }

  return {
    kind: 'exited',
    nextPlayerState: buildIdleAfterInteract(previousPlayerState),
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Interact key (E) — door toggle on nearby door
// ───────────────────────────────────────────────────────────────────────────

interface DoorToggleTarget {
  houseId: string
  roomIndex: number
  wallDir: WallDirection
  segmentIndex: number
}

interface HandleInteractKeyInput {
  currentPlayer: {
    health: number
    position: Position
  } | null
  consumeInteract: () => boolean
  findNearestDoor: (
    x: number,
    z: number,
    y: number,
    range: number
  ) => DoorToggleTarget | null
  sendToggleDoor: (
    houseId: string,
    roomIndex: number,
    wallDir: WallDirection,
    segmentIndex: number
  ) => void
}

export function handleInteractKey({
  currentPlayer,
  consumeInteract,
  findNearestDoor,
  sendToggleDoor,
}: HandleInteractKeyInput): boolean {
  if (!currentPlayer || currentPlayer.health <= 0) return false
  if (!consumeInteract()) return false

  const door = findNearestDoor(
    currentPlayer.position.x,
    currentPlayer.position.z,
    currentPlayer.position.y,
    2.0
  )
  if (!door) return false

  sendToggleDoor(door.houseId, door.roomIndex, door.wallDir, door.segmentIndex)
  return true
}

// ───────────────────────────────────────────────────────────────────────────
// Interaction exit classification (what, if anything, to exit before moving)
// ───────────────────────────────────────────────────────────────────────────

export type InteractionExitKind = 'none' | 'pickup' | 'object'

export function getInteractionExitKind(
  playerState: PlayerState
): InteractionExitKind {
  if (playerState.state !== 'interact') return 'none'
  return playerState.interactionAnim === 'pickup' ? 'pickup' : 'object'
}
