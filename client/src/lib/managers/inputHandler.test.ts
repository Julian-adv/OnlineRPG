import { beforeEach, describe, expect, it, vi } from 'vitest'
import * as THREE from 'three'

const MIN_FISHABLE_DEPTH_M = 0.3

vi.mock('../wasm/onlinerpg_shared', () => ({
  min_fishable_depth_m: () => MIN_FISHABLE_DEPTH_M,
}))

import { inputHandler, type RaycastContext } from './inputHandler'
import { resetFishingStore } from '../stores/fishingStore'

const RECT = { left: 0, top: 0, width: 100, height: 100 }

/** A canvas click at the viewport center, as processCanvasClick consumes it. */
function centerClick(): MouseEvent {
  return {
    clientX: RECT.width / 2,
    clientY: RECT.height / 2,
    target: { getBoundingClientRect: () => RECT },
  } as unknown as MouseEvent
}

/** Camera 10m above the origin looking straight down at a flat ground plane
 *  at y = 0 — the center click ray hits the ground at (0, 0, 0). */
function groundScene() {
  const camera = new THREE.PerspectiveCamera(50, 1, 0.1, 100)
  camera.position.set(0, 10, 0)
  camera.lookAt(0, 0, 0)
  camera.updateMatrixWorld(true)

  const ground = new THREE.Mesh(new THREE.PlaneGeometry(100, 100))
  ground.rotateX(-Math.PI / 2)
  ground.updateMatrixWorld(true)

  return { camera, ground }
}

function contextWith(
  overrides: Partial<RaycastContext> = {}
): RaycastContext {
  const { camera, ground } = groundScene()
  return {
    camera,
    monsterMeshes: [],
    npcMeshes: [],
    doorMeshes: [],
    objectMeshes: [],
    propMeshes: [],
    groundItemMeshes: [],
    groundMeshes: [ground],
    playerPosition: { x: 0, y: 0, z: 0 },
    playerFloorLevel: 0,
    isMonsterDead: () => false,
    ...overrides,
  }
}

describe('processCanvasClick cast-vs-walk', () => {
  beforeEach(() => {
    resetFishingStore()
    inputHandler.clearTransientInput()
  })

  it('casts when a rod is equipped and the click lands on deep enough water', () => {
    const intent = inputHandler.processCanvasClick(
      centerClick(),
      contextWith({
        canCastFishing: true,
        waterSurfaceAt: () => 1.0,
      })
    )

    expect(intent.type).toBe('cast_fishing')
    if (intent.type === 'cast_fishing') {
      expect(intent.position.x).toBeCloseTo(0)
      expect(intent.position.z).toBeCloseTo(0)
    }
  })

  it('walks when no rod is equipped, even over water', () => {
    const intent = inputHandler.processCanvasClick(
      centerClick(),
      contextWith({
        canCastFishing: false,
        waterSurfaceAt: () => 1.0,
      })
    )

    expect(intent.type).toBe('move_to_ground')
  })

  it('walks when the water is too shallow to fish', () => {
    const intent = inputHandler.processCanvasClick(
      centerClick(),
      contextWith({
        canCastFishing: true,
        waterSurfaceAt: () => MIN_FISHABLE_DEPTH_M - 0.05,
      })
    )

    expect(intent.type).toBe('move_to_ground')
  })

  it('walks at exactly the minimum depth — the cast needs strictly deeper water', () => {
    const intent = inputHandler.processCanvasClick(
      centerClick(),
      contextWith({
        canCastFishing: true,
        waterSurfaceAt: () => MIN_FISHABLE_DEPTH_M,
      })
    )

    expect(intent.type).toBe('move_to_ground')
  })

  it('walks when no water sampler is wired up (dungeon / upper floors)', () => {
    const intent = inputHandler.processCanvasClick(
      centerClick(),
      contextWith({ canCastFishing: true })
    )

    expect(intent.type).toBe('move_to_ground')
  })
})
