import type {
  HouseData,
  PassabilityGrid,
  RoomData,
  WallConfig,
  WallVariant,
} from '../types/housing'
import { floorYBase, type WallDirection } from '../utils/house-geometry'
import { WALL_DIR_INFO } from '../utils/house-geo-utils'

export function getWallByDir(room: RoomData, dir: WallDirection): WallConfig[] {
  switch (dir) {
    case 'north':
      return room.wallNorth
    case 'south':
      return room.wallSouth
    case 'east':
      return room.wallEast
    case 'west':
      return room.wallWest
  }
}

// Cell edge bitmask constants
export const EDGE_N = 1 // -Z edge (north wall)
export const EDGE_E = 2 // +X edge (east wall)
export const EDGE_S = 4 // +Z edge (south wall)
export const EDGE_W = 8 // -X edge (west wall)

export const ALL_WALL_DIRS: WallDirection[] = ['north', 'south', 'east', 'west']

/** Partner of a double-door half: consecutive halves pair from the run start; a trailing odd one gets -1. */
export function doubleDoorPartner(segs: WallConfig[], i: number): number {
  if (segs[i]?.variant !== 'double-door') return -1
  let r = i
  while (r > 0 && segs[r - 1].variant === 'double-door') r--
  const partner = (i - r) % 2 === 0 ? i + 1 : i - 1
  return segs[partner]?.variant === 'double-door' ? partner : -1
}

export interface DoorRef {
  roomIndex: number
  segmentIndex: number
}

export function wallLineCoord(room: RoomData, dir: WallDirection): number {
  switch (dir) {
    case 'north':
      return room.localZ
    case 'south':
      return room.localZ + room.sizeZ
    case 'east':
      return room.localX + room.sizeX
    case 'west':
      return room.localX
  }
}

/** Segment of an adjacent same-floor room whose collinear wall touches this wall's end segment `i`. */
export function crossRoomWallNeighbour(
  rooms: RoomData[],
  roomIndex: number,
  dir: WallDirection,
  i: number
): DoorRef | null {
  const room = rooms[roomIndex]
  const segs = getWallByDir(room, dir)
  if (segs.length === 0 || (i !== 0 && i !== segs.length - 1)) return null
  const isNS = dir === 'north' || dir === 'south'
  const a0 = isNS ? room.localX : room.localZ
  const aLen = isNS ? room.sizeX : room.sizeZ
  const edge = i === 0 ? a0 : a0 + aLen
  const line = wallLineCoord(room, dir)
  for (let ri = 0; ri < rooms.length; ri++) {
    if (ri === roomIndex) continue
    const o = rooms[ri]
    if (o.floorLevel !== room.floorLevel || o.roomType === 'stairwell') continue
    if (wallLineCoord(o, dir) !== line) continue
    const oa0 = isNS ? o.localX : o.localZ
    const oaLen = isNS ? o.sizeX : o.sizeZ
    if ((i === 0 ? oa0 + oaLen : oa0) !== edge) continue
    const oSegs = getWallByDir(o, dir)
    if (oSegs.length === 0) continue
    return { roomIndex: ri, segmentIndex: i === 0 ? oSegs.length - 1 : 0 }
  }
  return null
}

function isLoneHalf(segs: WallConfig[], i: number): boolean {
  return segs[i]?.variant === 'double-door' && doubleDoorPartner(segs, i) < 0
}

/** Cross-room partner of a lone double-door half at a wall end, if the neighbour is one too. */
export function crossRoomDoorPartner(
  rooms: RoomData[],
  roomIndex: number,
  dir: WallDirection,
  i: number
): DoorRef | null {
  if (!isLoneHalf(getWallByDir(rooms[roomIndex], dir), i)) return null
  const n = crossRoomWallNeighbour(rooms, roomIndex, dir, i)
  if (!n) return null
  const oSegs = getWallByDir(rooms[n.roomIndex], dir)
  return isLoneHalf(oSegs, n.segmentIndex) ? n : null
}

export function oppositeDir(dir: WallDirection): WallDirection {
  return WALL_DIR_INFO[dir].opposite
}

/** The same-floor room segment on the far side of segment `i` (an interior
 *  wall shared with a neighbour), or null on an exterior face. */
