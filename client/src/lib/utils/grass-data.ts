import { filterVegetationInstances } from './vegetation-instances'

/**
 * Grass placement data: binary encode/decode and placement computation.
 *
 * Binary format v3 (quantized):
 *   [u32 magic=0x47523033] [u32 shortCount] [u32 tallCount] [u32 flowerCount]
 *   [N × { u16 localX, u16 localZ, u8 rotation, u8 scale }]
 *   16-byte header + 6 bytes per instance.
 *
 * In-memory representation (GrassPlacementData.buffer) uses the v2 layout:
 *   [u32 shortCount] [u32 tallCount] [u32 flowerCount]
 *   [N × { f32 x, f32 y, f32 z, f32 rotation, f32 scale }]
 *   12-byte header + 20 bytes per instance.
 *
 * v2 data on disk is treated as stale and decoded as empty (needs regeneration).
 */

import {
  SHORT_GRASS_R_MIN,
  SHORT_GRASS_R_MAX,
  TALL_GRASS_R_MIN,
  TALL_GRASS_R_MAX,
} from '../shaders/grass-material'
import { getTreeInstanceData, type TreePlacementData } from './tree-data'
import { TERRAIN_TILE_SIZE } from '../components/game-scene/terrain-utils'
import { TILE_DIM, sampleHeight } from '../managers/terrain-height-types'
import { createRng } from './simplex-noise'
import type { TerrainHeightManager } from '../managers/terrainHeightManager'
import { getCurrentPreset } from '../stores/graphicsSettings'
import {
  BYTES_PER_CELL,
  VEGMETA_OFFSET,
  writeGrass,
} from '../terrain/splat-encoding'
import {
  buildGrassMask,
  scatterGrassCircles,
} from '../terrain/terrain-splat-gen'

const CHANNELS = 4
const FLOATS_PER_INSTANCE = 5 // x, y, z, rotation, scale

const V3_MAGIC = 0x47523033 // "GR03"
const V3_HEADER_BYTES = 16 // magic + 3 × u32
const V3_BYTES_PER_INSTANCE = 6 // u16 localX, u16 localZ, u8 rotation, u8 scale

const SHORT_SCALE_MIN = 0.4
const SHORT_SCALE_RANGE = 0.3
const TALL_SCALE_MIN = 0.5
const TALL_SCALE_RANGE = 1.0

export interface GrassPlacementData {
  shortCount: number
  tallCount: number
  flowerCount: number
  /** Interleaved f32: [x, y, z, rotation, scale] × shortCount, then × tallCount, then × flowerCount */
  buffer: ArrayBuffer
}

function tileSeed(tileX: number, tileZ: number): number {
  return ((tileX * 73856093) ^ (tileZ * 19349663)) | 0
}

interface VegParams {
  rMin: number
  rMax: number
  scaleMin: number
  scaleRange: number
  bladesPerAxis: number
}

const SHORT_BLADES_PER_AXIS = 12
const TALL_BLADES_PER_AXIS = 10
const BOUNDARY_BLEND_RATIO = 0.3

const SHORT_PARAMS: VegParams = {
  rMin: SHORT_GRASS_R_MIN,
  rMax: SHORT_GRASS_R_MAX,
  scaleMin: SHORT_SCALE_MIN,
  scaleRange: SHORT_SCALE_RANGE,
  bladesPerAxis: SHORT_BLADES_PER_AXIS,
}

const TALL_PARAMS: VegParams = {
  rMin: TALL_GRASS_R_MIN,
  rMax: TALL_GRASS_R_MAX,
  scaleMin: TALL_SCALE_MIN,
  scaleRange: TALL_SCALE_RANGE,
  bladesPerAxis: TALL_BLADES_PER_AXIS,
}

const NEIGHBOR_OFFSETS: [number, number][] = [
  [0, -1],
  [0, 1],
  [-1, 0],
  [1, 0],
]

