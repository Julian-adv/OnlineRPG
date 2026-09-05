import { Vector2, Raycaster } from 'three'
import * as THREE from 'three'
import { get } from 'svelte/store'
import { myFishing } from '../stores/fishingStore'
import { instrumentPanelVisible } from '../stores/instrumentStore'
import { pointInRect } from '../stores/dragStore'
import { sprintRequested } from '../stores/movementSettings'
import { isTypingTarget } from '../utils/dom'
import {
  max_cast_distance_m,
  min_fishable_depth_m,
} from '../wasm/onlinerpg_shared'
import type { Position } from '../utils/movementUtils'
import type { WallDirection } from '../utils/house-geometry'

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

const STAIR_CLICK_RAY_OFFSETS = [
  { dx: 0, dy: 0 },
  { dx: 0, dy: -14 },
  { dx: 14, dy: 0 },
  { dx: 0, dy: 14 },
  { dx: -14, dy: 0 },
  { dx: 10, dy: -10 },
  { dx: 10, dy: 10 },
  { dx: -10, dy: 10 },
  { dx: -10, dy: -10 },
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
      position: Position
      isWindow?: boolean
    }
  | {
      type: 'toggle_dungeon_door'
      depth: number
      doorId: number
      position: Position
    }
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
    }
  | {
      /** Clicked someone's tip hat: opens the tip dialog once in range. */
      type: 'tip_hat'
      hatId: number
      position: Position
    }
  | {
      /** Clicked a laid-out stall: opens a trade with its owner. */
      type: 'stall'
      stallId: number
      position: Position
    }
  | {
      /** Clicked a served plate: eaten in place when sitting at its chair. */
      type: 'meal'
      mealId: number
      position: Position
    }
  | {
      type: 'interact_npc'
      playerId: number
      position: Position
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
  | {
      type: 'move_to_ground'
      position: Position
      sprinting: boolean
      viaHousingStair?: boolean
    }
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
  mealMeshes: THREE.Object3D[]
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
  resolveHousingStairTarget?: (
    floorLevel: number,
    x: number,
    y: number,
    z: number,
    stairFloor: number
  ) => Position | null
}

/** What the cursor is over: a placed object carrying display text (e.g. a
 *  signpost), an interactable prop carrying a display name (tip hat, stall,
 *  chest), a ground item, a monster, or a remote player (NPC or not).
 *  A 'name' position is the ground point under the prop's footprint center;
 *  the label floats `labelY` above it and the ring drapes around it. */
export type HoverTarget =
  | { kind: 'text'; position: Position; text: string }
  | {
      kind: 'name'
      position: Position
      text: string
      labelY: number
      ringRadius: number
      floorLevel: number
      /** Ring drape amplitude override; 0 for a thing on a table top. */
      drape?: number
    }
  | { kind: 'groundItem'; instanceId: number }
  | { kind: 'monster'; monsterId: string }
  | { kind: 'player'; playerId: number }

/** Stable identity for a hover target, used to dedupe store writes. */
export function hoverTargetKey(target: HoverTarget | null): string | null {
  if (!target) return null
  switch (target.kind) {
    case 'text':
      return `text:${target.text}@${target.position.x.toFixed(1)},${target.position.z.toFixed(1)}`
    case 'name':
      return `name:${target.text}@${target.position.x.toFixed(1)},${target.position.z.toFixed(1)}`
    case 'groundItem':
      return `item:${target.instanceId}`
    case 'monster':
      return `monster:${target.monsterId}`
    case 'player':
      return `player:${target.playerId}`
  }
}

/** Inputs for the pointermove hover raycast. `monsterMeshes` and
 *  `playerMeshes` should be the invisible hover proxies, not the skinned
 *  models — this runs at ~20 Hz. */
export interface HoverContext {
  camera: THREE.Camera
  objectMeshes: THREE.Object3D[]
  tipHatMeshes: THREE.Object3D[]
  stallMeshes: THREE.Object3D[]
  mealMeshes: THREE.Object3D[]
  propMeshes: THREE.Object3D[]
  groundItemMeshes: THREE.Object3D[]
  monsterMeshes: THREE.Object3D[]
  playerMeshes: THREE.Object3D[]
  /** False when the target can't be named right now (corpse, item mid-pickup):
   *  it occludes nothing and the ray looks past it. */
  isHoverable: (target: HoverTarget) => boolean
  /** Live display name for a player id (a stall's owner), or null if unknown. */
  ownerName: (playerId: number) => string | null
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
    instrumentPanelVisible.subscribe(() => this.clearTransientInput())
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

  private raycastHousingStair(
    event: MouseEvent,
    rect: DOMRect,
    context: RaycastContext
  ): Position | null {
    if (!context.resolveHousingStairTarget) return null
    const raycaster = new Raycaster()
    for (const offset of STAIR_CLICK_RAY_OFFSETS) {
      const mouseNDC = new Vector2(
        ((event.clientX - rect.left + offset.dx) / rect.width) * 2 - 1,
        -((event.clientY - rect.top + offset.dy) / rect.height) * 2 + 1
      )
      raycaster.setFromCamera(mouseNDC, context.camera)
      const hits = raycaster.intersectObjects(context.groundMeshes, true)
      for (const hit of hits) {
        const stair = findAncestorWithUserData(hit.object, 'housingStairFloor')
        if (!stair) continue
        const target = context.resolveHousingStairTarget(
          context.playerVisualFloorLevel,
          hit.point.x,
          hit.point.y,
          hit.point.z,
          stair.userData.housingStairFloor as number
        )
        if (target) return target
      }
    }
    return null
  }

