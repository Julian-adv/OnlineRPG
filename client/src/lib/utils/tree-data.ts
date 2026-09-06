import { filterVegetationInstances } from './vegetation-instances'

/**
 * Tree placement data: binary encode/decode and placement computation.
 *
 * Binary format v1:
 *   [u32 magic=0x54523031] [u32 tree1Count] [u32 tree2Count]
 *   [N × { u16 localX, u16 localZ, u8 rotation, u8 scale }]
 *   12-byte header + 6 bytes per instance.
 *   Altitude/Y is not stored; decode re-samples the tile heightmap.
 *
 * Rust source of truth for V1 constants:
 *   shared/src/tree_format.rs
 *
 * In-memory representation (TreePlacementData.buffer):
 *   [u32 tree1Count] [u32 tree2Count]
 *   [N × { f32 x, f32 y, f32 z, f32 rotation, f32 scale }]
 *   8-byte header + 20 bytes per instance.
 */

import { SHORT_GRASS_R_MIN, TALL_GRASS_R_MAX } from '../shaders/grass-material'
import { TERRAIN_TILE_SIZE } from '../components/game-scene/terrain-utils'
import {
  TILE_DIM,
  VERTS_PER_SIDE,
  decodeHeight,
  sampleHeight,
} from '../managers/terrain-height-types'
import { createRng } from './simplex-noise'
import type { TerrainHeightManager } from '../managers/terrainHeightManager'

const CHANNELS = 4
const FLOATS_PER_INSTANCE = 5 // x, y, z, rotation, scale

const V1_MAGIC = 0x54523031 // "TR01"
const V1_HEADER_BYTES = 12 // magic + 2 × u32
const V1_BYTES_PER_INSTANCE = 6 // u16 localX, u16 localZ, u8 rotation, u8 scale

/** Scale [min, range] per type: index 0 = tree.glb, index 1 = tree2.glb */
export const TREE_SCALE: [[number, number], [number, number]] = [
  [0.7, 2.3], // tree1: 0.7 ~ 3.0
  [0.6, 0.8], // tree2: 0.6 ~ 1.4
]

/** Base exclusion radius at scale 1.0: [tree1, tree2]. Actual radius = base × scale. */
export const TREE_EXCLUSION_RADIUS: [number, number] = [2.0, 1.5]

/** Axis-aligned exclusion rect [minX, minZ, maxX, maxZ] in world coords */
export type ExclusionRect = readonly [number, number, number, number]

const TREE_PROBABILITY = 0.08

const HEADER_BYTES = 8 // 2 × u32

export interface TreePlacementData {
  tree1Count: number
  tree2Count: number
  /** Interleaved f32: [x, y, z, rotation, scale] × tree1Count, then × tree2Count */
  buffer: ArrayBuffer
}

function tileSeed(tileX: number, tileZ: number): number {
  return ((tileX * 48271) ^ (tileZ * 16807)) | 0
}

/** Compute slope at a given cell from heightmap. */
function computeSlope(heightmap: Uint16Array, cx: number, cz: number): number {
  const hC = decodeHeight(heightmap[cz * VERTS_PER_SIDE + cx])
  const hL = cx > 0 ? decodeHeight(heightmap[cz * VERTS_PER_SIDE + cx - 1]) : hC
  const hR =
    cx < TILE_DIM ? decodeHeight(heightmap[cz * VERTS_PER_SIDE + cx + 1]) : hC
  const hU =
    cz > 0 ? decodeHeight(heightmap[(cz - 1) * VERTS_PER_SIDE + cx]) : hC
  const hD =
    cz < TILE_DIM ? decodeHeight(heightmap[(cz + 1) * VERTS_PER_SIDE + cx]) : hC
  return Math.sqrt((hR - hL) * (hR - hL) + (hD - hU) * (hD - hU)) / 2
}

function packTreeBuffer(
  tree1Data: Float32Array,
  tree2Data: Float32Array
): TreePlacementData {
  const tree1Count = tree1Data.length / FLOATS_PER_INSTANCE
  const tree2Count = tree2Data.length / FLOATS_PER_INSTANCE

  const totalBytes = HEADER_BYTES + (tree1Data.length + tree2Data.length) * 4
  const buffer = new ArrayBuffer(totalBytes)
  const header = new Uint32Array(buffer, 0, 2)
  header[0] = tree1Count
  header[1] = tree2Count

  const body = new Float32Array(buffer, HEADER_BYTES)
  body.set(tree1Data, 0)
  body.set(tree2Data, tree1Data.length)

  return { tree1Count, tree2Count, buffer }
}

export function filterTreeData(
  data: TreePlacementData,
  shouldRemove: (x: number, z: number) => boolean
): TreePlacementData | null {
  const tree1 = getTreeInstanceData(data, 'tree1')
  const tree2 = getTreeInstanceData(data, 'tree2')
  const filtered1 = filterVegetationInstances(tree1, shouldRemove)
  const filtered2 = filterVegetationInstances(tree2, shouldRemove)
  return filtered1 === tree1 && filtered2 === tree2
    ? null
    : packTreeBuffer(filtered1, filtered2)
}

