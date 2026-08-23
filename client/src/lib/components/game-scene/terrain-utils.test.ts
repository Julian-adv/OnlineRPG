import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import {
  TERRAIN_TILE_SIZE,
  createTerrainTiles,
  heightPrefetchTiles,
} from './terrain-utils'
import { worldToTileCoord } from '../../managers/terrain-height-types'

// The ring only has to be this wide because the server hands a client every
// monster within EVENT_DELIVERY_RADIUS. Parse the Rust source so raising the
// radius fails here instead of silently reviving the held-move-report bug.
function rustAoi(): number {
  const source = readFileSync(
    path.resolve(__dirname, '../../../../../shared/src/world.rs'),
    'utf-8'
  )
  const match = source.match(
    /pub const EVENT_DELIVERY_RADIUS\s*:\s*f32\s*=\s*([0-9.]+)/
  )
  if (!match) throw new Error('EVENT_DELIVERY_RADIUS not found in world.rs')
  return Number(match[1])
}

/** Loaded ground between the player and the nearest edge of a tile set. */
function margin(tileXs: number[], playerX: number): number {
  const lo = Math.min(...tileXs) * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
  const hi = Math.max(...tileXs) * TERRAIN_TILE_SIZE + TERRAIN_TILE_SIZE / 2
  return Math.min(playerX - lo, hi - playerX)
}

describe('heightPrefetchTiles', () => {
  // A full chunk's worth of player positions, so the worst case is covered.
  const samples = Array.from({ length: 65 }, (_, i) => 1280 + i)
  const chunkOf = (x: number) => Math.floor(x / TERRAIN_TILE_SIZE)

  const worstMargin = (tilesFor: (chunk: number) => number[]) =>
    Math.min(...samples.map((px) => margin(tilesFor(chunkOf(px)), px)))

  it('covers the AOI the render grid leaves short', () => {
    const render = worstMargin((c) =>
      createTerrainTiles(c, 0).map((t) => t.position[0] / TERRAIN_TILE_SIZE)
    )
    const prefetch = worstMargin((c) =>
      heightPrefetchTiles(c, 0).map((t) => t.x)
    )

    expect(render).toBe(32)
    expect(prefetch).toBe(96)
    // The load-bearing claim: the ring outreaches the AOI with room for the
    // fetch latency of a chunk crossing.
    expect(render).toBeLessThanOrEqual(rustAoi())
    expect(prefetch).toBeGreaterThan(rustAoi())
  })

  it('puts the render tiles first so the loading screen never waits on the ring', () => {
    const first4 = heightPrefetchTiles(20, 69)
      .slice(0, 4)
      .map((t) => `${t.x}_${t.z}`)
    expect(new Set(first4)).toEqual(
      new Set(createTerrainTiles(20, 69).map((t) => t.id))
    )
  })

  it('holds every tile a monster inside the AOI could stand on', () => {
    const aoi = rustAoi()
    const offsets: [number, number][] = [
      [aoi, 0],
      [-aoi, 0],
      [0, aoi],
      [0, -aoi],
      [aoi, aoi],
      [-aoi, -aoi],
      [aoi, -aoi],
      [-aoi, aoi],
    ]
    for (const p of samples) {
      const loaded = new Set(
        heightPrefetchTiles(chunkOf(p), chunkOf(p)).map((t) => `${t.x}_${t.z}`)
      )
      for (const [dx, dz] of offsets) {
        expect(
          loaded.has(`${worldToTileCoord(p + dx)}_${worldToTileCoord(p + dz)}`)
        ).toBe(true)
      }
    }
  })
})
