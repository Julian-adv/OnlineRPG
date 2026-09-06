import { afterEach, describe, expect, it, vi } from 'vitest'
import { TerrainSplatManager } from './terrainSplatManager'

vi.mock('../utils/networkUtils', () => ({
  getTerrainApiUrl: () => '',
  apiFetch: vi.fn(),
}))

afterEach(() => vi.unstubAllGlobals())

function splat(palette: number) {
  const data = new Uint8Array(64 * 64 * 4)
  for (let index = 0; index < data.length; index += 4)
    data[index] = palette << 4
  return data
}

function pendingResponse() {
  let resolve!: (response: Response) => void
  const promise = new Promise<Response>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

describe('live landscaping updates', () => {
  it('keeps a server update that arrives while the original tile is loading', async () => {
    const pending = pendingResponse()
    vi.stubGlobal(
      'fetch',
      vi.fn(() => pending.promise)
    )
    const manager = new TerrainSplatManager()
    const load = manager.loadSplatmap(0, 0)
    manager.setSplatmap(0, 0, splat(5))
    pending.resolve(new Response(splat(0)))
    await load
    expect(manager.getSplatData(0, 0)?.[0]).toBe(0x50)
    await manager.destroy()
  })

  it('refetches stale in-flight data after a distant edit notification', async () => {
    const pending = pendingResponse()
    const fetch = vi
      .fn()
      .mockReturnValueOnce(pending.promise)
      .mockResolvedValue(new Response(splat(5)))
    vi.stubGlobal('fetch', fetch)
    const manager = new TerrainSplatManager()
    const load = manager.loadSplatmap(0, 0)
    manager.invalidateLandscaping(0, 0)
    pending.resolve(new Response(splat(0)))
    await load
    expect(fetch).toHaveBeenCalledTimes(2)
    expect(manager.getSplatData(0, 0)?.[0]).toBe(0x50)
    await manager.destroy()
  })

  it('keeps unrelated tile requests when one tile is invalidated', async () => {
    const first = pendingResponse()
    const second = pendingResponse()
    const fetch = vi
      .fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
    vi.stubGlobal('fetch', fetch)
    const manager = new TerrainSplatManager()
    const load = manager.loadSplatmap(0, 0)
    await Promise.resolve()
    manager.invalidateLandscaping(1, 0)
    const other = manager.loadSplatmap(1, 0)
    first.resolve(new Response(splat(0)))
    second.resolve(new Response(splat(5)))
    await Promise.all([load, other])
    expect(fetch).toHaveBeenCalledTimes(2)
    expect(manager.getSplatData(0, 0)?.[0]).toBe(0)
    expect(manager.getSplatData(1, 0)?.[0]).toBe(0x50)
    await manager.destroy()
  })

  it('drops an old cached tile so returning players see the latest terrain', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(splat(5))))
    const manager = new TerrainSplatManager()
    manager.setSplatmap(0, 0, splat(0))
    manager.invalidateLandscaping(0, 0)
    await manager.loadSplatmap(0, 0)
    expect(manager.getSplatData(0, 0)?.[0]).toBe(0x50)
    await manager.destroy()
  })
})
