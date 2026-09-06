import * as THREE from 'three'
import { apiFetch, getTerrainApiUrl } from '../utils/networkUtils'
import { TERRAIN_TILE_SIZE } from '../components/game-scene/terrain-utils'
import { worldToTileCoord } from './terrain-height-types'
import { smoothstep, SPLAT_PADDED_DIM } from '../terrain/terrain-constants'
import { wrapTileX } from '../terrain/world-wrap'
import {
  BYTES_PER_CELL,
  applyBrush,
  readCell,
  writeCell,
} from '../terrain/splat-encoding'

const TILE_DIM = 64
const PAD = SPLAT_PADDED_DIM // 66 — interior is [1..TILE_DIM]×[1..TILE_DIM]
const PADDED_BYTES = PAD * PAD * BYTES_PER_CELL

function tileKey(tileX: number, tileZ: number): string {
  return `${wrapTileX(tileX)},${tileZ}`
}

export function paddedOffset(cx: number, cz: number): number {
  return (cz * PAD + cx) * BYTES_PER_CELL
}

function copyCell4(
  src: Uint8Array,
  srcOff: number,
  dst: Uint8Array,
  dstOff: number
) {
  dst[dstOff] = src[srcOff]
  dst[dstOff + 1] = src[srcOff + 1]
  dst[dstOff + 2] = src[srcOff + 2]
  dst[dstOff + 3] = src[srcOff + 3]
}

// Offsets relative to the tile's own index: (0,0) is the interior write;
// the other 8 are the 4 orthogonal edges + 4 corners that pull a 1-cell
// border strip from the adjacent tile.
export const NEIGHBOR_OFFSETS_9: ReadonlyArray<readonly [number, number]> = [
  [-1, -1],
  [0, -1],
  [1, -1],
  [-1, 0],
  [0, 0],
  [1, 0],
  [-1, 1],
  [0, 1],
  [1, 1],
]

// Sign-driven indexing: `dx/dz = 0` fills the interior column/row of the
// padded grid (src col/row tracks padded col/row - 1); `dx/dz = ±1` fills a
// single-cell border strip, pulling the neighbor's matching edge column /
// row. When `srcIsOwn` is true we're falling back to the tile's own edge
// (equivalent to ClampToEdge for that side).
export function writePaddedRange(
  dst: Uint8Array,
  src: Uint8Array,
  srcIsOwn: boolean,
  dx: number,
  dz: number
) {
  const xStart = dx === -1 ? 0 : dx === 1 ? PAD - 1 : 1
  const xEnd = dx === -1 ? 1 : dx === 1 ? PAD : PAD - 1
  const zStart = dz === -1 ? 0 : dz === 1 ? PAD - 1 : 1
  const zEnd = dz === -1 ? 1 : dz === 1 ? PAD : PAD - 1
  const fixedCol =
    dx === -1
      ? srcIsOwn
        ? 0
        : TILE_DIM - 1
      : dx === 1
        ? srcIsOwn
          ? TILE_DIM - 1
          : 0
        : -1
  const fixedRow =
    dz === -1
      ? srcIsOwn
        ? 0
        : TILE_DIM - 1
      : dz === 1
        ? srcIsOwn
          ? TILE_DIM - 1
          : 0
        : -1
  for (let pz = zStart; pz < zEnd; pz++) {
    const srcZ = fixedRow < 0 ? pz - 1 : fixedRow
    for (let px = xStart; px < xEnd; px++) {
      const srcX = fixedCol < 0 ? px - 1 : fixedCol
      copyCell4(
        src,
        (srcZ * TILE_DIM + srcX) * BYTES_PER_CELL,
        dst,
        paddedOffset(px, pz)
      )
    }
  }
}

function paintCell(
  data: Uint8Array,
  cellIdx: number,
  paintIdx: number,
  strength: number
): boolean {
  if (strength <= 0) return false
  const before = readCell(data, cellIdx)
  const after = applyBrush(before, paintIdx, strength)
  if (
    before.primaryIdx === after.primaryIdx &&
    before.secondaryIdx === after.secondaryIdx &&
    before.blend === after.blend
  ) {
    return false
  }
  writeCell(data, cellIdx, after)
  return true
}

