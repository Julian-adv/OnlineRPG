import * as THREE from 'three'
import type { HouseData } from '../types/housing'
import { FLOOR_THICKNESS, floorYBase } from '../utils/house-geometry'
import { LAND_PLOT_SIZE } from './terrain-constants'
import { unwrapWorldXNear, wrapWorldX } from './world-wrap'

export interface EstatePlot {
  x: number
  z: number
}

export interface FurnitureFootprint {
  width: number
  depth: number
}

export interface FurnitureFootprintPose {
  x: number
  z: number
  rotationDeg: number
  footprint: FurnitureFootprint
}

export interface EstateFurniturePlacementDefinition {
  modelUrl: string
  snapStep: number
  rotationStep: number
  footprint: FurnitureFootprint
  floorEdgeClearance: number
  minFloor: number
  maxFloor: number
}

export interface EstateFurniturePlacement {
  position: { x: number; y: number; z: number }
  rotationDeg: number
  floorLevel: number
}

export function housingPlacementFloor(object: THREE.Object3D) {
  let current: THREE.Object3D | null = object
  while (current) {
    const floor = current.userData.housingPlacementFloorLevel
    if (typeof floor === 'number') return floor
    current = current.parent
  }
  return null
}

export function housingPlacementHouseId(object: THREE.Object3D) {
  let current: THREE.Object3D | null = object
  while (current) {
    const houseId = current.userData.housingPlacementHouseId
    if (typeof houseId === 'string') return houseId
    current = current.parent
  }
  return null
}

function furnitureBounds(
  x: number,
  z: number,
  rotationDeg: number,
  footprint: FurnitureFootprint
) {
  const radians = THREE.MathUtils.degToRad(rotationDeg)
  const cos = Math.abs(Math.cos(radians))
  const sin = Math.abs(Math.sin(radians))
  const halfWidth = (footprint.width * cos + footprint.depth * sin) / 2
  const halfDepth = (footprint.width * sin + footprint.depth * cos) / 2
  return {
    minX: x - halfWidth,
    maxX: x + halfWidth,
    minZ: z - halfDepth,
    maxZ: z + halfDepth,
  }
}

function intersectionArea(
  a: { minX: number; maxX: number; minZ: number; maxZ: number },
  b: { minX: number; maxX: number; minZ: number; maxZ: number }
) {
  return (
    Math.max(0, Math.min(a.maxX, b.maxX) - Math.max(a.minX, b.minX)) *
    Math.max(0, Math.min(a.maxZ, b.maxZ) - Math.max(a.minZ, b.minZ))
  )
}

export function furnitureFootprintsOverlap(
  placement: FurnitureFootprintPose,
  other: FurnitureFootprintPose
) {
  const bounds = furnitureBounds(
    placement.x,
    placement.z,
    placement.rotationDeg,
    placement.footprint
  )
  const otherBounds = furnitureBounds(
    unwrapWorldXNear(placement.x, other.x),
    other.z,
    other.rotationDeg,
    other.footprint
  )
  return intersectionArea(bounds, otherBounds) > 0.0001
}

function roomBounds(house: HouseData, room: HouseData['rooms'][number]) {
  return {
    minX: house.origin.x + room.localX,
    maxX: house.origin.x + room.localX + room.sizeX,
    minZ: house.origin.z + room.localZ,
    maxZ: house.origin.z + room.localZ + room.sizeZ,
  }
}

export function pointOnHouseFloor(
  house: HouseData,
  floorLevel: number,
  x: number,
  z: number
) {
  const worldX = unwrapWorldXNear(house.origin.x, x)
  const onFloor = house.rooms.some((room) => {
    const bounds = roomBounds(house, room)
    return (
      room.roomType !== 'stairwell' &&
      room.floorLevel === floorLevel &&
      worldX >= bounds.minX &&
      worldX <= bounds.maxX &&
      z >= bounds.minZ &&
      z <= bounds.maxZ
    )
  })
  if (!onFloor) return false
  return !house.rooms.some((room) => {
    const bounds = roomBounds(house, room)
    return (
      room.roomType === 'stairwell' &&
      floorLevel >= room.floorLevel &&
      floorLevel <= room.floorLevel + 1 &&
      worldX >= bounds.minX &&
      worldX <= bounds.maxX &&
      z >= bounds.minZ &&
      z <= bounds.maxZ
    )
  })
}

export function houseFloorY(
  house: HouseData,
  floorLevel: number,
  x: number,
  z: number
) {
  if (!pointOnHouseFloor(house, floorLevel, x, z)) return null
  const worldX = unwrapWorldXNear(house.origin.x, x)
  const room = house.rooms.find((candidate) => {
    const bounds = roomBounds(house, candidate)
    return (
      candidate.roomType !== 'stairwell' &&
      candidate.floorLevel === floorLevel &&
      worldX >= bounds.minX &&
      worldX <= bounds.maxX &&
      z >= bounds.minZ &&
      z <= bounds.maxZ
    )
  })
  return room
    ? house.origin.y +
        floorYBase(room.floorLevel, room.wallHeight) +
        FLOOR_THICKNESS / 2
    : null
}