export function facingSegmentRef(
  rooms: RoomData[],
  roomIndex: number,
  dir: WallDirection,
  i: number
): DoorRef | null {
  const room = rooms[roomIndex]
  if (room.roomType === 'stairwell') return null
  const isNS = WALL_DIR_INFO[dir].isNS
  const pos = (isNS ? room.localX : room.localZ) + i
  const line = wallLineCoord(room, dir)
  const back = oppositeDir(dir)
  for (let ri = 0; ri < rooms.length; ri++) {
    if (ri === roomIndex) continue
    const o = rooms[ri]
    if (o.floorLevel !== room.floorLevel || o.roomType === 'stairwell') continue
    if (wallLineCoord(o, back) !== line) continue
    const oa0 = isNS ? o.localX : o.localZ
    const oaLen = isNS ? o.sizeX : o.sizeZ
    if (pos < oa0 || pos >= oa0 + oaLen) continue
    return { roomIndex: ri, segmentIndex: pos - oa0 }
  }
  return null
}

/** The other half of a double door (same wall, else adjacent room), or null. */
export function doorPartnerRef(
  rooms: RoomData[],
  roomIndex: number,
  dir: WallDirection,
  i: number
): DoorRef | null {
  const p = doubleDoorPartner(getWallByDir(rooms[roomIndex], dir), i)
  if (p >= 0) return { roomIndex, segmentIndex: p }
  return crossRoomDoorPartner(rooms, roomIndex, dir, i)
}

/** Reset the partner half (if any) of segment `i` to solid. */
export function clearDoorPair(
  rooms: RoomData[],
  roomIndex: number,
  dir: WallDirection,
  i: number
) {
  const p = doorPartnerRef(rooms, roomIndex, dir, i)
  if (!p) return
  const segs = getWallByDir(rooms[p.roomIndex], dir)
  segs[p.segmentIndex] = { ...segs[p.segmentIndex], variant: 'solid' }
}

/**
 * Mark a partner half for a new double door at segment `i`: across the room
 * boundary when at a wall end, else the next free segment, else the previous.
 * Returns false when no partner is available.
 */
export function pairDoubleDoor(
  rooms: RoomData[],
  roomIndex: number,
  dir: WallDirection,
  i: number
): boolean {
  const segs = getWallByDir(rooms[roomIndex], dir)
  const free = (s: WallConfig[], j: number) =>
    s[j] !== undefined &&
    s[j].variant !== 'open' &&
    s[j].variant !== 'double-door'
  const n = crossRoomWallNeighbour(rooms, roomIndex, dir, i)
  const target =
    n && free(getWallByDir(rooms[n.roomIndex], dir), n.segmentIndex)
      ? n
      : free(segs, i + 1)
        ? { roomIndex, segmentIndex: i + 1 }
        : free(segs, i - 1)
          ? { roomIndex, segmentIndex: i - 1 }
          : null
  if (!target) return false
  const t = getWallByDir(rooms[target.roomIndex], dir)
  t[target.segmentIndex] = { ...t[target.segmentIndex], variant: 'double-door' }
  return true
}

export function isDoorVariant(v: WallVariant): boolean {
  return v === 'door' || v === 'double-door'
}

/** Segments players can toggle (doors and window shutters). */
export function isOpenable(v: WallVariant): boolean {
  return isDoorVariant(v) || v === 'window'
}

function isWallBlocking(seg: WallConfig): boolean {
  return seg.variant !== 'open'
}

/**
 * Build passability grids for a house. Stores static structure (all doors treated as blocked).
 * Returns one grid per floor level (including stairwell entries on both floors).
 */