/** Check if a cell borders a cell of the other grass type. */
function isBoundaryCell(
  splatData: Uint8Array,
  cx: number,
  cz: number,
  otherRMin: number,
  otherRMax: number
): boolean {
  for (const [dx, dz] of NEIGHBOR_OFFSETS) {
    const nx = cx + dx
    const nz = cz + dz
    if (nx < 0 || nx >= TILE_DIM || nz < 0 || nz >= TILE_DIM) continue
    const r = splatData[(nz * TILE_DIM + nx) * CHANNELS + 3]
    if (r >= otherRMin && r <= otherRMax) return true
  }
  return false
}

function concatFloat32(a: Float32Array, b: Float32Array): Float32Array {
  if (b.length === 0) return a
  if (a.length === 0) return b
  const out = new Float32Array(a.length + b.length)
  out.set(a, 0)
  out.set(b, a.length)
  return out
}

function computeInstances(
  params: VegParams,
  otherParams: VegParams,
  tileX: number,
  tileZ: number,
  splatData: Uint8Array,
  heightmap: Uint16Array
): { own: Float32Array; converted: Float32Array } {
  const tileMinX = tileX * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
  const tileMinZ = tileZ * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
  const bpa = params.bladesPerAxis
  const step = 1.0 / bpa
  const densityRange = params.rMax - params.rMin
  const rand = createRng(tileSeed(tileX, tileZ) ^ params.rMin)

  const ownInstances: number[] = []
  const convertedInstances: number[] = []

  for (let cz = 0; cz < TILE_DIM; cz++) {
    for (let cx = 0; cx < TILE_DIM; cx++) {
      const rVal = splatData[(cz * TILE_DIM + cx) * CHANNELS + 3]
      if (rVal < params.rMin || rVal > params.rMax) continue
      const density = densityRange > 0 ? (rVal - params.rMin) / densityRange : 1
      const boundary = isBoundaryCell(
        splatData,
        cx,
        cz,
        otherParams.rMin,
        otherParams.rMax
      )

      for (let dz = 0; dz < bpa; dz++) {
        for (let dx = 0; dx < bpa; dx++) {
          const localX = cx + dx * step + rand() * step
          const localZ = cz + dz * step + rand() * step
          if (rand() >= density) continue
          const worldY = sampleHeight(heightmap, localX, localZ)
          if (worldY < 0.05) continue

          const rotation = rand() * Math.PI * 2
          const isConverted = boundary && rand() < BOUNDARY_BLEND_RATIO
          const scale = isConverted
            ? otherParams.scaleMin + rand() * otherParams.scaleRange
            : params.scaleMin + rand() * params.scaleRange

          const target = isConverted ? convertedInstances : ownInstances
          target.push(
            tileMinX + localX,
            worldY,
            tileMinZ + localZ,
            rotation,
            scale
          )
        }
      }
    }
  }

  return {
    own: new Float32Array(ownInstances),
    converted: new Float32Array(convertedInstances),
  }
}

const FLOWER_SCALE_MIN = 0.42
const FLOWER_SCALE_RANGE = 0.18

/** Scale ranges per type index: [short=0, tall=1, flower=2] */
const TYPE_SCALE: [number, number][] = [
  [SHORT_PARAMS.scaleMin, SHORT_PARAMS.scaleRange],
  [TALL_PARAMS.scaleMin, TALL_PARAMS.scaleRange],
  [FLOWER_SCALE_MIN, FLOWER_SCALE_RANGE],
]

/**
 * Scatter flowers within short grass cells.
 * Lower grass density (lower R value) → higher flower probability.
 * R=230 (sparsest) → ~50%, R=239 (densest) → ~1%
 */
