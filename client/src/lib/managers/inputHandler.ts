import { Vector2, Raycaster } from 'three'
import * as THREE from 'three'
import { get } from 'svelte/store'
import { myFishing } from '../stores/fishingStore'
import { pointInRect } from '../stores/dragStore'
import { sprintRequested } from '../stores/movementSettings'
import { isTypingTarget } from '../utils/dom'
import {
  max_cast_distance_m,
  min_fishable_depth_m,
} from '../wasm/onlinerpg_shared'
import type { Position } from '../utils/movementUtils'
import type { WallDirection } from '../utils/house-geometry'

const MAX_DOOR_INTERACT_DISTANCE = 2.0
const MAX_OBJECT_INTERACT_DISTANCE = 3.0

function hasAncestorBridge(obj: THREE.Object3D | null): boolean {
  for (let o = obj; o; o = o.parent) {
    if (o.userData?.objectKind === 'bridge') return true
  }
  return false
}

/** Walk up the parent chain to the first object carrying `key` in userData. */
export function findAncestorWithUserData(
  obj: THREE.Object3D | null,
  key: string
): THREE.Object3D | null {
  for (let o = obj; o; o = o.parent) {
    if (o.userData?.[key] != null) return o
  }
  return null
}

/** True when the object sits under a house wall/roof group. The front and back
 *  groups (south/west and north/east walls plus roofs) are tagged
 *  `housingSurface: 'wall'` at construction; floor slabs and stairs are tagged
 *  `'floor'` and stay valid move-to-ground targets. Used to skip the outer
 *  walls that sit between the isometric camera and the interior floor when
 *  clicking a floor tile to walk inside. */
function isHouseWall(obj: THREE.Object3D | null): boolean {
  return (
    findAncestorWithUserData(obj, 'housingSurface')?.userData.housingSurface ===
    'wall'
  )
}

function withinCastRange(hit: THREE.Vector3, player: Position): boolean {
  const dx = hit.x - player.x
  const dz = hit.z - player.z
  const max = max_cast_distance_m()
  return dx * dx + dz * dz <= max * max
}

/** Entity clicks get a little slack: the click point plus 4 nearby offsets
 *  (10px up/right/down/left) are each raycast until one resolves. */
const CLICK_RAY_OFFSETS = [
  { dx: 0, dy: 0 },
  { dx: 0, dy: -10 }, // Screen coordinates: -y is up
  { dx: 10, dy: 0 },
  { dx: 0, dy: 10 },
  { dx: -10, dy: 0 },
]

export type ClickIntent =
  | {
      type: 'attack_monster'
      monsterId: string
      hitPoint: Position
      distance: number
    }
  | {
      type: 'toggle_door'
      houseId: string
      roomIndex: number
      wallDir: WallDirection
      segmentIndex: number
    }
  | { type: 'toggle_dungeon_door'; depth: number; doorId: number }
  | {
      type: 'interact_object'
      objectId: number
      objectType: string
      interaction: string
      position: Position
      rotation: number
      interactOffset?: Position
    }
  | {
      type: 'pickup_ground_item'
      instanceId: number
      position: Position
      distance: number
    }
  | {
      /** Clicked someone's tip hat: opens the tip dialog once in range. */
      type: 'tip_hat'
      hatId: number
      position: Position
      distance: number
    }
  | {
      /** Clicked a laid-out stall: opens a trade with its owner. */
      type: 'stall'
      stallId: number
      position: Position
      distance: number
    }
  | {
      type: 'interact_npc'
      playerId: number
      position: Position
      distance: number
    }
  | {
      type: 'break_prop'
      entranceId: string
      depth: number
      propId: number
      position: Position
    }
  | {
      type: 'open_prop'
      entranceId: string
      depth: number
      propId: number
      position: Position
    }
  | { type: 'move_to_ground'; position: Position; sprinting: boolean }
  | {
      /** Rod equipped + clicked point is underwater terrain: cast, don't walk. */
      type: 'cast_fishing'
      position: Position
    }
  | { type: 'none' }

