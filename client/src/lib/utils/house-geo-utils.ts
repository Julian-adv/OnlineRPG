/**
 * house-geo-utils.ts — Shared constants, types, and geometry helpers for house building.
 */
import * as THREE from 'three'
import { mergeGeometries } from 'three/examples/jsm/utils/BufferGeometryUtils.js'
import type { RoomData } from '../types/housing'
import { getHousingMaterial, HOUSING_TEXTURES } from './housing-textures'

export const WALL_THICKNESS = 0.1
export const FLOOR_THICKNESS = 0.1
export const DEFAULT_WALL_HEIGHT = 3
export const LANDING_DEPTH = 0.5
// Keep in sync with shared housing::MAX_FLOOR_LEVEL (the dungeon base derives
// from the Rust value; only this housing-UI copy needs manual bumping).
export const MAX_FLOOR_LEVEL = 3
export const ROOF_OVERHANG = 0.3
export const FLOOR_OVERHANG_PER_LEVEL = 0.15
export const ROOF_PITCH: Record<string, number> = {
  gabled: 0.8,
  steep: 1.4,
}

export { HOUSING_TEXTURES }

export const FRAME_PROTRUSION = 0.04
export const FRAME_DEPTH = WALL_THICKNESS + FRAME_PROTRUSION * 2
export const WOOD_TEXTURE_IDX = HOUSING_TEXTURES.findIndex(
  (e) => e.glb === 'housing/wood_shutter_1k'
)
export const SHUTTER_PANEL_TEXTURE_IDX = HOUSING_TEXTURES.findIndex(
  (e) => e.glb === 'housing/shutter_panel_1k'
)

/** Compute floor overhang for a given floor level (upper floors extend beyond walls). */
export function floorOverhang(floorLevel: number): number {
  return floorLevel * FLOOR_OVERHANG_PER_LEVEL
}

/** Y offset used to hide front walls instead of toggling visible (WebGPU workaround) */
export const OFFSCREEN_Y = -10000

export type WallDirection = 'north' | 'south' | 'east' | 'west'

/** Compute the Y base for a given floor level, accounting for floor thickness. */
export function floorYBase(floorLevel: number, wallHeight: number): number {
  return floorLevel * (wallHeight + FLOOR_THICKNESS)
}

// Wall direction descriptors
export interface WallDirInfo {
  isNS: boolean
  isFront: boolean
}

export const WALL_DIR_INFO: Record<WallDirection, WallDirInfo> = {
  north: { isNS: true, isFront: false },
  south: { isNS: true, isFront: true },
  east: { isNS: false, isFront: false },
  west: { isNS: false, isFront: true },
}

export interface DoorMeshInfo {
  /** Hinge pivot group (rotate .rotation.y to open/close) */
  pivot: THREE.Group
  roomIndex: number
  wallDir: WallDirection
  segmentIndex: number
  floorLevel: number
  isOpen: boolean
  /** rotation.y when closed */
  closedAngle: number
  /** rotation.y when open */
  openAngle: number
}

export interface HouseGroupResult {
  houseGroup: THREE.Group
  /** Per-floor groups: key = floorLevel, value = { front, back, floor, stair } */
  floorGroups: Map<
    number,
    {
      front: THREE.Group
      back: THREE.Group
      floor: THREE.Group
      stair: THREE.Group
    }
  >
  aabb: THREE.Box3
  /** Per-room AABBs for concave-aware spatial tests (L/T/U shapes). */
  roomAABBs: THREE.Box3[]
  /** JSON hash of rooms for change detection */
  roomsHash: string
  /** Number of merged meshes (for profiling). */
  mergedMeshCount: number
  /** Door panel meshes with hinge pivots for animation */
  doors: DoorMeshInfo[]
}

export interface GeoEntry {
  geo: THREE.BufferGeometry
  textureIndex: number
}

export interface RoomFootprint {
  x: number
  z: number
  sx: number
  sz: number
  fl: number
}

export type FloorEntries = {
  front: GeoEntry[]
  back: GeoEntry[]
  floor: GeoEntry[]
  stair: GeoEntry[]
  doors: DoorMeshInfo[]
}

const _tmpMatrix = new THREE.Matrix4()

/**
 * Create geometry with baked position and tiled UVs for a single piece.
 */
