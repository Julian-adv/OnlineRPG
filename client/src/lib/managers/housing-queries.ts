import type { HouseData } from '../types/housing'
import {
  FLOOR_THICKNESS,
  floorYBase,
  getStairwellYOffset,
  type WallDirection,
} from '../utils/house-geometry'
import { getWallByDir, isOpenable, wallLineCoord } from './housing-passability'

/** Find the first room containing a world point (fast, no allocation). */
export function findRoomAtPoint(
  housesById: ReadonlyMap<string, HouseData>,
  x: number,
  y: number,
  z: number
): { house: HouseData; roomIndex: number } | null {
  for (const house of housesById.values()) {
    for (let i = 0; i < house.rooms.length; i++) {
      const room = house.rooms[i]
      const rx = house.origin.x + room.localX
      const rz = house.origin.z + room.localZ
      const ryBase =
        house.origin.y + floorYBase(room.floorLevel, room.wallHeight)
      if (
        x >= rx &&
        x <= rx + room.sizeX &&
        z >= rz &&
        z <= rz + room.sizeZ &&
        y >= ryBase - 1 &&
        y <= ryBase + room.wallHeight + 1
      ) {
        return { house, roomIndex: i }
      }
    }
  }
  return null
}

/** Check if (x, z) falls inside any house room footprint, ignoring Y / floor. */
export function isPointUnderHouseXZ(
  housesById: ReadonlyMap<string, HouseData>,
  x: number,
  z: number
): boolean {
  for (const house of housesById.values()) {
    for (const room of house.rooms) {
      const rx = house.origin.x + room.localX
      const rz = house.origin.z + room.localZ
      if (x >= rx && x <= rx + room.sizeX && z >= rz && z <= rz + room.sizeZ) {
        return true
      }
    }
  }
  return false
}

/**
 * Ground Y on a given house floor at (x, z), stairwell ramps included — the
 * house analogue of `dungeonManager.floorHeightAt`, for entities whose floor
 * level is known but whose Y is not (remote players). Null when no room
 * covers the point on that floor.
 *
 * Inside a stairwell footprint the ramp always wins, even though the upper
 * floor's room overlaps it: that floor is punched through there
 * (`collectFloorGeometry`) and its passability seals everything but the top
 * landing, where the ramp height already equals the flat floor height.
 */
export function houseFloorHeightAt(
  housesById: ReadonlyMap<string, HouseData>,
  floorLevel: number,
  x: number,
  z: number
): number | null {
  let stairY: number | null = null
  let stairDist = Infinity
  let flatY: number | null = null

  for (const house of housesById.values()) {
    for (const room of house.rooms) {
      const rx = house.origin.x + room.localX
      const rz = house.origin.z + room.localZ
      if (x < rx || x > rx + room.sizeX || z < rz || z > rz + room.sizeZ) {
        continue
      }

      if (room.roomType === 'stairwell') {
        // A stairwell spans floorLevel..floorLevel+1, and the walker reports
        // either end depending on where the sender's hysteresis flipped.
        if (floorLevel < room.floorLevel || floorLevel > room.floorLevel + 1) {
          continue
        }
        const y =
          house.origin.y +
          getStairwellYOffset(room, house.origin.x, house.origin.z, x, z)
        // Stacked stairwells share an XZ column; the reported floor is always
        // within one rise of the true Y, so nearest-to-floor-base disambiguates.
        const dist = Math.abs(
          y - (house.origin.y + floorYBase(floorLevel, room.wallHeight))
        )
        if (dist < stairDist) {
          stairDist = dist
          stairY = y
        }
      } else if (room.floorLevel === floorLevel) {
        flatY =
          house.origin.y +
          floorYBase(room.floorLevel, room.wallHeight) +
          FLOOR_THICKNESS / 2
      }
    }
  }
  return stairY ?? flatY
}

export interface RoomAABB {
  minX: number
  maxX: number
  minZ: number
  maxZ: number
}

/** Collect XZ AABBs of all rooms whose footprint intersects the given region. */
export function collectRoomAABBsInRegion(
  housesById: ReadonlyMap<string, HouseData>,
  minX: number,
  maxX: number,
  minZ: number,
  maxZ: number
): RoomAABB[] {
  const result: RoomAABB[] = []
  for (const house of housesById.values()) {
    for (const room of house.rooms) {
      const rx = house.origin.x + room.localX
      const rz = house.origin.z + room.localZ
      const rMaxX = rx + room.sizeX
      const rMaxZ = rz + room.sizeZ
      if (rx > maxX || rMaxX < minX || rz > maxZ || rMaxZ < minZ) continue
      result.push({ minX: rx, maxX: rMaxX, minZ: rz, maxZ: rMaxZ })
    }
  }
  return result
}

/** Find ALL rooms containing a world point (for overlapping stairwells etc). */
export function findAllRoomsAtPoint(
  housesById: ReadonlyMap<string, HouseData>,
  x: number,
  y: number,
  z: number
): { house: HouseData; roomIndex: number }[] {
  const results: { house: HouseData; roomIndex: number }[] = []
  for (const house of housesById.values()) {
    for (let i = 0; i < house.rooms.length; i++) {
      const room = house.rooms[i]
      const rx = house.origin.x + room.localX
      const rz = house.origin.z + room.localZ
      const ryBase =
        house.origin.y + floorYBase(room.floorLevel, room.wallHeight)
      if (
        x >= rx &&
        x <= rx + room.sizeX &&
        z >= rz &&
        z <= rz + room.sizeZ &&
        y >= ryBase - 1 &&
        y <= ryBase + room.wallHeight + 1
      ) {
        results.push({ house, roomIndex: i })
      }
    }
  }
  return results
}