export function footprintOnHouseFloor(
  house: HouseData,
  floorLevel: number,
  x: number,
  z: number,
  rotationDeg: number,
  footprint: FurnitureFootprint,
  edgeClearance = 0
) {
  const bounds = furnitureBounds(
    unwrapWorldXNear(house.origin.x, x),
    z,
    rotationDeg,
    footprint
  )
  const clearanceBounds = {
    minX: bounds.minX - edgeClearance,
    maxX: bounds.maxX + edgeClearance,
    minZ: bounds.minZ - edgeClearance,
    maxZ: bounds.maxZ + edgeClearance,
  }
  const area =
    (clearanceBounds.maxX - clearanceBounds.minX) *
    (clearanceBounds.maxZ - clearanceBounds.minZ)
  const floorArea = house.rooms
    .filter(
      (room) => room.roomType !== 'stairwell' && room.floorLevel === floorLevel
    )
    .reduce(
      (covered, room) =>
        covered + intersectionArea(clearanceBounds, roomBounds(house, room)),
      0
    )
  const crossesStairs = house.rooms.some(
    (room) =>
      room.roomType === 'stairwell' &&
      floorLevel >= room.floorLevel &&
      floorLevel <= room.floorLevel + 1 &&
      intersectionArea(clearanceBounds, roomBounds(house, room)) > 0.0001
  )
  return !crossesStairs && floorArea >= area - 0.0001
}

export function snapPlacementCoordinate(value: number, step: number) {
  return Math.round(value / step) * step
}

export function pointOnOwnedEstate(x: number, z: number, plots: EstatePlot[]) {
  return plots.some((plot) => {
    const plotX = unwrapWorldXNear(x, plot.x)
    return (
      x >= plotX &&
      x < plotX + LAND_PLOT_SIZE &&
      z >= plot.z &&
      z < plot.z + LAND_PLOT_SIZE
    )
  })
}

export function footprintOnOwnedEstate(
  x: number,
  z: number,
  rotationDeg: number,
  footprint: FurnitureFootprint,
  plots: EstatePlot[]
) {
  const radians = THREE.MathUtils.degToRad(rotationDeg)
  const cos = Math.cos(radians)
  const sin = Math.sin(radians)
  const halfWidth = footprint.width / 2
  const halfDepth = footprint.depth / 2
  const samples = [
    [0, 0],
    [-halfWidth, -halfDepth],
    [-halfWidth, halfDepth],
    [halfWidth, -halfDepth],
    [halfWidth, halfDepth],
  ]
  return samples.every(([localX, localZ]) =>
    pointOnOwnedEstate(
      x + localX * cos + localZ * sin,
      z - localX * sin + localZ * cos,
      plots
    )
  )
}

export class EstatePlacementGrid {
  readonly object: THREE.LineSegments
  private lastPlots: EstatePlot[] | null = null
  private lastAnchorX = Infinity
  private dirty = true

  constructor(
    private readonly sampleGroundY: (x: number, z: number) => number | null,
    private readonly pointVisible: (x: number, z: number) => boolean = () =>
      true
  ) {
    const material = new THREE.LineDashedMaterial({
      color: '#b7ae93',
      transparent: true,
      opacity: 0.2,
      dashSize: 0.15,
      gapSize: 0.1,
      depthTest: false,
    })
    this.object = new THREE.LineSegments(new THREE.BufferGeometry(), material)
    this.object.visible = false
    this.object.renderOrder = 10
  }

  markDirty() {
    this.dirty = true
  }

  update(visible: boolean, plots: EstatePlot[], anchorX: number) {
    this.object.visible = visible
    if (!visible) return
    if (
      plots === this.lastPlots &&
      Math.abs(anchorX - this.lastAnchorX) < 100 &&
      !this.dirty
    )
      return
    this.lastPlots = plots
    this.lastAnchorX = anchorX
    this.dirty = false

    const positions: number[] = []
    const plotKeys = new Set(plots.map((plot) => `${plot.x},${plot.z}`))
    const segment = (x1: number, z1: number, x2: number, z2: number) => {
      if (!this.pointVisible(x1, z1) || !this.pointVisible(x2, z2)) return
      const y1 = this.sampleGroundY(wrapWorldX(x1), z1)
      const y2 = this.sampleGroundY(wrapWorldX(x2), z2)
      if (y1 === null || y2 === null) return
      positions.push(x1, y1 + 0.04, z1, x2, y2 + 0.04, z2)
    }

    for (const plot of plots) {
      const x = unwrapWorldXNear(anchorX, plot.x)
      const west = plotKeys.has(
        `${wrapWorldX(plot.x - LAND_PLOT_SIZE)},${plot.z}`
      )
      const south = plotKeys.has(`${plot.x},${plot.z - LAND_PLOT_SIZE}`)
      for (let offset = 0; offset <= LAND_PLOT_SIZE; offset++) {
        for (let step = 0; step < LAND_PLOT_SIZE; step++) {
          if (offset > 0 || !west)
            segment(x + offset, plot.z + step, x + offset, plot.z + step + 1)
          if (offset > 0 || !south)
            segment(x + step, plot.z + offset, x + step + 1, plot.z + offset)
        }
      }
    }

    this.object.geometry.dispose()
    this.object.geometry = new THREE.BufferGeometry()
    this.object.geometry.setAttribute(
      'position',
      new THREE.Float32BufferAttribute(positions, 3)
    )
    this.object.computeLineDistances()
    this.object.geometry.computeBoundingSphere()
  }

  dispose() {
    this.object.geometry.dispose()
    ;(this.object.material as THREE.Material).dispose()
  }
}
