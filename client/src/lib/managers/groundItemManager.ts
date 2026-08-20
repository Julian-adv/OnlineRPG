import { SvelteMap } from 'svelte/reactivity'
import { hmrSingleton } from '../utils/hmr'
import type { ServerGroundItem } from '../network/networkTypes'

export interface GroundItemData {
  instanceId: number
  itemDefId: string
  quantity: number
  enchant: number
  position: { x: number; y: number; z: number }
  floorLevel: number
  inHand?: boolean
  restingRotationY: number
  spawnAnimation?: GroundItemSpawnAnimation
}

export interface GroundItemSpawnAnimation {
  startTimeMs: number
  durationMs: number
  horizontalVelocity: { x: number; z: number }
  verticalVelocity: number
  gravity: number
  spinZTurns: number
}

export interface SpawnAnimationTransform {
  offsetX: number
  offsetY: number
  offsetZ: number
  spinZ: number
}

type SpawnOptions = {
  animateSpawn?: boolean
}

export function nowMs(): number {
  return typeof performance !== 'undefined' ? performance.now() : Date.now()
}

function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value))
}

// Decelerating ease (matches the polynomial-ease pattern used elsewhere).
function easeOut(progress: number): number {
  return 1 - (1 - progress) * (1 - progress)
}

// Pure function of absolute time: returns the item's current offset from its
// landing position plus its spin, so the view never needs to know the
// kinematic model. `position` stored on the item is the landing spot.
export function evaluateSpawnAnimation(
  anim: GroundItemSpawnAnimation,
  now: number
): SpawnAnimationTransform {
  const progress = clamp01((now - anim.startTimeMs) / anim.durationMs)
  const elapsedSeconds = (progress * anim.durationMs) / 1000
  const remainingSeconds = ((1 - progress) * anim.durationMs) / 1000
  const arcY = Math.max(
    0,
    anim.verticalVelocity * elapsedSeconds -
      0.5 * anim.gravity * elapsedSeconds * elapsedSeconds
  )
  return {
    offsetX: -anim.horizontalVelocity.x * remainingSeconds,
    offsetY: arcY,
    offsetZ: -anim.horizontalVelocity.z * remainingSeconds,
    spinZ: Math.PI * 2 * anim.spinZTurns * easeOut(progress),
  }
}

function createSpawnAnimation(rotationY: number): GroundItemSpawnAnimation {
  const horizontalSpeed = 0.65
  const verticalVelocity = 3.8
  const gravity = 6
  const modelLongAxisRotationY = rotationY + Math.PI / 2

  return {
    startTimeMs: nowMs(),
    durationMs: (2 * verticalVelocity * 1000) / gravity,
    horizontalVelocity: {
      x: Math.sin(modelLongAxisRotationY) * horizontalSpeed,
      z: Math.cos(modelLongAxisRotationY) * horizontalSpeed,
    },
    verticalVelocity,
    gravity,
    spinZTurns: 3,
  }
}

class GroundItemManager {
  items = new SvelteMap<number, GroundItemData>()
  private pickupInProgress = new Set<number>()
  private pendingRemoval = new Set<number>()

  spawn(item: ServerGroundItem, options: SpawnOptions = {}) {
    const restingRotationY = Math.random() * Math.PI * 2
    const spawnAnimation = options.animateSpawn
      ? createSpawnAnimation(restingRotationY)
      : undefined

    this.items.set(item.instance_id, {
      instanceId: item.instance_id,
      itemDefId: item.item_def_id,
      quantity: item.quantity,
      enchant: item.enchant,
      position: { ...item.position },
      floorLevel: item.floor_level,
      restingRotationY,
      spawnAnimation,
    })

    // Drop the animation once it finishes so the view stops re-deriving the
    // (now constant) transform every frame for a resting item.
    if (spawnAnimation) {
      setTimeout(
        () => this.clearSpawnAnimation(item.instance_id),
        spawnAnimation.durationMs
      )
    }
  }

  private clearSpawnAnimation(instanceId: number) {
    const item = this.items.get(instanceId)
    if (item?.spawnAnimation) {
      this.items.set(instanceId, { ...item, spawnAnimation: undefined })
    }
  }

  beginPickup(instanceId: number) {
    if (!this.items.has(instanceId)) return
    this.pickupInProgress.add(instanceId)
  }

  setInHand(instanceId: number) {
    const item = this.items.get(instanceId)
    if (!item) return
    this.items.set(instanceId, { ...item, inHand: true })
  }

  finishPickup(instanceId: number) {
    this.pickupInProgress.delete(instanceId)
    if (this.pendingRemoval.has(instanceId)) {
      this.pendingRemoval.delete(instanceId)
      this.items.delete(instanceId)
      return
    }
    // Pickup not confirmed by server (e.g., inventory full) — item returns to ground.
    const item = this.items.get(instanceId)
    if (item?.inHand) {
      this.items.set(instanceId, { ...item, inHand: false })
    }
  }

  remove(instanceId: number) {
    if (this.pickupInProgress.has(instanceId)) {
      this.pendingRemoval.add(instanceId)
      return
    }
    this.items.delete(instanceId)
  }

  // A pile someone took only part of: it stays, holding fewer units.
  setQuantity(instanceId: number, quantity: number) {
    const item = this.items.get(instanceId)
    if (!item) return
    this.items.set(instanceId, { ...item, quantity })
  }

  reset() {
    this.items.clear()
    this.pickupInProgress.clear()
    this.pendingRemoval.clear()
  }
}

export const groundItemManager = hmrSingleton(
  'groundItemManager',
  () => new GroundItemManager()
)