function computeFlowerInstances(
  tileX: number,
  tileZ: number,
  splatData: Uint8Array,
  heightmap: Uint16Array
): Float32Array {
  const tileMinX = tileX * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
  const tileMinZ = tileZ * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
  const rand = createRng(tileSeed(tileX, tileZ) ^ 0xf10e)
  const densityRange = SHORT_GRASS_R_MAX - SHORT_GRASS_R_MIN

  const instances: number[] = []

  for (let cz = 0; cz < TILE_DIM; cz++) {
    for (let cx = 0; cx < TILE_DIM; cx++) {
      const rVal = splatData[(cz * TILE_DIM + cx) * CHANNELS + 3]
      if (rVal < SHORT_GRASS_R_MIN || rVal > SHORT_GRASS_R_MAX) continue

      // Flower probability: high when grass is sparse, low when dense.
      // t=0 (low vegMeta, sparse) → 80%, t=1 (high, dense) → 10%
      const t = (rVal - SHORT_GRASS_R_MIN) / densityRange
      const flowerProb = 0.4 * Math.pow(0.125, t) // 0.40 → 0.05

      // One flower chance per cell
      const localX = cx + 0.5 + (rand() - 0.5) * 0.8
      const localZ = cz + 0.5 + (rand() - 0.5) * 0.8
      if (rand() >= flowerProb) continue

      const worldY = sampleHeight(heightmap, localX, localZ)
      if (worldY < 0.05) continue

      const rotation = rand() * Math.PI * 2
      const scale = FLOWER_SCALE_MIN + rand() * FLOWER_SCALE_RANGE

      instances.push(
        tileMinX + localX,
        worldY,
        tileMinZ + localZ,
        rotation,
        scale
      )
    }
  }

  return new Float32Array(instances)
}

const HEADER_BYTES = 12 // 3 × u32

/** Pack three instance arrays into a single GrassPlacementData buffer. */
function packGrassBuffer(
  shortData: Float32Array,
  tallData: Float32Array,
  flowerData: Float32Array
): GrassPlacementData {
  const shortCount = shortData.length / FLOATS_PER_INSTANCE
  const tallCount = tallData.length / FLOATS_PER_INSTANCE
  const flowerCount = flowerData.length / FLOATS_PER_INSTANCE

  const totalBytes =
    HEADER_BYTES + (shortData.length + tallData.length + flowerData.length) * 4
  const buffer = new ArrayBuffer(totalBytes)
  const header = new Uint32Array(buffer, 0, 3)
  header[0] = shortCount
  header[1] = tallCount
  header[2] = flowerCount

  const body = new Float32Array(buffer, HEADER_BYTES)
  body.set(shortData, 0)
  body.set(tallData, shortData.length)
  body.set(flowerData, shortData.length + tallData.length)

  return { shortCount, tallCount, flowerCount, buffer }
}

/**
 * Compute grass placement data for a single tile.
 * Requires heightmap and splatmap data to be already loaded in the manager.
 */
export function computeGrassPlacement(
  tileX: number,
  tileZ: number,
  splatData: Uint8Array,
  hMgr: TerrainHeightManager
): GrassPlacementData {
  const heightmap = hMgr.getHeightmap(tileX, tileZ)
  if (!heightmap) {
    return packGrassBuffer(
      new Float32Array(0),
      new Float32Array(0),
      new Float32Array(0)
    )
  }

  const shortResult = computeInstances(
    SHORT_PARAMS,
    TALL_PARAMS,
    tileX,
    tileZ,
    splatData,
    heightmap
  )
  const tallResult = computeInstances(
    TALL_PARAMS,
    SHORT_PARAMS,
    tileX,
    tileZ,
    splatData,
    heightmap
  )
  const shortInstances = concatFloat32(shortResult.own, tallResult.converted)
  const tallInstances = concatFloat32(tallResult.own, shortResult.converted)
  const flowerInstances = computeFlowerInstances(
    tileX,
    tileZ,
    splatData,
    heightmap
  )

  return packGrassBuffer(shortInstances, tallInstances, flowerInstances)
}

/**
 * Generate and save grass data for a batch of tiles via the manager.
 * Yields to the event loop periodically and saves in parallel batches.
 */
