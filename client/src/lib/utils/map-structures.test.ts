import { describe, expect, it } from 'vitest'
import { drawLandPlotGrid } from './map-structures'

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
  it('draws 32 m plot lines, then thick tile edges on odd multiples of 32', () => {
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
      [32, 1.5],
      [96, 1.5],
    ])
  })

  it('skips drawing when plots would be under 4 px', () => {
    const { ctx, lines } = stubCtx()
    drawLandPlotGrid(ctx, 1024, { ...transform, scale: 0.1 })
    expect(lines).toHaveLength(0)
  })
})