/** Find the house whose room contains a world point, or null. */
export function findHouseAtPoint(
  housesById: ReadonlyMap<string, HouseData>,
  x: number,
  y: number,
  z: number
): HouseData | null {
  const result = findRoomAtPoint(housesById, x, y, z)
  return result ? result.house : null
}

/** Find the nearest door segment within maxDist of (x, z). */
export function findNearestDoor(
  housesById: ReadonlyMap<string, HouseData>,
  x: number,
  z: number,
  y: number,
  maxDist: number
): {
  houseId: string
  roomIndex: number
  wallDir: WallDirection
  segmentIndex: number
  distance: number
} | null {
  let best: ReturnType<typeof findNearestDoor> = null

  const dirs: [WallDirection, number][] = [
    ['north', 0],
    ['south', 0],
    ['east', 1],
    ['west', 1],
  ]

  for (const house of housesById.values()) {
    for (let ri = 0; ri < house.rooms.length; ri++) {
      const room = house.rooms[ri]
      const ryBase =
        house.origin.y + floorYBase(room.floorLevel, room.wallHeight)
      if (y < ryBase - 0.5 || y >= ryBase + room.wallHeight) continue

      const rx = house.origin.x + room.localX
      const rz = house.origin.z + room.localZ

      for (const [dir, axis] of dirs) {
        const segs = getWallByDir(room, dir)
        const wallCoord =
          (axis === 0 ? house.origin.z : house.origin.x) +
          wallLineCoord(room, dir)

        for (let si = 0; si < segs.length; si++) {
          if (!isOpenable(segs[si].variant)) continue

          const segCenter = si + 0.5
          const startB = axis === 0 ? rx : rz
          const doorB = startB + segCenter

          const dx = axis === 0 ? doorB - x : wallCoord - x
          const dz = axis === 0 ? wallCoord - z : doorB - z
          const dist = Math.sqrt(dx * dx + dz * dz)

          if (dist < maxDist && (!best || dist < best.distance)) {
            best = {
              houseId: house.id,
              roomIndex: ri,
              wallDir: dir,
              segmentIndex: si,
              distance: dist,
            }
          }
        }
      }
    }
  }

  return best
}

/** Find an existing house that shares an edge with the given room footprint. */
export function findAdjacentHouse(
  housesById: ReadonlyMap<string, HouseData>,
  originX: number,
  originZ: number,
  sizeX: number,
  sizeZ: number
): HouseData | null {
  for (const house of housesById.values()) {
    for (const room of house.rooms) {
      const rx = house.origin.x + room.localX
      const rz = house.origin.z + room.localZ
      // Rooms share an edge if they overlap on one axis and touch exactly on the other
      const overlapX = originX < rx + room.sizeX && originX + sizeX > rx
      const overlapZ = originZ < rz + room.sizeZ && originZ + sizeZ > rz
      const touchN = originZ === rz + room.sizeZ
      const touchS = originZ + sizeZ === rz
      const touchE = originX === rx + room.sizeX
      const touchW = originX + sizeX === rx

      if (
        (overlapX && (touchN || touchS)) ||
        (overlapZ && (touchE || touchW))
      ) {
        return house
      }
    }
  }
  return null
}

/** Check if a room footprint overlaps any existing house on the same floor level. */
export function checkOverlap(
  housesById: ReadonlyMap<string, HouseData>,
  originX: number,
  originZ: number,
  sizeX: number,
  sizeZ: number,
  floorLevel: number = 0
): boolean {
  for (const house of housesById.values()) {
    for (const room of house.rooms) {
      if (room.floorLevel !== floorLevel) continue
      const rx = house.origin.x + room.localX
      const rz = house.origin.z + room.localZ
      if (
        originX < rx + room.sizeX &&
        originX + sizeX > rx &&
        originZ < rz + room.sizeZ &&
        originZ + sizeZ > rz
      ) {
        return true
      }
    }
  }
  return false
}

/**
 * Check if a room footprint on the given floor is fully supported by rooms
 * on the floor below. For floorLevel=1 checks floor 0, for floorLevel=2
 * checks floor 1, etc. Stairwells (floorLevel=0) also use supportFloor=0.
 */
export function hasFloorSupport(
  housesById: ReadonlyMap<string, HouseData>,
  originX: number,
  originZ: number,
  sizeX: number,
  sizeZ: number,
  opts?: { houseId?: string; floorLevel?: number }
): boolean {
  const supportFloor = Math.max(0, (opts?.floorLevel ?? 1) - 1)
  const houseId = opts?.houseId
  for (let x = originX; x < originX + sizeX; x++) {
    for (let z = originZ; z < originZ + sizeZ; z++) {
      let supported = false
      for (const house of housesById.values()) {
        if (houseId && house.id !== houseId) continue
        for (const room of house.rooms) {
          if (room.floorLevel !== supportFloor) continue
          const rx = house.origin.x + room.localX
          const rz = house.origin.z + room.localZ
          if (
            x >= rx &&
            x < rx + room.sizeX &&
            z >= rz &&
            z < rz + room.sizeZ
          ) {
            supported = true
            break
          }
        }
        if (supported) break
      }
      if (!supported) return false
    }
  }
  return true
}

/**
 * Find a house that has rooms on the floor below supporting the given footprint.
 */
export function findSupportingHouse(
  housesById: ReadonlyMap<string, HouseData>,
  originX: number,
  originZ: number,
  sizeX: number,
  sizeZ: number,
  floorLevel: number = 1
): HouseData | null {
  for (const house of housesById.values()) {
    if (
      hasFloorSupport(housesById, originX, originZ, sizeX, sizeZ, {
        houseId: house.id,
        floorLevel,
      })
    ) {
      return house
    }
  }
  return null
}
