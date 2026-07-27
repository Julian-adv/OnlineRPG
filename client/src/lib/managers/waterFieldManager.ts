import { getTerrainApiUrl } from '../utils/networkUtils'
import {
  decodeWaterFieldData,
  WATER_FIELD_GRID,
  type WaterFieldTileData,
} from '../utils/water-field-data'
import { tileKey, worldToTileCoord } from './terrain-height-types'
import {
  SEA_LEVEL,
  TERRAIN_TILE_SIZE,
} from '../components/game-scene/terrain-utils'

/** Per-tile WFD1 fetcher + decoder. Mirrors the pattern of the other
 *  per-tile binary loaders (grass / trees / splat) — fetch once, cache,
 *  return decoded channels. A 404 means "no river influence in this
 *  tile"; the water layer synthesizes a flat sea field for those.
 *  Texture construction lives in `water-quad-geometry.ts` next to the
 *  geometry helper. */
export class WaterFieldManager {
  private cache = new Map<string, WaterFieldTileData | null>()
  private inflight = new Map<string, Promise<WaterFieldTileData | null>>()
  private terrainApiUrl = getTerrainApiUrl()

  async loadWaterField(
    tileX: number,
    tileZ: number
  ): Promise<WaterFieldTileData | null> {
    const key = tileKey(tileX, tileZ)

    if (this.cache.has(key)) return this.cache.get(key) ?? null
    const existing = this.inflight.get(key)
    if (existing) return existing

    const promise = (async () => {
      try {
        const url = `${this.terrainApiUrl}/api/terrain/water-field/${tileX}/${tileZ}`
        const response = await fetch(url)
        if (response.status === 404) {
          this.cache.set(key, null)
          return null
        }
        if (!response.ok) {
          console.error(
            `Failed to load water field (${tileX}, ${tileZ}): ${response.status}`
          )
          return null
        }
        const buffer = await response.arrayBuffer()
        const data = decodeWaterFieldData(buffer)
        this.cache.set(key, data)
        return data
      } catch (e) {
        console.error(`Water field fetch error (${tileX}, ${tileZ}):`, e)
        return null
      } finally {
        this.inflight.delete(key)
      }
    })()
    this.inflight.set(key, promise)
    return promise
  }

  /** Baked water surface height at a world XZ (bilinear, cached tiles only;
   *  sea level when absent). Synchronous — never fetches on the click path.
   *  The server re-validates every cast. */
  surfaceAt(worldX: number, worldZ: number): number {
    const tileX = worldToTileCoord(worldX)
    const tileZ = worldToTileCoord(worldZ)
    const data = this.cache.get(tileKey(tileX, tileZ))
    if (!data) return SEA_LEVEL

    const tileMinX = tileX * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
    const tileMinZ = tileZ * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
    const localX = worldX - tileMinX
    const localZ = worldZ - tileMinZ

    const max = WATER_FIELD_GRID - 1
    // Edge-clamp instead of crossing into the neighbor tile: the server is
    // authoritative, so a one-cell approximation at a seam is harmless.
    const cx = Math.min(Math.max(Math.floor(localX), 0), max - 1)
    const cz = Math.min(Math.max(Math.floor(localZ), 0), max - 1)
    const fx = Math.min(Math.max(localX - cx, 0), 1)
    const fz = Math.min(Math.max(localZ - cz, 0), 1)

    const s = data.surfaceY
    const s00 = s[cz * WATER_FIELD_GRID + cx]
    const s10 = s[cz * WATER_FIELD_GRID + cx + 1]
    const s01 = s[(cz + 1) * WATER_FIELD_GRID + cx]
    const s11 = s[(cz + 1) * WATER_FIELD_GRID + cx + 1]
    const s0 = s00 + (s10 - s00) * fx
    const s1 = s01 + (s11 - s01) * fx
    return s0 + (s1 - s0) * fz
  }
}
