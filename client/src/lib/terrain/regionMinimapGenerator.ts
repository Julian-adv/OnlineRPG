import { getTerrainApiUrl } from '../utils/networkUtils'

const MINIMAP_RENDER_REVISION = 5

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
