import { afterEach, describe, expect, it, vi } from 'vitest'
import { bridgeManager } from './bridgeManager'
import type { TerrainHeightManager } from './terrainHeightManager'
import { groundItemBaseY } from './ground-item-ground'

function heightManager(loaded: boolean, height: number): TerrainHeightManager {
  return {
    hasHeightDataForGrid: vi.fn(() => loaded),
    getHeightAtWorldPosition: vi.fn(() => height),
  } as unknown as TerrainHeightManager
}

describe('groundItemBaseY', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('uses loaded terrain at the displayed drop coordinates', () => {
    const manager = heightManager(true, 8.5)

    expect(groundItemBaseY(manager, 0, false, 12, 20, 3)).toBe(8.5)
    expect(manager.hasHeightDataForGrid).toHaveBeenCalledWith(12, 20)
    expect(manager.getHeightAtWorldPosition).toHaveBeenCalledWith(12, 20)
  })

  it('keeps the stored height until terrain data is loaded', () => {
    const manager = heightManager(false, 8.5)

    expect(groundItemBaseY(manager, 0, false, 12, 20, 3)).toBe(3)
    expect(manager.getHeightAtWorldPosition).not.toHaveBeenCalled()
  })

  it('keeps the stored height while the item is in hand', () => {
    const manager = heightManager(true, 8.5)

    expect(groundItemBaseY(manager, 0, true, 12, 20, 3)).toBe(3)
    expect(manager.hasHeightDataForGrid).not.toHaveBeenCalled()
  })

  it('does not snap dungeon items to outdoor terrain', () => {
    const manager = heightManager(true, 8.5)

    expect(groundItemBaseY(manager, -1, false, 12, 20, 3)).toBe(3)
    expect(manager.hasHeightDataForGrid).not.toHaveBeenCalled()
  })

  it('uses the stored height to reject an elevated bridge substrate', () => {
    const manager = heightManager(true, 2.5)
    const findDeck = vi
      .spyOn(bridgeManager, 'findDeckYAt')
      .mockImplementation((_x, _z, currentY) => (currentY === null ? 9 : null))

    expect(groundItemBaseY(manager, 0, false, 12, 20, 2)).toBe(2.5)
    expect(findDeck).toHaveBeenCalledWith(12, 20, 2)
  })
})
