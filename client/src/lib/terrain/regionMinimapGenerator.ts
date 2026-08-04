import { getTerrainApiUrl } from '../utils/networkUtils'

/** Build the server URL for a region minimap. The version busts the browser
 *  cache when the editor regenerates bakes mid-session. */
export function regionMinimapServerUrl(
  rx: number,
  rz: number,
  version: number
): string {
  return `${getTerrainApiUrl()}/api/terrain/minimap/${rx}/${rz}?v=${version}`
}
