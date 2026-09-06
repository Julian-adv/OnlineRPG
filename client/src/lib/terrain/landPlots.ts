import {
  LAND_PLOT_SIZE,
  REGION_SIZE,
  TILE_DIM,
  tileToRegion,
} from './terrain-constants'
import { wrapWorldX } from './world-wrap'

/** Mirrors terrain/src/land.rs: one grade byte per plot, 1,024 per region. */
export const LandGrade = { Reserved: 0, Homestead: 1, Crown: 2 } as const
export const REGION_PLOTS = REGION_SIZE * REGION_SIZE * 4

export function nextGrade(grade: number): number {
  return grade === LandGrade.Reserved ? LandGrade.Crown : grade - 1
}

export interface PlotAddr {
  rx: number
  rz: number
  index: number
}

export interface OwnedLandPlot extends PlotAddr {
  ownerName: string
}

/** Tile index and quadrant bit along one axis. */
function tileAndQuadrant(world: number): [number, number] {
  const tile = Math.floor((world + TILE_DIM / 2) / TILE_DIM)
  const col = Math.floor(world / LAND_PLOT_SIZE)
  return [tile, col + 1 - 2 * tile]
}

export function plotAddress(x: number, z: number): PlotAddr {
  const [tx, qx] = tileAndQuadrant(wrapWorldX(x))
  const [tz, qz] = tileAndQuadrant(z)
  const rx = tileToRegion(tx)
  const rz = tileToRegion(tz)
  const lx = tx - rx * REGION_SIZE
  const lz = tz - rz * REGION_SIZE
  return { rx, rz, index: (lz * REGION_SIZE + lx) * 4 + qz * 2 + qx }
}

/** World-space min corner of a plot. */
export function plotOrigin(rx: number, rz: number, index: number) {
  const q = index % 4
  const tile = Math.floor(index / 4)
  const tx = rx * REGION_SIZE + (tile % REGION_SIZE)
  const tz = rz * REGION_SIZE + Math.floor(tile / REGION_SIZE)
  return {
    x: tx * TILE_DIM - LAND_PLOT_SIZE + (q % 2) * LAND_PLOT_SIZE,
    z: tz * TILE_DIM - LAND_PLOT_SIZE + Math.floor(q / 2) * LAND_PLOT_SIZE,
  }
}
