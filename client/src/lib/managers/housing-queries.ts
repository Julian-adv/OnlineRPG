import type { HouseData, RoomData } from '../types/housing'
import {
  FLOOR_THICKNESS,
  LANDING_DEPTH,
  floorYBase,
  getStairwellYOffset,
  type WallDirection,
} from '../utils/house-geometry'
import {
  getWallByDir,
  isDoorVariant,
  isOpenable,
  wallLineCoord,
} from './housing-passability'

/** Whether (x, z) lies within a room's footprint, optionally inset by margin. */
export function roomContainsXZ(
  house: HouseData,
  room: RoomData,
  x: number,
  z: number,
  margin = 0
): boolean {
  const rx = house.origin.x + room.localX
  const rz = house.origin.z + room.localZ
  return (
    x >= rx + margin &&
    x <= rx + room.sizeX - margin &&
    z >= rz + margin &&
    z <= rz + room.sizeZ - margin
  )
}

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
      const ryBase =
        house.origin.y + floorYBase(room.floorLevel, room.wallHeight)
      if (
        roomContainsXZ(house, room, x, z) &&
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
      if (roomContainsXZ(house, room, x, z)) {
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
      if (!roomContainsXZ(house, room, x, z)) {
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

interface StairCandidate {
  house: HouseData
  room: RoomData
}

export function resolveStairFloor(
  entryFloor: number,
  currentFloor: number,
  progress: number
): number {
  if (currentFloor <= entryFloor) {
    return progress >= 0.88 ? entryFloor + 1 : entryFloor
  }
  return progress <= 0.12 ? entryFloor : entryFloor + 1
}

export function shouldIgnoreImplicitHouseFloorChange(
  insideHouseId: string | null,
  currentFloor: number,
  targetFloor: number,
  viaStair: boolean
): boolean {
  return insideHouseId !== null && !viaStair && currentFloor !== targetFloor
}

function nearestStairAt(
  housesById: ReadonlyMap<string, HouseData>,
  floorLevel: number,
  x: number,
  y: number,
  z: number,
  stairFloor?: number
): StairCandidate | null {
  let best: StairCandidate | null = null
  let bestDistance = Infinity
  for (const house of housesById.values()) {
    for (const room of house.rooms) {
      if (
        room.roomType !== 'stairwell' ||
        (stairFloor !== undefined && room.floorLevel !== stairFloor) ||
        floorLevel < room.floorLevel ||
        floorLevel > room.floorLevel + 1 ||
        !roomContainsXZ(house, room, x, z)
      ) {
        continue
      }
      const stairY =
        house.origin.y +
        getStairwellYOffset(room, house.origin.x, house.origin.z, x, z)
      const distance = Math.abs(stairY - y)
      if (distance < bestDistance) {
        bestDistance = distance
        best = { house, room }
      }
    }
  }
  return best
}

export function assistStairMovementDirection(
  housesById: ReadonlyMap<string, HouseData>,
  floorLevel: number,
  position: { x: number; y: number; z: number },
  direction: { x: number; z: number }
): { x: number; z: number } {
  const match = nearestStairAt(
    housesById,
    floorLevel,
    position.x,
    position.y,
    position.z
  )
  if (!match) return direction

  const { house, room } = match
  const alongZ = room.sizeZ >= room.sizeX
  const length = alongZ ? room.sizeZ : room.sizeX
  const width = alongZ ? room.sizeX : room.sizeZ
  const alongPosition = alongZ
    ? position.z - (house.origin.z + room.localZ)
    : position.x - (house.origin.x + room.localX)
  const edgeDistance = Math.max(
    0,
    Math.min(alongPosition, length - alongPosition)
  )
  const edgeT = Math.min(1, edgeDistance / LANDING_DEPTH)
  const edgeWeight = edgeT * edgeT * (3 - 2 * edgeT)
  const alongInput = alongZ ? direction.z : direction.x
  const lateralInput = alongZ ? direction.x : direction.z
  const driveWeight = Math.min(1, Math.abs(alongInput) * Math.SQRT2)
  const weight = edgeWeight * driveWeight
  if (weight <= 0) return direction

  const lateralPosition = alongZ
    ? position.x - (house.origin.x + room.localX + room.sizeX / 2)
    : position.z - (house.origin.z + room.localZ + room.sizeZ / 2)
  const halfWidth = Math.max(width / 2, 0.01)
  const deadZone = Math.min(0.15, halfWidth * 0.3)
  const excess = Math.max(0, Math.abs(lateralPosition) - deadZone)
  const usableHalfWidth = Math.max(halfWidth - deadZone, 0.01)
  const centerPull =
    -Math.sign(lateralPosition) *
    Math.min(1, excess / usableHalfWidth) *
    0.35 *
    Math.abs(alongInput)
  const correctedLateral =
    lateralInput * (1 - 0.7 * weight) + centerPull * weight
  const originalLength = Math.hypot(direction.x, direction.z)
  const correctedLength = Math.hypot(alongInput, correctedLateral)
  if (correctedLength <= 1e-6) return direction
  const scale = originalLength / correctedLength

  return alongZ
    ? { x: correctedLateral * scale, z: alongInput * scale }
    : { x: alongInput * scale, z: correctedLateral * scale }
}

export function stairLandingTargetAt(
  housesById: ReadonlyMap<string, HouseData>,
  floorLevel: number,
  x: number,
  y: number,
  z: number,
  stairFloor?: number
): { x: number; y: number; z: number } | null {
  const match = nearestStairAt(housesById, floorLevel, x, y, z, stairFloor)
  if (!match) return null

  const { house, room } = match
  const upperFloor = room.floorLevel + 1
  const targetFloor =
    floorLevel === room.floorLevel ? upperFloor : room.floorLevel
  const alongZ = room.sizeZ >= room.sizeX
  const length = alongZ ? room.sizeZ : room.sizeX
  const targetUpper = targetFloor === upperFloor
  const targetAtMax = targetUpper !== (room.stairReversed ?? false)
  const along = targetAtMax ? length - LANDING_DEPTH / 2 : LANDING_DEPTH / 2
  const targetX = alongZ
    ? house.origin.x + room.localX + room.sizeX / 2
    : house.origin.x + room.localX + along
  const targetZ = alongZ
    ? house.origin.z + room.localZ + along
    : house.origin.z + room.localZ + room.sizeZ / 2

  return {
    x: targetX,
    y:
      house.origin.y +
      floorYBase(targetFloor, room.wallHeight) +
      FLOOR_THICKNESS / 2,
    z: targetZ,
  }
}

interface HousePathWaypoint {
  x: number
  z: number
  floor: number
}

function containsHouseFloorPoint(
  house: HouseData,
  floor: number,
  x: number,
  z: number
): boolean {
  return house.rooms.some(
    (room) =>
      room.roomType !== 'stairwell' &&
      room.floorLevel === floor &&
      roomContainsXZ(house, room, x, z)
  )
}

function segmentRectEntry(
  x0: number,
  z0: number,
  x1: number,
  z1: number,
  minX: number,
  maxX: number,
  minZ: number,
  maxZ: number
): number | null {
  const dx = x1 - x0
  const dz = z1 - z0
  let enter = 0
  let exit = 1
  for (const [p, q] of [
    [-dx, x0 - minX],
    [dx, maxX - x0],
    [-dz, z0 - minZ],
    [dz, maxZ - z0],
  ] as const) {
    if (Math.abs(p) <= 1e-9) {
      if (q < 0) return null
      continue
    }
    const t = q / p
    if (p < 0) enter = Math.max(enter, t)
    else exit = Math.min(exit, t)
    if (enter > exit) return null
  }
  return enter
}

function firstHouseEntryOnSegment(
  house: HouseData,
  floor: number,
  x0: number,
  z0: number,
  x1: number,
  z1: number
): number | null {
  const inset = 0.05
  let first: number | null = null
  for (const room of house.rooms) {
    if (room.roomType === 'stairwell' || room.floorLevel !== floor) continue
    const minX = house.origin.x + room.localX
    const minZ = house.origin.z + room.localZ
    const entry = segmentRectEntry(
      x0,
      z0,
      x1,
      z1,
      minX + inset,
      minX + room.sizeX - inset,
      minZ + inset,
      minZ + room.sizeZ - inset
    )
    if (entry !== null && (first === null || entry < first)) first = entry
  }
  return first
}

export function stopPathAtHouseEntrance(
  housesById: ReadonlyMap<string, HouseData>,
  current: { x: number; y: number; z: number },
  currentFloor: number,
  target: { x: number; y: number; z: number },
  waypoints: HousePathWaypoint[],
  insideDistance = 0.7
): HousePathWaypoint[] {
  if (currentFloor !== 0 || waypoints.length === 0) return waypoints
  const targetHouse = findHouseAtPoint(housesById, target.x, target.y, target.z)
  if (
    !targetHouse ||
    containsHouseFloorPoint(targetHouse, currentFloor, current.x, current.z)
  ) {
    return waypoints
  }

  const result: HousePathWaypoint[] = []
  let previous = { x: current.x, z: current.z, floor: currentFloor }
  let remainingInside = insideDistance
  let entered = false

  for (const waypoint of waypoints) {
    const dx = waypoint.x - previous.x
    const dz = waypoint.z - previous.z
    const distance = Math.hypot(dx, dz)
    let startT = 0

    if (!entered) {
      const entry = firstHouseEntryOnSegment(
        targetHouse,
        previous.floor,
        previous.x,
        previous.z,
        waypoint.x,
        waypoint.z
      )
      if (entry === null) {
        result.push(waypoint)
        previous = waypoint
        continue
      }
      entered = true
      startT = entry
    }

    const available = distance * (1 - startT)
    if (distance > 1e-6 && available >= remainingInside) {
      const stopT = startT + remainingInside / distance
      result.push({
        x: previous.x + dx * stopT,
        z: previous.z + dz * stopT,
        floor: waypoint.floor,
      })
      return result
    }

    remainingInside -= available
    result.push(waypoint)
    previous = waypoint
  }

  return result
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
      const ryBase =
        house.origin.y + floorYBase(room.floorLevel, room.wallHeight)
      if (
        roomContainsXZ(house, room, x, z) &&
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

export interface ClosedHouseDoor {
  houseId: string
  roomIndex: number
  wallDir: WallDirection
  segmentIndex: number
  position: { x: number; z: number }
}

function crossingAtAxisWall(
  fromAxis: number,
  fromAlong: number,
  toAxis: number,
  toAlong: number,
  wall: number,
  segmentStart: number
): number | null {
  const axisDelta = toAxis - fromAxis
  if (Math.abs(axisDelta) <= 1e-9) return null
  const t = (wall - fromAxis) / axisDelta
  if (t <= 1e-6 || t >= 1 - 1e-6) return null
  const along = fromAlong + (toAlong - fromAlong) * t
  return along >= segmentStart - 1e-6 && along <= segmentStart + 1 + 1e-6
    ? t
    : null
}

export function findClosedDoorOnSegment(
  housesById: ReadonlyMap<string, HouseData>,
  fromX: number,
  fromZ: number,
  toX: number,
  toZ: number,
  floorLevel: number
): ClosedHouseDoor | null {
  let nearest: ClosedHouseDoor | null = null
  let nearestT = Infinity

  for (const house of housesById.values()) {
    for (let roomIndex = 0; roomIndex < house.rooms.length; roomIndex++) {
      const room = house.rooms[roomIndex]
      if (room.floorLevel !== floorLevel) continue
      const roomX = house.origin.x + room.localX
      const roomZ = house.origin.z + room.localZ

      for (const wallDir of ['north', 'south', 'east', 'west'] as const) {
        const segments = getWallByDir(room, wallDir)
        const alongX = wallDir === 'north' || wallDir === 'south'
        const wall =
          (alongX ? house.origin.z : house.origin.x) +
          wallLineCoord(room, wallDir)

        for (
          let segmentIndex = 0;
          segmentIndex < segments.length;
          segmentIndex++
        ) {
          const segment = segments[segmentIndex]
          if (!isDoorVariant(segment.variant) || segment.isOpen) continue
          const start = (alongX ? roomX : roomZ) + segmentIndex
          const t = alongX
            ? crossingAtAxisWall(fromZ, fromX, toZ, toX, wall, start)
            : crossingAtAxisWall(fromX, fromZ, toX, toZ, wall, start)
          if (t === null || t >= nearestT) continue

          nearestT = t
          nearest = {
            houseId: house.id,
            roomIndex,
            wallDir,
            segmentIndex,
            position: alongX
              ? { x: start + 0.5, z: wall }
              : { x: wall, z: start + 0.5 },
          }
        }
      }
    }
  }

  return nearest
}

export function isHouseWallBlockingSegment(
  housesById: ReadonlyMap<string, HouseData>,
  fromX: number,
  fromZ: number,
  toX: number,
  toZ: number,
  floorLevel: number
): boolean {
  for (const house of housesById.values()) {
    for (const room of house.rooms) {
      if (room.floorLevel !== floorLevel) continue
      const roomX = house.origin.x + room.localX
      const roomZ = house.origin.z + room.localZ

      for (const wallDir of ['north', 'south', 'east', 'west'] as const) {
        const segments = getWallByDir(room, wallDir)
        const alongX = wallDir === 'north' || wallDir === 'south'
        const wall =
          (alongX ? house.origin.z : house.origin.x) +
          wallLineCoord(room, wallDir)

        for (
          let segmentIndex = 0;
          segmentIndex < segments.length;
          segmentIndex++
        ) {
          const segment = segments[segmentIndex]
          if (
            segment.variant === 'open' ||
            (isDoorVariant(segment.variant) && segment.isOpen)
          ) {
            continue
          }
          const start = (alongX ? roomX : roomZ) + segmentIndex
          const t = alongX
            ? crossingAtAxisWall(fromZ, fromX, toZ, toX, wall, start)
            : crossingAtAxisWall(fromX, fromZ, toX, toZ, wall, start)
          if (t !== null) return true
        }
      }
    }
  }

  return false
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
