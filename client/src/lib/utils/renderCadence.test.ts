import { describe, expect, it } from 'vitest'
import { createRenderCadence } from './renderCadence'
import { FRAME_TIME_MS } from './frameTiming'
import { createRng } from './simplex-noise'

function countRenders(refreshRate: number, seconds = 1, jitterMs = 0): number {
  const cadence = createRenderCadence(60)
  const frameMs = 1000 / refreshRate
  const rng = createRng(1)
  let renders = 0

  for (let frame = 0; frame < refreshRate * seconds; frame++) {
    if (cadence.shouldRender(frameMs + (rng() - 0.5) * 2 * jitterMs)) renders++
  }

  return renders
}

describe('createRenderCadence', () => {
  it('targets 60 renders per second on common high-refresh displays', () => {
    expect(countRenders(120, 10)).toBe(600)
    expect(countRenders(144, 10)).toBe(600)
    expect(countRenders(240, 10)).toBe(600)
  })

  it('keeps every frame on displays at or below the target', () => {
    expect(countRenders(60, 10)).toBe(600)
    expect(countRenders(30, 10)).toBe(300)
  })

  it('still keeps every frame at 60Hz once vsync jitter is added', () => {
    // A fixed 0.5ms slack dropped ~11 frames here; the tolerance scales with
    // the interval so real displays no longer hitch once a second.
    expect(countRenders(60, 10, 1)).toBe(600)
  })

  it('stays within one percent of the target under jitter', () => {
    for (const refreshRate of [120, 144, 240]) {
      const renders = countRenders(refreshRate, 10, 1)
      expect(renders).toBeGreaterThanOrEqual(594)
      expect(renders).toBeLessThanOrEqual(606)
    }
  })

  it('emits one render per simulation step when driven by the game loop', () => {
    const cadence = createRenderCadence(60)
    const halfRate = createRenderCadence(30)
    let full = 0
    let half = 0

    for (let step = 0; step < 600; step++) {
      if (cadence.shouldRender(FRAME_TIME_MS)) full++
      if (halfRate.shouldRender(FRAME_TIME_MS)) half++
    }

    expect(full).toBe(600)
    expect(half).toBe(300)
  })

  it('ignores negative deltas', () => {
    const cadence = createRenderCadence(60)

    expect(cadence.shouldRender(-1)).toBe(false)
    expect(cadence.shouldRender(FRAME_TIME_MS)).toBe(true)
  })

  it('does not catch up with multiple renders after a stall', () => {
    const cadence = createRenderCadence(60)

    expect(cadence.shouldRender(100)).toBe(true)
    expect(cadence.shouldRender(0)).toBe(false)
  })
})
