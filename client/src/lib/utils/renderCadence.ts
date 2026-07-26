import { RENDER_TOLERANCE_RATIO } from './frameTiming'

/** Gates rendering to `targetFps`. Feed it milliseconds — fixed simulation
 *  steps in game, wall-clock deltas on the character screens. */
export function createRenderCadence(targetFps: number) {
  const frameIntervalMs = 1000 / targetFps
  const toleranceMs = frameIntervalMs * RENDER_TOLERANCE_RATIO
  let elapsedMs = 0

  return {
    shouldRender(deltaMs: number): boolean {
      elapsedMs += Math.max(0, deltaMs)
      if (elapsedMs + toleranceMs < frameIntervalMs) return false

      // Keep the sub-interval remainder so pacing stays even, but drop any
      // backlog — a stall must not burst a run of catch-up renders.
      elapsedMs -= frameIntervalMs
      if (elapsedMs < 0 || elapsedMs >= frameIntervalMs) elapsedMs = 0
      return true
    },
  }
}
