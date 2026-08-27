<script lang="ts">
  import * as THREE from 'three'
  import { onMount } from 'svelte'
  import {
    hoveredCell,
    brushSize,
    brushStrength,
    brushRaiseMode,
    brushMode,
    brushWorldPos,
    cursorHeight,
    editorTool,
    splatLayer,
    editorPanOffset,
    currentEditorRegion,
    editorHeightManager,
    editorSplatManager,
    zoneDrawStart,
    zoneSubTool,
    editorZoneManager,
    currentZoneData,
    spawnFormMonsterType,
    spawnFormMaxPerPlayer,
    spawnFormMaxTotal,
    spawnFormIntervalSecs,
    noSpawnFormLabel,
    npcNames,
    selectedNpc,
    selectedNpcSchedule,
    selectedScheduleIndex,
    draggingWaypointIndex,
    selectedObjectType,
    objectRotation,
    currentObjectData,
    selectedObjectPlacementId,
    objectPreviewPos,
    objectSubTool,
    roadDrawStart,
  } from '../../stores/editorStore'
  import type {
    EditorTool,
    ZoneSubTool,
    ObjectSubTool,
  } from '../../stores/editorStore'
  import { NpcScheduleManager } from '../../managers/npcScheduleManager'
  import type { NpcScheduleData } from '../../managers/npcScheduleManager'
  import { objectManager } from '../../managers/objectManager'
  import { furnitureManager } from '../../managers/furnitureManager'
  import {
    commitPlacements,
    deleteSelectedPlacement,
    duplicateSelectedPlacement,
    nextPlacementId,
  } from './object-edit'
  import { findAncestorWithUserData } from '../../managers/inputHandler'
  import { housingManager } from '../../managers/housingManager'
  import { playerVisualFloorLevel } from '../../stores/housingStore'
  import { floorYBase, DEFAULT_WALL_HEIGHT } from '../../utils/house-geo-utils'
  import { TERRAIN_TILE_SIZE } from '../game-scene/terrain-utils'
  import { ORTHOGRAPHIC_FRUSTUM_HEIGHT } from '../game-scene/camera-utils'
  import { get } from 'svelte/store'
  import type { TerrainTile } from '../game-scene/terrain-utils'
  import type { TerrainHeightManager } from '../../managers/terrainHeightManager'
  import type { TerrainSplatManager } from '../../managers/terrainSplatManager'
  import type { TerrainGrassDataManager } from '../../managers/terrainGrassDataManager'
  import { TILE_DIM } from '../../managers/terrain-height-types'
  import { tileToRegion } from '../../terrain/terrain-constants'
  import { gameStore } from '../../stores/gameStore'
  import { remotePlayerManager } from '../../managers/remotePlayerManager'
  import { SvelteSet } from 'svelte/reactivity'
  import { filterGrassData } from '../../utils/grass-data'
  import { SHORT_GRASS_R_MIN } from '../../shaders/grass-material'

  interface Props {
    camera: THREE.OrthographicCamera | undefined
    terrainMeshes: (THREE.Mesh | undefined)[]
    terrainTiles: TerrainTile[]
    heightManager: TerrainHeightManager | null
    splatManager: TerrainSplatManager | null
    grassDataManager: TerrainGrassDataManager | null
    /** Live accessor for the object-overlay group, so object selection can cast
     *  a ray at the placed object meshes. */
    getObjectGroup?: () => THREE.Group | null
  }

  let {
    camera,
    terrainMeshes,
    terrainTiles: _terrainTiles,
    heightManager,
    splatManager,
    grassDataManager = null,
    getObjectGroup,
  }: Props = $props()

  let isPainting = $state(false)
  let isPanning = $state(false)
  let lastPanX = $state(0)
  let lastPanY = $state(0)
  let shiftHeld = $state(false)
  let ctrlHeld = $state(false)
  let lastPaintTime = $state(0)

  let currentBrushSize = $state(3)
  let currentBrushStrength = $state(5)
  let currentBrushRaise = $state(true)
  let currentTool = $state<EditorTool>('height')
  let currentSplatLayer = $state(0)
  let currentZoneSubTool = $state<ZoneSubTool>('noSpawn')
  let currentZoneDrawStart = $state<{ x: number; z: number } | null>(null)
  let currentRoadDrawStart = $state<{ x: number; z: number } | null>(null)

  // NPC editor state
  let currentNpcNames = $state<string[]>([])
  let currentNpcSchedule = $state<NpcScheduleData | null>(null)
  let currentSchedIdx = $state(0)
  let currentDragWaypoint = $state<number | null>(null)
  let npcManager: NpcScheduleManager | null = null

  // Object editor state
  let currentObjectType = $state<string | null>(null)
  let currentObjectRot = $state(0)
  let currentObjectSubTool = $state<ObjectSubTool>('place')
  let currentPlayerFloor = $state(0)

  function snapXZ(x: number, z: number): { x: number; z: number } {
    if (!currentObjectType) return { x, z }
    const def = objectManager.getCatalogEntry(currentObjectType)
    if (def?.gridAlign) {
      return { x: Math.round(x), z: Math.round(z) }
    }
    return { x, z }
  }

  brushSize.subscribe((v) => (currentBrushSize = v))
  brushStrength.subscribe((v) => (currentBrushStrength = v))
  brushRaiseMode.subscribe((v) => {
    currentBrushRaise = v
    syncBrushMode()
  })
  editorTool.subscribe((v) => {
    currentTool = v
    // Clear draw state when switching away from the owning tool
    if (v !== 'zone') {
      zoneDrawStart.set(null)
    }
    if (v !== 'road') {
      roadDrawStart.set(null)
    }
  })
  splatLayer.subscribe((v) => (currentSplatLayer = v))
  zoneSubTool.subscribe((v) => (currentZoneSubTool = v))
  zoneDrawStart.subscribe((v) => (currentZoneDrawStart = v))
  roadDrawStart.subscribe((v) => (currentRoadDrawStart = v))
  npcNames.subscribe((v) => (currentNpcNames = v))
  selectedNpcSchedule.subscribe((v) => (currentNpcSchedule = v))
  selectedScheduleIndex.subscribe((v) => (currentSchedIdx = v))
  draggingWaypointIndex.subscribe((v) => (currentDragWaypoint = v))
  selectedObjectType.subscribe((v) => (currentObjectType = v))
  objectRotation.subscribe((v) => (currentObjectRot = v))
  objectSubTool.subscribe((v) => (currentObjectSubTool = v))
  playerVisualFloorLevel.subscribe((v) => (currentPlayerFloor = v))

  function syncBrushMode() {
    if (ctrlHeld) {
      brushMode.set('flatten')
    } else {
      const raise = shiftHeld ? !currentBrushRaise : currentBrushRaise
      brushMode.set(raise ? 'raise' : 'lower')
    }
  }

  const raycaster = new THREE.Raycaster()
  const mouseNDC = new THREE.Vector2()
  const _panRight = new THREE.Vector3()
  const _panUp = new THREE.Vector3()
  const _panFwd = new THREE.Vector3()

  let lastWorldPos = { x: 0, z: 0 }
  let lastRegionX = NaN
  let lastRegionZ = NaN

  /** Aim `raycaster` down the ray through the click point. Returns false if
   *  there's no camera yet. */
  function setRayFromEvent(event: MouseEvent): boolean {
    if (!camera) return false
    const rect = (event.target as HTMLElement).getBoundingClientRect()
    mouseNDC.set(
      ((event.clientX - rect.left) / rect.width) * 2 - 1,
      -((event.clientY - rect.top) / rect.height) * 2 + 1
    )
    raycaster.setFromCamera(mouseNDC, camera)
    return true
  }

  function raycastTerrain(event: MouseEvent): THREE.Intersection | null {
    const meshes = terrainMeshes.filter((m): m is THREE.Mesh => m !== undefined)
    if (meshes.length === 0 || !setRayFromEvent(event)) return null

    const intersects = raycaster.intersectObjects(meshes, false)
    return intersects.length > 0 ? intersects[0] : null
  }

  /** Pick the placement to select from a click, casting a real ray against the
   *  object meshes so a small prop resting on a big one (e.g. a sword on a
   *  table) can be picked by clicking it. Returns the nearest hit's placement;
   *  if that placement is already selected, cycles to the next object further
   *  down the same ray so stacked objects can all be reached by re-clicking.
   *  Returns null when the ray misses every object (caller falls back to the
   *  forgiving XZ-nearest test). */
  function pickObjectIdAlongRay(event: MouseEvent): number | null {
    const group = getObjectGroup?.()
    if (!group || !setRayFromEvent(event)) return null
    const hits = raycaster.intersectObjects(group.children, true)
    // Unique placement ids in ray order (nearest surface first).
    const ids: number[] = []
    for (const h of hits) {
      // The selection box is a LineSegments child of the selected clone; its
      // wide raycast threshold would otherwise inject phantom hits.
      if (h.object instanceof THREE.LineSegments) continue
      const owner = findAncestorWithUserData(h.object, 'objectId')
      const id = owner?.userData.objectId as number | undefined
      if (id != null && !ids.includes(id)) ids.push(id)
    }
    if (ids.length === 0) return null
    const cur = get(selectedObjectPlacementId)
    const idx = cur == null ? -1 : ids.indexOf(cur)
    // Re-clicking the same stack cycles to the next object down the ray;
    // clicking a fresh spot selects the topmost.
    return idx === -1 ? ids[0] : ids[(idx + 1) % ids.length]
  }

  function updateCursorFromHit(hit: THREE.Intersection) {
    const mesh = hit.object as THREE.Mesh

    const localX = hit.point.x - mesh.position.x
    const localZ = hit.point.z - mesh.position.z

    const cellX = Math.max(
      0,
      Math.min(63, Math.floor(localX + TERRAIN_TILE_SIZE / 2))
    )
    const cellZ = Math.max(
      0,
      Math.min(63, Math.floor(localZ + TERRAIN_TILE_SIZE / 2))
    )

    const tileX = Math.round(mesh.position.x / TERRAIN_TILE_SIZE)
    const tileZ = Math.round(mesh.position.z / TERRAIN_TILE_SIZE)

    const worldX = mesh.position.x - TERRAIN_TILE_SIZE / 2 + cellX + 0.5
    const worldZ = mesh.position.z - TERRAIN_TILE_SIZE / 2 + cellZ + 0.5

    hoveredCell.set({ tileX, tileZ, cellX, cellZ, worldX, worldZ })
    lastWorldPos = { x: hit.point.x, z: hit.point.z }
    // Only show brush overlay for height/splat tools
    if (
      currentTool === 'height' ||
      currentTool === 'splat' ||
      currentTool === 'road'
    ) {
      brushWorldPos.set({ x: hit.point.x, z: hit.point.z })
    } else {
      brushWorldPos.set(null)
    }

    if (heightManager) {
      cursorHeight.set(
        heightManager.getHeightAtCell(tileX, tileZ, cellX, cellZ)
      )
    }

    // Track which region the cursor is in so panels / objects can react.
    const rx = tileToRegion(tileX)
    const rz = tileToRegion(tileZ)
    if (rx !== lastRegionX || rz !== lastRegionZ) {
      lastRegionX = rx
      lastRegionZ = rz
      currentEditorRegion.set({ rx, rz })
      loadObjectForRegion(rx, rz)
    }
  }

  function getPaintIntervalMs(): number {
    return (11 - currentBrushStrength) * 100
  }

  /**
   * Build an `isProtected` callback that rejects vertices under houses.
   * Pre-filters to rooms intersecting the given region so the hot-path check
   * is O(nearby rooms) rather than O(all rooms in world).
   */
  function makeHouseProtector(
    minX: number,
    maxX: number,
    minZ: number,
    maxZ: number
  ): ((wx: number, wz: number) => boolean) | undefined {
    const aabbs = housingManager.collectRoomAABBsInRegion(
      minX,
      maxX,
      minZ,
      maxZ
    )
    if (aabbs.length === 0) return undefined
    return (wx, wz) => {
      for (const a of aabbs) {
        if (wx >= a.minX && wx <= a.maxX && wz >= a.minZ && wz <= a.maxZ) {
          return true
        }
      }
      return false
    }
  }

  const SPLAT_CHANNELS = 4
  const vegetationDirtyTiles = new SvelteSet<string>()
  let vegetationTimer: ReturnType<typeof setTimeout> | null = null

  function markVegetationDirty(tiles: { tileX: number; tileZ: number }[]) {
    for (const { tileX, tileZ } of tiles) {
      vegetationDirtyTiles.add(`${tileX},${tileZ}`)
    }
    if (vegetationTimer !== null) clearTimeout(vegetationTimer)
    vegetationTimer = setTimeout(flushVegetationRemoval, 500)
  }

  function flushVegetationRemoval() {
    vegetationTimer = null
    if (!splatManager || vegetationDirtyTiles.size === 0) return

    for (const key of vegetationDirtyTiles) {
      const [tileX, tileZ] = key.split(',').map(Number)
      const splatData = splatManager.getSplatData(tileX, tileZ)
      if (!splatData) continue

      const tileMinX = tileX * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
      const tileMinZ = tileZ * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2

      const shouldRemove = (x: number, z: number) => {
        const cx = Math.floor(x - tileMinX)
        const cz = Math.floor(z - tileMinZ)
        if (cx < 0 || cx >= TILE_DIM || cz < 0 || cz >= TILE_DIM) return false
        return (
          splatData[(cz * TILE_DIM + cx) * SPLAT_CHANNELS + 3] <
          SHORT_GRASS_R_MIN
        )
      }

      if (grassDataManager) {
        const grassData = grassDataManager.getCachedGrassData(tileX, tileZ)
        if (grassData) {
          const filtered = filterGrassData(grassData, shouldRemove)
          if (filtered) grassDataManager.saveGrassData(tileX, tileZ, filtered)
        }
      }
    }
    vegetationDirtyTiles.clear()
  }

  function applyBrushAtCursor() {
    const now = performance.now()
    if (lastPaintTime === 0) {
      lastPaintTime = now
      return
    }
    const elapsed = now - lastPaintTime
    if (elapsed < getPaintIntervalMs()) return
    lastPaintTime = now

    if (currentTool === 'splat') {
      if (!splatManager) return
      const affectedTiles = splatManager.applySplatBrush(
        lastWorldPos.x,
        lastWorldPos.z,
        currentBrushSize,
        currentSplatLayer,
        currentBrushStrength / 50
      )
      if (currentSplatLayer !== 0 && affectedTiles.length > 0) {
        markVegetationDirty(affectedTiles)
      }
    } else {
      if (!heightManager) return
      const r = currentBrushSize
      const protectHouses = makeHouseProtector(
        lastWorldPos.x - r,
        lastWorldPos.x + r,
        lastWorldPos.z - r,
        lastWorldPos.z + r
      )
      if (ctrlHeld) {
        heightManager.applyFlatten(
          lastWorldPos.x,
          lastWorldPos.z,
          currentBrushSize,
          protectHouses
        )
      } else {
        const raise = shiftHeld ? !currentBrushRaise : currentBrushRaise
        heightManager.applyBrush(
          lastWorldPos.x,
          lastWorldPos.z,
          currentBrushSize,
          0.1,
          raise,
          1,
          protectHouses
        )
      }
    }
  }

  function handleMouseMove(event: MouseEvent) {
    if (isPanning) {
      if (!camera) return
      const dx = event.clientX - lastPanX
      const dy = event.clientY - lastPanY
      lastPanX = event.clientX
      lastPanY = event.clientY

      // Get camera basis vectors projected onto XZ plane
      camera.matrixWorld.extractBasis(_panRight, _panUp, _panFwd)
      _panRight.y = 0
      _panRight.normalize()
      _panFwd.y = 0
      _panFwd.normalize()

      // Convert screen pixels to world units for orthographic camera
      const rect = (event.target as HTMLElement).getBoundingClientRect()
      const scale = ORTHOGRAPHIC_FRUSTUM_HEIGHT / (camera.zoom * rect.height)

      const current = get(editorPanOffset)
      editorPanOffset.set({
        x: current.x - (_panRight.x * dx + _panFwd.x * dy) * scale,
        z: current.z - (_panRight.z * dx + _panFwd.z * dy) * scale,
      })
      return
    }

    const hit = raycastTerrain(event)

    if (!hit) {
      hoveredCell.set(null)
      brushWorldPos.set(null)
      return
    }

    updateCursorFromHit(hit)

    if (
      currentTool === 'object' &&
      currentObjectSubTool === 'place' &&
      currentObjectType
    ) {
      const snapped = snapXZ(hit.point.x, hit.point.z)
      const terrainY = heightManager
        ? heightManager.getHeightAtWorldPosition(snapped.x, snapped.z)
        : 0
      const y = objectSpawnY(currentObjectType, terrainY, currentPlayerFloor)
      objectPreviewPos.set({ x: snapped.x, y, z: snapped.z })
    } else if (currentTool !== 'object') {
      objectPreviewPos.set(null)
    }

    if (currentTool === 'npc' && currentDragWaypoint !== null) {
      handleNpcDrag(hit.point.x, hit.point.z)
      return
    }

    if (isPainting) {
      applyBrushAtCursor()
    }
  }

  // --- NPC waypoint interaction ---

  function findClosestWaypoint(worldX: number, worldZ: number): number | null {
    if (!currentNpcSchedule) return null
    const entry = currentNpcSchedule.schedule[currentSchedIdx]
    if (!entry) return null

    const threshold = 4
    let bestDist = threshold * threshold
    let bestIdx: number | null = null

    // Check home position (index -1)
    const hdx = worldX - entry.pos[0]
    const hdz = worldZ - entry.pos[2]
    const homeDist = hdx * hdx + hdz * hdz
    if (homeDist < bestDist) {
      bestDist = homeDist
      bestIdx = -1
    }

    // Check waypoints
    const waypoints = entry.waypoints
    for (let i = 0; i < waypoints.length; i++) {
      const wp = waypoints[i]
      const dx = worldX - wp[0]
      const dz = worldZ - wp[2]
      const dist = dx * dx + dz * dz
      if (dist < bestDist) {
        bestDist = dist
        bestIdx = i
      }
    }

    return bestIdx
  }

  function findClosestNpc(
    worldX: number,
    worldZ: number,
    threshold: number
  ): string | null {
    const state = get(gameStore)
    const names = currentNpcNames
    if (names.length === 0) return null

    let bestDist = threshold * threshold
    let bestName: string | null = null

    for (const [id, player] of state.otherPlayers) {
      const nameLower = player.name.toLowerCase()
      if (!names.includes(nameLower)) continue
      const rp = remotePlayerManager.players.get(id)
      if (!rp) continue
      const dx = worldX - rp.position.x
      const dz = worldZ - rp.position.z
      const dist = dx * dx + dz * dz
      if (dist < bestDist) {
        bestDist = dist
        bestName = nameLower
      }
    }
    return bestName
  }

  async function loadNpcSchedule(name: string) {
    selectedNpcSchedule.set(null)
    selectedScheduleIndex.set(0)
    selectedNpc.set(name)
    try {
      if (!npcManager) npcManager = new NpcScheduleManager()
      const data = await npcManager.fetchSchedule(name)
      if (get(selectedNpc) === name) {
        selectedNpcSchedule.set(data)
      }
    } catch (e) {
      console.error(`[NPC] Failed to fetch schedule for '${name}':`, e)
    }
  }

  function handleNpcMouseDown(worldX: number, worldZ: number) {
    // NPC selection takes priority over waypoint dragging (tight 3-unit radius)
    const clickedNpc = findClosestNpc(worldX, worldZ, 3)
    if (clickedNpc) {
      loadNpcSchedule(clickedNpc)
      return
    }
    // Then try to grab a waypoint
    const wpIdx = findClosestWaypoint(worldX, worldZ)
    if (wpIdx !== null) {
      draggingWaypointIndex.set(wpIdx)
      return
    }
    // Fall back to wider NPC search (5-unit radius)
    const nearbyNpc = findClosestNpc(worldX, worldZ, 5)
    if (nearbyNpc) loadNpcSchedule(nearbyNpc)
  }

  function handleNpcDrag(worldX: number, worldZ: number) {
    if (currentDragWaypoint === null || !currentNpcSchedule) return
    const entry = currentNpcSchedule.schedule[currentSchedIdx]
    if (!entry) return

    const y = heightManager
      ? heightManager.getHeightAtWorldPosition(worldX, worldZ)
      : 0

    // Clone schedule to trigger Svelte reactivity (new object reference)
    const updated: NpcScheduleData = {
      schedule: currentNpcSchedule.schedule.map((s, i) => {
        if (i !== currentSchedIdx) return s
        const newEntry = { ...s }
        if (currentDragWaypoint === -1) {
          newEntry.pos = [worldX, y, worldZ]
        } else {
          newEntry.waypoints = [...s.waypoints]
          newEntry.waypoints[currentDragWaypoint!] = [worldX, y, worldZ]
        }
        return newEntry
      }),
    }
    selectedNpcSchedule.set(updated)
  }

  // --- Object interaction ---

  async function loadObjectForRegion(rx: number, rz: number) {
    const data = await objectManager.fetchObject(rx, rz)
    currentObjectData.set(data)
    selectedObjectPlacementId.set(null)
  }

  /** Spawn height for a new placement: the catalog's absolute `defaultY` if set
   *  (e.g. shop signs), otherwise the terrain height plus the floor base. */
  function objectSpawnY(
    objectType: string,
    terrainY: number,
    floor: number
  ): number {
    const def = objectManager.getCatalogEntry(objectType)
    return def?.defaultY ?? terrainY + floorYBase(floor, DEFAULT_WALL_HEIGHT)
  }

  async function handleObjectMouseDown(
    worldX: number,
    worldZ: number,
    event: MouseEvent
  ) {
    if (currentObjectSubTool === 'place') {
      if (!currentObjectType) return
      const snapped = snapXZ(worldX, worldZ)
      const terrainY = heightManager
        ? heightManager.getHeightAtWorldPosition(snapped.x, snapped.z)
        : 0
      const y = objectSpawnY(currentObjectType, terrainY, currentPlayerFloor)
      const data = get(currentObjectData)
      const placement = {
        id: nextPlacementId(data),
        type: currentObjectType,
        x: snapped.x,
        y,
        z: snapped.z,
        rotation: furnitureManager.snapRotation(
          currentObjectType,
          currentObjectRot
        ),
        floorLevel: currentPlayerFloor,
      }
      await commitPlacements([...data.placements, placement])
    } else {
      // Precise pick first: cast a ray at the actual object meshes so clicking
      // a small prop on top of a larger one selects the prop (and re-clicking
      // cycles through the stack).
      const picked = pickObjectIdAlongRay(event)
      if (picked != null) {
        selectedObjectPlacementId.set(picked)
        return
      }
      // Fallback: forgiving XZ-nearest test (click near an object's base, or
      // hit nothing pickable such as a flat/procedural object).
      const data = get(currentObjectData)
      const threshold = 4
      let bestDist = threshold * threshold
      let bestId: number | null = null
      for (const p of data.placements) {
        // Skip orphans whose type was removed from the catalog — they have no
        // visible mesh, so letting them win the click intercepts selection.
        const def = objectManager.getCatalogEntry(p.type)
        if (!def) continue
        const dx = worldX - p.x
        const dz = worldZ - p.z
        let distSq: number
        if (def.kind === 'bridge' && def.bridge) {
          // Bridges are long — distance-to-center misses clicks on the deck
          // ends. Measure distance to the rotated deck rect instead so any
          // click on (or near) the deck selects this placement.
          const m = def.bridge
          const rot = (p.rotation * Math.PI) / 180
          const cos = Math.cos(rot)
          const sin = Math.sin(rot)
          const lx = dx * cos - dz * sin
          const lz = dx * sin + dz * cos
          const ddx = Math.max(m.deckMinX - lx, 0, lx - m.deckMaxX)
          const ddz = Math.max(m.deckMinZ - lz, 0, lz - m.deckMaxZ)
          distSq = ddx * ddx + ddz * ddz
        } else {
          distSq = dx * dx + dz * dz
        }
        if (distSq < bestDist) {
          bestDist = distSq
          bestId = p.id
        }
      }
      selectedObjectPlacementId.set(bestId)
    }
  }

  function handleRoadClick(worldX: number, worldZ: number) {
    if (!currentRoadDrawStart) {
      roadDrawStart.set({ x: worldX, z: worldZ })
      return
    }

    const x1 = currentRoadDrawStart.x
    const z1 = currentRoadDrawStart.z
    const x2 = worldX
    const z2 = worldZ

    const dx = x2 - x1
    const dz = z2 - z1
    if (dx * dx + dz * dz < 0.01) return // ignore duplicate click, keep start

    if (heightManager) {
      const margin = currentBrushSize * 2 // matches applyFlattenLine's blendRadius
      const protectHouses = makeHouseProtector(
        Math.min(x1, x2) - margin,
        Math.max(x1, x2) + margin,
        Math.min(z1, z2) - margin,
        Math.max(z1, z2) + margin
      )
      heightManager.applyFlattenLine(
        x1,
        z1,
        x2,
        z2,
        currentBrushSize,
        protectHouses
      )
    }
    if (splatManager) {
      const strength = Math.max(0.1, currentBrushStrength / 10)
      const affectedTiles = splatManager.applySplatLine(
        x1,
        z1,
        x2,
        z2,
        currentBrushSize,
        currentSplatLayer,
        strength
      )
      if (currentSplatLayer !== 0 && affectedTiles.length > 0) {
        markVegetationDirty(affectedTiles)
      }
    }

    roadDrawStart.set(null)
  }

  async function handleZoneClick(worldX: number, worldZ: number) {
    if (currentZoneDrawStart) {
      // Second click: finish the rectangle
      const minX = Math.min(currentZoneDrawStart.x, worldX)
      const minZ = Math.min(currentZoneDrawStart.z, worldZ)
      const maxX = Math.max(currentZoneDrawStart.x, worldX)
      const maxZ = Math.max(currentZoneDrawStart.z, worldZ)

      const mgr = get(editorZoneManager)
      const region = get(currentEditorRegion)
      if (mgr && region) {
        const zoneData = get(currentZoneData)
        let updated
        if (currentZoneSubTool === 'noSpawn') {
          const label = get(noSpawnFormLabel).trim()
          const zone = { minX, minZ, maxX, maxZ, ...(label ? { label } : {}) }
          const zones = [...(zoneData.noSpawnZones ?? []), zone]
          updated = { ...zoneData, noSpawnZones: zones }
        } else {
          const spawns = [
            ...(zoneData.monsterSpawns ?? []),
            {
              monsterType: get(spawnFormMonsterType),
              maxPerPlayer: get(spawnFormMaxPerPlayer),
              maxTotal: get(spawnFormMaxTotal),
              spawnIntervalSecs: get(spawnFormIntervalSecs),
              minX,
              minZ,
              maxX,
              maxZ,
            },
          ]
          updated = { ...zoneData, monsterSpawns: spawns }
        }
        await mgr.saveZone(region.rx, region.rz, updated)
        currentZoneData.set(updated)
        if (currentZoneSubTool === 'noSpawn') noSpawnFormLabel.set('')
      }

      zoneDrawStart.set(null)
    } else {
      // First click: store the start corner
      zoneDrawStart.set({ x: worldX, z: worldZ })
    }
  }

  function handleMouseDown(event: MouseEvent) {
    if (event.button === 1) {
      event.preventDefault()
      isPanning = true
      lastPanX = event.clientX
      lastPanY = event.clientY
      return
    }
    if (event.button !== 0) return
    event.preventDefault()
    const hit = raycastTerrain(event)
    if (!hit) return

    if (currentTool === 'zone') {
      updateCursorFromHit(hit)
      handleZoneClick(hit.point.x, hit.point.z)
      return
    }

    if (currentTool === 'road') {
      updateCursorFromHit(hit)
      handleRoadClick(hit.point.x, hit.point.z)
      return
    }

    if (currentTool === 'npc') {
      updateCursorFromHit(hit)
      handleNpcMouseDown(hit.point.x, hit.point.z)
      return
    }

    if (currentTool === 'object') {
      updateCursorFromHit(hit)
      handleObjectMouseDown(hit.point.x, hit.point.z, event)
      return
    }

    isPainting = true
    lastPaintTime = 0
    updateCursorFromHit(hit)
  }

  function handleMouseUp(event: MouseEvent) {
    if (event.button === 1) {
      isPanning = false
      return
    }
    if (event.button !== 0) return
    if (currentDragWaypoint !== null) {
      draggingWaypointIndex.set(null)
    }
    isPainting = false
    lastPaintTime = 0
    flushVegetationRemoval()
  }

  function handleKeyDown(event: KeyboardEvent) {
    if (event.key === 'Shift') {
      shiftHeld = true
      syncBrushMode()
    }
    if (event.key === 'Control') {
      ctrlHeld = true
      syncBrushMode()
    }
    if (currentTool === 'object') {
      if (event.key === 'r' || event.key === 'R') {
        // Solid furniture rotates in 90° steps so its footprint lands on whole
        // cells; other objects keep the finer 45° step.
        const step =
          currentObjectType && furnitureManager.isSolid(currentObjectType)
            ? 90
            : 45
        objectRotation.update((r) => (r + step) % 360)
      }
      if (event.key === 'Delete' || event.key === 'Backspace') {
        deleteSelectedPlacement()
      }
      if ((event.ctrlKey || event.metaKey) && event.key === 'd') {
        event.preventDefault()
        duplicateSelectedPlacement()
      }
    }
    if (
      event.key === 'Escape' &&
      currentTool === 'road' &&
      currentRoadDrawStart
    ) {
      roadDrawStart.set(null)
    }
  }

  function handleKeyUp(event: KeyboardEvent) {
    if (event.key === 'Shift') {
      shiftHeld = false
      syncBrushMode()
    }
    if (event.key === 'Control') {
      ctrlHeld = false
      syncBrushMode()
    }
  }

  function handleWheel(event: WheelEvent) {
    if (event.ctrlKey) {
      event.preventDefault()
      const delta = event.deltaY > 0 ? -1 : 1
      const newSize = Math.max(1, Math.min(10, currentBrushSize + delta))
      brushSize.set(newSize)
    } else {
      if (!camera) return
      event.preventDefault()
      const factor = event.deltaY > 0 ? 0.95 : 1 / 0.95
      camera.zoom = Math.max(0.15, Math.min(2, camera.zoom * factor))
      camera.updateProjectionMatrix()
    }
  }

  function handleMouseOut() {
    hoveredCell.set(null)
    cursorHeight.set(null)
    brushWorldPos.set(null)
    isPainting = false
    isPanning = false
    lastPaintTime = 0
  }

  onMount(() => {
    if (heightManager) editorHeightManager.set(heightManager)
    if (splatManager) editorSplatManager.set(splatManager)

    const canvas = document.querySelector('canvas')
    if (!canvas) return

    canvas.addEventListener('mousemove', handleMouseMove, true)
    canvas.addEventListener('mousedown', handleMouseDown, true)
    canvas.addEventListener('mouseup', handleMouseUp, true)
    canvas.addEventListener('mouseleave', handleMouseOut)
    canvas.addEventListener('wheel', handleWheel, { passive: false })
    window.addEventListener('keydown', handleKeyDown)
    window.addEventListener('keyup', handleKeyUp)

    return () => {
      canvas.removeEventListener('mousemove', handleMouseMove, true)
      canvas.removeEventListener('mousedown', handleMouseDown, true)
      canvas.removeEventListener('mouseup', handleMouseUp, true)
      canvas.removeEventListener('mouseleave', handleMouseOut)
      canvas.removeEventListener('wheel', handleWheel)
      window.removeEventListener('keydown', handleKeyDown)
      window.removeEventListener('keyup', handleKeyUp)
      hoveredCell.set(null)
      brushWorldPos.set(null)
    }
  })
</script>
