import { MathUtils } from 'three'
import { apiFetch, getTerrainApiUrl } from '../utils/networkUtils'
import type {
  ObjectDef,
  ObjectPlacement,
  ObjectRegionData,
} from '../stores/editorStore'
import type { Position } from '../network/networkTypes'
import { TERRAIN_TILE_SIZE } from '../components/game-scene/terrain-utils'
import { regionKey, tileToRegion } from '../terrain/terrain-constants'
import { loadGLB } from '../utils/gltfCache'
import { getObjectModelPath } from '../utils/modelPaths'
import { detectFootprint, type FootprintData } from '../utils/objectFootprint'

export class ObjectManager {
  private cache = new Map<string, ObjectRegionData>()
  private terrainApiUrl: string
  private catalogCache: ObjectDef[] | null = null
  private footprintCache = new Map<string, FootprintData>()

  constructor() {
    this.terrainApiUrl = getTerrainApiUrl()
  }

  async fetchCatalog(): Promise<ObjectDef[]> {
    if (this.catalogCache) return this.catalogCache
    const resp = await fetch('/models/objects/catalog.json')
    const data: ObjectDef[] = await resp.json()
    this.catalogCache = data
    return data
  }

  async fetchFootprint(objectType: string): Promise<FootprintData | null> {
    const cached = this.footprintCache.get(objectType)
    if (cached) return cached
    await this.fetchCatalog()
    const def = this.getCatalogEntry(objectType)
    if (!def || !def.model) return null
    const gltf = await loadGLB(getObjectModelPath(def.model))
    const data = detectFootprint(gltf.scene)
    this.footprintCache.set(objectType, data)
    return data
  }

  async fetchObject(rx: number, rz: number): Promise<ObjectRegionData> {
    const key = regionKey(rx, rz)
    const cached = this.cache.get(key)
    if (cached) return cached

    try {
      const resp = await fetch(
        `${this.terrainApiUrl}/api/terrain/objects/${rx}/${rz}`
      )
      const json = await resp.json()
      const data: ObjectRegionData = {
        placements: json.placements ?? [],
      }
      this.cache.set(key, data)
      return data
    } catch {
      const data: ObjectRegionData = { placements: [] }
      this.cache.set(key, data)
      return data
    }
  }

  async saveObject(
    rx: number,
    rz: number,
    data: ObjectRegionData
  ): Promise<void> {
    await apiFetch(`${this.terrainApiUrl}/api/terrain/objects/${rx}/${rz}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(data),
    })
    this.cache.set(regionKey(rx, rz), data)
  }

  getCached(rx: number, rz: number): ObjectRegionData | null {
    return this.cache.get(regionKey(rx, rz)) ?? null
  }

  invalidate(rx: number, rz: number): void {
    this.cache.delete(regionKey(rx, rz))
  }

  /** Look up a object definition by type id (e.g. "bed"). Returns null if catalog not loaded or not found. */
  getCatalogEntry(objectType: string): ObjectDef | null {
    if (!this.catalogCache) return null
    return this.catalogCache.find((d) => d.id === objectType) ?? null
  }

  /** The placement a player occupies: the one with `objectId` when the
   *  server named it, else the nearest of that type — two chairs at one
   *  table are closer to each other than a sitter's stored position is to
   *  either, so distance alone seats neighbours on the same chair. */
  findNearestPlacement(
    objectType: string,
    wx: number,
    wz: number,
    objectId?: number | null
  ): ObjectPlacement | null {
    let best: ObjectPlacement | null = null
    let bestDist = Infinity
    for (const region of this.cache.values()) {
      for (const p of region.placements) {
        if (p.type !== objectType) continue
        if (objectId != null && p.id === objectId) return p
        const dx = p.x - wx
        const dz = p.z - wz
        const dist = dx * dx + dz * dz
        if (dist < bestDist) {
          bestDist = dist
          best = p
        }
      }
    }
    return best
  }

  /** Like findNearestPlacement but fetches the region first if not cached. */
  /** Resolve the pose for a player interacting with `objectType` near
   *  (wx, wz): the clip, the seat offset, and the placement's position and
   *  yaw. Placements store degrees while a player's rotation is radians
   *  everywhere else (a bed at 270° once laid its sleeper out crosswise). */
  async resolvePose(
    objectType: string,
    wx: number,
    wz: number,
    objectId?: number | null
  ): Promise<{
    anim: string
    interactOffset?: Position
    placement: ObjectPlacement | null
    rotation?: number
  }> {
    const [, placement] = await Promise.all([
      this.fetchCatalog(),
      this.findNearestPlacementAsync(objectType, wx, wz, objectId),
    ])
    const def = this.getCatalogEntry(objectType)
    return {
      anim: def?.interaction ?? objectType,
      interactOffset: def?.interactOffset,
      placement,
      rotation: placement ? MathUtils.degToRad(placement.rotation) : undefined,
    }
  }

  async findNearestPlacementAsync(
    objectType: string,
    wx: number,
    wz: number,
    objectId?: number | null
  ): Promise<ObjectPlacement | null> {
    // Ensure the region containing this position is loaded
    const tileX = Math.floor(wx / TERRAIN_TILE_SIZE)
    const tileZ = Math.floor(wz / TERRAIN_TILE_SIZE)
    const rx = tileToRegion(tileX)
    const rz = tileToRegion(tileZ)
    await this.fetchObject(rx, rz)
    return this.findNearestPlacement(objectType, wx, wz, objectId)
  }
}

/** Shared singleton instance */
export const objectManager = new ObjectManager()