export interface RaycastContext {
  camera: THREE.Camera
  monsterMeshes: THREE.Group[]
  npcMeshes: THREE.Object3D[]
  doorMeshes: THREE.Object3D[]
  objectMeshes: THREE.Object3D[]
  /** Breakable dungeon props (barrels/crates). Clicked from any range — the
   *  player walks up before the break fires. */
  propMeshes: THREE.Object3D[]
  groundItemMeshes: THREE.Object3D[]
  tipHatMeshes: THREE.Object3D[]
  stallMeshes: THREE.Object3D[]
  groundMeshes: THREE.Object3D[]
  playerPosition: Position
  /** Gates house door clicks to the door's own floor (0 = ground/outdoors). */
  playerVisualFloorLevel: number
  isMonsterDead: (monsterId: string) => boolean
  /** Rod in the main hand and standing on castable ground (surface, not a
   *  dungeon or upper house floor) — water clicks become casts. */
  canCastFishing?: boolean
  /** Baked water surface height at a world XZ (sea level where none). Lets a
   *  cast fire over rivers, whose beds sit above sea level, not just ocean. */
  waterSurfaceAt?: (x: number, z: number) => number
}

/** Result of hovering a placed object that carries display text (e.g. signpost). */
export interface HoverText {
  position: Position
  text: string
}

class InputHandler {
  private keysPressed = new Set<string>()
  private _interactJustPressed = false
  /** Dedicated raycaster reused across pointermove hover queries. */
  private _hoverRaycaster = new Raycaster()
  private readonly _hoverNDC = new Vector2()
  private readonly _hoverWorldPos = new THREE.Vector3()
  private readonly _fallbackGroundPlane = new THREE.Plane()
  private readonly _fallbackGroundPoint = new THREE.Vector3()
  private readonly _fallbackGroundNormal = new THREE.Vector3(0, 1, 0)

  constructor() {
    // A key held since before the bite stays in keysPressed (no new keydown
    // fires), and held S walks backward — aborting the session server-side.
    myFishing.subscribe((f) => {
      if (f.phase === 'bite' || f.phase === 'fight') {
        this.clearTransientInput()
      }
    })
  }

  get hasKeysPressed(): boolean {
    return this.getMovementDirection() !== null
  }

  get isSprintRequested(): boolean {
    return sprintRequested(
      this.keysPressed.has('ShiftLeft') || this.keysPressed.has('ShiftRight')
    )
  }

  clearTransientInput() {
    this.keysPressed.clear()
    this._interactJustPressed = false
  }

  /** Returns true once per E key press, then resets. */
  consumeInteract(): boolean {
    if (this._interactJustPressed) {
      this._interactJustPressed = false
      return true
    }
    return false
  }

  getMovementDirection(): { x: number; z: number } | null {
    let moveX = 0
    let moveZ = 0

    if (this.keysPressed.has('KeyW') || this.keysPressed.has('ArrowUp'))
      moveZ -= 1
    if (this.keysPressed.has('KeyS') || this.keysPressed.has('ArrowDown'))
      moveZ += 1
    if (this.keysPressed.has('KeyA') || this.keysPressed.has('ArrowLeft'))
      moveX -= 1
    if (this.keysPressed.has('KeyD') || this.keysPressed.has('ArrowRight'))
      moveX += 1

    if (moveX === 0 && moveZ === 0) return null

    // Normalize diagonal movement
    if (moveX !== 0 && moveZ !== 0) {
      moveX *= 0.707 // 1/sqrt(2)
      moveZ *= 0.707
    }

    return { x: moveX, z: moveZ }
  }

  /** Cast the click ray plus the 4 offset rays against `meshes`, returning the
   *  first non-null result of `resolve` over each ray's closest intersection. */
  private raycastWithOffsets<T>(
    event: MouseEvent,
    rect: DOMRect,
    camera: THREE.Camera,
    meshes: THREE.Object3D[],
    resolve: (hit: THREE.Intersection) => T | null
  ): T | null {
    const raycaster = new Raycaster()
    for (const offset of CLICK_RAY_OFFSETS) {
      const mouseNDC = new Vector2(
        ((event.clientX - rect.left + offset.dx) / rect.width) * 2 - 1,
        -((event.clientY - rect.top + offset.dy) / rect.height) * 2 + 1
      )

      raycaster.setFromCamera(mouseNDC, camera)
      const hits = raycaster.intersectObjects(meshes, true)
      if (hits.length === 0) continue

      const result = resolve(hits[0])
      if (result !== null) return result
    }
    return null
  }