// Cells adjacent to a painted cell need paintIdx in one of their palette slots
// so the shader's weight-space bilerp has a matching P/S pair across the
// boundary. Without this, `cellWeight` collapses for cells that have no
// matching slot, and when `nearestBlend` flips between cells with disjoint
// palettes the rendered mix jumps — a hard edge.
//
// A slot is considered "free" when its bilerp contribution is small enough
// (≤10%) that overwriting it with paintIdx produces at most a subtle tint
// shift but unlocks smooth blending across the road boundary. Genuinely
// mixed cells (both slots contributing >10%) are preserved and may still
// show a hard edge — the tradeoff favors user-painted mixes.
const FRINGE_REMAP_THRESHOLD = 25 // ≈10% of 255
function paintCellFringe(
  data: Uint8Array,
  cellIdx: number,
  paintIdx: number
): boolean {
  const base = cellIdx * BYTES_PER_CELL
  const indices = data[base]
  const primary = (indices >> 4) & 0x0f
  const secondary = indices & 0x0f
  if (primary === paintIdx || secondary === paintIdx) return false
  const blend = data[base + 2]
  if (primary === secondary || blend <= FRINGE_REMAP_THRESHOLD) {
    data[base] = (primary << 4) | (paintIdx & 0x0f)
    return true
  }
  if (blend >= 255 - FRINGE_REMAP_THRESHOLD) {
    data[base] = ((paintIdx & 0x0f) << 4) | secondary
    return true
  }
  return false
}

// Covers 8-connected neighbors (max diagonal distance √2, rounded up).
const FRINGE_PAD = 1.5

export class TerrainSplatManager {
  private splatmaps = new Map<string, Uint8Array>()
  private inflightSplatmaps = new Map<string, Promise<THREE.DataTexture>>()
  private textures = new Map<string, THREE.DataTexture>()
  private dirtyTiles = new Set<string>()
  private saveTimer: ReturnType<typeof setTimeout> | null = null
  private terrainApiUrl: string

  constructor() {
    this.terrainApiUrl = getTerrainApiUrl()
  }

  private textureData(tileX: number, tileZ: number): Uint8Array | null {
    const tex = this.textures.get(tileKey(tileX, tileZ))
    if (!tex) return null
    return (tex.image as unknown as { data: Uint8Array }).data
  }

  /** Write one 3×3 neighborhood offset — interior when (0,0), border strip
   *  otherwise. When the neighbor is missing, falls back to the tile's own
   *  edge (ClampToEdge behaviour). */
  private writeOffset(
    dst: Uint8Array,
    tileX: number,
    tileZ: number,
    own: Uint8Array,
    dx: number,
    dz: number
  ) {
    const nb =
      dx === 0 && dz === 0
        ? own
        : (this.splatmaps.get(tileKey(tileX + dx, tileZ + dz)) ?? own)
    writePaddedRange(dst, nb, nb === own, dx, dz)
  }

  private writePaddedAll(
    dst: Uint8Array,
    tileX: number,
    tileZ: number,
    own: Uint8Array
  ) {
    for (const [dx, dz] of NEIGHBOR_OFFSETS_9) {
      this.writeOffset(dst, tileX, tileZ, own, dx, dz)
    }
  }

  /** Re-upload just the border pixels for a tile whose neighbor changed. */
  private refreshBorders(tileX: number, tileZ: number) {
    const data = this.textureData(tileX, tileZ)
    if (!data) return
    const own = this.splatmaps.get(tileKey(tileX, tileZ))
    if (!own) return
    for (const [dx, dz] of NEIGHBOR_OFFSETS_9) {
      if (dx === 0 && dz === 0) continue
      this.writeOffset(data, tileX, tileZ, own, dx, dz)
    }
    const tex = this.textures.get(tileKey(tileX, tileZ))
    if (tex) tex.needsUpdate = true
  }

  /** Rewrite the full padded buffer after this tile's own data changed. */
  private refreshAll(tileX: number, tileZ: number) {
    const data = this.textureData(tileX, tileZ)
    if (!data) return
    const own = this.splatmaps.get(tileKey(tileX, tileZ))
    if (!own) return
    this.writePaddedAll(data, tileX, tileZ, own)
    const tex = this.textures.get(tileKey(tileX, tileZ))
    if (tex) tex.needsUpdate = true
  }

  private refreshNeighborBorders(tileX: number, tileZ: number) {
    for (const [dx, dz] of NEIGHBOR_OFFSETS_9) {
      if (dx === 0 && dz === 0) continue
      this.refreshBorders(tileX + dx, tileZ + dz)
    }
  }