export function bakedGeo(
  baseGeo: THREE.BufferGeometry,
  px: number,
  py: number,
  pz: number,
  rotY: number = 0,
  uvScaleX: number = 1,
  uvScaleY: number = 1,
  uvOffsetX: number = 0,
  uvOffsetY: number = 0
): THREE.BufferGeometry {
  if (rotY !== 0) {
    _tmpMatrix.makeRotationY(rotY)
    _tmpMatrix.setPosition(px, py, pz)
  } else {
    _tmpMatrix.makeTranslation(px, py, pz)
  }
  baseGeo.applyMatrix4(_tmpMatrix)

  const uv = baseGeo.getAttribute('uv')
  if (uv) {
    for (let i = 0; i < uv.count; i++) {
      uv.setXY(
        i,
        uv.getX(i) * uvScaleX + uvOffsetX,
        uv.getY(i) * uvScaleY + uvOffsetY
      )
    }
  }

  return baseGeo
}

/** Group entries by texture index, merge geometries per group, create meshes. Returns mesh count. */
export function addMergedMeshes(
  group: THREE.Group,
  entries: GeoEntry[]
): number {
  if (entries.length === 0) return 0

  const byTex = new Map<number, THREE.BufferGeometry[]>()
  for (const e of entries) {
    const list = byTex.get(e.textureIndex)
    if (list) {
      list.push(e.geo)
    } else {
      byTex.set(e.textureIndex, [e.geo])
    }
  }

  let count = 0
  for (const [texIdx, geos] of byTex) {
    const merged = mergeGeometries(geos, false)
    for (const g of geos) g.dispose()
    if (merged) {
      const mesh = new THREE.Mesh(merged, getHousingMaterial(texIdx))
      mesh.castShadow = true
      mesh.receiveShadow = true
      // Record the source texture index so any caller can look up a matching
      // material variant for this mesh (e.g. a ghost material for fading).
      mesh.userData.textureIndex = texIdx
      group.add(mesh)
      count++
    }
  }
  return count
}

export function collectFootprints(
  rooms: RoomData[],
  predicate: (room: RoomData) => boolean
): RoomFootprint[] {
  const result: RoomFootprint[] = []
  for (const room of rooms) {
    if (predicate(room)) {
      result.push({
        x: room.localX,
        z: room.localZ,
        sx: room.sizeX,
        sz: room.sizeZ,
        fl: room.floorLevel,
      })
    }
  }
  return result
}

export function cellInFootprint(
  cx: number,
  cz: number,
  fp: RoomFootprint
): boolean {
  return cx >= fp.x && cx < fp.x + fp.sx && cz >= fp.z && cz < fp.z + fp.sz
}

export function getOrCreateFloorEntries(
  perFloor: Map<number, FloorEntries>,
  fl: number
): FloorEntries {
  let entries = perFloor.get(fl)
  if (!entries) {
    entries = { front: [], back: [], floor: [], stair: [], doors: [] }
    perFloor.set(fl, entries)
  }
  return entries
}

export function computeHouseAABB(house: {
  origin: { x: number; y: number; z: number }
  rooms: RoomData[]
}): THREE.Box3 {
  const merged = new THREE.Box3()
  for (const box of computeRoomAABBs(house)) merged.union(box)
  return merged
}

export function computeRoomAABBs(
  house: {
    origin: { x: number; y: number; z: number }
    rooms: RoomData[]
  },
  spanByRoom = roofSpanByRoom(house.rooms)
): THREE.Box3[] {
  return house.rooms.map((room) => {
    const yBase = floorYBase(room.floorLevel, room.wallHeight)
    const minX = house.origin.x + room.localX
    const minZ = house.origin.z + room.localZ
    let maxY = room.wallHeight
    let roofOh = 0
    const span = spanByRoom.get(room)
    if (span) {
      maxY += span.ridgeHeight
      roofOh = ROOF_OVERHANG
    }
    const oh = Math.max(roofOh, floorOverhang(room.floorLevel))
    return new THREE.Box3(
      new THREE.Vector3(minX - oh, house.origin.y + yBase, minZ - oh),
      new THREE.Vector3(
        minX + room.sizeX + oh,
        house.origin.y + yBase + maxY,
        minZ + room.sizeZ + oh
      )
    )
  })
}

/** Compute gabled roof dimensions from room data. */
export function gabledRoofDims(room: RoomData) {
  const dir = room.roofRidgeDir ?? 'auto'
  const ridgeAlongX =
    dir === 'x' ? true : dir === 'z' ? false : room.sizeX >= room.sizeZ
  const shortDim = ridgeAlongX ? room.sizeZ : room.sizeX
  const ridgeHeight = (shortDim / 2) * ROOF_PITCH[room.roofType!]
  return { ridgeAlongX, shortDim, ridgeHeight }
}

