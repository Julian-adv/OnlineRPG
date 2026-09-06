import { apiFetch, getTerrainApiUrl } from '../utils/networkUtils'
import {
  TERRAIN_TILE_SIZE,
  getTerrainChunkFromPosition,
} from '../components/game-scene/terrain-utils'
import type { HouseData } from '../types/housing'
import type { WallDirection } from '../utils/house-geometry'
import { shortestWrappedDeltaX } from '../terrain/world-wrap'
import { setHouseMapFootprints } from '../stores/housingMapStore'
import {
  ALL_WALL_DIRS,
  buildPassability,
  buildRuntimePassability,
  doorPartnerRef,
  getWallByDir,
  isDoorVariant,
  updateDoorEdge,
  type RuntimePassability,
} from './housing-passability'
import {
  passability_add_house,
  passability_remove_house,
  passability_update_door,
  passability_is_movement_blocked,
  passability_attack_line_blocked,
  passability_is_circle_blocked,
} from '../wasm/onlinerpg_shared'
import {
  assistStairMovementDirection,
  checkOverlap,
  collectRoomAABBsInRegion,
  findAdjacentHouse,
  findAllRoomsAtPoint,
  findClosedDoorOnSegment,
  findClosedDoorOnPath,
  findHouseAtPoint,
  findNearestDoor,
  findRoomAtPoint,
  findSupportingHouse,
  hasFloorSupport,
  houseFloorHeightAt,
  isHouseWallBlockingSegment,
  isPointUnderHouseXZ,
  stairLandingTargetAt,
  stopPathAtHouseEntrance,
  type RoomAABB,
  type ClosedHouseDoor,
} from './housing-queries'

// Re-export for external consumers
export { getWallByDir } from './housing-passability'

function chunkKey(cx: number, cz: number): string {
  return `${cx},${cz}`
}

/** Chunks loaded around the player, as a Chebyshev radius (so 1 = a 3x3 block). */
const LOAD_RADIUS = 1
/** Wider than `LOAD_RADIUS` so loitering on a chunk boundary can't thrash a
 *  chunk in and out. Also keeps the house a player stands in safe without a
 *  special case: houses reach ~14 m from their origin against a 64 m chunk, so
 *  the one you are inside is always within a chunk of you. */
const EVICT_RADIUS = 2

export class HousingManager {
  private apiUrl: string
  private chunkCache = new Map<string, HouseData[]>()
  private housesById = new Map<string, HouseData>()
  private inflight = new Map<string, Promise<void>>()

  private housesChangedListeners: ((houses: HouseData[]) => void)[] = []

  /** Subscribe to house data changes. Returns an unsubscribe function. */
  onHousesChanged(cb: (houses: HouseData[]) => void): () => void {
    this.housesChangedListeners.push(cb)
    return () => {
      this.housesChangedListeners = this.housesChangedListeners.filter(
        (l) => l !== cb
      )
    }
  }

  constructor() {
    this.apiUrl = getTerrainApiUrl()
  }

  /** Bring the streamed set in line with the player's position. Owns the
   *  load-then-evict ordering so collision is never momentarily absent. */
  updateStreaming(wx: number, wz: number) {
    this.loadChunksAround(wx, wz)
    this.evictDistantChunks(wx, wz)
  }

  /** Chunks in the `LOAD_RADIUS` block around a world position. */
  private chunksAround(wx: number, wz: number): [number, number][] {
    const { x: ccx, z: ccz } = getTerrainChunkFromPosition(
      { x: wx, y: 0, z: wz },
      TERRAIN_TILE_SIZE
    )
    const chunks: [number, number][] = []
    for (let dx = -LOAD_RADIUS; dx <= LOAD_RADIUS; dx++) {
      for (let dz = -LOAD_RADIUS; dz <= LOAD_RADIUS; dz++) {
        chunks.push([ccx + dx, ccz + dz])
      }
    }
    return chunks
  }

  /** Load houses for chunks around a world position. */
  loadChunksAround(wx: number, wz: number) {
    for (const [cx, cz] of this.chunksAround(wx, wz)) {
      this.ensureChunkLoaded(cx, cz)
    }
  }