export function computeTreePlacement(
  tileX: number,
  tileZ: number,
  splatData: Uint8Array,
  hMgr: TerrainHeightManager,
  exclusionRects?: readonly ExclusionRect[]
): TreePlacementData {
  const heightmap = hMgr.getHeightmap(tileX, tileZ)
  if (!heightmap) {
    return packTreeBuffer(new Float32Array(0), new Float32Array(0))
  }

  const tileMinX = tileX * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
  const tileMinZ = tileZ * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
  const rand = createRng(tileSeed(tileX, tileZ))

  const tree1Instances: number[] = []
  const tree2Instances: number[] = []

  for (let cz = 0; cz < TILE_DIM; cz++) {
    for (let cx = 0; cx < TILE_DIM; cx++) {
      const rVal = splatData[(cz * TILE_DIM + cx) * CHANNELS + 3]
      if (rVal < SHORT_GRASS_R_MIN || rVal > TALL_GRASS_R_MAX) continue

      if (rand() >= TREE_PROBABILITY) continue

      const slope = computeSlope(heightmap, cx, cz)
      if (slope > 1.5) continue

      const localX = cx + rand() * 0.8 + 0.1
      const localZ = cz + rand() * 0.8 + 0.1
      const worldY = sampleHeight(heightmap, localX, localZ)
      if (worldY < 0.5) continue

      const rotation = rand() * Math.PI * 2
      const isTree1 = rand() < 0.5
      const [scaleMin, scaleRange] = TREE_SCALE[isTree1 ? 0 : 1]
      const scale = scaleMin + rand() * scaleRange

      // Check exclusion zones (house footprints expanded by tree radius)
      if (exclusionRects && exclusionRects.length > 0) {
        const worldX = tileMinX + localX
        const worldZ = tileMinZ + localZ
        const r = TREE_EXCLUSION_RADIUS[isTree1 ? 0 : 1] * scale
        let blocked = false
        for (const [rMinX, rMinZ, rMaxX, rMaxZ] of exclusionRects) {
          if (
            worldX > rMinX - r &&
            worldX < rMaxX + r &&
            worldZ > rMinZ - r &&
            worldZ < rMaxZ + r
          ) {
            blocked = true
            break
          }
        }
        if (blocked) continue
      }

      const target = isTree1 ? tree1Instances : tree2Instances
      target.push(tileMinX + localX, worldY, tileMinZ + localZ, rotation, scale)
    }
  }

  return packTreeBuffer(
    new Float32Array(tree1Instances),
    new Float32Array(tree2Instances)
  )
}

export function getTreeInstanceData(
  data: TreePlacementData,
  type: 'tree1' | 'tree2'
): Float32Array {
  const tree1Floats = data.tree1Count * FLOATS_PER_INSTANCE

  switch (type) {
    case 'tree1':
      return new Float32Array(data.buffer, HEADER_BYTES, tree1Floats)
    case 'tree2':
      return new Float32Array(
        data.buffer,
        HEADER_BYTES + tree1Floats * 4,
        data.tree2Count * FLOATS_PER_INSTANCE
      )
  }
}

function emptyTrees(): TreePlacementData {
  return packTreeBuffer(new Float32Array(0), new Float32Array(0))
}

export function decodeTreeData(
  buffer: ArrayBuffer,
  tileX: number,
  tileZ: number,
  heightmap: Uint16Array | null
): TreePlacementData {
  if (buffer.byteLength < 4) return emptyTrees()

  const magic = new Uint32Array(buffer, 0, 1)[0]
  if (magic !== V1_MAGIC) return emptyTrees()

  const header = new Uint32Array(buffer, 0, 3)
  const tree1Count = header[1]
  const tree2Count = header[2]
  const totalInstances = tree1Count + tree2Count

  if (totalInstances === 0) return emptyTrees()

  const tileMinX = tileX * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
  const tileMinZ = tileZ * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2

  const outBuf = new ArrayBuffer(
    HEADER_BYTES + totalInstances * FLOATS_PER_INSTANCE * 4
  )
  const outHeader = new Uint32Array(outBuf, 0, 2)
  outHeader[0] = tree1Count
  outHeader[1] = tree2Count

  const outFloats = new Float32Array(outBuf, HEADER_BYTES)
  const view = new DataView(buffer)
  const counts = [tree1Count, tree2Count]
  let readOffset = V1_HEADER_BYTES
  let writeIdx = 0

  const posScale = TILE_DIM / 65535
  const rotScale = (Math.PI * 2) / 255

  for (let t = 0; t < 2; t++) {
    const [scaleMin, scaleRange] = TREE_SCALE[t]
    const scaleScale = scaleRange / 255
    const n = counts[t]

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
      readOffset += V1_BYTES_PER_INSTANCE
    }
  }

  return { tree1Count, tree2Count, buffer: outBuf }
}
