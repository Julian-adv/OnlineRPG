/** World-unit margin per side added to the hovered entity's raycast proxy,
 *  so the pointer must drift further out before the target drops. Acquisition
 *  still tests the exact-size proxy. */
export const HOVER_STICKY_MARGIN = 0.5

export const HOVER_SCALE_IDLE: [number, number, number] = [1, 1, 1]

/** Per-axis scale that inflates a proxy box of `size` by the margin on each
 *  side. `parentScale` undoes a scaled parent group. */
export function stickyHoverScale(
  size: { x: number; y: number; z: number },
  parentScale = 1
): [number, number, number] {
  const m = (HOVER_STICKY_MARGIN * 2) / parentScale
  const grow = (s: number) => (s + m) / Math.max(s, 0.001)
  return [grow(size.x), grow(size.y), grow(size.z)]
}
