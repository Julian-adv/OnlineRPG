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
  type DoorMeshInfo,
  type FloorEntries,
  type HouseGroupResult,
  type InteriorWall,
  type InteriorWallGroup,
  interiorWallOccludes,
  type RoomFootprint,
} from './house-geo-utils'
import { ALL_WALL_DIRS, getWallByDir } from '../managers/housing-passability'
import { setMeshGhost } from './housing-textures'
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
  roomsHash?: string,
  opts?: { roofs?: boolean }
): HouseGroupResult {
  const houseGroup = new THREE.Group()
  houseGroup.position.set(house.origin.x, house.origin.y, house.origin.z)
  houseGroup.name = `house_${house.id}`
  houseGroup.userData.housingPlacementHouseId = house.id

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
    opts?.roofs === false
      ? house.rooms
      : house.rooms.filter((r) =>
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
      entries,
      suppressed.has(room),
      spanByRoom.get(room),
      house.rooms,
      stairwellFootprints
    )
  }

  const floorGroups: HouseGroupResult['floorGroups'] = new Map()

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
    floor.userData.housingPlacementFloorLevel = fl
    const stair = new THREE.Group()
    stair.name = `stair_f${fl}`
    stair.userData.housingSurface = 'floor'
    stair.userData.housingStairFloor = fl
    mergedMeshCount += addMergedMeshes(front, entries.front)
    mergedMeshCount += addMergedMeshes(back, entries.back)
    mergedMeshCount += addMergedMeshes(floor, entries.floor)
    mergedMeshCount += addMergedMeshes(stair, entries.stair)
    const interior: InteriorWallGroup[] = []
    for (const { wall, entries: wallEntries } of entries.interior.values()) {
      const group = new THREE.Group()
      group.name = `interior_f${fl}_${wall.isNS ? 'z' : 'x'}${wall.line}`
      group.userData.housingSurface = 'wall'
      mergedMeshCount += addMergedMeshes(group, wallEntries)
      houseGroup.add(group)
      interior.push({ wall, group, ghost: false })
    }

    for (const door of entries.doors) {
      allDoors.push(door)
    }

    houseGroup.add(front)
    houseGroup.add(back)
    houseGroup.add(floor)
    houseGroup.add(stair)
    floorGroups.set(fl, { front, back, floor, stair, interior })
  }

  for (const door of allDoors) {
    const userData = {
      doorHouseId: house.id,
      doorRoomIndex: door.roomIndex,
      doorWallDir: door.wallDir,
      doorSegmentIndex: door.segmentIndex,
      doorFloorLevel: door.floorLevel,
      doorInteractionPosition: {
        x: house.origin.x + door.interactionPosition.x,
        z: house.origin.z + door.interactionPosition.z,
      },
      doorIsWindow: door.isWindow,
    }
    door.pivot.userData = userData
    if (door.clickTarget) {
      door.clickTarget.userData = { ...userData, housingSurface: 'wall' }
      houseGroup.add(door.clickTarget)
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
  entries: FloorEntries,
  suppressRoof: boolean,
  roofSpan: RoofSpan | undefined,
  allRooms: RoomData[],
  stairwellFootprints: RoomFootprint[]
) {
  if (room.roomType === 'stairwell') {
    collectStairwellGeometries(room, entries.stair, allRooms)
    return
  }

  collectFloorGeometry(room, entries.floor, stairwellFootprints)
  if (!suppressRoof)
    collectRoofGeometry(room, roofSpan, entries.front, entries.back, allRooms)

  for (const dir of ALL_WALL_DIRS)
    collectWallSegments(
      getWallByDir(room, dir),
      dir,
      room,
      roomIndex,
      entries,
      allRooms
    )
}

/** Swap door/window materials to semi-transparent ghost versions for interior view. */
export function applyDoorGhostMaterials(
  result: HouseGroupResult,
  floor: number
) {
  for (const door of result.doors) {
    const isFront = WALL_DIR_INFO[door.wallDir].isFront
    if (door.floorLevel > floor) {
      // Hide upper floor doors/windows entirely
      if (door.pivot.userData.originalPosY === undefined) {
        door.pivot.userData.originalPosY = door.pivot.position.y
      }
      door.pivot.position.y = OFFSCREEN_Y
    } else if (door.floorLevel === floor && isFront && !door.interior) {
      setMeshGhost(door.pivot.children[0] as THREE.Mesh, true)
    }
  }
}

function setWallGhost(w: InteriorWallGroup, ghost: boolean) {
  if (w.ghost === ghost) return
  w.ghost = ghost
  for (const mesh of w.group.children) {
    if (!(mesh instanceof THREE.Mesh)) continue
    if (mesh.userData.decor) mesh.visible = !ghost
    else setMeshGhost(mesh, ghost)
  }
}

/** Fade the shared walls (and their doors) that hide a player at room-local
 *  (px, pz) on `floor` from the camera; their timber trim is hidden outright
 *  so the faded panel stays readable. `floor = null` clears everything. */
export function applyInteriorGhosts(
  result: HouseGroupResult,
  floor: number | null,
  px = 0,
  pz = 0
) {
  const ghosted = new Set<InteriorWall>()
  for (const [fl, groups] of result.floorGroups) {
    for (const w of groups.interior) {
      const ghost = fl === floor && interiorWallOccludes(w.wall, px, pz)
      if (ghost) ghosted.add(w.wall)
      setWallGhost(w, ghost)
    }
  }
  for (const door of result.doors) {
    if (door.interior)
      setMeshGhost(
        door.pivot.children[0] as THREE.Mesh,
        ghosted.has(door.interior)
      )
  }
}

/** Restore door/window materials from ghost back to opaque. */
export function resetDoorGhostMaterials(result: HouseGroupResult) {
  for (const door of result.doors) {
    if (door.pivot.userData.originalPosY !== undefined) {
      door.pivot.position.y = door.pivot.userData.originalPosY
      delete door.pivot.userData.originalPosY
    }
    if (!door.interior)
      setMeshGhost(door.pivot.children[0] as THREE.Mesh, false)
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