  private createTexture(tileX: number, tileZ: number): THREE.DataTexture {
    const own = this.splatmaps.get(tileKey(tileX, tileZ))
    if (!own) {
      throw new Error(
        `createTexture: tile (${tileX}, ${tileZ}) has no splat data loaded`
      )
    }
    const padded = new Uint8Array(PADDED_BYTES)
    this.writePaddedAll(padded, tileX, tileZ, own)
    const texture = new THREE.DataTexture(
      padded,
      PAD,
      PAD,
      THREE.RGBAFormat,
      THREE.UnsignedByteType
    )
    texture.wrapS = texture.wrapT = THREE.ClampToEdgeWrapping
    // V2 bytes 0/1/3 are integer fields — must not be bilinearly interpolated.
    texture.minFilter = THREE.NearestFilter
    texture.magFilter = THREE.NearestFilter
    texture.generateMipmaps = false
    // PlaneGeometry UV v=0 is maxZ, v=1 is minZ (v decreases with Z).
    // flipY=true so data row 0 maps to v=1 (minZ), matching cz=0 = minZ.
    texture.flipY = true
    texture.needsUpdate = true
    return texture
  }

  async loadSplatmap(tileX: number, tileZ: number): Promise<THREE.DataTexture> {
    const key = tileKey(tileX, tileZ)
    const cached = this.textures.get(key)
    if (cached) return cached

    const finishTexture = () => {
      const texture = this.createTexture(tileX, tileZ)
      this.textures.set(key, texture)
      this.refreshNeighborBorders(tileX, tileZ)
      return texture
    }
    if (this.splatmaps.has(key)) return finishTexture()

    const inflight = this.inflightSplatmaps.get(key)
    if (inflight) return inflight

    const defaultFallback = (): THREE.DataTexture => {
      const updated = this.textures.get(key)
      if (updated) return updated
      // V2 default: all zeros → every cell is 100% palette slot 0.
      const data = new Uint8Array(TILE_DIM * TILE_DIM * BYTES_PER_CELL)
      if (!this.splatmaps.has(key)) this.splatmaps.set(key, data)
      return finishTexture()
    }

    const promise = Promise.resolve().then(
      async (): Promise<THREE.DataTexture> => {
        try {
          const url = `${this.terrainApiUrl}/api/terrain/splat/${tileX}/${tileZ}`
          const response = await fetch(url)
          if (!response.ok) {
            console.error(
              `Failed to load splatmap (${tileX}, ${tileZ}): ${response.status}`
            )
            return defaultFallback()
          }
          const buffer = await response.arrayBuffer()
          if (this.inflightSplatmaps.get(key) !== promise)
            return this.loadSplatmap(tileX, tileZ)
          const data = new Uint8Array(buffer)
          const updated = this.textures.get(key)
          if (updated) return updated
          if (!this.splatmaps.has(key)) this.splatmaps.set(key, data)
          return finishTexture()
        } catch (e) {
          console.error(`Splatmap fetch error (${tileX}, ${tileZ}):`, e)
          return defaultFallback()
        } finally {
          if (this.inflightSplatmaps.get(key) === promise)
            this.inflightSplatmaps.delete(key)
        }
      }
    )
    this.inflightSplatmaps.set(key, promise)
    return promise
  }

  getSplatTexture(tileX: number, tileZ: number): THREE.DataTexture | null {
    return this.textures.get(tileKey(tileX, tileZ)) ?? null
  }

  /** Get raw splatmap bytes for a tile (64×64×4 Uint8Array, V2 encoding), or null if not loaded. */
  getSplatData(tileX: number, tileZ: number): Uint8Array | null {
    return this.splatmaps.get(tileKey(tileX, tileZ)) ?? null
  }

