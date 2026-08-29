// --- Constants ---

export const TILE_DIM = 64
export const VERTS_PER_SIDE = TILE_DIM + 1 // 65
/** Splat texture padded by 1 cell on each side with neighbor-tile data so the
 *  shader's +1 texel bilerp reads the neighbor instead of clamping to its own
 *  edge cell (which produced visible seams at tile boundaries). */
export const SPLAT_PADDED_DIM = TILE_DIM + 2 // 66
export const REGION_SIZE = 16
export const REGION_CELLS = REGION_SIZE * TILE_DIM // 1024

/** Height threshold at/above which water is considered shallow sea (upper bound of sea) */
export const SHALLOW_WATER_THRESHOLD = -0.1

/** Height threshold below which water is considered deep sea */
export const DEEP_WATER_THRESHOLD = -1.5

/** Absolute height (meters) at which snow begins to blend in */
export const SNOW_START_HEIGHT = 300
/** Absolute height (meters) at which terrain is fully snow */
export const SNOW_FULL_HEIGHT = 350

// --- Utility functions ---

/** Tile → region (floor division by REGION_SIZE). Matches Rust's `i32.div_euclid(16)`. */
export function tileToRegion(tile: number): number {
  return Math.floor(tile / REGION_SIZE)
}

export function regionKey(rx: number, rz: number): string {
  return `${rx},${rz}`
}

/** True when (ax, az) is within the 3×3 block of regions around (bx, bz). */
export function isNeighbourRegion(
  ax: number,
  az: number,
  bx: number,
  bz: number
): boolean {
  return Math.abs(ax - bx) <= 1 && Math.abs(az - bz) <= 1
}

export function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t
}

export function smoothstep(edge0: number, edge1: number, x: number): number {
  const t = Math.max(0, Math.min(1, (x - edge0) / (edge1 - edge0)))
  return t * t * (3 - 2 * t)
}