export async function generateAndSaveGrassData(
  tiles: { tileX: number; tileZ: number; splatmap: Uint8Array }[],
  hMgr: TerrainHeightManager,
  grassMgr: {
    saveGrassData(
      tileX: number,
      tileZ: number,
      data: GrassPlacementData
    ): Promise<void>
  },
  onProgress?: (label: string) => void
): Promise<void> {
  const BATCH_SIZE = 8
  const grassResults: {
    tileX: number
    tileZ: number
    data: GrassPlacementData
  }[] = []
  for (let i = 0; i < tiles.length; i++) {
    const tile = tiles[i]
    const data = computeGrassPlacement(
      tile.tileX,
      tile.tileZ,
      tile.splatmap,
      hMgr
    )
    grassResults.push({ tileX: tile.tileX, tileZ: tile.tileZ, data })
    if (i % 4 === 3) {
      onProgress?.(`Generating grass... ${i + 1}/${tiles.length}`)
      await new Promise((r) => setTimeout(r, 0))
    }
  }
  for (let i = 0; i < grassResults.length; i += BATCH_SIZE) {
    const batch = grassResults.slice(i, i + BATCH_SIZE)
    onProgress?.(
      `Saving grass... ${Math.min(i + BATCH_SIZE, grassResults.length)}/${grassResults.length}`
    )
    await Promise.all(
      batch.map((g) => grassMgr.saveGrassData(g.tileX, g.tileZ, g.data))
    )
  }
}

/** Base radius around each tree center in which grass/flowers are removed, scaled by tree scale. */
export const TREE_CLEAR_RADIUS_BASE = 0.7

/**
 * Filter all grass/flower types by a removal predicate.
 * Returns null if no instances were removed (caller can skip saving).
 */
export function filterGrassData(
  data: GrassPlacementData,
  shouldRemove: (x: number, z: number) => boolean
): GrassPlacementData | null {
  const shortRaw = getInstanceData(data, 'short')
  const tallRaw = getInstanceData(data, 'tall')
  const flowerRaw = getInstanceData(data, 'flower')

  const shortFiltered = filterVegetationInstances(shortRaw, shouldRemove)
  const tallFiltered = filterVegetationInstances(tallRaw, shouldRemove)
  const flowerFiltered = filterVegetationInstances(flowerRaw, shouldRemove)

  if (
    shortFiltered === shortRaw &&
    tallFiltered === tallRaw &&
    flowerFiltered === flowerRaw
  ) {
    return null
  }

  return packGrassBuffer(shortFiltered, tallFiltered, flowerFiltered)
}

/**
 * Remove grass/flower instances that overlap with tree trunks.
 * Returns null if no instances were removed (caller can skip saving).
 */
export function removeGrassNearTrees(
  grassData: GrassPlacementData,
  treeData: TreePlacementData
): GrassPlacementData | null {
  const totalTrees = treeData.tree1Count + treeData.tree2Count
  if (totalTrees === 0) return null

  // Store x, z, r² per tree (radius proportional to tree scale)
  const treeInfo = new Float64Array(totalTrees * 3)
  let idx = 0
  for (const type of ['tree1', 'tree2'] as const) {
    const raw = getTreeInstanceData(treeData, type)
    const count = raw.length / 5
    for (let i = 0; i < count; i++) {
      const scale = raw[i * 5 + 4]
      const r = TREE_CLEAR_RADIUS_BASE * scale
      treeInfo[idx++] = raw[i * 5] // x
      treeInfo[idx++] = raw[i * 5 + 2] // z
      treeInfo[idx++] = r * r // r²
    }
  }

  return filterGrassData(grassData, (x, z) => {
    for (let i = 0; i < totalTrees; i++) {
      const base = i * 3
      const dx = x - treeInfo[base]
      const dz = z - treeInfo[base + 1]
      if (dx * dx + dz * dz <= treeInfo[base + 2]) return true
    }
    return false
  })
}

/**
 * Remove grass instances that fall within a world-space rectangle.
 * Returns null if no instances were removed (caller can skip saving).
 */
export function removeGrassInRect(
  data: GrassPlacementData,
  minX: number,
  minZ: number,
  maxX: number,
  maxZ: number
): GrassPlacementData | null {
  return filterGrassData(
    data,
    (x, z) => x >= minX && x <= maxX && z >= minZ && z <= maxZ
  )
}

/**
 * Encode GrassPlacementData (in-memory v2 layout) to v3 quantized binary for storage.
 */
