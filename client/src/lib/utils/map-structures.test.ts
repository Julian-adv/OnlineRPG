import { describe, expect, it } from 'vitest'
import { drawLandPlotCells, drawLandPlotGrid } from './map-structures'
import {
  LandGrade,
  plotAddress,
  plotOrigin,
  REGION_PLOTS,
} from '../terrain/landPlots'

function stubCtx() {
  const lines: {
    x0: number
    y0: number
    x1: number
    y1: number
    width: number
  }[] = []
  let start = { x: 0, y: 0 }
  const ctx = {
    lineWidth: 1,
    save() {},
    restore() {},
    beginPath() {},
    stroke() {},
    moveTo(x: number, y: number) {
      start = { x, y }
    },
    lineTo(x: number, y: number) {
      lines.push({
        x0: start.x,
        y0: start.y,
        x1: x,
        y1: y,
        width: ctx.lineWidth,
      })
    },
  }
  return { ctx: ctx as unknown as CanvasRenderingContext2D, lines }
}

const transform = { centerX: 0, viewLeft: 0, viewTop: 0, scale: 1 }

describe('drawLandPlotGrid', () => {
  it('draws 32 m plot lines', () => {
    const { ctx, lines } = stubCtx()
    drawLandPlotGrid(ctx, 128, transform)
    const vertical = lines
      .filter((l) => l.x0 === l.x1)
      .map((l) => [l.x0, l.width])
    expect(vertical).toEqual([
      [0, 0.75],
      [32, 0.75],
      [64, 0.75],
      [96, 0.75],
      [128, 0.75],
    ])
  })

  it('skips drawing when plots would be under 4 px', () => {
    const { ctx, lines } = stubCtx()
    drawLandPlotGrid(ctx, 1024, { ...transform, scale: 0.1 })
    expect(lines).toHaveLength(0)
  })

  it('fills graded plots at their world origin', () => {
    const rects: number[][] = []
    const ctx = {
      save() {},
      restore() {},
      fillStyle: '',
      fillRect(x: number, y: number, w: number, h: number) {
        rects.push([x, y, w, h])
      },
    } as unknown as CanvasRenderingContext2D
    const grades = new Uint8Array(REGION_PLOTS).fill(LandGrade.Homestead)
    const addr = plotAddress(1000, 500)
    grades[addr.index] = LandGrade.Crown
    drawLandPlotCells(ctx, [{ rx: addr.rx, rz: addr.rz, grades }], transform)
    const o = plotOrigin(addr.rx, addr.rz, addr.index)
    expect(rects).toEqual([[o.x, o.z, 32, 32]])
  })
})
