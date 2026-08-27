/**
 * house-geometry.ts — Assembles a THREE.Group from HouseData.
 *
 * Geometries are grouped by (isFront, textureIndex) and merged into one mesh
 * per group. Each mesh uses a shared MeshStandardMaterial from housing-textures.ts.
 *
 * Front group: south walls + west walls + roofs (hidden when player is inside)
 * Back group:  north walls + east walls + floors (always visible)
 */
import * as THREE from 'three'
import type { HouseData, RoomData } from '../types/housing'
import {
  addMergedMeshes,
  collectFootprints,
  computeRoomAABBs,
  roofSpanByRoom,
  type RoofSpan,
  getOrCreateFloorEntries,
  OFFSCREEN_Y,
  WALL_DIR_INFO,
  WOOD_TEXTURE_IDX,
  SHUTTER_PANEL_TEXTURE_IDX,
  type DoorMeshInfo,
  type FloorEntries,
  type GeoEntry,
  type HouseGroupResult,
  type RoomFootprint,
} from './house-geo-utils'
import { getHousingMaterial, getGhostHousingMaterial } from './housing-textures'
import { collectFloorGeometry } from './house-geo-floor'
import { collectRoofGeometry, shouldSuppressRoof } from './house-geo-roof'
import { collectStairwellGeometries } from './house-geo-stairwell'
import { collectWallSegments } from './house-geo-walls'

// Re-export public API so existing imports continue to work
export {
  WALL_THICKNESS,
  FLOOR_THICKNESS,
  DEFAULT_WALL_HEIGHT,
  LANDING_DEPTH,
  MAX_FLOOR_LEVEL,
  OFFSCREEN_Y,
  floorOverhang,
  floorYBase,
  type WallDirection,
  type DoorMeshInfo,
  type HouseGroupResult,
} from './house-geo-utils'
export { getStairwellYOffset } from './house-geo-stairwell'

export function buildHouseGroup(
  house: HouseData,
  roomsHash?: string
): HouseGroupResult {
  const houseGroup = new THREE.Group()
  houseGroup.position.set(house.origin.x, house.origin.y, house.origin.z)
  houseGroup.name = `house_${house.id}`

  const stairwellFootprints = collectFootprints(
    house.rooms,
    (r) => r.roomType === 'stairwell'
  )

  // Pre-compute footprints per floor level for roof suppression checks
  const footprintsByFloor = new Map<number, RoomFootprint[]>()
  for (const room of house.rooms) {
    if (!footprintsByFloor.has(room.floorLevel)) {
      footprintsByFloor.set(
        room.floorLevel,
        collectFootprints(house.rooms, (r) => r.floorLevel === room.floorLevel)
      )
    }
  }

  const suppressed = new Set(
    house.rooms.filter((r) =>
      shouldSuppressRoof(r, footprintsByFloor.get(r.floorLevel + 1) ?? [])
    )
  )
  const allSpans = roofSpanByRoom(house.rooms)
  const spanByRoom = roofSpanByRoom(house.rooms, (r) => suppressed.has(r))

  const perFloor = new Map<number, FloorEntries>()

  for (let ri = 0; ri < house.rooms.length; ri++) {
    const room = house.rooms[ri]
    const fl = room.floorLevel
    const entries = getOrCreateFloorEntries(perFloor, fl)

    collectRoomGeometries(
      room,
      ri,
      entries.front,
      entries.back,
      entries.floor,
      entries.stair,
      entries.doors,
      suppressed.has(room),
      spanByRoom.get(room),
      house.rooms,
      stairwellFootprints
    )
  }

  const floorGroups = new Map<
    number,
    {
      front: THREE.Group
      back: THREE.Group
      floor: THREE.Group
      stair: THREE.Group
    }
  >()

  let mergedMeshCount = 0
  const allDoors: DoorMeshInfo[] = []

  for (const [fl, entries] of perFloor) {
    const front = new THREE.Group()
    front.name = `front_f${fl}`
    front.userData.housingSurface = 'wall'
    const back = new THREE.Group()
    back.name = `back_f${fl}`
    back.userData.housingSurface = 'wall'
    const floor = new THREE.Group()
    floor.name = `floor_f${fl}`
    floor.userData.housingSurface = 'floor'
    const stair = new THREE.Group()
    stair.name = `stair_f${fl}`
    stair.userData.housingSurface = 'floor'
    mergedMeshCount += addMergedMeshes(front, entries.front)
    mergedMeshCount += addMergedMeshes(back, entries.back)
    mergedMeshCount += addMergedMeshes(floor, entries.floor)
    mergedMeshCount += addMergedMeshes(stair, entries.stair)

    for (const door of entries.doors) {
      allDoors.push(door)
    }

    houseGroup.add(front)
    houseGroup.add(back)
    houseGroup.add(floor)
    houseGroup.add(stair)
    floorGroups.set(fl, { front, back, floor, stair })
  }

  for (const door of allDoors) {
    door.pivot.userData = {
      doorHouseId: house.id,
      doorRoomIndex: door.roomIndex,
      doorWallDir: door.wallDir,
      doorSegmentIndex: door.segmentIndex,
      doorFloorLevel: door.floorLevel,
    }
    houseGroup.add(door.pivot)
  }

  const roomAABBs = computeRoomAABBs(house, allSpans)
  return {
    houseGroup,
    floorGroups,
    aabb: roomAABBs.reduce((b, r) => b.union(r), new THREE.Box3()),
    roomAABBs,
    roomsHash: roomsHash ?? JSON.stringify(house.rooms),
    mergedMeshCount,
    doors: allDoors,
  }
}

