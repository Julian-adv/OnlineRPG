import type { DungeonEntranceDef } from '../data/dungeonDefs'
import type { HouseMapFootprint } from '../types/housing'
import { unwrapWorldXNear } from '../terrain/world-wrap'
import { LAND_PLOT_SIZE, TILE_DIM } from '../terrain/terrain-constants'

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

const LAND_GRID_MIN_PLOT_PX = 4

function drawWorldGrid(
  ctx: CanvasRenderingContext2D,
  spacing: number,
  offset: number,
  size: number,
  transform: MapCanvasTransform
) {
  const sizePx = size * transform.scale
  const first = (v: number) =>
    Math.ceil((v - offset) / spacing) * spacing + offset
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

/** Debug overlay: 32 m plot lines, with the 64 m tile edges drawn over them. */
export function drawLandPlotGrid(
  ctx: CanvasRenderingContext2D,
  size: number,
  transform: MapCanvasTransform
) {
  if (LAND_PLOT_SIZE * transform.scale < LAND_GRID_MIN_PLOT_PX) return
  ctx.save()
  ctx.strokeStyle = 'rgba(255, 255, 255, 0.35)'
  ctx.lineWidth = 0.75
  drawWorldGrid(ctx, LAND_PLOT_SIZE, 0, size, transform)
  ctx.strokeStyle = 'rgba(255, 230, 120, 0.85)'
  ctx.lineWidth = 1.5
  drawWorldGrid(ctx, TILE_DIM, TILE_DIM / 2, size, transform)
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
