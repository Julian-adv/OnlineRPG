// Zoom falloff shared by every camera-facing overlay (player and monster
// nametags, chat bubbles) so they stay a stable size on screen.
const MIN_DIST = 5
const MAX_DIST = 20

/** 0 at or below MIN_DIST, 1 at or above MAX_DIST. */
export function billboardZoomT(dist: number): number {
  return Math.max(0, Math.min(1, (dist - MIN_DIST) / (MAX_DIST - MIN_DIST)))
}

/** Billboard scale: 0.5 up close, 1.0 zoomed out. */
export function billboardScale(dist: number): number {
  return 0.5 + billboardZoomT(dist) * 0.5
}
