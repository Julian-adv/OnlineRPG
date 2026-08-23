import {
  TERRAIN_TILE_SIZE,
  type TerrainTile,
  type TerrainChunk,
  type Vector3Like,
  createTerrainTiles,
  getTerrainChunkFromPosition,
  heightPrefetchTiles,
} from './terrain-utils'

export interface TerrainTileManagerCallbacks {
  getTiles(): TerrainTile[]
  setTiles(tiles: TerrainTile[]): void
  getCenterChunk(): TerrainChunk
  setCenterChunk(chunk: TerrainChunk): void
}

/** The slice of TerrainHeightManager the tile manager warms tiles through. */
export interface HeightWarmer {
  warmHeightmap(tileX: number, tileZ: number): Promise<void>
}

export interface TerrainTileManager {
  rebuild(centerChunkX: number, centerChunkZ: number): void
  drainQueue(): void
  drainAll(): void
  hasPending(): boolean
  updateFromPlayerPosition(position: Vector3Like | null): void
  resetForForceRebuild(): void
}

export function createTerrainTileManager(
  cb: TerrainTileManagerCallbacks,
  heightWarmer: HeightWarmer
): TerrainTileManager {
  let pendingTileQueue: TerrainTile[] = []

  function rebuild(centerChunkX: number, centerChunkZ: number) {
    const allTiles = createTerrainTiles(
      centerChunkX,
      centerChunkZ,
      TERRAIN_TILE_SIZE
    )

    const newTileIds = new Set(allTiles.map((t) => t.id))
    const keptTiles = cb.getTiles().filter((t) => newTileIds.has(t.id))
    const keptIds = new Set(keptTiles.map((t) => t.id))

    cb.setTiles(keptTiles)

    pendingTileQueue = allTiles.filter((t) => !keptIds.has(t.id))

    for (const tile of heightPrefetchTiles(centerChunkX, centerChunkZ)) {
      heightWarmer.warmHeightmap(tile.x, tile.z).catch((e) => {
        console.debug(`heightmap warm failed (${tile.x}, ${tile.z}):`, e)
      })
    }
  }

  function drainQueue() {
    if (pendingTileQueue.length === 0) return
    const tile = pendingTileQueue.shift()!
    cb.setTiles([...cb.getTiles(), tile])
  }

  function drainAll() {
    if (pendingTileQueue.length === 0) return
    cb.setTiles([...cb.getTiles(), ...pendingTileQueue])
    pendingTileQueue = []
  }

  function updateFromPlayerPosition(position: Vector3Like | null) {
    if (!position) return
    const center = cb.getCenterChunk()
    const nextChunk = getTerrainChunkFromPosition(position, TERRAIN_TILE_SIZE)
    if (nextChunk.x === center.x && nextChunk.z === center.z) return
    cb.setCenterChunk(nextChunk)
    rebuild(nextChunk.x, nextChunk.z)
  }

  function resetForForceRebuild() {
    cb.setTiles([])
    pendingTileQueue = []
    cb.setCenterChunk({ x: NaN, z: NaN })
  }

  return {
    rebuild,
    drainQueue,
    drainAll,
    hasPending: () => pendingTileQueue.length > 0,
    updateFromPlayerPosition,
    resetForForceRebuild,
  }
}
