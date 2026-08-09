import { apiFetch, getTerrainApiUrl } from '../utils/networkUtils'
import {
  decodeGrassData,
  encodeGrassBuffer,
  removeGrassInRect,
  type GrassPlacementData,
} from '../utils/grass-data'
import { worldRectToTileBounds } from '../components/game-scene/terrain-utils'
import { tileKey } from './terrain-height-types'
import type { TerrainHeightManager } from './terrainHeightManager'

export interface GrassCarveRect {
  minX: number
  minZ: number
  maxX: number
  maxZ: number
}

export class TerrainGrassDataManager {
  private cache = new Map<string, GrassPlacementData>()
  private originalGrass = new Map<string, GrassPlacementData>()
  private inflight = new Map<string, Promise<GrassPlacementData | null>>()
  private ensureInflight = new Map<string, Promise<boolean>>()
  /** Tiles known to have no server data (404). Prevents repeated fetches. */
  private missingTiles = new Set<string>()
  private terrainApiUrl: string
  private generation = 0
  private tileUpdateListeners: ((tileX: number, tileZ: number) => void)[] = []
  private heightManager: TerrainHeightManager
  private _suppressListeners = false

  constructor(heightManager: TerrainHeightManager) {
    this.terrainApiUrl = getTerrainApiUrl()
    this.heightManager = heightManager
  }

  /** Suppress tile-update listeners during bulk operations.
   *  Call with `true` before batch saves, `false` when done. */
  set suppressListeners(v: boolean) {
    this._suppressListeners = v
  }

  /** Subscribe to tile data updates. Returns unsubscribe function. */
  onTileUpdated(cb: (tileX: number, tileZ: number) => void): () => void {
    this.tileUpdateListeners.push(cb)
    return () => {
      this.tileUpdateListeners = this.tileUpdateListeners.filter(
        (l) => l !== cb
      )
    }
  }

  /**
   * Load pre-computed grass data for a tile.
   * Returns null if no data exists on the server.
   */
  async loadGrassData(
    tileX: number,
    tileZ: number
  ): Promise<GrassPlacementData | null> {
    const key = tileKey(tileX, tileZ)

    const cached = this.cache.get(key)
    if (cached) return cached

    if (this.missingTiles.has(key)) return null

    const existing = this.inflight.get(key)
    if (existing) return existing

    const gen = this.generation
    const promise = (async () => {
      try {
        const url = `${this.terrainApiUrl}/api/terrain/grass/${tileX}/${tileZ}`
        const response = await fetch(url)
        if (gen !== this.generation) return null
        if (response.status === 404) {
          this.missingTiles.add(key)
          return null
        }
        if (!response.ok) {
          console.error(
            `Failed to load grass data (${tileX}, ${tileZ}): ${response.status}`
          )
          return null
        }
        const buffer = await response.arrayBuffer()
        if (gen !== this.generation) return null
        let heightmap = this.heightManager.getHeightmap(tileX, tileZ)
        if (!heightmap) {
          heightmap = await this.heightManager.loadHeightmap(tileX, tileZ)
          if (gen !== this.generation) return null
        }
        const data = decodeGrassData(buffer, tileX, tileZ, heightmap)
        this.cache.set(key, data)
        return data
      } catch (e) {
        console.error(`Grass data fetch error (${tileX}, ${tileZ}):`, e)
        return null
      } finally {
        this.inflight.delete(key)
      }
    })()
    this.inflight.set(key, promise)
    return promise
  }

  /** Ensure an original grass snapshot exists for the given tile.
   *  Tells the server to copy current grass as original if none exists yet,
   *  and caches a local copy. Must complete before saving carved grass —
   *  if the carved PUT lands first, the server snapshots the carved tile as
   *  "original" and the pristine state is lost. Resolves false if the
   *  snapshot could not be guaranteed. */
  ensureOriginalGrass(tileX: number, tileZ: number): Promise<boolean> {
    const key = tileKey(tileX, tileZ)
    if (this.originalGrass.has(key)) return Promise.resolve(true)
    const pending = this.ensureInflight.get(key)
    if (pending) return pending
    const current = this.cache.get(key)
    if (!current) return Promise.resolve(false)
    const snapshot: GrassPlacementData = {
      shortCount: current.shortCount,
      tallCount: current.tallCount,
      flowerCount: current.flowerCount,
      buffer: current.buffer.slice(0),
    }
    const promise = (async () => {
      const response = await apiFetch(
        `${this.terrainApiUrl}/api/terrain/grass-original/${tileX}/${tileZ}/ensure`,
        { method: 'POST' }
      ).catch(() => null)
      this.ensureInflight.delete(key)
      if (!response?.ok) {
        console.error(
          `Grass original ensure failed (${tileX}, ${tileZ}): ${response?.status ?? 'network error'}`
        )
        return false
      }
      this.originalGrass.set(key, snapshot)
      return true
    })()
    this.ensureInflight.set(key, promise)
    return promise
  }

