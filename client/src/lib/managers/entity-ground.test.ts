import { afterEach, describe, expect, it, vi } from 'vitest'
import { bridgeManager } from './bridgeManager'
import { TerrainHeightManager } from './terrainHeightManager'
import { VERTS_PER_SIDE, encodeHeight } from './terrain-height-types'
import { entityGroundY } from './entity-ground'

function fakeHeightManager(
  loaded: boolean,
  height: number
): TerrainHeightManager {
  return {
    hasHeightDataForGrid: vi.fn(() => loaded),
    getHeightAtWorldPosition: vi.fn(() => height),
  } as unknown as TerrainHeightManager
}

describe('entityGroundY', () => {
  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('uses loaded terrain at the query coordinates', () => {
    const manager = fakeHeightManager(true, 8.5)

    expect(entityGroundY(manager, 0, 12, 20, 3)).toBe(8.5)
    expect(manager.hasHeightDataForGrid).toHaveBeenCalledWith(12, 20)
    expect(manager.getHeightAtWorldPosition).toHaveBeenCalledWith(12, 20)
  })

  it('keeps the fallback until terrain data is loaded', () => {
    const manager = fakeHeightManager(false, 8.5)

    expect(entityGroundY(manager, 0, 12, 20, 3)).toBe(3)
    expect(manager.getHeightAtWorldPosition).not.toHaveBeenCalled()
  })

  it('does not snap dungeon entities to outdoor terrain', () => {
    const manager = fakeHeightManager(true, 8.5)

    expect(entityGroundY(manager, -1, 12, 20, 3)).toBe(3)
    expect(manager.hasHeightDataForGrid).not.toHaveBeenCalled()
  })

  it('passes the reference Y through to the bridge deck query', () => {
    const manager = fakeHeightManager(true, 2.5)
    const findDeck = vi
      .spyOn(bridgeManager, 'findDeckYAt')
      .mockReturnValue(null)

    expect(entityGroundY(manager, 0, 12, 20, 2, 2)).toBe(2.5)
    expect(findDeck).toHaveBeenCalledWith(12, 20, 2)
  })

  it('updates from fallback when height tiles load', async () => {
    const manager = new TerrainHeightManager()
    const heightmap = new Uint16Array(VERTS_PER_SIDE ** 2).fill(
      encodeHeight(8.5)
    )
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => ({
        ok: true,
        status: 200,
        arrayBuffer: async () => heightmap.buffer.slice(0),
      }))
    )

    const x = 12
    const z = 20
    const fallbackY = 3
    let renderedY = entityGroundY(manager, 0, x, z, fallbackY)
    const unsubscribe = manager.onHeightChanged(() => {
      renderedY = entityGroundY(manager, 0, x, z, fallbackY)
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
