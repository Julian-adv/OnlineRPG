import { afterEach, describe, expect, it, vi } from 'vitest'
import { bridgeManager } from './bridgeManager'
import { TerrainHeightManager } from './terrainHeightManager'
import { VERTS_PER_SIDE, encodeHeight } from './terrain-height-types'
import { groundItemBaseY } from './ground-item-ground'

function fakeHeightManager(
  loaded: boolean,
  height: number
): TerrainHeightManager {
  return {
    hasHeightDataForGrid: vi.fn(() => loaded),
    getHeightAtWorldPosition: vi.fn(() => height),
  } as unknown as TerrainHeightManager
}

describe('groundItemBaseY', () => {
  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('uses loaded terrain at the landing coordinates', () => {
    const manager = fakeHeightManager(true, 8.5)

    expect(groundItemBaseY(manager, 0, false, 12, 20, 3)).toBe(8.5)
    expect(manager.hasHeightDataForGrid).toHaveBeenCalledWith(12, 20)
    expect(manager.getHeightAtWorldPosition).toHaveBeenCalledWith(12, 20)
  })

  it('keeps the stored height until terrain data is loaded', () => {
    const manager = fakeHeightManager(false, 8.5)

    expect(groundItemBaseY(manager, 0, false, 12, 20, 3)).toBe(3)
    expect(manager.getHeightAtWorldPosition).not.toHaveBeenCalled()
  })

  it('keeps the stored height while the item is in hand', () => {
    const manager = fakeHeightManager(true, 8.5)

    expect(groundItemBaseY(manager, 0, true, 12, 20, 3)).toBe(3)
    expect(manager.hasHeightDataForGrid).not.toHaveBeenCalled()
  })

  it('does not snap dungeon items to outdoor terrain', () => {
    const manager = fakeHeightManager(true, 8.5)

    expect(groundItemBaseY(manager, -1, false, 12, 20, 3)).toBe(3)
    expect(manager.hasHeightDataForGrid).not.toHaveBeenCalled()
  })

  it('uses the stored height to reject an elevated bridge substrate', () => {
    const manager = fakeHeightManager(true, 2.5)
    const findDeck = vi
      .spyOn(bridgeManager, 'findDeckYAt')
      .mockImplementation((_x, _z, currentY) => (currentY === null ? 9 : null))

    expect(groundItemBaseY(manager, 0, false, 12, 20, 2)).toBe(2.5)
    expect(findDeck).toHaveBeenCalledWith(12, 20, 2)
  })

  it('updates from fallback when height tiles load without moving the item', async () => {
    const manager = new TerrainHeightManager()
    const heightmap = new Uint16Array(VERTS_PER_SIDE ** 2).fill(
      encodeHeight(8.5)
    )
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input)
        if (url.includes('height-original')) {
          return { ok: false, status: 404 }
        }
        return {
          ok: true,
          status: 200,
          arrayBuffer: async () => heightmap.buffer.slice(0),
        }
      })
    )

    const x = 12
    const z = 20
    const fallbackY = 3
    let renderedY = groundItemBaseY(manager, 0, false, x, z, fallbackY)
    const unsubscribe = manager.onHeightChanged(() => {
      renderedY = groundItemBaseY(manager, 0, false, x, z, fallbackY)
    })

    expect(renderedY).toBe(fallbackY)

    await Promise.all([
      manager.loadHeightmap(0, 0),
      manager.loadHeightmap(1, 0),
      manager.loadHeightmap(0, 1),
      manager.loadHeightmap(1, 1),
    ])

    expect(renderedY).toBe(8.5)
    unsubscribe()
  })
})
