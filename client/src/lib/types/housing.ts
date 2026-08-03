export type RoomType = 'normal' | 'stairwell'

export type WallVariant = 'solid' | 'door' | 'window' | 'open'

export interface WallConfig {
  variant: WallVariant
  texture: number
  isOpen?: boolean
}

export type RoofType = 'flat' | 'gabled' | 'steep'
export type RoofRidgeDir = 'auto' | 'x' | 'z'

export interface RoomData {
  roomType?: RoomType
  roofType?: RoofType
  roofRidgeDir?: RoofRidgeDir
  /** Stairwell ascends in reverse direction (180°/270° rotation) */
  stairReversed?: boolean
  localX: number
  localZ: number
  sizeX: number
  sizeZ: number
  floorLevel: number
  floorTexture: number
  roofTexture: number
  wallHeight: number
  /** 1m segments: north wall (length = sizeX) */
  wallNorth: WallConfig[]
  /** 1m segments: south wall (length = sizeX) */
  wallSouth: WallConfig[]
  /** 1m segments: east wall (length = sizeZ) */
  wallEast: WallConfig[]
  /** 1m segments: west wall (length = sizeZ) */
  wallWest: WallConfig[]
}

export interface PassabilityGrid {
  floorLevel: number
  originX: number
  originZ: number
  width: number
  depth: number
  /** Packed edge bits per cell (N=1, E=2, S=4, W=8). Length = width * depth. */
  cells: number[]
}

export interface HouseData {
  id: string
  ownerId: string
  origin: { x: number; y: number; z: number }
  rooms: RoomData[]
  passability?: PassabilityGrid[]
}

export interface HouseMapFootprint {
  /** Absolute world X/Z followed by width/depth. */
  rects: [number, number, number, number][]
}
