import { describe, expect, it } from 'vitest'
import { createRenderCadence } from './renderCadence'

function countRenders(refreshRate: number, seconds = 1): number {
  const cadence = createRenderCadence(60)
  let renders = 0

  for (let frame = 0; frame < refreshRate * seconds; frame++) {
    if (cadence.shouldRender(1 / refreshRate)) renders++
  }

  return renders
}

describe('createRenderCadence', () => {
  it('targets 60 renders per second on common high-refresh displays', () => {
    expect(countRenders(120)).toBe(60)
    expect(countRenders(144)).toBe(60)
    expect(countRenders(240)).toBe(60)
  })

  it('keeps every frame on displays at or below the target', () => {
    expect(countRenders(60)).toBe(60)
    expect(countRenders(30)).toBe(30)
  })

  it('ignores negative deltas', () => {
    const cadence = createRenderCadence(60)

    expect(cadence.shouldRender(-1)).toBe(false)
    expect(cadence.shouldRender(1 / 60)).toBe(true)
  })

  it('does not catch up with multiple renders after a stall', () => {
    const cadence = createRenderCadence(60)

    expect(cadence.shouldRender(1)).toBe(true)
    expect(cadence.shouldRender(0)).toBe(false)
  })
})
