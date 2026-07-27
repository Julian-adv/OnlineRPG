import { wrapWorldX } from '../terrain/world-wrap'
import { bridgeManager } from './bridgeManager'
import { dungeonManager } from './dungeonManager'
import { housingManager } from './housingManager'
import type { TerrainHeightManager } from './terrainHeightManager'

/**
 * Ground Y for an entity whose floor level is known but whose Y is not
 * (remote players, NPCs). The local-player equivalent is `sampleHeight` in
 * player-physics, which resolves the dungeon/house floor from the local
 * player's own depth and offset state; here the floor comes in explicitly,
 * so nothing depends on where the *observer* is standing.
 */
export function entityGroundY(
  heightManager: TerrainHeightManager | null,
  floorLevel: number,
  x: number,
  z: number,
  fallbackY: number,
  bridgeReferenceY: number | null = null
): number {
  const wx = wrapWorldX(x)

  if (floorLevel < 0) {
    return dungeonManager.floorHeightAt(-floorLevel, wx, z) ?? fallbackY
  }
  if (floorLevel > 0) {
    return housingManager.floorHeightAt(floorLevel, wx, z) ?? fallbackY
  }

  // Floor 0 still checks both structures: the dungeon entrance ramp and a
  // house stairwell (which spans floors 0..1) are both reported as floor 0
  // until the climber crosses into the floor above.
  const rampY = dungeonManager.entranceRampHeightAt(wx, z)
  if (rampY !== null) return rampY
  const houseY = housingManager.floorHeightAt(0, wx, z)
  if (houseY !== null) return houseY
  const deckY = bridgeManager.findDeckYAt(wx, z, bridgeReferenceY)
  if (deckY !== null) return deckY

  // getHeightAtWorldPosition returns 0 for unloaded tiles rather than null,
  // so the guard is what keeps an entity from snapping to sea level.
  if (heightManager?.hasHeightDataForGrid(wx, z)) {
    return heightManager.getHeightAtWorldPosition(wx, z)
  }
  return fallbackY
}
