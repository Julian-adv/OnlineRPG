import type { TerrainHeightManager } from './terrainHeightManager'
import { entityGroundY } from './entity-ground'

export function groundItemBaseY(
  heightManager: TerrainHeightManager | null,
  floorLevel: number,
  inHand: boolean,
  x: number,
  z: number,
  fallbackY: number
): number {
  return inHand
    ? fallbackY
    : entityGroundY(heightManager, floorLevel, x, z, fallbackY, fallbackY)
}