export function encodeGrassBuffer(
  data: GrassPlacementData,
  tileX: number,
  tileZ: number
): ArrayBuffer {
  const tileMinX = tileX * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
  const tileMinZ = tileZ * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
  const totalInstances = data.shortCount + data.tallCount + data.flowerCount
  const buf = new ArrayBuffer(
    V3_HEADER_BYTES + totalInstances * V3_BYTES_PER_INSTANCE
  )

  const header = new Uint32Array(buf, 0, 4)
  header[0] = V3_MAGIC
  header[1] = data.shortCount
  header[2] = data.tallCount
  header[3] = data.flowerCount

  const view = new DataView(buf)
  const types: ('short' | 'tall' | 'flower')[] = ['short', 'tall', 'flower']
  let writeOffset = V3_HEADER_BYTES

  for (let t = 0; t < 3; t++) {
    const raw = getInstanceData(data, types[t])
    const [scaleMin, scaleRange] = TYPE_SCALE[t]
    const n = raw.length / FLOATS_PER_INSTANCE
    const posScale = 65535 / TILE_DIM
    const rotScale = 255 / (Math.PI * 2)
    const scaleScale = 255 / scaleRange

    for (let i = 0; i < n; i++) {
      const base = i * FLOATS_PER_INSTANCE
      const localX = raw[base] - tileMinX
      const localZ = raw[base + 2] - tileMinZ

      view.setUint16(writeOffset, Math.round(localX * posScale), true)
      view.setUint16(writeOffset + 2, Math.round(localZ * posScale), true)
      view.setUint8(
        writeOffset + 4,
        Math.round(raw[base + 3] * rotScale) & 0xff
      )
      view.setUint8(
        writeOffset + 5,
        Math.min(
          255,
          Math.max(0, Math.round((raw[base + 4] - scaleMin) * scaleScale))
        )
      )
      writeOffset += V3_BYTES_PER_INSTANCE
    }
  }

  return buf
}

function emptyGrass(): GrassPlacementData {
  return packGrassBuffer(
    new Float32Array(0),
    new Float32Array(0),
    new Float32Array(0)
  )
}

/**
 * Decode binary grass data. v3 (quantized) is expanded to in-memory v2 layout.
 * v2 (legacy) returns empty data — tile needs regeneration.
 */
export function decodeGrassData(
  buffer: ArrayBuffer,
  tileX: number,
  tileZ: number,
  heightmap: Uint16Array | null
): GrassPlacementData {
  if (buffer.byteLength < 4) return emptyGrass()

  const magic = new Uint32Array(buffer, 0, 1)[0]
  if (magic !== V3_MAGIC) {
    // v2 legacy format — return empty so tile gets regenerated
    return emptyGrass()
  }

  const header = new Uint32Array(buffer, 0, 4)
  const shortCount = header[1]
  const tallCount = header[2]
  const flowerCount = header[3]
  const totalInstances = shortCount + tallCount + flowerCount

  if (totalInstances === 0) return emptyGrass()

  const tileMinX = tileX * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
  const tileMinZ = tileZ * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2

  // Expand to v2 in-memory layout
  const outBuf = new ArrayBuffer(
    HEADER_BYTES + totalInstances * FLOATS_PER_INSTANCE * 4
  )
  const outHeader = new Uint32Array(outBuf, 0, 3)
  outHeader[0] = shortCount
  outHeader[1] = tallCount
  outHeader[2] = flowerCount

  const outFloats = new Float32Array(outBuf, HEADER_BYTES)
  const view = new DataView(buffer)
  const counts = [shortCount, tallCount, flowerCount]
  let readOffset = V3_HEADER_BYTES
  let writeIdx = 0

  for (let t = 0; t < 3; t++) {
    const [scaleMin, scaleRange] = TYPE_SCALE[t]
    const n = counts[t]

    const posScale = TILE_DIM / 65535
    const rotScale = (Math.PI * 2) / 255
    const scaleScale = scaleRange / 255

    for (let i = 0; i < n; i++) {
      const localX = view.getUint16(readOffset, true) * posScale
      const localZ = view.getUint16(readOffset + 2, true) * posScale
      const rotation = view.getUint8(readOffset + 4) * rotScale
      const scale = scaleMin + view.getUint8(readOffset + 5) * scaleScale

      const worldX = tileMinX + localX
      const worldZ = tileMinZ + localZ
      const worldY = heightmap ? sampleHeight(heightmap, localX, localZ) : 0

      outFloats[writeIdx] = worldX
      outFloats[writeIdx + 1] = worldY
      outFloats[writeIdx + 2] = worldZ
      outFloats[writeIdx + 3] = rotation
      outFloats[writeIdx + 4] = scale
      writeIdx += FLOATS_PER_INSTANCE
      readOffset += V3_BYTES_PER_INSTANCE
    }
  }

  return { shortCount, tallCount, flowerCount, buffer: outBuf }
}