  /** Which remote player, if any, sits under the cursor. Its own raycast
   *  rather than a `ClickIntent`: only the right-click menu asks, and folding
   *  players into the click intents would turn every left-click on a passer-by
   *  into an interaction. */
  pickPlayer(
    event: MouseEvent,
    camera: THREE.Camera,
    playerMeshes: THREE.Object3D[]
  ): number | null {
    if (playerMeshes.length === 0) return null
    const rect = (event.target as HTMLCanvasElement).getBoundingClientRect()
    return this.raycastWithOffsets<number>(
      event,
      rect,
      camera,
      playerMeshes,
      (hit) => {
        const owner = findAncestorWithUserData(hit.object, 'remotePlayerId')
        return owner ? (owner.userData.remotePlayerId as number) : null
      }
    )
  }

  processCanvasClick(event: MouseEvent, context: RaycastContext): ClickIntent {
    if (get(instrumentPanelVisible)) return { type: 'none' }
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
          return {
            type: 'interact_npc',
            playerId: owner.userData.npcPlayerId as number,
            position: {
              x: npcPosition.x,
              y: npcPosition.y,
              z: npcPosition.z,
            },
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

    // Check intersection with door meshes. No distance gate: the player walks
    // up to a far door and opens it on arrival. The door face is vertical, so
    // the walk-up target takes the player's own Y — the hit point's would
    // resolve to the wrong floor.
    if (context.doorMeshes?.length > 0) {
      const doorHits = raycaster.intersectObjects(context.doorMeshes, true)
      for (const hit of doorHits) {
        const hitPoint = hit.point
        const position = {
          x: hitPoint.x,
          y: context.playerPosition.y,
          z: hitPoint.z,
        }
        let obj: THREE.Object3D | null = hit.object
        while (obj) {
          const d = obj.userData
          if (d && d.dungeonDoorKey) {
            return {
              type: 'toggle_dungeon_door',
              depth: d.dungeonDoorKey.depth,
              doorId: d.dungeonDoorKey.doorId,
              position,
            }
          }
          if (
            d &&
            d.doorHouseId &&
            d.doorFloorLevel === context.playerVisualFloorLevel
          ) {
            const interactionPosition = d.doorInteractionPosition as
              | { x: number; z: number }
              | undefined
            return {
              type: 'toggle_door',
              houseId: d.doorHouseId,
              roomIndex: d.doorRoomIndex,
              wallDir: d.doorWallDir,
              segmentIndex: d.doorSegmentIndex,
              position: {
                x: interactionPosition?.x ?? position.x,
                y: position.y,
                z: interactionPosition?.z ?? position.z,
              },
              isWindow: d.doorIsWindow === true,
            }
          }
          obj = obj.parent
        }
      }
    }

    // Plates stand inside their table's hit volume, so they go before
    // furniture or the table would swallow the click.
    if (context.mealMeshes.length > 0) {
      const mealHits = raycaster.intersectObjects(context.mealMeshes, true)
      const plate = mealHits.length
        ? findAncestorWithUserData(mealHits[0].object, 'mealId')
        : null
      if (plate) {
        const platePosition = new THREE.Vector3()
        plate.getWorldPosition(platePosition)
        return {
          type: 'meal',
          mealId: plate.userData.mealId as number,
          position: {
            x: platePosition.x,
            y: platePosition.y,
            z: platePosition.z,
          },
        }
      }
    }

    // Check intersection with object meshes. No distance gate: the player walks
    // up to a far chair/bench and sits on arrival.
    if (context.objectMeshes.length > 0) {
      const objectHits = raycaster.intersectObjects(context.objectMeshes, true)
      if (objectHits.length > 0) {
        const hitPoint = objectHits[0].point
        let obj: THREE.Object3D | null = objectHits[0].object
        while (obj) {
          const d = obj.userData
          if (d && d.objectId != null && d.objectType && d.objectInteraction) {
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
        let obj: THREE.Object3D | null = itemHits[0].object
        while (obj) {
          if (obj.userData && obj.userData.groundItemId != null) {
            const itemPosition = new THREE.Vector3()
            obj.getWorldPosition(itemPosition)
            return {
              type: 'pickup_ground_item',
              instanceId: obj.userData.groundItemId as number,
              position: {
                x: itemPosition.x,
                y: itemPosition.y,
                z: itemPosition.z,
              },
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
        return {
          type: 'tip_hat',
          hatId: owner.userData.tipHatId as number,
          position: { x: hatPosition.x, y: hatPosition.y, z: hatPosition.z },
        }
      }
    }

    // Stalls, same shape as tip hats. The walk-up targets the table, which is
    // what the server checks too, rather than the owner behind it.
    if (context.stallMeshes.length > 0) {
      const stallHits = raycaster.intersectObjects(context.stallMeshes, true)
      const table = stallHits.length
        ? findAncestorWithUserData(stallHits[0].object, 'stallId')
        : null
      if (table) {
        const stallPosition = new THREE.Vector3()
        table.getWorldPosition(stallPosition)
        return {
          type: 'stall',
          stallId: table.userData.stallId as number,
          position: {
            x: stallPosition.x,
            y: stallPosition.y,
            z: stallPosition.z,
          },
        }
      }
    }

    const stairTarget = this.raycastHousingStair(event, rect, context)
    if (stairTarget) {
      return {
        type: 'move_to_ground',
        sprinting: sprintRequested(event.shiftKey),
        position: stairTarget,
        viaHousingStair: true,
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
   * Raycast the pointer against the placed-object, tip-hat, stall, dungeon-
   * prop, ground-item and monster meshes only, returning what the frontmost hoverable hit belongs
   * to: an object's display text (userData.objectText), an interactable
   * prop's name (userData.hoverName), a ground item (userData.groundItemId)
   * or a monster (userData.monsterId). Hits the
   * context deems unhoverable (a corpse, an item mid-pickup) are looked past —
   * loot often lands exactly where a monster died.
   * Cheap enough to run on pointermove: it intersects those groups, not
   * the whole scene. Returns null when the cursor is over none of them.
   */
  processHover(event: MouseEvent, context: HoverContext): HoverTarget | null {
    const targets = [
      ...context.objectMeshes,
      ...context.tipHatMeshes,
      ...context.stallMeshes,
      ...context.mealMeshes,
      ...context.propMeshes,
      ...context.groundItemMeshes,
      ...context.monsterMeshes,
      ...context.playerMeshes,
    ]
    if (targets.length === 0) return null
    const rect = (event.target as HTMLCanvasElement).getBoundingClientRect()
    this._hoverNDC.set(
      ((event.clientX - rect.left) / rect.width) * 2 - 1,
      -((event.clientY - rect.top) / rect.height) * 2 + 1
    )
    this._hoverRaycaster.setFromCamera(this._hoverNDC, context.camera)
    const hits = this._hoverRaycaster.intersectObjects(targets, true)

    // A mesh yields one hit per intersected triangle; skip repeats.
    let lastObject: THREE.Object3D | null = null
    for (const hit of hits) {
      if (hit.object === lastObject) continue
      lastObject = hit.object
      const target = this.resolveHoverTarget(hit.object, context)
      if (target && context.isHoverable(target)) return target
    }
    return null
  }

  /** One walk up the parent chain, checking every hover key per ancestor. */
  private resolveHoverTarget(
    obj: THREE.Object3D,
    context: HoverContext
  ): HoverTarget | null {
    for (let o: THREE.Object3D | null = obj; o; o = o.parent) {
      const data = o.userData
      if (data?.groundItemId != null) {
        return { kind: 'groundItem', instanceId: data.groundItemId as number }
      }
      if (data?.monsterId != null) {
        return { kind: 'monster', monsterId: data.monsterId as string }
      }
      if (data?.remotePlayerId != null) {
        return { kind: 'player', playerId: data.remotePlayerId as number }
      }
      if (data?.hoverName) {
        // Anchor at the footprint center on the ground plane, in the prop's
        // local frame (hoverCenter), carried to world space with its yaw.
        const center = data.hoverCenter as { x: number; y: number; z: number }
        this._hoverWorldPos.set(center.x, center.y, center.z)
        o.localToWorld(this._hoverWorldPos)
        // A stall is named for its live owner, resolved at hover time so the
        // layer's userData stays static (no store reads on the render path).
        let text = data.hoverName as string
        const ownerId = data.hoverOwnerId as number | undefined
        if (ownerId != null) {
          const owner = context.ownerName(ownerId)
          if (owner) text = `${owner}'s ${text}`
        }
        return {
          kind: 'name',
          position: {
            x: this._hoverWorldPos.x,
            y: this._hoverWorldPos.y,
            z: this._hoverWorldPos.z,
          },
          text,
          labelY: data.hoverLabelY as number,
          ringRadius: data.hoverRingRadius as number,
          floorLevel: data.hoverFloorLevel as number,
          drape: data.hoverDrape as number | undefined,
        }
      }
      if (data?.objectText) {
        // World position (robust if the overlay group is ever transformed);
        // equals obj.position today since the group sits at the scene root.
        o.getWorldPosition(this._hoverWorldPos)
        return {
          kind: 'text',
          position: {
            x: this._hoverWorldPos.x,
            y: this._hoverWorldPos.y,
            z: this._hoverWorldPos.z,
          },
          text: data.objectText as string,
        }
      }
    }
    return null
  }

  handleKeyDown(event: KeyboardEvent): boolean {
    if (isTypingTarget(event.target)) {
      return false
    }
    if (event.ctrlKey) return false
    if (get(instrumentPanelVisible)) return true

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