  /** Whether every chunk `loadChunksAround` wants at (wx, wz) has arrived. */
  isLoadedAround(wx: number, wz: number) {
    return this.chunksAround(wx, wz).every(([cx, cz]) =>
      this.chunkCache.has(chunkKey(cx, cz))
    )
  }

  stopPathAtHouseEntrance(
    current: { x: number; y: number; z: number },
    currentFloor: number,
    target: { x: number; y: number; z: number },
    waypoints: { x: number; z: number; floor: number }[]
  ): { x: number; z: number; floor: number }[] {
    return stopPathAtHouseEntrance(
      this.housesById,
      current,
      currentFloor,
      target,
      waypoints
    )
  }
  /**
   * Drop chunks beyond `EVICT_RADIUS` of (wx, wz), undoing `loadChunksAround`.
   * Without this the cache grows by one chunk per chunk walked and never
   * shrinks. See `doc/RUNTIME_PERFORMANCE.md` for why the radius is what it is.
   */
  evictDistantChunks(wx: number, wz: number) {
    const { x: ccx, z: ccz } = getTerrainChunkFromPosition(
      { x: wx, y: 0, z: wz },
      TERRAIN_TILE_SIZE
    )
    const centreX = ccx * TERRAIN_TILE_SIZE
    const reach = EVICT_RADIUS * TERRAIN_TILE_SIZE

    let removed = false
    for (const [key, houses] of this.chunkCache) {
      const [cx, cz] = key.split(',').map(Number)
      // X wraps: the world is a cylinder, so a chunk can be adjacent across
      // the seam despite a large index difference.
      const dx = shortestWrappedDeltaX(cx * TERRAIN_TILE_SIZE, centreX)
      if (Math.abs(dx) <= reach && Math.abs(cz - ccz) <= EVICT_RADIUS) continue

      for (const house of houses) {
        this.housesById.delete(house.id)
        passability_remove_house(house.id)
      }
      // Drop the key itself, not just its houses: `ensureChunkLoaded` treats a
      // present key as "already loaded" and would never refetch the chunk.
      this.chunkCache.delete(key)
      removed = true
    }
    if (removed) this.notifyChanged()
  }

  private chunkOf(house: HouseData): string {
    const { x, z } = getTerrainChunkFromPosition(
      house.origin,
      TERRAIN_TILE_SIZE
    )
    return chunkKey(x, z)
  }

  private ensureChunkLoaded(cx: number, cz: number) {
    const key = chunkKey(cx, cz)
    if (this.chunkCache.has(key) || this.inflight.has(key)) return

    this.inflight.set(key, this.fetchChunk(cx, cz, key))
  }

  /** Wait for all currently in-flight chunk fetches to complete. */
  async waitForPending(): Promise<void> {
    if (this.inflight.size === 0) return
    await Promise.all(this.inflight.values())
  }

  private async fetchChunk(cx: number, cz: number, key: string) {
    try {
      const resp = await fetch(`${this.apiUrl}/api/housing/area/${cx}/${cz}`)
      if (!resp.ok) {
        this.chunkCache.set(key, []) // Cache as empty to prevent retry storm
        return
      }
      const houses: HouseData[] = await resp.json()
      // Record the chunk even when empty, so `isLoadedAround` can see it.
      if (!this.chunkCache.has(key)) this.chunkCache.set(key, [])
      for (const h of houses) this.addToCache(h)
      this.notifyChanged()
    } catch {
      this.chunkCache.set(key, []) // Cache as empty to prevent retry storm
    } finally {
      this.inflight.delete(key)
    }
  }

  /** Create a house on the server (ID assigned by server) and add to local cache. */
  async saveHouse(house: HouseData): Promise<HouseData | null> {
    return this.sendHouse('POST', `${this.apiUrl}/api/housing`, house)
  }

  /** Update an existing house on the server (e.g. add room). */
  async updateHouse(house: HouseData): Promise<HouseData | null> {
    return this.sendHouse(
      'PUT',
      `${this.apiUrl}/api/housing/${house.id}`,
      house
    )
  }