  /** Apply splat brush and return the list of tiles that were modified. */
  applySplatBrush(
    worldX: number,
    worldZ: number,
    radius: number,
    paintIdx: number,
    strength: number
  ): { tileX: number; tileZ: number }[] {
    const sigma = radius / 2.5
    const outerR = radius + FRINGE_PAD

    const minWorldX = worldX - outerR
    const maxWorldX = worldX + outerR
    const minWorldZ = worldZ - outerR
    const maxWorldZ = worldZ + outerR

    const minTileX = Math.floor(
      (minWorldX + TERRAIN_TILE_SIZE / 2) / TERRAIN_TILE_SIZE
    )
    const maxTileX = Math.floor(
      (maxWorldX + TERRAIN_TILE_SIZE / 2) / TERRAIN_TILE_SIZE
    )
    const minTileZ = Math.floor(
      (minWorldZ + TERRAIN_TILE_SIZE / 2) / TERRAIN_TILE_SIZE
    )
    const maxTileZ = Math.floor(
      (maxWorldZ + TERRAIN_TILE_SIZE / 2) / TERRAIN_TILE_SIZE
    )

    const affectedTiles: { tileX: number; tileZ: number }[] = []

    for (let tz = minTileZ; tz <= maxTileZ; tz++) {
      for (let tx = minTileX; tx <= maxTileX; tx++) {
        const key = tileKey(tx, tz)
        const data = this.splatmaps.get(key)
        if (!data) continue

        const tileMinX = tx * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
        const tileMinZ = tz * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2

        const startCX = Math.max(0, Math.floor(minWorldX - tileMinX))
        const endCX = Math.min(TILE_DIM - 1, Math.floor(maxWorldX - tileMinX))
        const startCZ = Math.max(0, Math.floor(minWorldZ - tileMinZ))
        const endCZ = Math.min(TILE_DIM - 1, Math.floor(maxWorldZ - tileMinZ))

        let changed = false

        for (let cz = startCZ; cz <= endCZ; cz++) {
          for (let cx = startCX; cx <= endCX; cx++) {
            const dx = tileMinX + cx - worldX
            const dz = tileMinZ + cz - worldZ
            const dist = Math.sqrt(dx * dx + dz * dz)
            if (dist > outerR) continue

            const cellIdx = cz * TILE_DIM + cx
            if (dist > radius) {
              if (paintCellFringe(data, cellIdx, paintIdx)) changed = true
            } else {
              const weight = Math.exp(-(dist * dist) / (2 * sigma * sigma))
              if (paintCell(data, cellIdx, paintIdx, weight * strength)) {
                changed = true
              }
            }
          }
        }

        if (changed) {
          this.refreshAll(tx, tz)
          this.refreshNeighborBorders(tx, tz)
          this.dirtyTiles.add(key)
          affectedTiles.push({ tileX: tx, tileZ: tz })
        }
      }
    }

    if (this.dirtyTiles.size > 0) this.scheduleSave()
    return affectedTiles
  }

  /** Apply splat brush along a line segment (used by the road tool). */
  applySplatLine(
    x1: number,
    z1: number,
    x2: number,
    z2: number,
    radius: number,
    paintIdx: number,
    strength: number
  ): { tileX: number; tileZ: number }[] {
    const lineDx = x2 - x1
    const lineDz = z2 - z1
    const lenSq = lineDx * lineDx + lineDz * lineDz
    if (lenSq < 1e-6) return []

    // Flat-core falloff: fully saturate within innerR, smoothstep to 0 at radius.
    const innerR = radius * 0.3
    const outerR = radius + FRINGE_PAD

    const minWorldX = Math.min(x1, x2) - outerR
    const maxWorldX = Math.max(x1, x2) + outerR
    const minWorldZ = Math.min(z1, z2) - outerR
    const maxWorldZ = Math.max(z1, z2) + outerR

    const minTileX = worldToTileCoord(minWorldX)
    const maxTileX = worldToTileCoord(maxWorldX)
    const minTileZ = worldToTileCoord(minWorldZ)
    const maxTileZ = worldToTileCoord(maxWorldZ)

    const affectedTiles: { tileX: number; tileZ: number }[] = []

    for (let tz = minTileZ; tz <= maxTileZ; tz++) {
      for (let tx = minTileX; tx <= maxTileX; tx++) {
        const key = tileKey(tx, tz)
        const data = this.splatmaps.get(key)
        if (!data) continue

        const tileMinX = tx * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
        const tileMinZ = tz * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2

        const startCX = Math.max(0, Math.floor(minWorldX - tileMinX))
        const endCX = Math.min(TILE_DIM - 1, Math.floor(maxWorldX - tileMinX))
        const startCZ = Math.max(0, Math.floor(minWorldZ - tileMinZ))
        const endCZ = Math.min(TILE_DIM - 1, Math.floor(maxWorldZ - tileMinZ))

        let changed = false

        for (let cz = startCZ; cz <= endCZ; cz++) {
          for (let cx = startCX; cx <= endCX; cx++) {
            const wx = tileMinX + cx
            const wz = tileMinZ + cz

            const vx = wx - x1
            const vz = wz - z1
            let t = (vx * lineDx + vz * lineDz) / lenSq
            if (t < 0) t = 0
            else if (t > 1) t = 1
            const ddx = wx - (x1 + t * lineDx)
            const ddz = wz - (z1 + t * lineDz)
            const dist = Math.sqrt(ddx * ddx + ddz * ddz)
            if (dist > outerR) continue

            const cellIdx = cz * TILE_DIM + cx
            if (dist > radius) {
              if (paintCellFringe(data, cellIdx, paintIdx)) changed = true
            } else {
              const weight = 1 - smoothstep(innerR, radius, dist)
              if (paintCell(data, cellIdx, paintIdx, weight * strength)) {
                changed = true
              }
            }
          }
        }

        if (changed) {
          this.refreshAll(tx, tz)
          this.refreshNeighborBorders(tx, tz)
          this.dirtyTiles.add(key)
          affectedTiles.push({ tileX: tx, tileZ: tz })
        }
      }
    }

    if (this.dirtyTiles.size > 0) this.scheduleSave()
    return affectedTiles
  }

