import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { WaterFieldManager } from './waterFieldManager'
import {
  WATER_FIELD_GRID,
  type WaterFieldTileData,
} from '../utils/water-field-data'
import { SEA_LEVEL } from '../components/game-scene/terrain-utils'

vi.mock('../utils/networkUtils', () => ({
  getTerrainApiUrl: () => 'http://test',
}))

const decodeWaterFieldData = vi.hoisted(() => vi.fn())
vi.mock('../utils/water-field-data', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../utils/water-field-data')>()),
  decodeWaterFieldData,
}))

function tileWithSurface(fill: (cx: number, cz: number) => number) {
  const count = WATER_FIELD_GRID * WATER_FIELD_GRID
  const surfaceY = new Float32Array(count)
  for (let cz = 0; cz < WATER_FIELD_GRID; cz++) {
    for (let cx = 0; cx < WATER_FIELD_GRID; cx++) {
      surfaceY[cz * WATER_FIELD_GRID + cx] = fill(cx, cz)
    }
  }
  const zeros = () => new Float32Array(count)
  return {
    surfaceY,
    flowX: zeros(),
    flowZ: zeros(),
    riverness: zeros(),
    turbulence: zeros(),
  } satisfies WaterFieldTileData
}

function fetchResponding(status: number) {
  return vi.fn(async () => ({
    ok: status >= 200 && status < 300,
    status,
    arrayBuffer: async () => new ArrayBuffer(0),
  })) as unknown as typeof fetch
}

describe('WaterFieldManager', () => {
  let manager: WaterFieldManager

  beforeEach(() => {
    manager = new WaterFieldManager()
    decodeWaterFieldData.mockReset()
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  describe('surfaceAt', () => {
    it('returns sea level when the tile is not cached — never fetches on the click path', () => {
      const fetchSpy = fetchResponding(200)
      vi.stubGlobal('fetch', fetchSpy)

      expect(manager.surfaceAt(0, 0)).toBe(SEA_LEVEL)
      expect(fetchSpy).not.toHaveBeenCalled()
    })

    it('returns the baked height inside a loaded tile', async () => {
      vi.stubGlobal('fetch', fetchResponding(200))
      decodeWaterFieldData.mockReturnValue(tileWithSurface(() => 5))
      await manager.loadWaterField(0, 0)

      expect(manager.surfaceAt(0, 0)).toBeCloseTo(5)
      expect(manager.surfaceAt(-31.9, 31.9)).toBeCloseTo(5)
    })

    it('bilinearly interpolates between grid cells', async () => {
      vi.stubGlobal('fetch', fetchResponding(200))
      // 1m per cell; height = local x in meters makes interpolation visible.
      decodeWaterFieldData.mockReturnValue(tileWithSurface((cx) => cx))
      await manager.loadWaterField(0, 0)

      // Tile 0 spans [-32, 32): world -32 is local 0, world -31.5 is local 0.5.
      expect(manager.surfaceAt(-32, 0)).toBeCloseTo(0)
      expect(manager.surfaceAt(-31.5, 0)).toBeCloseTo(0.5)
      expect(manager.surfaceAt(0, 0)).toBeCloseTo(32)
    })

    it('returns sea level for a tile the server marked riverless (404)', async () => {
      vi.stubGlobal('fetch', fetchResponding(404))
      await manager.loadWaterField(0, 0)

      expect(manager.surfaceAt(0, 0)).toBe(SEA_LEVEL)
    })
  })

  describe('loadWaterField', () => {
    it('fetches a tile once and serves repeats from cache', async () => {
      const fetchSpy = fetchResponding(200)
      vi.stubGlobal('fetch', fetchSpy)
      decodeWaterFieldData.mockReturnValue(tileWithSurface(() => 1))

      await manager.loadWaterField(0, 0)
      await manager.loadWaterField(0, 0)

      expect(fetchSpy).toHaveBeenCalledTimes(1)
    })

    it('caches a 404 — a riverless tile is an answer, not an error', async () => {
      const fetchSpy = fetchResponding(404)
      vi.stubGlobal('fetch', fetchSpy)

      expect(await manager.loadWaterField(0, 0)).toBeNull()
      expect(await manager.loadWaterField(0, 0)).toBeNull()
      expect(fetchSpy).toHaveBeenCalledTimes(1)
    })

    it('dedupes concurrent requests for the same tile', async () => {
      const fetchSpy = fetchResponding(200)
      vi.stubGlobal('fetch', fetchSpy)
      decodeWaterFieldData.mockReturnValue(tileWithSurface(() => 1))

      await Promise.all([
        manager.loadWaterField(0, 0),
        manager.loadWaterField(0, 0),
      ])

      expect(fetchSpy).toHaveBeenCalledTimes(1)
    })

    it('does not cache a server error, so the next call retries', async () => {
      const fetchSpy = fetchResponding(500)
      vi.stubGlobal('fetch', fetchSpy)

      expect(await manager.loadWaterField(0, 0)).toBeNull()
      expect(await manager.loadWaterField(0, 0)).toBeNull()
      expect(fetchSpy).toHaveBeenCalledTimes(2)
    })
  })
})