  private async sendHouse(
    method: 'POST' | 'PUT',
    url: string,
    house: HouseData
  ): Promise<HouseData | null> {
    try {
      const payload = { ...house, passability: buildPassability(house) }
      const resp = await apiFetch(url, {
        method,
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      })
      if (!resp.ok) return null

      const saved: HouseData = await resp.json()
      this.addToCache(saved)
      this.notifyChanged()
      return saved
    } catch {
      return null
    }
  }

  /** Delete a house from the server and remove from local cache. */
  async deleteHouse(houseId: string): Promise<boolean> {
    try {
      const resp = await apiFetch(`${this.apiUrl}/api/housing/${houseId}`, {
        method: 'DELETE',
      })
      if (!resp.ok) return false

      this.removeFromCache(houseId)
      this.notifyChanged()
      return true
    } catch {
      return false
    }
  }

  /** Handle a batch of houses from WebSocket (HousesInArea, etc.). */
  handleRemoteHousesBatch(houses: HouseData[]) {
    for (const h of houses) this.addToCache(h)
    this.notifyChanged()
  }

  /** Handle a single house spawned/updated by another player. */
  handleRemoteHouseSpawned(house: HouseData) {
    this.addToCache(house)
    this.notifyChanged()
  }

  /** Handle a house removed by another player. */
  handleRemoteHouseRemoved(houseId: string) {
    this.removeFromCache(houseId)
    this.notifyChanged()
  }

  /** Handle a door toggle from the server (authoritative state). */
  handleDoorToggled(
    houseId: string,
    roomIndex: number,
    wallDir: WallDirection,
    segmentIndex: number,
    isOpen: boolean
  ) {
    const house = this.housesById.get(houseId)
    if (!house) return
    const room = house.rooms[roomIndex]
    if (!room) return

    const wall = getWallByDir(room, wallDir)
    if (!wall[segmentIndex]) return

    const refs = [{ roomIndex, segmentIndex }]
    const partner = doorPartnerRef(
      house.rooms,
      roomIndex,
      wallDir,
      segmentIndex
    )
    if (partner) refs.push(partner)
    for (const ref of refs) {
      const r = house.rooms[ref.roomIndex]
      const seg = getWallByDir(r, wallDir)[ref.segmentIndex]
      seg.isOpen = isOpen
      // Windows stay blocking when open
      if (isDoorVariant(seg.variant)) {
        passability_update_door(houseId, r, wallDir, ref.segmentIndex, isOpen)
      }
    }
    this.notifyChanged(false)
  }

  /** Find the nearest door segment within maxDist of (x, z). */
  findNearestDoor(x: number, z: number, y: number, maxDist: number) {
    return findNearestDoor(this.housesById, x, z, y, maxDist)
  }

  findClosedDoorOnSegment(
    fromX: number,
    fromZ: number,
    toX: number,
    toZ: number,
    floorLevel: number
  ): ClosedHouseDoor | null {
    return findClosedDoorOnSegment(
      this.housesById,
      fromX,
      fromZ,
      toX,
      toZ,
      floorLevel
    )
  }

  findClosedDoorOnPath(
    fromX: number,
    fromZ: number,
    waypoints: readonly { x: number; z: number }[],
    floorLevel: number
  ): ClosedHouseDoor | null {
    return findClosedDoorOnPath(
      this.housesById,
      fromX,
      fromZ,
      waypoints,
      floorLevel
    )
  }

  withClosedDoorsOpen<T>(floorLevel: number, fn: () => T): T {
    const closed: {
      houseId: string
      room: HouseData['rooms'][number]
      wallDir: WallDirection
      segmentIndex: number
    }[] = []

    try {
      for (const house of this.housesById.values()) {
        for (const room of house.rooms) {
          if (room.floorLevel !== floorLevel) continue
          for (const wallDir of ALL_WALL_DIRS) {
            const wall = getWallByDir(room, wallDir)
            for (
              let segmentIndex = 0;
              segmentIndex < wall.length;
              segmentIndex++
            ) {
              const segment = wall[segmentIndex]
              if (!isDoorVariant(segment.variant) || segment.isOpen) continue
              closed.push({ houseId: house.id, room, wallDir, segmentIndex })
              passability_update_door(
                house.id,
                room,
                wallDir,
                segmentIndex,
                true
              )
            }
          }
        }
      }
      return fn()
    } finally {
      for (const door of closed) {
        passability_update_door(
          door.houseId,
          door.room,
          door.wallDir,
          door.segmentIndex,
          false
        )
      }
    }
  }

