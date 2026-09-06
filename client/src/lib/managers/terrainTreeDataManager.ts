import { getTerrainApiUrl } from '../utils/networkUtils'
import {
  decodeTreeData,
  filterTreeData,
  type TreePlacementData,
} from '../utils/tree-data'
import { tileKey } from './terrain-height-types'
import type { TerrainHeightManager } from './terrainHeightManager'
import { clearedCellAt } from '../terrain/landscaping'
import { wrapTileX } from '../terrain/world-wrap'

export class TerrainTreeDataManager {
  private cache = new Map<string, TreePlacementData>()
  private inflight = new Map<string, Promise<TreePlacementData | null>>()
  private missingTiles = new Set<string>()
  private terrainApiUrl: string
  private generation = 0
  private landscapingMasks = new Map<string, Uint8Array>()
  private heightManager: TerrainHeightManager
  private invalidateListeners: (() => void)[] = []
  private tileUpdateListeners: ((tileX: number, tileZ: number) => void)[] = []

  constructor(heightManager: TerrainHeightManager) {
    this.terrainApiUrl = getTerrainApiUrl()
    this.heightManager = heightManager
  }

  async loadTreeData(
    tileX: number,
    tileZ: number
  ): Promise<TreePlacementData | null> {
    const key = tileKey(tileX, tileZ)

    const cached = this.cache.get(key)
    if (cached) return cached

    if (this.missingTiles.has(key)) return null

    const existing = this.inflight.get(key)
    if (existing) return existing

    const gen = this.generation
    const promise = Promise.resolve().then(
      async (): Promise<TreePlacementData | null> => {
        try {
          const url = `${this.terrainApiUrl}/api/terrain/trees/${tileX}/${tileZ}`
          const response = await fetch(url)
          if (gen !== this.generation) return null
          if (this.inflight.get(key) !== promise)
            return this.loadTreeData(tileX, tileZ)
          if (response.status === 404) {
            this.missingTiles.add(key)
            return null
          }
          if (!response.ok) {
            console.error(
              `Failed to load tree data (${tileX}, ${tileZ}): ${response.status}`
            )
            return null
          }
          const buffer = await response.arrayBuffer()
          if (gen !== this.generation) return null
          if (this.inflight.get(key) !== promise)
            return this.loadTreeData(tileX, tileZ)
          let heightmap = this.heightManager.getHeightmap(tileX, tileZ)
          if (!heightmap) {
            heightmap = await this.heightManager.loadHeightmap(tileX, tileZ)
            if (gen !== this.generation) return null
            if (this.inflight.get(key) !== promise)
              return this.loadTreeData(tileX, tileZ)
          }
          const data = this.filterLandscaping(
            tileX,
            tileZ,
            decodeTreeData(buffer, tileX, tileZ, heightmap)
          )
          this.cache.set(key, data)
          return data
        } catch (e) {
          console.error(`Tree data fetch error (${tileX}, ${tileZ}):`, e)
          return null
        } finally {
          if (this.inflight.get(key) === promise) this.inflight.delete(key)
        }
      }
    )
    this.inflight.set(key, promise)
    return promise
  }

  getCachedTreeData(tileX: number, tileZ: number): TreePlacementData | null {
    return this.cache.get(tileKey(tileX, tileZ)) ?? null
  }

  private filterLandscaping(
    tileX: number,
    tileZ: number,
    data: TreePlacementData
  ) {
    const mask = this.landscapingMasks.get(tileKey(wrapTileX(tileX), tileZ))
    return mask
      ? (filterTreeData(data, (x, z) =>
          clearedCellAt(mask, tileX, tileZ, x, z)
        ) ?? data)
      : data
  }

  applyLandscapingMask(tileX: number, tileZ: number, mask: Uint8Array) {
    const key = tileKey(wrapTileX(tileX), tileZ)
    const previous = this.landscapingMasks.get(key)
    if (
      previous?.length === mask.length &&
      mask.every((byte, i) => byte === previous[i])
    )
      return
    this.landscapingMasks.set(key, mask)
    for (const [key, data] of this.cache) {
      const [tx, tz] = key.split(',').map(Number)
      if (wrapTileX(tx) !== wrapTileX(tileX) || tz !== tileZ) continue
      const filtered = this.filterLandscaping(tx, tz, data)
      if (filtered === data) continue
      this.cache.set(key, filtered)
      for (const cb of this.tileUpdateListeners) cb(tx, tz)
    }
  }

  invalidateLandscaping(tileX: number, tileZ: number): void {
    this.landscapingMasks.delete(tileKey(wrapTileX(tileX), tileZ))
    for (const key of [
      ...this.cache.keys(),
      ...this.missingTiles,
      ...this.inflight.keys(),
    ]) {
      const [tx, tz] = key.split(',').map(Number)
      if (wrapTileX(tx) === wrapTileX(tileX) && tz === tileZ) {
        this.inflight.delete(key)
        this.invalidate(tx, tz)
      }
    }
  }

  invalidate(tileX: number, tileZ: number): void {
    const key = tileKey(tileX, tileZ)
    this.cache.delete(key)
    this.missingTiles.delete(key)
  }

  async refreshTiles(
    tiles: readonly (readonly [number, number])[]
  ): Promise<void> {
    const unique = new Map<string, readonly [number, number]>()
    for (const tile of tiles) {
      unique.set(tileKey(tile[0], tile[1]), tile)
    }

    await Promise.all(
      Array.from(unique.values(), ([tileX, tileZ]) => {
        this.invalidate(tileX, tileZ)
        return this.loadTreeData(tileX, tileZ).finally(() => {
          for (const cb of this.tileUpdateListeners) cb(tileX, tileZ)
        })
      })
    )
  }

  /** Subscribe to per-tile data updates. Returns unsubscribe function. */
  onTileUpdated(cb: (tileX: number, tileZ: number) => void): () => void {
    this.tileUpdateListeners.push(cb)
    return () => {
      this.tileUpdateListeners = this.tileUpdateListeners.filter(
        (l) => l !== cb
      )
    }
  }

  onInvalidateAll(cb: () => void): () => void {
    this.invalidateListeners.push(cb)
    return () => {
      this.invalidateListeners = this.invalidateListeners.filter(
        (l) => l !== cb
      )
    }
  }

  invalidateAll(): void {
    this.generation++
    this.cache.clear()
    this.missingTiles.clear()
    this.inflight.clear()
    this.landscapingMasks.clear()
    for (const cb of this.invalidateListeners) cb()
  }

  evictExcept(keepKeys: Set<string>): void {
    for (const key of this.cache.keys()) {
      if (!keepKeys.has(key)) {
        this.cache.delete(key)
      }
    }
  }
}