  processCanvasClick(event: MouseEvent, context: RaycastContext): ClickIntent {
    const rect = (event.target as HTMLCanvasElement).getBoundingClientRect()

    // Check intersection with monsters
    if (context.monsterMeshes.length > 0) {
      const monsterIntent = this.raycastWithOffsets<ClickIntent>(
        event,
        rect,
        context.camera,
        context.monsterMeshes,
        (hit) => {
          const owner = findAncestorWithUserData(hit.object, 'monsterId')
          if (!owner) return null
          const monsterId = owner.userData.monsterId as string
          if (context.isMonsterDead(monsterId)) return null // Try other rays

          const hitPoint = hit.point
          const dist = new THREE.Vector3(
            context.playerPosition.x,
            0,
            context.playerPosition.z
          ).distanceTo(new THREE.Vector3(hitPoint.x, 0, hitPoint.z))

          return {
            type: 'attack_monster',
            monsterId,
            hitPoint: { x: hitPoint.x, y: hitPoint.y, z: hitPoint.z },
            distance: dist,
          }
        }
      )
      if (monsterIntent) return monsterIntent
    }

    // Check intersection with NPC models
    if (context.npcMeshes.length > 0) {
      const npcIntent = this.raycastWithOffsets<ClickIntent>(
        event,
        rect,
        context.camera,
        context.npcMeshes,
        (hit) => {
          const owner = findAncestorWithUserData(hit.object, 'npcPlayerId')
          if (!owner) return null

          const npcPosition = new THREE.Vector3()
          owner.getWorldPosition(npcPosition)
          const dx = npcPosition.x - context.playerPosition.x
          const dz = npcPosition.z - context.playerPosition.z
          return {
            type: 'interact_npc',
            playerId: owner.userData.npcPlayerId as number,
            position: {
              x: npcPosition.x,
              y: npcPosition.y,
              z: npcPosition.z,
            },
            distance: Math.sqrt(dx * dx + dz * dz),
          }
        }
      )
      if (npcIntent) return npcIntent
    }

    const raycaster = new Raycaster()
    const centerNDC = new Vector2(
      ((event.clientX - rect.left) / rect.width) * 2 - 1,
      -((event.clientY - rect.top) / rect.height) * 2 + 1
    )
    raycaster.setFromCamera(centerNDC, context.camera)

    // Check intersection with door meshes
    if (context.doorMeshes?.length > 0) {
      const doorHits = raycaster.intersectObjects(context.doorMeshes, true)
      if (doorHits.length > 0) {
        const hitPoint = doorHits[0].point
        const pp = context.playerPosition
        const dx = hitPoint.x - pp.x
        const dz = hitPoint.z - pp.z
        if (
          dx * dx + dz * dz <=
          MAX_DOOR_INTERACT_DISTANCE * MAX_DOOR_INTERACT_DISTANCE
        ) {
          let obj: THREE.Object3D | null = doorHits[0].object
          while (obj) {
            const d = obj.userData
            if (d && d.dungeonDoorKey) {
              return {
                type: 'toggle_dungeon_door',
                depth: d.dungeonDoorKey.depth,
                doorId: d.dungeonDoorKey.doorId,
              }
            }
            if (
              d &&
              d.doorHouseId &&
              d.doorFloorLevel === context.playerVisualFloorLevel
            ) {
              return {
                type: 'toggle_door',
                houseId: d.doorHouseId,
                roomIndex: d.doorRoomIndex,
                wallDir: d.doorWallDir,
                segmentIndex: d.doorSegmentIndex,
              }
            }
            obj = obj.parent
          }
        }
      }
    }

    // Check intersection with object meshes
    if (context.objectMeshes.length > 0) {
      const objectHits = raycaster.intersectObjects(context.objectMeshes, true)
      if (objectHits.length > 0) {
        const hitPoint = objectHits[0].point
        const pp = context.playerPosition
        const dx = hitPoint.x - pp.x
        const dz = hitPoint.z - pp.z
        if (
          dx * dx + dz * dz <=
          MAX_OBJECT_INTERACT_DISTANCE * MAX_OBJECT_INTERACT_DISTANCE
        ) {
          let obj: THREE.Object3D | null = objectHits[0].object
          while (obj) {
            const d = obj.userData
            if (
              d &&
              d.objectId != null &&
              d.objectType &&
              d.objectInteraction
            ) {
              return {
                type: 'interact_object',
                objectId: d.objectId as number,
                objectType: d.objectType,
                interaction: d.objectInteraction,
                position: {
                  x: obj.position.x,
                  y: obj.position.y,
                  z: obj.position.z,
                },
                rotation: obj.rotation.y,
                interactOffset: d.objectInteractOffset,
              }
            }
            obj = obj.parent
          }
        }
        const face = objectHits[0].face
        if (
          face &&
          face.normal.y > 0.5 &&
          hasAncestorBridge(objectHits[0].object)
        ) {
          return {
            type: 'move_to_ground',
            sprinting: sprintRequested(event.shiftKey),
            position: { x: hitPoint.x, y: hitPoint.y, z: hitPoint.z },
          }
        }
      }
    }

    // Check intersection with interactive dungeon props (breakable barrels/
    // crates, openable chests). No distance gate: the player walks up to the
    // prop and the break/open fires on arrival.
    if (context.propMeshes.length > 0) {
      const propHits = raycaster.intersectObjects(context.propMeshes, true)
      if (propHits.length > 0) {
        const owner = findAncestorWithUserData(propHits[0].object, 'propId')
        if (owner) {
          const wp = new THREE.Vector3()
          owner.getWorldPosition(wp)
          const target = {
            entranceId: owner.userData.propEntranceId as string,
            depth: owner.userData.propDepth as number,
            propId: owner.userData.propId as number,
            position: { x: wp.x, y: wp.y, z: wp.z },
          }
          if (owner.userData.propOpenable) {
            return { type: 'open_prop', ...target }
          }
          if (owner.userData.propBreakable) {
            return { type: 'break_prop', ...target }
          }
        }
      }
    }

    // Check intersection with ground items
    if (context.groundItemMeshes.length > 0) {
      const itemHits = raycaster.intersectObjects(
        context.groundItemMeshes,
        true
      )
      if (itemHits.length > 0) {
        const pp = context.playerPosition
        let obj: THREE.Object3D | null = itemHits[0].object
        while (obj) {
          if (obj.userData && obj.userData.groundItemId != null) {
            const itemPosition = new THREE.Vector3()
            obj.getWorldPosition(itemPosition)
            const dx = itemPosition.x - pp.x
            const dz = itemPosition.z - pp.z
            const distSq = dx * dx + dz * dz
            return {
              type: 'pickup_ground_item',
              instanceId: obj.userData.groundItemId as number,
              position: {
                x: itemPosition.x,
                y: itemPosition.y,
                z: itemPosition.z,
              },
              distance: Math.sqrt(distSq),
            }
          }
          obj = obj.parent
        }
      }
    }

    // Check intersection with tip hats. No distance gate: the player walks
    // into range and the tip dialog opens there.
    if (context.tipHatMeshes.length > 0) {
      const hatHits = raycaster.intersectObjects(context.tipHatMeshes, true)
      const owner = hatHits.length
        ? findAncestorWithUserData(hatHits[0].object, 'tipHatId')
        : null
      if (owner) {
        const hatPosition = new THREE.Vector3()
        owner.getWorldPosition(hatPosition)
        const pp = context.playerPosition
        const dx = hatPosition.x - pp.x
        const dz = hatPosition.z - pp.z
        return {
          type: 'tip_hat',
          hatId: owner.userData.tipHatId as number,
          position: { x: hatPosition.x, y: hatPosition.y, z: hatPosition.z },
          distance: Math.sqrt(dx * dx + dz * dz),
        }
      }
    }

    // Stalls, same shape as tip hats: click from anywhere, the server checks
    // the distance to the table rather than to its owner.
    if (context.stallMeshes.length > 0) {
      const stallHits = raycaster.intersectObjects(context.stallMeshes, true)
      const table = stallHits.length
        ? findAncestorWithUserData(stallHits[0].object, 'stallId')
        : null
      if (table) {
        const stallPosition = new THREE.Vector3()
        table.getWorldPosition(stallPosition)
        const pp = context.playerPosition
        const dx = stallPosition.x - pp.x
        const dz = stallPosition.z - pp.z
        return {
          type: 'stall',
          stallId: table.userData.stallId as number,
          position: {
            x: stallPosition.x,
            y: stallPosition.y,
            z: stallPosition.z,
          },
          distance: Math.sqrt(dx * dx + dz * dz),
        }
      }
    }

    // Check intersection with ground meshes. During floor/scene transitions
    // (notably dungeon death -> surface respawn), the control layer can be
    // mounted before the visible ground mesh list has caught up. Fall back to
    // the player's current horizontal plane so a valid canvas click still
    // becomes a move request instead of silently producing `none`.
    const intersects = raycaster.intersectObjects(context.groundMeshes, true)

    // Hits are sorted nearest-first. A house's outer walls/roof sit between the
    // isometric camera and the interior floor, so the naive closest hit would
    // land the player on the south/west wall when they click a floor tile to
    // walk inside. Skip wall/roof hits and take the first floor/terrain surface
    // behind them (pathfinding then routes around solid walls to the door).
    const groundHit = intersects.find((hit) => !isHouseWall(hit.object))
    if (groundHit) {
      // Rod + water surface above the clicked terrain: cast, don't walk.
      // The water field (not "y < 0") makes rivers castable; the server
      // re-validates, so this only decides cast-vs-walk.
      const waterSurface = context.waterSurfaceAt?.(
        groundHit.point.x,
        groundHit.point.z
      )
      // Out-of-range water falls through to a walk (mirrors the server's
      // XZ range check) instead of sending a cast it would reject.
      if (
        context.canCastFishing &&
        waterSurface !== undefined &&
        waterSurface - groundHit.point.y > min_fishable_depth_m() &&
        withinCastRange(groundHit.point, context.playerPosition)
      ) {
        return {
          type: 'cast_fishing',
          position: {
            x: groundHit.point.x,
            y: groundHit.point.y,
            z: groundHit.point.z,
          },
        }
      }
      return {
        type: 'move_to_ground',
        sprinting: sprintRequested(event.shiftKey),
        position: {
          x: groundHit.point.x,
          y: groundHit.point.y,
          z: groundHit.point.z,
        },
      }
    }

    this._fallbackGroundPlane.set(
      this._fallbackGroundNormal,
      -context.playerPosition.y
    )
    if (
      raycaster.ray.intersectPlane(
        this._fallbackGroundPlane,
        this._fallbackGroundPoint
      )
    ) {
      return {
        type: 'move_to_ground',
        sprinting: sprintRequested(event.shiftKey),
        position: {
          x: this._fallbackGroundPoint.x,
          y: this._fallbackGroundPoint.y,
          z: this._fallbackGroundPoint.z,
        },
      }
    }

    return { type: 'none' }
  }