/** Deterministically thin a Float32Array of instances by keeping only a fraction. */
function thinInstances(src: Float32Array, keep: number): Float32Array {
  if (keep >= 1) return src
  const count = src.length / FLOATS_PER_INSTANCE

  // Pass 1: count survivors
  let survived = 0
  for (let i = 0; i < count; i++) {
    const x = src[i * FLOATS_PER_INSTANCE]
    const z = src[i * FLOATS_PER_INSTANCE + 2]
    const h =
      ((Math.imul((x * 374761) | 0, 668265263) ^
        Math.imul((z * 550929) | 0, 374761393)) >>>
        0) /
      0xffffffff
    if (h < keep) survived++
  }
  if (survived === count) return src

  // Pass 2: copy survivors into pre-allocated buffer
  const out = new Float32Array(survived * FLOATS_PER_INSTANCE)
  let offset = 0
  for (let i = 0; i < count; i++) {
    const base = i * FLOATS_PER_INSTANCE
    const x = src[base]
    const z = src[base + 2]
    const h =
      ((Math.imul((x * 374761) | 0, 668265263) ^
        Math.imul((z * 550929) | 0, 374761393)) >>>
        0) /
      0xffffffff
    if (h < keep) {
      out.set(src.subarray(base, base + FLOATS_PER_INSTANCE), offset)
      offset += FLOATS_PER_INSTANCE
    }
  }
  return out
}

/** Extract raw instance Float32Array for a given type from decoded data. */
export function getInstanceData(
  data: GrassPlacementData,
  type: 'short' | 'tall' | 'flower'
): Float32Array {
  const headerBytes = 12
  const shortFloats = data.shortCount * FLOATS_PER_INSTANCE
  const tallFloats = data.tallCount * FLOATS_PER_INSTANCE

  switch (type) {
    case 'short':
      return new Float32Array(data.buffer, headerBytes, shortFloats)
    case 'tall':
      return new Float32Array(
        data.buffer,
        headerBytes + shortFloats * 4,
        tallFloats
      )
    case 'flower':
      return new Float32Array(
        data.buffer,
        headerBytes + (shortFloats + tallFloats) * 4,
        data.flowerCount * FLOATS_PER_INSTANCE
      )
  }
}

/**
 * Extract instance data with density thinning applied.
 * Use this for display; use getInstanceData() for data manipulation.
 */
export function getThinnedInstanceData(
  data: GrassPlacementData,
  type: 'short' | 'tall' | 'flower'
): Float32Array {
  const raw = getInstanceData(data, type)
  if (type === 'flower') return raw
  return thinInstances(raw, getCurrentPreset().grassDensity)
}

/**
 * Regenerate vegMeta (byte 3) for a tile's splatmap in-place. Uses the same
 * scatter helper and seed as the region generator, so the resulting pattern
 * matches what `generateSplatMap` would have produced for this tile — grass
 * circles continue naturally across tile borders without seams.
 */
export function regenerateVegMeta(
  splatData: Uint8Array,
  tileX: number,
  tileZ: number
): void {
  const D = TILE_DIM
  const grassMask = buildGrassMask(splatData, D * D)
  const densityGrid = new Uint8Array(D * D)
  const typeGrid = new Uint8Array(D * D)
  scatterGrassCircles(
    D,
    D,
    tileX * D,
    tileZ * D,
    grassMask,
    densityGrid,
    typeGrid,
    0
  )

  for (let i = 0; i < D * D; i++) {
    splatData[i * BYTES_PER_CELL + VEGMETA_OFFSET] = writeGrass(
      densityGrid[i],
      typeGrid[i] === 1
    )
  }
}
