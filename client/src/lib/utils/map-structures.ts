import type { DungeonEntranceDef } from '../data/dungeonDefs'
import type { HouseMapFootprint } from '../types/housing'
import { unwrapWorldXNear } from '../terrain/world-wrap'
import { LAND_PLOT_SIZE, REGION_SIZE } from '../terrain/terrain-constants'
import { LandGrade, plotOrigin, REGION_PLOTS } from '../terrain/landPlots'

/** Map rotation so screen-up matches walking up (tracks the camera's initial yaw). */
export const MAP_ROTATE_ANGLE = -Math.PI / 4

/** Player rotation = atan2(dx, dz), i.e. a compass heading; convert to a screen angle. */
export function headingToMapAngle(heading: number): number {
  return Math.PI / 2 - heading + MAP_ROTATE_ANGLE
}

export interface MapCanvasTransform {
  centerX: number
  viewLeft: number
  viewTop: number
  scale: number
}

function worldToCanvas(x: number, z: number, transform: MapCanvasTransform) {
  return {
    x:
      (unwrapWorldXNear(transform.centerX, x) - transform.viewLeft) *
      transform.scale,
    y: (z - transform.viewTop) * transform.scale,
  }
}

export function drawHouseMapFootprints(
  ctx: CanvasRenderingContext2D,
  footprints: HouseMapFootprint[],
  transform: MapCanvasTransform
) {
  ctx.save()
  ctx.fillStyle = 'rgba(58, 48, 39, 0.88)'
  ctx.strokeStyle = 'rgba(232, 205, 154, 0.95)'
  ctx.lineWidth = 1
  for (const house of footprints) {
    for (const [x, z, width, depth] of house.rects) {
      const p = worldToCanvas(x, z, transform)
      const drawWidth = Math.max(1, width * transform.scale)
      const drawDepth = Math.max(1, depth * transform.scale)
      ctx.fillRect(p.x, p.y, drawWidth, drawDepth)
      ctx.strokeRect(p.x, p.y, drawWidth, drawDepth)
    }
  }
  ctx.restore()
}

const PLOT_GRADE_FILL: (string | null)[] = []
PLOT_GRADE_FILL[LandGrade.Reserved] = 'rgba(40, 40, 40, 0.55)'
PLOT_GRADE_FILL[LandGrade.Homestead] = null
PLOT_GRADE_FILL[LandGrade.Crown] = 'rgba(255, 196, 64, 0.5)'

export interface LandGradeRegion {
  rx: number
  rz: number
  grades: Uint8Array
}

const LAND_GRID_MIN_PLOT_PX = 4

/** Below this zoom the plot overlay is neither drawn nor editable. */
export function plotsLegible(scale: number): boolean {
  return LAND_PLOT_SIZE * scale >= LAND_GRID_MIN_PLOT_PX
}

export function drawLandPlotCells(
  ctx: CanvasRenderingContext2D,
  regions: LandGradeRegion[],
  transform: MapCanvasTransform
) {
  if (!plotsLegible(transform.scale)) return
  const cellPx = LAND_PLOT_SIZE * transform.scale
  ctx.save()
  let fillStyle: string | null = null
  for (const region of regions) {
    const origin = plotOrigin(region.rx, region.rz, 0)
    const base = worldToCanvas(origin.x, origin.z, transform)
    for (let i = 0; i < REGION_PLOTS; i++) {
      const fill = PLOT_GRADE_FILL[region.grades[i]]
      if (!fill) continue
      if (fill !== fillStyle) ctx.fillStyle = fillStyle = fill
      const tile = i >> 2
      const col = (tile % REGION_SIZE) * 2 + (i & 1)
      const row = Math.floor(tile / REGION_SIZE) * 2 + ((i >> 1) & 1)
      ctx.fillRect(base.x + col * cellPx, base.y + row * cellPx, cellPx, cellPx)
    }
  }
  ctx.restore()
}

function drawWorldGrid(
  ctx: CanvasRenderingContext2D,
  spacing: number,
  size: number,
  transform: MapCanvasTransform
) {
  const sizePx = size * transform.scale
  const first = (v: number) => Math.ceil(v / spacing) * spacing
  ctx.beginPath()
  for (
    let x = first(transform.viewLeft);
    x <= transform.viewLeft + size;
    x += spacing
  ) {
    const px = (x - transform.viewLeft) * transform.scale
    ctx.moveTo(px, 0)
    ctx.lineTo(px, sizePx)
  }
  for (
    let z = first(transform.viewTop);
    z <= transform.viewTop + size;
    z += spacing
  ) {
    const py = (z - transform.viewTop) * transform.scale
    ctx.moveTo(0, py)
    ctx.lineTo(sizePx, py)
  }
  ctx.stroke()
}

/** Debug overlay: 32 m plot lines. */
export function drawLandPlotGrid(
  ctx: CanvasRenderingContext2D,
  size: number,
  transform: MapCanvasTransform
) {
  if (!plotsLegible(transform.scale)) return
  ctx.save()
  ctx.strokeStyle = 'rgba(255, 255, 255, 0.35)'
  ctx.lineWidth = 0.75
  drawWorldGrid(ctx, LAND_PLOT_SIZE, size, transform)
  ctx.restore()
}

export function drawDungeonEntranceMarkers(
  ctx: CanvasRenderingContext2D,
  entrances: DungeonEntranceDef[],
  transform: MapCanvasTransform
) {
  ctx.save()
  ctx.fillStyle = '#35333d'
  ctx.strokeStyle = '#b0503c'
  ctx.lineWidth = 2
  ctx.shadowColor = 'rgba(176, 80, 60, 0.7)'
  ctx.shadowBlur = 4
  for (const entrance of entrances) {
    const p = worldToCanvas(entrance.x, entrance.z, transform)
    ctx.fillRect(p.x - 4, p.y - 4, 8, 8)
    ctx.strokeRect(p.x - 4, p.y - 4, 8, 8)
  }
  ctx.restore()
}