  /**
   * Raycast the pointer against the placed-object meshes only and return the
   * display text of the first object that carries one (set on userData.objectText).
   * Cheap enough to run on pointermove: it intersects just the object overlay
   * group, not the whole scene. Returns null when nothing texted is under the cursor.
   */
  processHover(
    event: MouseEvent,
    camera: THREE.Camera,
    objectMeshes: THREE.Object3D[]
  ): HoverText | null {
    if (objectMeshes.length === 0) return null
    const rect = (event.target as HTMLCanvasElement).getBoundingClientRect()
    this._hoverNDC.set(
      ((event.clientX - rect.left) / rect.width) * 2 - 1,
      -((event.clientY - rect.top) / rect.height) * 2 + 1
    )
    this._hoverRaycaster.setFromCamera(this._hoverNDC, camera)
    const hits = this._hoverRaycaster.intersectObjects(objectMeshes, true)
    if (hits.length === 0) return null

    let obj: THREE.Object3D | null = hits[0].object
    while (obj) {
      const text = obj.userData?.objectText
      if (typeof text === 'string' && text.length > 0) {
        // World position (robust if the overlay group is ever transformed);
        // equals obj.position today since the group sits at the scene root.
        obj.getWorldPosition(this._hoverWorldPos)
        return {
          position: {
            x: this._hoverWorldPos.x,
            y: this._hoverWorldPos.y,
            z: this._hoverWorldPos.z,
          },
          text,
        }
      }
      obj = obj.parent
    }
    return null
  }