  /** Remove grass inside the given world rects across every affected tile.
   *  Owns the carve invariant: the original snapshot is ensured before a
   *  carved tile is saved, and the save is skipped if that guarantee fails. */
  async removeGrassInRects(rects: GrassCarveRect[]): Promise<void> {
    if (rects.length === 0) return

    const tileBuckets = new Map<
      string,
      { tx: number; tz: number; rects: GrassCarveRect[] }
    >()
    for (const rect of rects) {
      const { tileMinX, tileMaxX, tileMinZ, tileMaxZ } = worldRectToTileBounds(
        rect.minX,
        rect.minZ,
        rect.maxX,
        rect.maxZ
      )
      for (let tz = tileMinZ; tz <= tileMaxZ; tz++) {
        for (let tx = tileMinX; tx <= tileMaxX; tx++) {
          const key = tileKey(tx, tz)
          let bucket = tileBuckets.get(key)
          if (!bucket) {
            bucket = { tx, tz, rects: [] }
            tileBuckets.set(key, bucket)
          }
          bucket.rects.push(rect)
        }
      }
    }

    await Promise.all(
      [...tileBuckets.values()].map(async ({ tx, tz, rects }) => {
        let data =
          this.getCachedGrassData(tx, tz) ?? (await this.loadGrassData(tx, tz))
        if (!data) return
        let changed = false
        for (const rect of rects) {
          const filtered = removeGrassInRect(
            data,
            rect.minX,
            rect.minZ,
            rect.maxX,
            rect.maxZ
          )
          if (filtered) {
            data = filtered
            changed = true
          }
        }
        if (!changed) return
        // Snapshot from the still-uncarved cache before persisting the carve.
        if (!(await this.ensureOriginalGrass(tx, tz))) {
          console.error(`Skipping grass carve save (${tx}, ${tz}): no original`)
          return
        }
        await this.saveGrassData(tx, tz, data)
      })
    )
  }

  /** Load original (pre-housing) grass data from server. Returns null if none exists. */
  async loadOriginalGrass(
    tileX: number,
    tileZ: number
  ): Promise<GrassPlacementData | null> {
    const key = tileKey(tileX, tileZ)
    if (this.originalGrass.has(key)) return this.originalGrass.get(key)!
    try {
      const url = `${this.terrainApiUrl}/api/terrain/grass-original/${tileX}/${tileZ}`
      const response = await fetch(url)
      if (response.status === 404 || !response.ok) return null
      const buffer = await response.arrayBuffer()
      const heightmap =
        this.heightManager.getHeightmap(tileX, tileZ) ??
        (await this.heightManager.loadHeightmap(tileX, tileZ))
      const data = decodeGrassData(buffer, tileX, tileZ, heightmap)
      this.originalGrass.set(key, data)
      return data
    } catch {
      return null
    }
  }

  /** Restore grass data for a tile from the original snapshot.
   *  Returns true if restored, false if no original exists. */
  async restoreFromOriginal(tileX: number, tileZ: number): Promise<boolean> {
    const key = tileKey(tileX, tileZ)
    const original =
      this.originalGrass.get(key) ??
      // Try loading from server (e.g. after page refresh)
      (await this.loadOriginalGrass(tileX, tileZ))
    if (!original) return false
    // Deep copy so original stays pristine
    const restored: GrassPlacementData = {
      shortCount: original.shortCount,
      tallCount: original.tallCount,
      flowerCount: original.flowerCount,
      buffer: original.buffer.slice(0),
    }
    await this.saveGrassData(tileX, tileZ, restored)
    return true
  }

  /** Save pre-computed grass data to the server. */
  async saveGrassData(
    tileX: number,
    tileZ: number,
    data: GrassPlacementData
  ): Promise<void> {
    const key = tileKey(tileX, tileZ)
    this.cache.set(key, data)
    this.missingTiles.delete(key)

    if (!this._suppressListeners) {
      for (const cb of this.tileUpdateListeners) cb(tileX, tileZ)
    }

    try {
      const url = `${this.terrainApiUrl}/api/terrain/grass/${tileX}/${tileZ}`
      const wireBuffer = encodeGrassBuffer(data, tileX, tileZ)
      const response = await apiFetch(url, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/octet-stream' },
        body: wireBuffer,
      })
      if (!response.ok) {
        console.error(
          `Failed to save grass data (${tileX}, ${tileZ}): ${response.status}`
        )
      }
    } catch (e) {
      console.error(`Grass data save error (${tileX}, ${tileZ}):`, e)
    }
  }

  /** Get cached grass data (synchronous). */
  getCachedGrassData(tileX: number, tileZ: number): GrassPlacementData | null {
    return this.cache.get(tileKey(tileX, tileZ)) ?? null
  }

  /** Invalidate cache for a tile. */
  invalidate(tileX: number, tileZ: number): void {
    const key = tileKey(tileX, tileZ)
    this.cache.delete(key)
    this.missingTiles.delete(key)
  }

  /** Clear all caches so every tile is re-fetched from the server. */
  invalidateAll(): void {
    this.generation++
    this.cache.clear()
    this.originalGrass.clear()
    this.missingTiles.clear()
    this.inflight.clear()
  }

  /** Evict cached data for tiles not in the given set. */
  evictExcept(keepKeys: Set<string>): void {
    for (const key of this.cache.keys()) {
      if (!keepKeys.has(key)) {
        this.cache.delete(key)
        this.originalGrass.delete(key)
      }
    }
  }
}