export function buildPassability(house: HouseData): PassabilityGrid[] {
  // Group rooms by floor level, collecting bounding boxes
  const floorMap = new Map<
    number,
    { minX: number; minZ: number; maxX: number; maxZ: number }
  >()

  for (const room of house.rooms) {
    const rx = room.localX
    const rz = room.localZ
    const levels =
      room.roomType === 'stairwell'
        ? [room.floorLevel, room.floorLevel + 1] // stairwell registers on both its floor and the floor above
        : [room.floorLevel]

    for (const fl of levels) {
      const existing = floorMap.get(fl)
      if (existing) {
        existing.minX = Math.min(existing.minX, rx)
        existing.minZ = Math.min(existing.minZ, rz)
        existing.maxX = Math.max(existing.maxX, rx + room.sizeX)
        existing.maxZ = Math.max(existing.maxZ, rz + room.sizeZ)
      } else {
        floorMap.set(fl, {
          minX: rx,
          minZ: rz,
          maxX: rx + room.sizeX,
          maxZ: rz + room.sizeZ,
        })
      }
    }
  }

  const grids: PassabilityGrid[] = []

  for (const [floorLevel, bounds] of floorMap) {
    const originX = bounds.minX
    const originZ = bounds.minZ
    const width = bounds.maxX - bounds.minX
    const depth = bounds.maxZ - bounds.minZ
    const cells = new Array<number>(width * depth).fill(0)

    const setEdge = (cx: number, cz: number, edge: number) => {
      const gx = cx - originX
      const gz = cz - originZ
      if (gx >= 0 && gx < width && gz >= 0 && gz < depth) {
        cells[gx + gz * width] |= edge
      }
    }

    for (const room of house.rooms) {
      const rx = room.localX
      const rz = room.localZ

      if (room.roomType === 'stairwell') {
        if (
          floorLevel === room.floorLevel ||
          floorLevel === room.floorLevel + 1
        ) {
          const blockExitEnd =
            floorLevel === room.floorLevel &&
            !hasOverlappingStairwell(room, house.rooms, 'exit')
          const blockEntryEnd =
            floorLevel === room.floorLevel + 1 &&
            !hasOverlappingStairwell(room, house.rooms, 'entry')
          buildStairwellEdges(
            room,
            rx,
            rz,
            floorLevel,
            setEdge,
            blockExitEnd,
            blockEntryEnd
          )
        }
        continue
      }

      if (room.floorLevel !== floorLevel) continue

      for (let i = 0; i < room.sizeX; i++) {
        if (i < room.wallNorth.length && isWallBlocking(room.wallNorth[i])) {
          setEdge(rx + i, rz, EDGE_N)
          setEdge(rx + i, rz - 1, EDGE_S)
        }
      }
      for (let i = 0; i < room.sizeX; i++) {
        if (i < room.wallSouth.length && isWallBlocking(room.wallSouth[i])) {
          setEdge(rx + i, rz + room.sizeZ - 1, EDGE_S)
          setEdge(rx + i, rz + room.sizeZ, EDGE_N)
        }
      }
      for (let i = 0; i < room.sizeZ; i++) {
        if (i < room.wallWest.length && isWallBlocking(room.wallWest[i])) {
          setEdge(rx, rz + i, EDGE_W)
          setEdge(rx - 1, rz + i, EDGE_E)
        }
      }
      for (let i = 0; i < room.sizeZ; i++) {
        if (i < room.wallEast.length && isWallBlocking(room.wallEast[i])) {
          setEdge(rx + room.sizeX - 1, rz + i, EDGE_E)
          setEdge(rx + room.sizeX, rz + i, EDGE_W)
        }
      }
    }

    grids.push({ floorLevel, originX, originZ, width, depth, cells })
  }

  return grids
}

/**
 * Build passability edges for a stairwell room on a specific floor level.
 *
 * Both ends along the stair axis are always open (no end walls).
 * Only side walls on the stair-run rows are blocked, skipping the
 * landing row for this floor:
 * - Entry floor: skip row 0 (entry landing)
 * - Exit floor: skip last row (exit landing)
 */
