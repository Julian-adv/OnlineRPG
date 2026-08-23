import * as THREE from 'three'

export interface TerrainTile {
  id: string
  position: [number, number, number]
}

export interface TerrainChunk {
  x: number
  z: number
}

export interface Vector3Like {
  x: number
  y: number
  z: number
}

export const SEA_LEVEL = 0.0
export const SEA_LEVEL_ENCODED = 10000

export const TERRAIN_TILE_SIZE = 64
export const TERRAIN_TILE_SEGMENTS = 64

export function worldToTileCell(wx: number, wz: number) {
  const S = TERRAIN_TILE_SIZE
  const tileX = Math.round(wx / S)
  const tileZ = Math.round(wz / S)
  const cellX = Math.max(0, Math.min(S - 1, Math.floor(wx - tileX * S + S / 2)))
  const cellZ = Math.max(0, Math.min(S - 1, Math.floor(wz - tileZ * S + S / 2)))
  return { tileX, tileZ, cellX, cellZ }
}

export function worldRectToTileBounds(
  minX: number,
  minZ: number,
  maxX: number,
  maxZ: number
) {
  const S = TERRAIN_TILE_SIZE
  return {
    tileMinX: Math.floor((minX + S / 2) / S),
    tileMaxX: Math.floor((maxX + S / 2) / S),
    tileMinZ: Math.floor((minZ + S / 2) / S),
    tileMaxZ: Math.floor((maxZ + S / 2) / S),
  }
}

/**
 * Create a 2×2 tile grid based on the player's floor-rounded chunk position.
 * The 4 tiles always cover the player: (fx,fz), (fx+1,fz), (fx,fz+1), (fx+1,fz+1).
 * This is sufficient for an orthographic camera viewport with 64-unit tiles.
 */
export function createTerrainTiles(
  floorChunkX: number,
  floorChunkZ: number,
  tileSize = TERRAIN_TILE_SIZE
): TerrainTile[] {
  const tiles: TerrainTile[] = []
  for (let dz = 0; dz <= 1; dz++) {
    for (let dx = 0; dx <= 1; dx++) {
      const cx = floorChunkX + dx
      const cz = floorChunkZ + dz
      tiles.push({
        id: `${cx}_${cz}`,
        position: [cx * tileSize, 0, cz * tileSize],
      })
    }
  }
  return tiles
}

/**
 * Tiles whose heightmaps must be resident, widest-first-needed order. The 2x2
 * render grid guarantees only 32m of loaded ground around the player, so a
 * monster its client owns can stand on an unstreamed tile and have its move
 * reports held; this ring guarantees 96m. The render tiles come first so the
 * loading screen never waits behind a ring fetch.
 */
export function heightPrefetchTiles(
  floorChunkX: number,
  floorChunkZ: number
): TerrainChunk[] {
  const tiles: TerrainChunk[] = []
  for (let dz = -1; dz <= 2; dz++) {
    for (let dx = -1; dx <= 2; dx++) {
      tiles.push({ x: floorChunkX + dx, z: floorChunkZ + dz })
    }
  }
  const isRender = (t: TerrainChunk) =>
    t.x >= floorChunkX &&
    t.x <= floorChunkX + 1 &&
    t.z >= floorChunkZ &&
    t.z <= floorChunkZ + 1
  return [...tiles.filter(isRender), ...tiles.filter((t) => !isRender(t))]
}

/** Inverse of `createTerrainTiles`'s id format. Returns null on a malformed id. */
export function parseTileId(
  id: string
): { tileX: number; tileZ: number } | null {
  const [sx, sz] = id.split('_')
  const tileX = Number(sx)
  const tileZ = Number(sz)
  if (!Number.isFinite(tileX) || !Number.isFinite(tileZ)) return null
  return { tileX, tileZ }
}

/**
 * Get the floor-based chunk coordinate for a world position.
 * Combined with the 2×2 grid, this ensures the player is always
 * surrounded by terrain regardless of where they stand within a tile.
 */
export function getTerrainChunkFromPosition(
  position: Vector3Like,
  tileSize = TERRAIN_TILE_SIZE
): TerrainChunk {
  return {
    x: Math.floor(position.x / tileSize),
    z: Math.floor(position.z / tileSize),
  }
}

export function createTerrainGeometry(
  tileSize = TERRAIN_TILE_SIZE,
  tileSegments = TERRAIN_TILE_SEGMENTS
): THREE.BufferGeometry {
  const plane = new THREE.PlaneGeometry(
    tileSize,
    tileSize,
    tileSegments,
    tileSegments
  )
  plane.rotateX(-Math.PI / 2) // Lay flat on XZ
  return plane
}
