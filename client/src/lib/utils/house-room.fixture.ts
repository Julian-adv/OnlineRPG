import type { RoomData } from '../types/housing'

/** Test-only RoomData with solid walls sized to the room. */
export function makeRoom(partial: Partial<RoomData> = {}): RoomData {
  const base = {
    roomType: 'normal' as const,
    roofType: 'steep' as const,
    roofRidgeDir: 'auto' as const,
    localX: 0,
    localZ: 0,
    sizeX: 4,
    sizeZ: 4,
    floorLevel: 0,
    floorTexture: 0,
    roofTexture: 0,
    wallHeight: 3,
    ...partial,
  }
  const seg = { variant: 'solid' as const, texture: 0 }
  return {
    wallNorth: Array(base.sizeX).fill(seg),
    wallSouth: Array(base.sizeX).fill(seg),
    wallEast: Array(base.sizeZ).fill(seg),
    wallWest: Array(base.sizeZ).fill(seg),
    ...base,
  }
}