function buildStairwellEdges(
  room: RoomData,
  rx: number,
  rz: number,
  floorLevel: number,
  setEdge: (cx: number, cz: number, edge: number) => void,
  blockExitEnd: boolean,
  blockEntryEnd: boolean
) {
  const alongZ = room.sizeZ >= room.sizeX
  const alongSize = alongZ ? room.sizeZ : room.sizeX
  const reversed = room.stairReversed ?? false

  // Skip the landing row for this floor's open end
  const isEntryFloor = floorLevel === room.floorLevel
  const isExitFloor = floorLevel === room.floorLevel + 1
  // Skip landing row when it connects to an adjacent floor (open end)
  const skipEntryLanding = isEntryFloor || (isExitFloor && !blockEntryEnd)
  const skipExitLanding = isExitFloor || (isEntryFloor && !blockExitEnd)

  // When reversed, entry/exit physical positions swap (first row ↔ last row)
  const skipFirstRow = reversed ? skipExitLanding : skipEntryLanding
  const skipLastRow = reversed ? skipEntryLanding : skipExitLanding
  const blockFirstRow = reversed ? blockExitEnd : blockEntryEnd
  const blockLastRow = reversed ? blockEntryEnd : blockExitEnd

  const sideStart = skipFirstRow ? 1 : 0
  const sideEnd = skipLastRow ? alongSize - 1 : alongSize

  if (alongZ) {
    for (let i = sideStart; i < sideEnd; i++) {
      setEdge(rx, rz + i, EDGE_W)
      setEdge(rx - 1, rz + i, EDGE_E)
      setEdge(rx + room.sizeX - 1, rz + i, EDGE_E)
      setEdge(rx + room.sizeX, rz + i, EDGE_W)
    }
    if (blockLastRow) {
      for (let x = 0; x < room.sizeX; x++) {
        setEdge(rx + x, rz + room.sizeZ - 1, EDGE_S)
        setEdge(rx + x, rz + room.sizeZ, EDGE_N)
      }
    }
    if (blockFirstRow) {
      for (let x = 0; x < room.sizeX; x++) {
        setEdge(rx + x, rz, EDGE_N)
        setEdge(rx + x, rz - 1, EDGE_S)
      }
    }
  } else {
    for (let i = sideStart; i < sideEnd; i++) {
      setEdge(rx + i, rz, EDGE_N)
      setEdge(rx + i, rz - 1, EDGE_S)
      setEdge(rx + i, rz + room.sizeZ - 1, EDGE_S)
      setEdge(rx + i, rz + room.sizeZ, EDGE_N)
    }
    if (blockLastRow) {
      for (let z = 0; z < room.sizeZ; z++) {
        setEdge(rx + room.sizeX - 1, rz + z, EDGE_E)
        setEdge(rx + room.sizeX, rz + z, EDGE_W)
      }
    }
    if (blockFirstRow) {
      for (let z = 0; z < room.sizeZ; z++) {
        setEdge(rx, rz + z, EDGE_W)
        setEdge(rx - 1, rz + z, EDGE_E)
      }
    }
  }
}

/**
 * Check if a stairwell landing overlaps with any stairwell on an adjacent floor.
 * 'exit' checks exit landing vs floor below; 'entry' checks entry landing vs floor above.
 */
function hasOverlappingStairwell(
  stairwell: RoomData,
  rooms: RoomData[],
  end: 'entry' | 'exit'
): boolean {
  const alongZ = stairwell.sizeZ >= stairwell.sizeX
  const reversed = stairwell.stairReversed ?? false
  const rx = stairwell.localX
  const rz = stairwell.localZ

  // Landing bounding box: entry = first row, exit = last row along stair axis
  // When reversed, physical positions swap
  const physicalEnd = reversed ? (end === 'exit' ? 'entry' : 'exit') : end
  let minX: number, maxX: number, minZ: number, maxZ: number
  if (physicalEnd === 'exit') {
    if (alongZ) {
      minX = rx
      maxX = rx + stairwell.sizeX
      minZ = rz + stairwell.sizeZ - 1
      maxZ = rz + stairwell.sizeZ
    } else {
      minX = rx + stairwell.sizeX - 1
      maxX = rx + stairwell.sizeX
      minZ = rz
      maxZ = rz + stairwell.sizeZ
    }
  } else {
    if (alongZ) {
      minX = rx
      maxX = rx + stairwell.sizeX
      minZ = rz
      maxZ = rz + 1
    } else {
      minX = rx
      maxX = rx + 1
      minZ = rz
      maxZ = rz + stairwell.sizeZ
    }
  }

  const targetFloor =
    end === 'exit' ? stairwell.floorLevel - 1 : stairwell.floorLevel + 1

  for (const other of rooms) {
    if (other === stairwell) continue
    if (other.roomType !== 'stairwell') continue
    if (other.floorLevel !== targetFloor) continue

    if (
      minX < other.localX + other.sizeX &&
      maxX > other.localX &&
      minZ < other.localZ + other.sizeZ &&
      maxZ > other.localZ
    ) {
      return true
    }
  }
  return false
}

/** Runtime passability grid with Y-range info for floor matching */
interface RuntimeFloorGrid {
  floorLevel: number
  originX: number
  originZ: number
  width: number
  depth: number
  yBase: number
  wallHeight: number
  cells: number[]
}

export interface StairwellInfo {
  /** House-local cell bounds (integers, max exclusive) */
  localMinX: number
  localMinZ: number
  localMaxX: number
  localMaxZ: number
  lowerFloor: number
  upperFloor: number
  alongZ: boolean
  reversed: boolean
}

export interface RuntimePassability {
  houseOriginX: number
  houseOriginZ: number
  minX: number
  maxX: number
  minZ: number
  maxZ: number
  floors: RuntimeFloorGrid[]
  stairwells: StairwellInfo[]
}

