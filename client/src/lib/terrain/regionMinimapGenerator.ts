import { getTerrainApiUrl } from '../utils/networkUtils'

const MINIMAP_RENDER_REVISION = 8

/** Tile sizes the server bakes, coarsest first. */
export const MINIMAP_SOURCE_SIZES = [128, 256, 512, 1024] as const

/** Smallest baked tile that still covers `projectedPx` on screen. `minSize`
 *  lets a caller refuse the coarsest LODs. */
export function pickMinimapSourceSize(
  projectedPx: number,
  minSize = 128
): number {
  const size = MINIMAP_SOURCE_SIZES.find(
    (candidate) => projectedPx <= candidate
  )
  return Math.max(size ?? 1024, minSize)
}

/** Build the server URL for a region minimap. The version busts the browser
 *  cache when the editor regenerates bakes mid-session. */
export function regionMinimapServerUrl(
  rx: number,
  rz: number,
  version: number,
  size = 1024
): string {
  return `${getTerrainApiUrl()}/api/terrain/minimap/${rx}/${rz}?v=${MINIMAP_RENDER_REVISION}-${version}&size=${size}`
}
