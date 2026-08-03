import { REGION_CELLS } from './terrain-constants'

/** Regions per wrap of the cylindrical world: -16 through +15. */
export const WORLD_REGIONS_X = 32
/** Full baked circumference in meters. */
export const WORLD_WIDTH_X = WORLD_REGIONS_X * REGION_CELLS
/** Tile -256 extends half a 64 m tile west of its center at -16,384. */
export const WORLD_MIN_X = -16 * REGION_CELLS - 32
/** Exclusive east edge; periodically identical to WORLD_MIN_X. */
export const WORLD_MAX_X = WORLD_MIN_X + WORLD_WIDTH_X

export function wrapWorldX(x: number): number {
  return (
    ((((x - WORLD_MIN_X) % WORLD_WIDTH_X) + WORLD_WIDTH_X) % WORLD_WIDTH_X) +
    WORLD_MIN_X
  )
}

/** Shortest signed X offset from `fromX` to `toX` on the cylindrical world. */
export function shortestWrappedDeltaX(fromX: number, toX: number): number {
  const rawDelta = toX - fromX
  if (rawDelta >= -WORLD_WIDTH_X / 2 && rawDelta < WORLD_WIDTH_X / 2) {
    return rawDelta
  }
  return (
    ((((rawDelta + WORLD_WIDTH_X / 2) % WORLD_WIDTH_X) + WORLD_WIDTH_X) %
      WORLD_WIDTH_X) -
    WORLD_WIDTH_X / 2
  )
}

/** Periodic representation of `x` nearest to the given reference position. */
export function unwrapWorldXNear(referenceX: number, x: number): number {
  return referenceX + shortestWrappedDeltaX(referenceX, x)
}

/** Wrap a region x index into the baked -16..15 range (z never wraps). */
export function wrapRegionX(rx: number): number {
  const half = WORLD_REGIONS_X / 2
  return (
    ((((rx + half) % WORLD_REGIONS_X) + WORLD_REGIONS_X) % WORLD_REGIONS_X) -
    half
  )
}