function collectRoomGeometries(
  room: RoomData,
  roomIndex: number,
  frontEntries: GeoEntry[],
  backEntries: GeoEntry[],
  floorEntries: GeoEntry[],
  stairEntries: GeoEntry[],
  doors: DoorMeshInfo[],
  suppressRoof: boolean,
  roofSpan: RoofSpan | undefined,
  allRooms: RoomData[],
  stairwellFootprints: RoomFootprint[]
) {
  if (room.roomType === 'stairwell') {
    collectStairwellGeometries(room, stairEntries, allRooms)
    return
  }

  collectFloorGeometry(room, floorEntries, stairwellFootprints)
  if (!suppressRoof)
    collectRoofGeometry(room, roofSpan, frontEntries, backEntries, allRooms)

  collectWallSegments(
    room.wallNorth,
    'north',
    room,
    roomIndex,
    frontEntries,
    backEntries,
    doors
  )
  collectWallSegments(
    room.wallSouth,
    'south',
    room,
    roomIndex,
    frontEntries,
    backEntries,
    doors
  )
  collectWallSegments(
    room.wallEast,
    'east',
    room,
    roomIndex,
    frontEntries,
    backEntries,
    doors
  )
  collectWallSegments(
    room.wallWest,
    'west',
    room,
    roomIndex,
    frontEntries,
    backEntries,
    doors
  )
}

/** Swap door/window materials to semi-transparent ghost versions for interior view. */
export function applyDoorGhostMaterials(
  result: HouseGroupResult,
  floor: number
) {
  const doorMat = getHousingMaterial(WOOD_TEXTURE_IDX)
  const shutterMat = getHousingMaterial(SHUTTER_PANEL_TEXTURE_IDX)

  for (const door of result.doors) {
    const isFront = WALL_DIR_INFO[door.wallDir].isFront
    if (door.floorLevel > floor) {
      // Hide upper floor doors/windows entirely
      if (door.pivot.userData.originalPosY === undefined) {
        door.pivot.userData.originalPosY = door.pivot.position.y
      }
      door.pivot.position.y = OFFSCREEN_Y
    } else if (door.floorLevel === floor && isFront) {
      const mesh = door.pivot.children[0] as THREE.Mesh
      if (mesh.userData.originalMaterial) continue
      mesh.userData.originalMaterial = mesh.material
      if (mesh.material === doorMat) {
        mesh.material = getGhostHousingMaterial(WOOD_TEXTURE_IDX)
      } else if (mesh.material === shutterMat) {
        mesh.material = getGhostHousingMaterial(SHUTTER_PANEL_TEXTURE_IDX)
      }
    }
  }
}

/** Restore door/window materials from ghost back to opaque. */
export function resetDoorGhostMaterials(result: HouseGroupResult) {
  for (const door of result.doors) {
    if (door.pivot.userData.originalPosY !== undefined) {
      door.pivot.position.y = door.pivot.userData.originalPosY
      delete door.pivot.userData.originalPosY
    }
    const mesh = door.pivot.children[0] as THREE.Mesh
    if (mesh.userData.originalMaterial) {
      mesh.material = mesh.userData.originalMaterial
      delete mesh.userData.originalMaterial
    }
  }
}

/** Dispose merged geometries in a house group */
export function disposeHouseGroup(group: THREE.Group) {
  group.traverse((obj) => {
    if (obj instanceof THREE.Mesh) {
      obj.geometry?.dispose()
    }
  })
}
