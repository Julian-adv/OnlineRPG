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
// Keep in sync with shared housing::FLOOR_OVERHANG_PER_LEVEL.
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
  opposite: WallDirection
}

export const WALL_DIR_INFO: Record<WallDirection, WallDirInfo> = {
  north: { isNS: true, isFront: false, opposite: 'south' },
  south: { isNS: true, isFront: true, opposite: 'north' },
  east: { isNS: false, isFront: false, opposite: 'west' },
  west: { isNS: false, isFront: true, opposite: 'east' },
}

export interface DoorMeshInfo {
  /** Hinge pivot group (rotate .rotation.y to open/close) */
  pivot: THREE.Group
  /** Fixed window opening target that remains clickable while shutters move. */
  clickTarget?: THREE.Object3D
  roomIndex: number
  wallDir: WallDirection
  segmentIndex: number
  floorLevel: number
  isOpen: boolean
  /** rotation.y when closed */
  closedAngle: number
  /** rotation.y when open */
  openAngle: number
  /** Set for doors in walls shared by two rooms */
  interior?: InteriorWall
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
      /** One group per shared wall line, ghosted when it hides the player */
      interior: InteriorWallGroup[]
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
  /** Timber trim (pillars, beams, braces) — hidden when the wall is ghosted */
  decor?: boolean
}

export interface RoomFootprint {
  x: number
  z: number
  sx: number
  sz: number
  fl: number
}

/** A wall line shared by two rooms, with the 1m spans it actually covers. */
export interface InteriorWall {
  isNS: boolean
  /** Room-local wall line coordinate (z for N/S walls, x for E/W) */
  line: number
  height: number
  /** [start, end) along the wall in room-local coordinates */
  spans: [number, number][]
}

export interface InteriorWallEntries {
  wall: InteriorWall
  entries: GeoEntry[]
}

export interface InteriorWallGroup {
  wall: InteriorWall
  group: THREE.Group
  ghost: boolean
}

export type FloorEntries = {
  front: GeoEntry[]
  back: GeoEntry[]
  floor: GeoEntry[]
  stair: GeoEntry[]
  doors: DoorMeshInfo[]
  interior: Map<number, InteriorWallEntries>
}

/** Bucket for a shared wall line, registering the segment [a0, a1) on it. */
export function interiorWall(
  entries: FloorEntries,
  isNS: boolean,
  line: number,
  height: number,
  a0: number,
  a1: number
): InteriorWallEntries {
  const key = line * 2 + (isNS ? 1 : 0)
  let bucket = entries.interior.get(key)
  if (!bucket) {
    bucket = { wall: { isNS, line, height, spans: [] }, entries: [] }
    entries.interior.set(key, bucket)
  }
  bucket.wall.height = Math.max(bucket.wall.height, height)
  bucket.wall.spans.push([a0, a1])
  return bucket
}

/** True when `wall` stands between the isometric SW camera and a player at
 *  room-local (px, pz): the view ray toward the camera climbs 1m per 1m of
 *  north→south (or east→west) travel, so the wall hides the player only
 *  within `height` metres, and only where a span covers the crossing. */