  private scheduleSave() {
    if (this.saveTimer !== null) {
      clearTimeout(this.saveTimer)
    }
    this.saveTimer = setTimeout(() => {
      this.saveDirtyTiles()
      this.saveTimer = null
    }, 1000)
  }

  private async saveDirtyTiles() {
    const tilesToSave = [...this.dirtyTiles]

    for (const key of tilesToSave) {
      const data = this.splatmaps.get(key)
      if (!data) {
        this.dirtyTiles.delete(key)
        continue
      }

      const [txStr, tzStr] = key.split(',')
      const tx = parseInt(txStr)
      const tz = parseInt(tzStr)

      const url = `${this.terrainApiUrl}/api/terrain/splat/${tx}/${tz}`
      const body = new Uint8Array(data).buffer as ArrayBuffer

      try {
        await apiFetch(url, {
          method: 'PUT',
          headers: { 'Content-Type': 'application/octet-stream' },
          body,
        })
        this.dirtyTiles.delete(key)
      } catch (e) {
        console.error(`Failed to save splatmap for tile (${tx}, ${tz}):`, e)
      }
    }
  }

  /** Directly set splatmap data for a tile (used by terrain generator). */
  setSplatmap(tileX: number, tileZ: number, data: Uint8Array): void {
    const key = tileKey(tileX, tileZ)
    this.splatmaps.set(key, data)
    if (this.textures.has(key)) this.refreshAll(tileX, tileZ)
    this.refreshNeighborBorders(tileX, tileZ)
  }

  /** Mark a tile as dirty so it will be saved on next save cycle. */
  markDirty(tileX: number, tileZ: number): void {
    this.dirtyTiles.add(tileKey(tileX, tileZ))
  }

  /** Force-save all dirty tiles immediately (cancels pending debounce). */
  async saveAllDirty(): Promise<void> {
    if (this.saveTimer !== null) {
      clearTimeout(this.saveTimer)
      this.saveTimer = null
    }
    await this.saveDirtyTiles()
  }

  invalidateLandscaping(tileX: number, tileZ: number) {
    this.inflightSplatmaps.delete(tileKey(tileX, tileZ))
    this.unloadTile(tileX, tileZ)
  }

  unloadTile(tileX: number, tileZ: number) {
    const key = tileKey(tileX, tileZ)
    const texture = this.textures.get(key)
    if (texture) {
      texture.dispose()
    }
    this.splatmaps.delete(key)
    this.textures.delete(key)
    // Loaded neighbors used our edge cells for their borders — refresh them
    // so they fall back to their own edge copy instead of stale data.
    this.refreshNeighborBorders(tileX, tileZ)
  }

  /** Remove cached data without disposing GPU textures (they may still be rendered).
   *  Refuses to evict if the tile has unsaved changes. */
  evictCachedData(tileX: number, tileZ: number) {
    const key = tileKey(tileX, tileZ)
    if (this.dirtyTiles.has(key)) return
    this.splatmaps.delete(key)
    this.textures.delete(key)
    this.refreshNeighborBorders(tileX, tileZ)
  }

  async destroy() {
    if (this.saveTimer !== null) {
      clearTimeout(this.saveTimer)
      this.saveTimer = null
    }
    if (this.dirtyTiles.size > 0) {
      await this.saveDirtyTiles()
    }
    for (const texture of this.textures.values()) {
      texture.dispose()
    }
  }
}