  handleKeyDown(event: KeyboardEvent): boolean {
    if (isTypingTarget(event.target)) {
      return false
    }
    if (event.ctrlKey) return false

    // SPACE and S belong to the fishing minigame during a bite/fight;
    // treating S as backward movement would abort the session server-side.
    const fishingPhase = get(myFishing).phase
    if (
      (fishingPhase === 'bite' || fishingPhase === 'fight') &&
      (event.code === 'Space' || event.code === 'KeyS')
    ) {
      return true
    }

    if (event.code === 'KeyE' && !event.repeat) {
      this._interactJustPressed = true
    }
    this.keysPressed.add(event.code)
    return true
  }

  handleKeyUp(event: KeyboardEvent): boolean {
    // Always remove from tracked keys on keyup, to prevent stuck keys
    // especially when focus changes (e.g. Enter to open chat)
    if (this.keysPressed.has(event.code)) {
      this.keysPressed.delete(event.code)
    }

    if (isTypingTarget(event.target)) {
      return false
    }
    return true
  }

  setupEventListeners(
    canvas: HTMLCanvasElement,
    onCanvasClick: (event: MouseEvent) => void
  ): () => void {
    const onKeyDown = (event: KeyboardEvent) => {
      if (this.handleKeyDown(event)) {
        event.preventDefault()
      }
    }
    const onKeyUp = (event: KeyboardEvent) => {
      if (this.handleKeyUp(event)) {
        event.preventDefault()
      }
    }

    // OS shortcuts (e.g. Win+Shift+S) can swallow keyup of held modifiers,
    // leaving keys "stuck" and blocking click-to-move via hasKeysPressed.
    const onWindowBlur = () => this.clearTransientInput()

    document.addEventListener('keydown', onKeyDown)
    document.addEventListener('keyup', onKeyUp)
    window.addEventListener('blur', onWindowBlur)

    const onContextMenu = (event: MouseEvent) => {
      const canvasTarget = event.target instanceof HTMLCanvasElement
      const suppress = shouldSuppressContextMenu({
        canvasTarget,
        typingTarget: isTypingTarget(event.target),
        selectionAtPointer:
          !canvasTarget &&
          hasSelectionAtPoint(
            window.getSelection(),
            event.clientX,
            event.clientY
          ),
      })
      if (suppress) event.preventDefault()
    }
    canvas.addEventListener('mousedown', onCanvasClick)
    document.addEventListener('contextmenu', onContextMenu, true)

    return () => {
      document.removeEventListener('keydown', onKeyDown)
      document.removeEventListener('keyup', onKeyUp)
      window.removeEventListener('blur', onWindowBlur)
      canvas.removeEventListener('mousedown', onCanvasClick)
      document.removeEventListener('contextmenu', onContextMenu, true)
    }
  }
}

function hasSelectionAtPoint(
  selection: Selection | null,
  x: number,
  y: number
): boolean {
  if (!selection || selection.isCollapsed) return false
  for (let i = 0; i < selection.rangeCount; i++) {
    for (const rect of selection.getRangeAt(i).getClientRects()) {
      if (pointInRect(x, y, rect)) return true
    }
  }
  return false
}

/** Suppress native menus except when editing or copying selected text.
 *  Canvases are never exempt — a selection elsewhere must not leak the
 *  browser menu into the 3D view. */
export function shouldSuppressContextMenu(state: {
  canvasTarget: boolean
  typingTarget: boolean
  selectionAtPointer: boolean
}): boolean {
  if (state.canvasTarget) return true
  return !state.typingTarget && !state.selectionAtPointer
}

export const inputHandler = new InputHandler()
