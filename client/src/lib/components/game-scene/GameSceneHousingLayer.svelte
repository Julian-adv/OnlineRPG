<script lang="ts">
  import { T } from '@threlte/core'
  import * as THREE from 'three'
  import { onDestroy } from 'svelte'
  import { SvelteMap } from 'svelte/reactivity'
  import type { HouseData } from '../../types/housing'
  import {
    buildHouseGroup,
    disposeHouseGroup,
    applyDoorGhostMaterials,
    resetDoorGhostMaterials,
    applyInteriorGhosts,
    DEFAULT_WALL_HEIGHT,
    FLOOR_THICKNESS,
    MAX_FLOOR_LEVEL,
    OFFSCREEN_Y,
    floorYBase,
    getStairwellYOffset,
    type HouseGroupResult,
  } from '../../utils/house-geometry'
  import {
    initHousingTextures,
    disposeHousingMaterials,
    getHousingMaterial,
    getGhostHousingMaterial,
    HOUSING_TEXTURES,
  } from '../../utils/housing-textures'
  import {
    WOOD_TEXTURE_IDX,
    SHUTTER_PANEL_TEXTURE_IDX,
    WALL_THICKNESS,
    ROOF_OVERHANG,
  } from '../../utils/house-geo-utils'
  import { getWallByDir } from '../../managers/housingManager'
  import { housingManager } from '../../managers/housingManager'
  import {
    resolveStairFloor,
    roomContainsXZ,
  } from '../../managers/housing-queries'
  import { furnitureManager } from '../../managers/furnitureManager'
  import {
    TERRAIN_TILE_SIZE,
    getTerrainChunkFromPosition,
  } from './terrain-utils'
  import {
    playerFloorOffset,
    playerVisualFloorLevel,
    playerInsideHouseId,
  } from '../../stores/housingStore'
  import {
    debugVisible,
    housingEditorMode,
    passabilityDebugVisible,
  } from '../../stores/debugStore'
  import { pushPassabilityEdges } from '../../utils/passability-wireframe'
  import { get } from 'svelte/store'

  interface Props {
    playerPosition: { x: number; y: number; z: number } | null
  }

  let { playerPosition }: Props = $props()

  const housingGroup = new THREE.Group()
  housingGroup.name = 'housingLayer'

  const houses = new SvelteMap<string, HouseGroupResult>()
  let currentInsideHouseId: string | null = null
  let playerInsideFloor = 0
  let lastFloorOffset = 0
  /** Whether last frame resolved onto a stairwell (acquire/release hysteresis). */
  let wasOnStairs = false
  // Preallocated for per-frame room detection (avoid GC)
  const _allRooms: { house: HouseData; roomIndex: number }[] = []
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const _seenRooms = new Set<string>()
  let lastChunkX = NaN
  let lastChunkZ = NaN
  // Whether the houses around the player have all arrived — see update().
  let housesLoadedHere = false
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const occludedHouseIds = new Set<string>()

  // Debug passability wireframe
  const debugPassGroup = new THREE.Group()
  debugPassGroup.name = 'passabilityDebug'
  debugPassGroup.visible = false
  housingGroup.add(debugPassGroup)

  const debugLineMaterial = new THREE.LineBasicMaterial({ color: 0xff0000 })
  // Furniture cells drawn in orange to distinguish them from house walls (red).
  const debugFurnitureMaterial = new THREE.LineBasicMaterial({
    color: 0xffaa00,
  })
  let debugPassDirty = false

  // Server floor syncs (join/teleport/correction) also write the store; follow
  // them so stacked rooms resolve to that storey.
  const unsubFloor = playerVisualFloorLevel.subscribe((v) => {
    if (playerInsideFloor !== v && debugPassGroup.visible) {
      // The overlay draws only the player's floor; redraw on floor change.
      debugPassDirty = true
    }
    playerInsideFloor = v
  })

  const unsubPassDebug = passabilityDebugVisible.subscribe((v) => {
    debugPassGroup.visible = v
    if (v) debugPassDirty = true
  })

  // Furniture collision is synced separately (per region) from ObjectOverlay;
  // rebuild the overlay when it changes so newly placed furniture shows up.
  const unsubFurniture = furnitureManager.onChanged(() => {
    if (debugPassGroup.visible) debugPassDirty = true
  })

  /** Build a LineSegments from flat xyz verts and add it to the debug group.
   *  No-op when there are no verts. */
  function addDebugLines(verts: number[], material: THREE.LineBasicMaterial) {
    if (verts.length === 0) return
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.Float32BufferAttribute(verts, 3))
    const lines = new THREE.LineSegments(geo, material)
    lines.frustumCulled = false
    debugPassGroup.add(lines)
  }

  function rebuildPassabilityDebug() {
    // Clear old
    while (debugPassGroup.children.length > 0) {
      const child = debugPassGroup.children[0]
      debugPassGroup.remove(child)
      if (child instanceof THREE.LineSegments) {
        child.geometry.dispose()
      }
    }

    for (const [houseId, rp] of housingManager.getPassabilityEntries()) {
      const house = housingManager.getHouseById(houseId)
      if (!house) continue

      const vertices: number[] = []

      // Only the player's floor: drawing every storey at once stacks the
      // grids into an unreadable tangle.
      for (const floor of rp.floors) {
        if (floor.floorLevel !== playerInsideFloor) continue
        pushPassabilityEdges(
          vertices,
          floor.cells,
          floor.width,
          floor.depth,
          house.origin.x + floor.originX,
          house.origin.z + floor.originZ,
          floor.yBase
        )
      }

      addDebugLines(vertices, debugLineMaterial)
    }

    // Solid furniture: each sealed cell is drawn as a full box (all four edges),
    // matching how EDGE_ALL is stored in the cache.
    const EDGE_ALL = 15
    const furnitureVerts: number[] = []
    for (const piece of furnitureManager.getDebugPieces()) {
      if (piece.floorLevel !== playerInsideFloor) continue
      for (const [cellX, cellZ] of piece.cells) {
        pushPassabilityEdges(
          furnitureVerts,
          [EDGE_ALL],
          1,
          1,
          cellX,
          cellZ,
          piece.yBase
        )
      }
    }
    addDebugLines(furnitureVerts, debugFurnitureMaterial)

    debugPassDirty = false
  }

  // Load housing textures (materials update in-place via needsUpdate)
  initHousingTextures().then(() => {
    // Re-apply ghost materials now that textures are loaded
    if (currentInsideHouseId) {
      const curr = houses.get(currentInsideHouseId)
      if (curr) {
        resetDoorGhostMaterials(curr)
        applyDoorGhostMaterials(curr, playerInsideFloor)
      }
    }
  })

  // Listen for housing data changes from the manager
  const unsubHouses = housingManager.onHousesChanged((allHouses) => {
    syncHouses(allHouses)
    if (playerPosition) {
      housesLoadedHere = housingManager.isLoadedAround(
        playerPosition.x,
        playerPosition.z
      )
    }
    if (debugPassGroup.visible) debugPassDirty = true
  })

  // Roofs hide the interior while editing; the flag is part of the house
  // hash so the toggle flows through the normal rebuild path.
  let roofs = true
  const unsubEditor = housingEditorMode.subscribe((v) => {
    roofs = !v
    syncHouses(housingManager.getAllHouses())
  })

  onDestroy(() => {
    unsubFloor()
    unsubHouses()
    unsubEditor()
    unsubPassDebug()
    unsubFurniture()
    for (const [, result] of houses) {
      disposeHouseGroup(result.houseGroup)
    }
    houses.clear()
    disposeHousingMaterials()
    debugLineMaterial.dispose()
    debugFurnitureMaterial.dispose()
  })

  function syncHouses(allHouses: HouseData[]) {
    const incomingById = new Map(allHouses.map((h) => [h.id, h]))

    // Remove houses no longer present
    for (const [id, result] of houses) {
      if (!incomingById.has(id)) {
        occludedHouseIds.delete(id)
        housingGroup.remove(result.houseGroup)
        disposeHouseGroup(result.houseGroup)
        houses.delete(id)
      }
    }

    // Add or rebuild changed houses
    for (const data of allHouses) {
      const existing = houses.get(data.id)
      const newHash = JSON.stringify({ roofs, rooms: data.rooms })

      // Fast path: if only door isOpen changed, sync door states without rebuild
      if (existing && existing.roomsHash === newHash) continue
      if (existing && syncDoorStates(existing, data, newHash)) continue

      if (existing) {
        housingGroup.remove(existing.houseGroup)
        disposeHouseGroup(existing.houseGroup)
      }
      const result = buildHouseGroup(data, newHash, { roofs })
      houses.set(data.id, result)
      housingGroup.add(result.houseGroup)

      // Re-apply visibility if player is inside this house
      if (data.id === currentInsideHouseId) {
        applyFloorVisibility(result, playerInsideFloor)
      }
    }

    if (houses.size > 0 && get(debugVisible)) {
      const s = getStats()
      console.log(
        `[housing] ${s.houses} houses | ${s.mergedMeshes} merged meshes (draw calls)`
      )
    }
  }

  const isOpenReplacer = (_k: string, v: unknown) =>
    _k === 'isOpen' ? undefined : v

  /** Returns true if the only changes were door isOpen flags (no geometry rebuild needed). */
  function syncDoorStates(
    existing: HouseGroupResult,
    data: HouseData,
    newHash: string
  ): boolean {
    // Compare geometry excluding isOpen — both sides stripped from their full hashes
    if (
      JSON.stringify(JSON.parse(newHash), isOpenReplacer) !==
      JSON.stringify(JSON.parse(existing.roomsHash), isOpenReplacer)
    )
      return false

    for (const door of existing.doors) {
      const room = data.rooms[door.roomIndex]
      if (!room) continue
      const seg = getWallByDir(room, door.wallDir)[door.segmentIndex]
      if (seg) door.isOpen = seg.isOpen ?? false
    }

    existing.roomsHash = newHash
    return true
  }

  const DOOR_SWING_SPEED = Math.PI // radians per second (~0.5s for 90°)

  // How far inside the shaft footprint the player must be to acquire the
  // stairs; release uses the full footprint.
  const STAIR_ACQUIRE_MARGIN = 0.1

  /** Called from game loop — loads chunks + checks player inside state */
  export function update(_deltaTime: number) {
    if (!playerPosition) return

    // Rebuild passability debug lines if needed
    if (debugPassDirty && debugPassGroup.visible) rebuildPassabilityDebug()

    // Load housing chunks around player when chunk changes
    const { x: cx, z: cz } = getTerrainChunkFromPosition(
      playerPosition,
      TERRAIN_TILE_SIZE
    )
    if (cx !== lastChunkX || cz !== lastChunkZ) {
      lastChunkX = cx
      lastChunkZ = cz
      housingManager.updateStreaming(playerPosition.x, playerPosition.z)
      housesLoadedHere = housingManager.isLoadedAround(
        playerPosition.x,
        playerPosition.z
      )
    }

    // A respawn/teleport lands before its houses arrive; judging "outdoors"
    // from an empty chunk would clobber the server-synced floor.
    if (!housesLoadedHere) return

    // Player-inside detection (per-room, floor-aware)
    // Use ground-level Y for AABB check, then try multiple floor levels
    // to detect both 1F and 2F rooms
    const groundY = playerPosition.y - lastFloorOffset
    let insideId: string | null = null
    let newOffset = 0
    let effectiveFloor = 0
    let onStairsNow = false

    for (const [id, result] of houses) {
      // XZ-only broad-phase: player.y is forced to terrainY each frame
      // (PlayerControl), which can sit below the house's min.y when terrain
      // dips inside the footprint. findAllRoomsAtPoint does the Y check
      // per floor with a ±1m tolerance, so we don't need Y here.
      if (
        playerPosition.x < result.aabb.min.x ||
        playerPosition.x > result.aabb.max.x ||
        playerPosition.z < result.aabb.min.z ||
        playerPosition.z > result.aabb.max.z
      )
        continue

      // Try all floor levels to find matching rooms
      _allRooms.length = 0
      _seenRooms.clear()
      for (let fl = MAX_FLOOR_LEVEL; fl >= 0; fl--) {
        const testY = groundY + floorYBase(fl, DEFAULT_WALL_HEIGHT) + 1
        for (const r of housingManager.findAllRoomsAtPoint(
          playerPosition.x,
          testY,
          playerPosition.z
        )) {
          const key = `${r.house.id}:${r.roomIndex}`
          if (!_seenRooms.has(key)) {
            _seenRooms.add(key)
            _allRooms.push(r)
          }
        }
      }

      // Pick the stairwell whose Y offset is closest to the player's
      // current Y (lastFloorOffset). This correctly handles stacked
      // stairwells at the same XZ — the player smoothly transitions
      // onto the nearest one rather than jumping to a distant floor.
      let stairResult: (typeof _allRooms)[0] | null = null
      let bestStairDist = Infinity
      let bestStairOffset = 0
      let floorResult: (typeof _allRooms)[0] | null = null
      for (const roomResult of _allRooms) {
        if (roomResult.house.id !== id) continue
        const room = roomResult.house.rooms[roomResult.roomIndex]
        if (room.roomType === 'stairwell') {
          // When already inside, only consider stairwells whose floor range
          // includes the player's current floor — prevents adjacent/stacked
          // stairwells on other floors from catching the player
          if (
            currentInsideHouseId !== null &&
            (playerInsideFloor > room.floorLevel + 1 ||
              playerInsideFloor < room.floorLevel)
          )
            continue
          // Require an inset only when acquiring stairs to prevent edge snaps.
          if (
            !wasOnStairs &&
            !roomContainsXZ(
              roomResult.house,
              room,
              playerPosition.x,
              playerPosition.z,
              STAIR_ACQUIRE_MARGIN
            )
          )
            continue
          const offset = getStairwellYOffset(
            room,
            roomResult.house.origin.x,
            roomResult.house.origin.z,
            playerPosition.x,
            playerPosition.z
          )
          const dist = Math.abs(offset - lastFloorOffset)
          // Reject half-storey entry snaps while already inside a house.
          const maxEntryRise = (room.wallHeight + FLOOR_THICKNESS) / 2
          if (currentInsideHouseId !== null && dist > maxEntryRise) continue
          if (dist < bestStairDist) {
            bestStairDist = dist
            bestStairOffset = offset
            stairResult = roomResult
          }
        } else if (!floorResult || room.floorLevel === playerInsideFloor) {
          floorResult = roomResult
        }
      }

      // Compensate for terrain height difference under the house.
      // The floor mesh is fixed at house.origin.y, but player Y uses
      // terrainHeight(playerPos) + offset. When terrain varies within
      // the house footprint, (origin.y - groundY) corrects the offset.
      const matchedHouse = (stairResult ?? floorResult)?.house
      const terrainComp = matchedHouse ? matchedHouse.origin.y - groundY : 0

      if (stairResult) {
        const room = stairResult.house.rooms[stairResult.roomIndex]
        insideId = id
        onStairsNow = true
        newOffset = terrainComp + bestStairOffset
        const entryFloor = room.floorLevel
        const exitFloor = room.floorLevel + 1
        const entryFloorY =
          terrainComp +
          floorYBase(entryFloor, room.wallHeight) +
          FLOOR_THICKNESS / 2
        const exitFloorY =
          terrainComp +
          floorYBase(exitFloor, room.wallHeight) +
          FLOOR_THICKNESS / 2
        const progress = (newOffset - entryFloorY) / (exitFloorY - entryFloorY)
        effectiveFloor = resolveStairFloor(
          entryFloor,
          playerInsideFloor,
          progress
        )
      } else if (floorResult) {
        const room = floorResult.house.rooms[floorResult.roomIndex]
        insideId = id
        newOffset =
          terrainComp +
          floorYBase(room.floorLevel, room.wallHeight) +
          FLOOR_THICKNESS / 2
        effectiveFloor = room.floorLevel
      }
      if (insideId) break
    }
    wasOnStairs = onStairsNow

    // Update visibility when house or floor changes
    if (
      insideId !== currentInsideHouseId ||
      effectiveFloor !== playerInsideFloor
    ) {
      // Restore previous house
      if (currentInsideHouseId) {
        const prev = houses.get(currentInsideHouseId)
        if (prev) resetFloorVisibility(prev)
      }
      // Clear occlusion if entering a previously-occluded house
      if (insideId && occludedHouseIds.has(insideId)) {
        const curr = houses.get(insideId)
        if (curr) resetOcclusionVisibility(curr)
        occludedHouseIds.delete(insideId)
      }
      // Apply new visibility
      if (insideId) {
        const curr = houses.get(insideId)
        if (curr) applyFloorVisibility(curr, effectiveFloor)
      }
      currentInsideHouseId = insideId
      playerInsideFloor = effectiveFloor
      playerVisualFloorLevel.set(effectiveFloor)
      playerInsideHouseId.set(insideId)
    }

    if (newOffset !== lastFloorOffset) {
      lastFloorOffset = newOffset
      playerFloorOffset.set(newOffset)
    }

    if (currentInsideHouseId) {
      const curr = houses.get(currentInsideHouseId)
      if (curr) {
        const o = curr.houseGroup.position
        applyInteriorGhosts(
          curr,
          playerInsideFloor,
          playerPosition.x - o.x,
          playerPosition.z - o.z
        )
      }
    }

    // Animate door pivots
    const dt = _deltaTime / 1000
    for (const [, result] of houses) {
      for (const door of result.doors) {
        const target = door.isOpen ? door.openAngle : door.closedAngle
        const current = door.pivot.rotation.y
        if (Math.abs(current - target) > 0.01) {
          const step = DOOR_SWING_SPEED * dt
          if (current < target) {
            door.pivot.rotation.y = Math.min(current + step, target)
          } else {
            door.pivot.rotation.y = Math.max(current - step, target)
          }
        }
      }
    }

    // Occlusion pass: hide houses that block the camera view of the player
    // Mark-and-sweep to avoid per-frame Set allocation
    for (const [id, result] of houses) {
      if (id === currentInsideHouseId) continue
      if (
        houseOccludesPlayer(
          result.roomAABBs,
          playerPosition.x,
          playerPosition.y,
          playerPosition.z
        )
      ) {
        if (!occludedHouseIds.has(id)) {
          occludedHouseIds.add(id)
          applyOcclusionVisibility(result)
        }
      } else if (occludedHouseIds.has(id)) {
        occludedHouseIds.delete(id)
        resetOcclusionVisibility(result)
      }
    }
  }

  /**
   * Hide front/back groups based on player floor.
   * Current floor: hide front (south+west walls, roof)
   * Higher floors: hide front, back, and floor; keep stair visible
   * Lower floors: fully visible
   */
  function applyFloorVisibility(result: HouseGroupResult, floor: number) {
    for (const [fl, groups] of result.floorGroups) {
      if (fl === floor) {
        groups.front.position.y = OFFSCREEN_Y
      } else if (fl > floor) {
        groups.front.position.y = OFFSCREEN_Y
        groups.back.position.y = OFFSCREEN_Y
        groups.floor.position.y = OFFSCREEN_Y
        for (const w of groups.interior) w.group.position.y = OFFSCREEN_Y
      }
    }
    applyDoorGhostMaterials(result, floor)
  }

  function resetAllFloorGroupPositions(result: HouseGroupResult) {
    for (const [, groups] of result.floorGroups) {
      groups.front.position.y = 0
      groups.back.position.y = 0
      groups.floor.position.y = 0
      groups.stair.position.y = 0
      for (const w of groups.interior) w.group.position.y = 0
    }
  }

  function resetFloorVisibility(result: HouseGroupResult) {
    resetAllFloorGroupPositions(result)
    resetDoorGhostMaterials(result)
    applyInteriorGhosts(result, null)
  }

  /**
   * Check if a house occludes the player from the isometric SW camera.
   *
   * Camera pitch = atan(1/√2), forward in XZ = (1,0,−1)/√2.
   * A camera ray from (px, py, pz) toward the camera hits a room volume
   * iff there exists s ∈ [sLow, sHigh] such that the room footprint shifted
   * by (s, −s) in XZ contains (px, pz).
   *
   *   sHigh = aabb.max.y − py  (top of room vs player height)
   *   sLow  = max(aabb.min.y − py, 0)
   *
   * Tests each room AABB rather than the merged house AABB so that
   * concave shapes (L/T/U) don't falsely occlude when the player stands
   * in the outdoor concave gap — the ray passes through the gap and
   * misses every room.
   *
   * Requires MIN_OCCLUSION_DEPTH of ray inside the AABB before counting
   * it as occluding. The AABB extends ROOF_OVERHANG past walls, so a
   * player standing right at a wall grazes the AABB without the wall
   * actually being between them and the camera.
   */
  const MIN_OCCLUSION_DEPTH = ROOF_OVERHANG + WALL_THICKNESS
  function houseOccludesPlayer(
    roomAABBs: THREE.Box3[],
    px: number,
    py: number,
    pz: number
  ): boolean {
    for (const aabb of roomAABBs) {
      const sHigh = aabb.max.y - py
      if (sHigh <= 0) continue
      const sLow = Math.max(aabb.min.y - py, 0)
      const sMin = Math.max(px - aabb.max.x, aabb.min.z - pz, sLow)
      const sMax = Math.min(px - aabb.min.x, aabb.max.z - pz, sHigh)
      if (sMax - sMin > MIN_OCCLUSION_DEPTH) return true
    }
    return false
  }

  const _noop = () => {}

  /** Disable/enable raycasting on all meshes inside a group. */
  function setGroupRaycast(group: THREE.Group, enabled: boolean) {
    group.traverse((obj) => {
      if (!(obj instanceof THREE.Mesh)) return
      if (enabled) {
        if (obj.userData._origRaycast) {
          obj.raycast = obj.userData._origRaycast
          delete obj.userData._origRaycast
        }
      } else {
        if (!obj.userData._origRaycast) {
          obj.userData._origRaycast = obj.raycast
        }
        obj.raycast = _noop
      }
    })
  }

  // Toggling .visible avoids matrixWorld recalculations on occlusion changes.
  function applyOcclusionVisibility(result: HouseGroupResult) {
    for (const [fl, groups] of result.floorGroups) {
      groups.front.visible = false
      groups.back.visible = false
      groups.stair.visible = false
      for (const w of groups.interior) w.group.visible = false
      if (fl !== 0) {
        groups.floor.visible = false
      }
    }
    for (const door of result.doors) {
      door.pivot.visible = false
    }
    setGroupRaycast(result.houseGroup, false)
  }

  function resetOcclusionVisibility(result: HouseGroupResult) {
    for (const [, groups] of result.floorGroups) {
      groups.front.visible = true
      groups.back.visible = true
      groups.floor.visible = true
      groups.stair.visible = true
      for (const w of groups.interior) w.group.visible = true
    }
    for (const door of result.doors) {
      door.pivot.visible = true
    }
    setGroupRaycast(result.houseGroup, true)
  }

  /** Pre-load housing chunks around the player so geometry is ready before
   *  the loading screen is dismissed. Without this, chunk data arrives during
   *  gameplay and the first render of each house stalls WebGPU. */
  export async function preloadChunks(px: number, pz: number) {
    housingManager.updateStreaming(px, pz)
    await housingManager.waitForPending()
  }

  export function warmupHousingPipelines() {
    const boxGeo = new THREE.BoxGeometry(0.1, 0.1, 0.1)
    const warmupGroup = new THREE.Group()
    warmupGroup.name = 'housingWarmup'
    warmupGroup.position.y = OFFSCREEN_Y

    const addDummy = (mat: THREE.Material) => {
      const mesh = new THREE.Mesh(boxGeo, mat)
      mesh.castShadow = true
      mesh.receiveShadow = true
      mesh.frustumCulled = false
      warmupGroup.add(mesh)
    }

    for (let i = 0; i < HOUSING_TEXTURES.length; i++)
      addDummy(getHousingMaterial(i))
    for (const idx of [WOOD_TEXTURE_IDX, SHUTTER_PANEL_TEXTURE_IDX])
      addDummy(getGhostHousingMaterial(idx))

    housingGroup.add(warmupGroup)

    // Keep dummies alive long enough for ALL render passes to compile their
    // pipelines — main, shadow, AND refraction (which starts after
    // MULTI_PASS_WARMUP_FRAMES and only renders every other frame).
    // 3 frames was too few: the refraction pass hadn't started yet, so housing
    // materials were never compiled for the refraction render target, causing
    // synchronous pipeline stalls when houses first entered the refraction camera.
    let framesLeft = 12
    const tick = () => {
      if (--framesLeft > 0) {
        requestAnimationFrame(tick)
        return
      }
      housingGroup.remove(warmupGroup)
      boxGeo.dispose()
    }
    requestAnimationFrame(tick)
  }

  export function getGroup(): THREE.Group {
    return housingGroup
  }

  export function getDoorMeshes(): THREE.Object3D[] {
    const result: THREE.Object3D[] = []
    for (const h of houses.values()) {
      for (const door of h.doors) result.push(door.pivot)
    }
    return result
  }

  /** Return housing draw call stats for profiling. */
  export function getStats() {
    let mergedMeshes = 0
    for (const [, result] of houses) {
      mergedMeshes += result.mergedMeshCount
    }
    return {
      houses: houses.size,
      mergedMeshes,
    }
  }
</script>

<T is={housingGroup} />