  isDoorOpen(door: ClosedHouseDoor): boolean {
    const room = this.housesById.get(door.houseId)?.rooms[door.roomIndex]
    return (
      !!room && !!getWallByDir(room, door.wallDir)[door.segmentIndex]?.isOpen
    )
  }

  isHouseWallBlockingSegment(
    fromX: number,
    fromZ: number,
    toX: number,
    toZ: number,
    floorLevel: number
  ): boolean {
    return isHouseWallBlockingSegment(
      this.housesById,
      fromX,
      fromZ,
      toX,
      toZ,
      floorLevel
    )
  }

  /** Get all currently loaded houses. */
  getAllHouses(): HouseData[] {
    return Array.from(this.housesById.values())
  }

  /** Get a house by its ID, or undefined if not loaded. */
  getHouseById(id: string): HouseData | undefined {
    return this.housesById.get(id)
  }

  /** Find the house whose room contains a world point, or null. */
  findHouseAtPoint(x: number, y: number, z: number) {
    return findHouseAtPoint(this.housesById, x, y, z)
  }

  /** Find the first room containing a world point (fast, no allocation). */
  findRoomAtPoint(x: number, y: number, z: number) {
    return findRoomAtPoint(this.housesById, x, y, z)
  }

  /** Check if (x, z) falls inside any house room footprint, ignoring Y. */
  isPointUnderHouseXZ(x: number, z: number): boolean {
    return isPointUnderHouseXZ(this.housesById, x, z)
  }

  /** Collect XZ AABBs of all rooms whose footprint intersects the given region. */
  collectRoomAABBsInRegion(
    minX: number,
    maxX: number,
    minZ: number,
    maxZ: number
  ): RoomAABB[] {
    return collectRoomAABBsInRegion(this.housesById, minX, maxX, minZ, maxZ)
  }

  /** Find ALL rooms containing a world point (for overlapping stairwells etc). */
  findAllRoomsAtPoint(x: number, y: number, z: number) {
    return findAllRoomsAtPoint(this.housesById, x, y, z)
  }

  /** Ground Y on a given house floor at (x, z), stairwell ramps included. */
  floorHeightAt(floorLevel: number, x: number, z: number): number | null {
    return houseFloorHeightAt(this.housesById, floorLevel, x, z)
  }

  assistStairMovementDirection(
    floorLevel: number,
    position: { x: number; y: number; z: number },
    direction: { x: number; z: number }
  ) {
    return assistStairMovementDirection(
      this.housesById,
      floorLevel,
      position,
      direction
    )
  }

  stairLandingTargetAt(
    floorLevel: number,
    x: number,
    y: number,
    z: number,
    stairFloor?: number
  ) {
    return stairLandingTargetAt(
      this.housesById,
      floorLevel,
      x,
      y,
      z,
      stairFloor
    )
  }

  /**
   * Check if movement from→to crosses any blocked cell edge. `floorLevel` is
   * the passability floor index (see `dungeonManager.passabilityFloor`), not
   * the wire floor. A player furniture has sealed in gets one step out; walls
   * still refuse.
   */
  isMovementBlocked(
    fromX: number,
    fromZ: number,
    toX: number,
    toZ: number,
    floorLevel: number,
    y: number
  ): boolean {
    return passability_is_movement_blocked(
      fromX,
      fromZ,
      toX,
      toZ,
      floorLevel,
      y
    )
  }

