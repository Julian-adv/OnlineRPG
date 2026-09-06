import {
  brushInnerRadius,
  LAND_PLOT_SIZE,
  smoothstep,
} from './terrain-constants'
import type { FencePlot } from './fenceEdges'
import {
  shortestWrappedDeltaX,
  wrapWorldX,
  WORLD_MIN_X,
  WORLD_MAX_X,
} from './world-wrap'

export type LandscapingTool = 'Ground' | 'Road' | 'Fence'

export function snapBrushCoordinate(value: number, radius: number): number {
  return radius < 1 ? Math.round(value) : value
}

export interface LandscapingStroke {
  start: [number, number]
  end: [number, number] | null
  radius: number
  strength: number
  palette: number
}

export interface LandscapingTile {
  tile_x: number
  tile_z: number
  splat: number[]
  cleared: number[]
}

export function ownsEstatePosition(plots: FencePlot[], x: number, z: number) {
  x = wrapWorldX(x)
  return plots.some(
    (p) =>
      x >= p.x &&
      x < p.x + LAND_PLOT_SIZE &&
      z >= p.z &&
      z < p.z + LAND_PLOT_SIZE
  )
}

export function ownsEstateSample(plots: FencePlot[], x: number, z: number) {
  return [-0.5, 0.5].every((dx) =>
    [-0.5, 0.5].every((dz) => ownsEstatePosition(plots, x + dx, z + dz))
  )
}

export function landscapingSamples(
  stroke: LandscapingStroke,
  plots: FencePlot[] | null
) {
  const [x1, z1] = stroke.start.map((v) =>
    snapBrushCoordinate(v, stroke.radius)
  )
  const end = (stroke.end ?? stroke.start).map((v) =>
    snapBrushCoordinate(v, stroke.radius)
  )
  const dx = shortestWrappedDeltaX(x1, end[0])
  const dz = end[1] - z1
  if (Math.hypot(dx, dz) > 362.1) return []
  const lengthSq = dx * dx + dz * dz
  const outer = stroke.radius + 1.5
  const inner = brushInnerRadius(stroke.radius)
  const samples: { x: number; z: number; weight: number; fringe: boolean }[] =
    []
  for (
    let z = Math.floor(Math.min(z1, end[1]) - outer);
    z <= Math.floor(Math.max(z1, end[1]) + outer);
    z++
  ) {
    for (
      let x = Math.floor(Math.min(x1, x1 + dx) - outer);
      x <= Math.floor(Math.max(x1, x1 + dx) + outer);
      x++
    ) {
      const t =
        lengthSq > 1e-6
          ? Math.max(0, Math.min(1, ((x - x1) * dx + (z - z1) * dz) / lengthSq))
          : 0
      const distance = Math.hypot(x - (x1 + t * dx), z - (z1 + t * dz))
      if (
        distance > outer ||
        z < WORLD_MIN_X ||
        z >= WORLD_MAX_X ||
        (plots !== null && !ownsEstateSample(plots, x, z))
      )
        continue
      const weight = 1 - smoothstep(inner, stroke.radius, distance)
      const fringe = distance > stroke.radius
      samples.push({ x, z, weight: fringe ? 0.05 : weight, fringe })
    }
  }
  return samples
}

export function clearedCellAt(
  mask: Uint8Array,
  tileX: number,
  tileZ: number,
  x: number,
  z: number
) {
  const cx = Math.floor(shortestWrappedDeltaX(tileX * 64 - 32, x))
  const cz = Math.floor(z - (tileZ * 64 - 32))
  if (cx < 0 || cx >= 64 || cz < 0 || cz >= 64) return false
  const cell = cz * 64 + cx
  return (mask[cell >> 3] & (1 << (cell & 7))) !== 0
}
