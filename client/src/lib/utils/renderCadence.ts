export const TARGET_RENDER_FPS = 60

const FRAME_TOLERANCE_SECONDS = 0.0005

export function createRenderCadence(targetFps: number) {
  const frameInterval = 1 / targetFps
  let elapsed = 0

  return {
    shouldRender(delta: number): boolean {
      elapsed += Math.max(0, delta)
      if (elapsed + FRAME_TOLERANCE_SECONDS < frameInterval) return false

      elapsed = elapsed >= frameInterval ? elapsed % frameInterval : 0
      return true
    },
  }
}