  /**
   * Whether a wall stands between attacker and target — the gate the server
   * applies to every blow. `floorLevel` is the passability floor index, as above.
   */
  attackLineBlocked(
    fromX: number,
    fromZ: number,
    toX: number,
    toZ: number,
    floorLevel: number
  ): boolean {
    return passability_attack_line_blocked(fromX, fromZ, toX, toZ, floorLevel)
  }

  /** Check if a circle of radius r at (x, z) overlaps any blocking wall. */
  isCircleBlocked(
    x: number,
    z: number,
    r: number,
    floorLevel: number,
    y: number
  ): boolean {
    return passability_is_circle_blocked(x, z, r, floorLevel, y)
  }

  /** Update local cache without server call (triggers geometry rebuild). */
  updateLocalCache(house: HouseData) {
    this.addToCache(house)
    this.notifyChanged()
  }

  /** Build passability entries on the fly for debug visualization. */
  getPassabilityEntries(): Map<string, RuntimePassability> {
    const map = new Map<string, RuntimePassability>()
    for (const house of this.housesById.values()) {
      map.set(house.id, buildRuntimePassability(house))
      for (const room of house.rooms) {
        for (const dir of ALL_WALL_DIRS) {
          const segs = getWallByDir(room, dir)
          for (let i = 0; i < segs.length; i++) {
            if (isDoorVariant(segs[i].variant) && segs[i].isOpen) {
              updateDoorEdge(map, house.id, room, dir, i, true)
            }
          }
        }
      }
    }
    return map
  }

  /** Find an existing house that shares an edge with the given room footprint. */
  findAdjacentHouse(
    originX: number,
    originZ: number,
    sizeX: number,
    sizeZ: number
  ) {
    return findAdjacentHouse(this.housesById, originX, originZ, sizeX, sizeZ)
  }

  /** Check if a room footprint overlaps any existing house on the same floor level. */
  checkOverlap(
    originX: number,
    originZ: number,
    sizeX: number,
    sizeZ: number,
    floorLevel: number = 0
  ): boolean {
    return checkOverlap(
      this.housesById,
      originX,
      originZ,
      sizeX,
      sizeZ,
      floorLevel
    )
  }

  /**
   * Check if a room footprint is fully supported by rooms on the floor below.
   */
  hasFloorSupport(
    originX: number,
    originZ: number,
    sizeX: number,
    sizeZ: number,
    opts?: { houseId?: string; floorLevel?: number }
  ): boolean {
    return hasFloorSupport(
      this.housesById,
      originX,
      originZ,
      sizeX,
      sizeZ,
      opts
    )
  }

  /**
   * Find a house that has rooms on the floor below supporting the given footprint.
   */
  findSupportingHouse(
    originX: number,
    originZ: number,
    sizeX: number,
    sizeZ: number,
    floorLevel: number = 1
  ) {
    return findSupportingHouse(
      this.housesById,
      originX,
      originZ,
      sizeX,
      sizeZ,
      floorLevel
    )
  }

  private addToCache(house: HouseData) {
    this.housesById.set(house.id, house)
    const key = this.chunkOf(house)
    const chunk = this.chunkCache.get(key)
    if (chunk) {
      const idx = chunk.findIndex((h) => h.id === house.id)
      if (idx >= 0) {
        chunk[idx] = house
      } else {
        chunk.push(house)
      }
    } else {
      this.chunkCache.set(key, [house])
    }

    // Ensure passability grids exist (compute from room data if missing)
    if (!house.passability?.length) {
      house.passability = buildPassability(house)
    }
    passability_add_house(house)
  }

  private removeFromCache(houseId: string) {
    const house = this.housesById.get(houseId)
    if (!house) return
    this.housesById.delete(houseId)
    passability_remove_house(houseId)
    const chunk = this.chunkCache.get(this.chunkOf(house))
    if (chunk) {
      const idx = chunk.findIndex((h) => h.id === houseId)
      if (idx >= 0) chunk.splice(idx, 1)
    }
  }

  private notifyChanged(updateMap = true) {
    const all = this.getAllHouses()
    if (updateMap) setHouseMapFootprints(all)
    for (const cb of this.housesChangedListeners) cb(all)
  }
}

export const housingManager = new HousingManager()