/** Max rooms a single gabled roof may bridge across its ridge; longer chains split into an M-shape. */
export const MAX_ROOF_SPAN_ROOMS = 3

export interface RoofSpan {
  rooms: RoomData[]
  localX: number
  localZ: number
  sizeX: number
  sizeZ: number
  ridgeAlongX: boolean
  ridgeHeight: number
  /** Across-ridge sides that meet another span (valley, no eave) */
  innerLow: boolean
  innerHigh: boolean
}

interface RoofRect {
  rooms: RoomData[]
  a0: number
  aLen: number
  b0: number
  bLen: number
}

/**
 * Merge adjacent gabled rooms with parallel ridges into roof spans:
 * along the ridge when their across extents match, then across the ridge
 * when their along extents match. Across chains longer than
 * MAX_ROOF_SPAN_ROOMS split into evenly sized chunks.
 */
export function roofSpanByRoom(
  rooms: RoomData[],
  skip?: (room: RoomData) => boolean
): Map<RoomData, RoofSpan> {
  const map = new Map<RoomData, RoofSpan>()
  for (const span of computeRoofSpans(rooms))
    for (const r of span.rooms) if (!skip?.(r)) map.set(r, span)
  return map
}

export function computeRoofSpans(
  rooms: RoomData[],
  skip?: (room: RoomData) => boolean
): RoofSpan[] {
  const groups = new Map<string, { ridgeAlongX: boolean; rects: RoofRect[] }>()
  for (const room of rooms) {
    if (!room.roofType || room.roofType === 'flat') continue
    if (room.roomType === 'stairwell') continue
    if (skip?.(room)) continue
    const { ridgeAlongX } = gabledRoofDims(room)
    const key = `${room.floorLevel}|${room.roofType}|${room.wallHeight}|${ridgeAlongX}`
    let g = groups.get(key)
    if (!g) groups.set(key, (g = { ridgeAlongX, rects: [] }))
    g.rects.push({
      rooms: [room],
      a0: ridgeAlongX ? room.localX : room.localZ,
      aLen: ridgeAlongX ? room.sizeX : room.sizeZ,
      b0: ridgeAlongX ? room.localZ : room.localX,
      bLen: ridgeAlongX ? room.sizeZ : room.sizeX,
    })
  }

  const spans: RoofSpan[] = []
  for (const { ridgeAlongX, rects } of groups.values()) {
    rects.sort((p, q) => p.b0 - q.b0 || p.bLen - q.bLen || p.a0 - q.a0)
    const alongMerged: RoofRect[] = []
    for (const r of rects) {
      const prev = alongMerged[alongMerged.length - 1]
      if (
        prev &&
        prev.b0 === r.b0 &&
        prev.bLen === r.bLen &&
        prev.a0 + prev.aLen === r.a0
      ) {
        prev.aLen += r.aLen
        prev.rooms.push(...r.rooms)
      } else alongMerged.push(r)
    }

    alongMerged.sort((p, q) => p.a0 - q.a0 || p.aLen - q.aLen || p.b0 - q.b0)
    const chains: RoofRect[][] = []
    for (const r of alongMerged) {
      const chain = chains[chains.length - 1]
      const prev = chain?.[chain.length - 1]
      if (
        prev &&
        prev.a0 === r.a0 &&
        prev.aLen === r.aLen &&
        prev.b0 + prev.bLen === r.b0
      )
        chain.push(r)
      else chains.push([r])
    }

    for (const chain of chains) {
      const n = chain.length
      const k = Math.ceil(n / MAX_ROOF_SPAN_ROOMS)
      const base = Math.floor(n / k)
      const extra = n % k
      let i = 0
      for (let c = 0; c < k; c++) {
        const cnt = base + (c < extra ? 1 : 0)
        const chunk = chain.slice(i, i + cnt)
        i += cnt
        const a0 = chunk[0].a0
        const aLen = chunk[0].aLen
        const b0 = chunk[0].b0
        const bLen = chunk.reduce((acc, r) => acc + r.bLen, 0)
        const pitch = ROOF_PITCH[chunk[0].rooms[0].roofType!]
        spans.push({
          rooms: chunk.flatMap((r) => r.rooms),
          localX: ridgeAlongX ? a0 : b0,
          localZ: ridgeAlongX ? b0 : a0,
          sizeX: ridgeAlongX ? aLen : bLen,
          sizeZ: ridgeAlongX ? bLen : aLen,
          ridgeAlongX,
          ridgeHeight: (bLen / 2) * pitch,
          innerLow: c > 0,
          innerHigh: c < k - 1,
        })
      }
    }
  }
  return spans
}