export function interiorWallOccludes(
  wall: InteriorWall,
  px: number,
  pz: number
): boolean {
  const d = wall.isNS ? wall.line - pz : px - wall.line
  if (d <= 0 || d >= wall.height) return false
  const a = wall.isNS ? px - d : pz + d
  const margin = 0.6
  return wall.spans.some(([s0, s1]) => a >= s0 - margin && a <= s1 + margin)
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
    const key = e.textureIndex * 2 + (e.decor ? 1 : 0)
    const list = byTex.get(key)
    if (list) {
      list.push(e.geo)
    } else {
      byTex.set(key, [e.geo])
    }
  }

  let count = 0
  for (const [key, geos] of byTex) {
    const texIdx = key >> 1
    const decor = (key & 1) === 1
    const merged = mergeGeometries(geos, false)
    for (const g of geos) g.dispose()
    if (merged) {
      const mesh = new THREE.Mesh(merged, getHousingMaterial(texIdx))
      mesh.castShadow = true
      mesh.receiveShadow = true
      // Record the source texture index so any caller can look up a matching
      // material variant for this mesh (e.g. a ghost material for fading).
      mesh.userData.textureIndex = texIdx
      mesh.userData.decor = decor
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
    entries = {
      front: [],
      back: [],
      floor: [],
      stair: [],
      doors: [],
      interior: new Map(),
    }
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

/** Deepest across-ridge run one gabled roof may cover; longer footprints
 *  split into an M-shape. */
const MAX_ROOF_SPAN_DEPTH = 12
/** How far a split may move to land on a room boundary. */
const SPLIT_SNAP = 3
/** Auto-ridge rooms narrower than this (corridors) follow a neighbour's ridge
 *  instead of getting their own roof. */
const CORRIDOR_MAX_WIDTH = 2

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

/** Rectangle in ridge space: `a` runs along the ridge, `b` across it. */
interface RoofRect {
  rooms: Set<RoomData>
  a0: number
  aLen: number
  b0: number
  bLen: number
}

/** A room's extent in ridge space. */
function ridgeExtent(r: RoomData, ridgeAlongX: boolean) {
  return ridgeAlongX
    ? { a0: r.localX, aLen: r.sizeX, b0: r.localZ, bLen: r.sizeZ }
    : { a0: r.localZ, aLen: r.sizeZ, b0: r.localX, bLen: r.sizeX }
}

const autoRidgeAlongX = (r: RoomData) => gabledRoofDims(r).ridgeAlongX
const isWide = (r: RoomData) => Math.min(r.sizeX, r.sizeZ) > CORRIDOR_MAX_WIDTH

/**
 * Roof spans keyed by room, skipping rooms whose roof is suppressed. Spans
 * are still shaped by every room so a covered room keeps its neighbours' roof
 * intact; only its own entry (and emission) drops out.
 */
export function roofSpanByRoom(
  rooms: RoomData[],
  skip?: (room: RoomData) => boolean
): Map<RoomData, RoofSpan> {
  const map = new Map<RoomData, RoofSpan>()
  for (const span of computeRoofSpans(rooms)) {
    const kept = skip ? span.rooms.filter((r) => !skip(r)) : span.rooms
    if (kept.length === 0) continue
    span.rooms = kept
    for (const r of kept) map.set(r, span)
  }
  return map
}

function sharedEdgeLength(a: RoomData, b: RoomData): number {
  const overlap = (s0: number, l0: number, s1: number, l1: number) =>
    Math.max(0, Math.min(s0 + l0, s1 + l1) - Math.max(s0, s1))
  if (a.localX + a.sizeX === b.localX || b.localX + b.sizeX === a.localX)
    return overlap(a.localZ, a.sizeZ, b.localZ, b.sizeZ)
  if (a.localZ + a.sizeZ === b.localZ || b.localZ + b.sizeZ === a.localZ)
    return overlap(a.localX, a.sizeX, b.localX, b.sizeX)
  return 0
}

/** Same-floor rooms linked by shared edges. */
function roomComponents(rooms: RoomData[]): RoomData[][] {
  const parent = rooms.map((_, i) => i)
  const find = (i: number): number =>
    parent[i] === i ? i : (parent[i] = find(parent[i]))
  for (let i = 0; i < rooms.length; i++)
    for (let j = i + 1; j < rooms.length; j++)
      if (
        rooms[i].floorLevel === rooms[j].floorLevel &&
        sharedEdgeLength(rooms[i], rooms[j]) > 0
      )
        parent[find(i)] = find(j)
  const groups = new Map<number, RoomData[]>()
  rooms.forEach((r, i) => {
    const root = find(i)
    let g = groups.get(root)
    if (!g) groups.set(root, (g = []))
    g.push(r)
  })
  return [...groups.values()]
}

/** Direction the wide rooms of a rectangular block would mostly pick alone,
 *  weighted by area; null when the block is not one filled rectangle. */
function rectangularVote(comp: RoomData[]): boolean | null {
  let minX = Infinity
  let minZ = Infinity
  let maxX = -Infinity
  let maxZ = -Infinity
  let area = 0
  let voteX = 0
  for (const r of comp) {
    minX = Math.min(minX, r.localX)
    minZ = Math.min(minZ, r.localZ)
    maxX = Math.max(maxX, r.localX + r.sizeX)
    maxZ = Math.max(maxZ, r.localZ + r.sizeZ)
    const a = r.sizeX * r.sizeZ
    area += a
    if (isWide(r)) voteX += autoRidgeAlongX(r) ? a : -a
  }
  if (area !== (maxX - minX) * (maxZ - minZ)) return null
  return voteX !== 0 ? voteX > 0 : maxX - minX >= maxZ - minZ
}

/** Give each undecided room the direction of the decided neighbour sharing
 *  its longest edge, repeating until nothing changes. */
function inheritFromNeighbours(comp: RoomData[], dirs: Map<RoomData, boolean>) {
  for (let progress = true; progress; ) {
    progress = false
    for (const r of comp) {
      if (dirs.has(r)) continue
      let best: RoomData | undefined
      let bestLen = 0
      for (const o of comp) {
        if (o === r || !dirs.has(o)) continue
        const len = sharedEdgeLength(r, o)
        if (len > bestLen) {
          bestLen = len
          best = o
        }
      }
      if (best) {
        dirs.set(r, dirs.get(best)!)
        progress = true
      }
    }
  }
}

/** Ridge direction per room. Explicit wins. Rooms filling one rectangle
 *  share the direction most of their area would pick alone, so interior
 *  partitions roof like the single room they replace; other layouts keep
 *  each room's long axis, with corridors following their neighbours. */
function resolveRidgeDirs(rooms: RoomData[]): Map<RoomData, boolean> {
  const dirs = new Map<RoomData, boolean>()
  const isAuto = (r: RoomData) => (r.roofRidgeDir ?? 'auto') === 'auto'
  for (const r of rooms) if (!isAuto(r)) dirs.set(r, autoRidgeAlongX(r))

  for (const comp of roomComponents(rooms.filter(isAuto))) {
    const vote = rectangularVote(comp)
    if (vote !== null) {
      for (const r of comp) dirs.set(r, vote)
      continue
    }
    for (const r of comp) if (isWide(r)) dirs.set(r, autoRidgeAlongX(r))
    inheritFromNeighbours(comp, dirs)
    for (const r of comp) if (!dirs.has(r)) dirs.set(r, autoRidgeAlongX(r))
  }
  return dirs
}

/** Rasterize rooms to 1m cells and rebuild maximal rectangles: runs along
 *  the ridge per across-row, then equal runs merged across. Interior room
 *  boundaries vanish, so a partitioned floor roofs like a single room. */
function footprintRects(rooms: RoomData[], ridgeAlongX: boolean): RoofRect[] {
  const cells = new Map<number, Map<number, RoomData>>()
  for (const room of rooms) {
    const { a0, aLen, b0, bLen } = ridgeExtent(room, ridgeAlongX)
    for (let b = b0; b < b0 + bLen; b++) {
      let row = cells.get(b)
      if (!row) cells.set(b, (row = new Map()))
      for (let a = a0; a < a0 + aLen; a++) row.set(a, room)
    }
  }
  const strips: RoofRect[] = []
  for (const b of [...cells.keys()].sort((p, q) => p - q)) {
    const row = cells.get(b)!
    let strip: RoofRect | undefined
    for (const a of [...row.keys()].sort((p, q) => p - q)) {
      if (strip && strip.a0 + strip.aLen === a) strip.aLen++
      else
        strips.push(
          (strip = { rooms: new Set(), a0: a, aLen: 1, b0: b, bLen: 1 })
        )
      strip.rooms.add(row.get(a)!)
    }
  }
  strips.sort((p, q) => p.a0 - q.a0 || p.aLen - q.aLen || p.b0 - q.b0)
  const rects: RoofRect[] = []
  for (const s of strips) {
    const prev = rects[rects.length - 1]
    if (
      prev &&
      prev.a0 === s.a0 &&
      prev.aLen === s.aLen &&
      prev.b0 + prev.bLen === s.b0
    ) {
      prev.bLen++
      for (const r of s.rooms) prev.rooms.add(r)
    } else rects.push(s)
  }
  return rects
}

/** Across-ridge boundaries of a rect's chunks, at most MAX_ROOF_SPAN_DEPTH
 *  deep each, snapped to a nearby room boundary so rooms stay under one roof. */
function splitPoints(rect: RoofRect, ridgeAlongX: boolean): number[] {
  const k = Math.ceil(rect.bLen / MAX_ROOF_SPAN_DEPTH)
  const end = rect.b0 + rect.bLen
  const bounds = new Set<number>()
  for (const r of rect.rooms) {
    const { b0, bLen } = ridgeExtent(r, ridgeAlongX)
    bounds.add(b0)
    bounds.add(b0 + bLen)
  }
  const points = [rect.b0]
  for (let c = 1; c < k; c++) {
    const last = points[points.length - 1]
    const ideal = rect.b0 + Math.round((c * rect.bLen) / k)
    const near = [...bounds]
      .filter((b) => b > last && b < end)
      .sort((p, q) => Math.abs(p - ideal) - Math.abs(q - ideal))[0]
    const pick =
      near !== undefined && Math.abs(near - ideal) <= SPLIT_SNAP ? near : ideal
    if (pick > last) points.push(pick)
  }
  points.push(end)
  return points
}

export function computeRoofSpans(rooms: RoomData[]): RoofSpan[] {
  const eligible = rooms.filter(
    (r) => r.roofType && r.roofType !== 'flat' && r.roomType !== 'stairwell'
  )
  const dirs = resolveRidgeDirs(eligible)
  const groups = new Map<string, { ridgeAlongX: boolean; rooms: RoomData[] }>()
  for (const room of eligible) {
    const ridgeAlongX = dirs.get(room)!
    const key = `${room.floorLevel}|${room.roofType}|${room.wallHeight}|${ridgeAlongX}`
    let g = groups.get(key)
    if (!g) groups.set(key, (g = { ridgeAlongX, rooms: [] }))
    g.rooms.push(room)
  }

  const spans: RoofSpan[] = []
  for (const { ridgeAlongX, rooms: groupRooms } of groups.values()) {
    const pitch = ROOF_PITCH[groupRooms[0].roofType!]
    for (const rect of footprintRects(groupRooms, ridgeAlongX)) {
      const points = splitPoints(rect, ridgeAlongX)
      for (let c = 0; c + 1 < points.length; c++) {
        const b0 = points[c]
        const bLen = points[c + 1] - b0
        const chunkRooms = [...rect.rooms].filter((r) => {
          const e = ridgeExtent(r, ridgeAlongX)
          return e.b0 < b0 + bLen && e.b0 + e.bLen > b0
        })
        spans.push({
          rooms: chunkRooms,
          localX: ridgeAlongX ? rect.a0 : b0,
          localZ: ridgeAlongX ? b0 : rect.a0,
          sizeX: ridgeAlongX ? rect.aLen : bLen,
          sizeZ: ridgeAlongX ? bLen : rect.aLen,
          ridgeAlongX,
          ridgeHeight: (bLen / 2) * pitch,
          innerLow: c > 0,
          innerHigh: c + 2 < points.length,
        })
      }
    }
  }
  return spans
}