/** Build runtime passability from stored grids (or compute if missing). */
export function buildRuntimePassability(house: HouseData): RuntimePassability {
  const grids = house.passability?.length
    ? house.passability
    : buildPassability(house)

  // Compute world-space AABB across all floors
  let minX = Infinity
  let maxX = -Infinity
  let minZ = Infinity
  let maxZ = -Infinity

  const floors: RuntimeFloorGrid[] = grids.map((g) => {
    const worldMinX = house.origin.x + g.originX
    const worldMinZ = house.origin.z + g.originZ
    const worldMaxX = worldMinX + g.width
    const worldMaxZ = worldMinZ + g.depth
    minX = Math.min(minX, worldMinX)
    maxX = Math.max(maxX, worldMaxX)
    minZ = Math.min(minZ, worldMinZ)
    maxZ = Math.max(maxZ, worldMaxZ)

    // Find wallHeight for this floor level from rooms
    let wallHeight = 3
    let yBase = house.origin.y
    for (const room of house.rooms) {
      if (room.floorLevel === g.floorLevel) {
        wallHeight = room.wallHeight
        yBase = house.origin.y + floorYBase(room.floorLevel, room.wallHeight)
        break
      }
      // For upper-floor grid derived from a stairwell
      if (
        room.roomType === 'stairwell' &&
        g.floorLevel === room.floorLevel + 1
      ) {
        wallHeight = room.wallHeight
        yBase = house.origin.y + floorYBase(g.floorLevel, room.wallHeight)
        break
      }
    }

    return {
      floorLevel: g.floorLevel,
      originX: g.originX,
      originZ: g.originZ,
      width: g.width,
      depth: g.depth,
      yBase,
      wallHeight,
      cells: g.cells,
    }
  })

  const stairwells: StairwellInfo[] = []
  for (const room of house.rooms) {
    if (room.roomType === 'stairwell') {
      stairwells.push({
        localMinX: room.localX,
        localMinZ: room.localZ,
        localMaxX: room.localX + room.sizeX,
        localMaxZ: room.localZ + room.sizeZ,
        lowerFloor: room.floorLevel,
        upperFloor: room.floorLevel + 1,
        alongZ: room.sizeZ >= room.sizeX,
        reversed: room.stairReversed ?? false,
      })
    }
  }

  return {
    houseOriginX: house.origin.x,
    houseOriginZ: house.origin.z,
    minX,
    maxX,
    minZ,
    maxZ,
    floors,
    stairwells,
  }
}

/** Update passability edge bits when a door is opened or closed. */
export function updateDoorEdge(
  passabilityCache: ReadonlyMap<string, RuntimePassability>,
  houseId: string,
  room: RoomData,
  wallDir: WallDirection,
  segmentIndex: number,
  isOpen: boolean
) {
  const rp = passabilityCache.get(houseId)
  if (!rp) return

  const floor = rp.floors.find((f) => f.floorLevel === room.floorLevel)
  if (!floor) return

  const rx = room.localX - floor.originX
  const rz = room.localZ - floor.originZ

  let cx: number,
    cz: number,
    edge: number,
    adjCx: number,
    adjCz: number,
    adjEdge: number
  switch (wallDir) {
    case 'north': {
      cx = rx + segmentIndex
      cz = rz
      edge = EDGE_N
      adjCx = cx
      adjCz = cz - 1
      adjEdge = EDGE_S
      break
    }
    case 'south': {
      cx = rx + segmentIndex
      cz = rz + room.sizeZ - 1
      edge = EDGE_S
      adjCx = cx
      adjCz = cz + 1
      adjEdge = EDGE_N
      break
    }
    case 'west': {
      cx = rx
      cz = rz + segmentIndex
      edge = EDGE_W
      adjCx = cx - 1
      adjCz = cz
      adjEdge = EDGE_E
      break
    }
    case 'east': {
      cx = rx + room.sizeX - 1
      cz = rz + segmentIndex
      edge = EDGE_E
      adjCx = cx + 1
      adjCz = cz
      adjEdge = EDGE_W
      break
    }
  }

  const setOrClear = (gx: number, gz: number, bit: number) => {
    if (gx < 0 || gx >= floor.width || gz < 0 || gz >= floor.depth) return
    const idx = gx + gz * floor.width
    if (isOpen) {
      floor.cells[idx] &= ~bit
    } else {
      floor.cells[idx] |= bit
    }
  }

  setOrClear(cx, cz, edge)
  setOrClear(adjCx, adjCz, adjEdge)
}
