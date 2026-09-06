import { wrapWorldX, shortestWrappedDeltaX } from './world-wrap'
import { LAND_PLOT_SIZE } from './terrain-constants'

export interface FenceEdge {
  x: number
  z: number
  axis: 'X' | 'Z'
}
export interface Fence {
  edge: FenceEdge
  y: number
  owner_id: number
}
export interface FencePlot {
  x: number
  z: number
}

export function fenceKey(edge: FenceEdge): string {
  return `${edge.x},${edge.z},${edge.axis}`
}

export function fenceCenter(edge: FenceEdge) {
  return {
    x: edge.x + (edge.axis === 'X' ? 0.5 : 0),
    z: edge.z + (edge.axis === 'Z' ? 0.5 : 0),
  }
}

export function nearestFenceEdge(x: number, z: number): FenceEdge {
  x = wrapWorldX(x)
  const cx = Math.floor(x),
    cz = Math.floor(z)
  const candidates: [number, FenceEdge][] = [
    [z - cz, { x: cx, z: cz, axis: 'X' }],
    [cx + 1 - x, { x: wrapWorldX(cx + 1), z: cz, axis: 'Z' }],
    [cz + 1 - z, { x: cx, z: cz + 1, axis: 'X' }],
    [x - cx, { x: cx, z: cz, axis: 'Z' }],
  ]
  return candidates.reduce((a, b) => (b[0] < a[0] ? b : a))[1]
}

export function fenceOnOwnedPlot(edge: FenceEdge, plots: FencePlot[]): boolean {
  const center = fenceCenter(edge)
  return [-0.5, 0.5].some((offset) => {
    const x = wrapWorldX(center.x + (edge.axis === 'Z' ? offset : 0))
    const z = center.z + (edge.axis === 'X' ? offset : 0)
    return plots.some(
      (plot) =>
        x >= plot.x &&
        x < plot.x + LAND_PLOT_SIZE &&
        z >= plot.z &&
        z < plot.z + LAND_PLOT_SIZE
    )
  })
}

export function fenceInReach(
  edge: FenceEdge,
  player: { x: number; z: number }
): boolean {
  const center = fenceCenter(edge)
  return (
    Math.hypot(
      shortestWrappedDeltaX(player.x, center.x),
      center.z - player.z
    ) <= 5
  )
}
