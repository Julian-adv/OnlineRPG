import { apiFetch } from '../utils/networkUtils'
import { tileKey, type TerrainHeightState } from './terrain-height-types'

export async function loadHeightmap(
  state: TerrainHeightState,
  inflightHeightmaps: Map<string, Promise<Uint16Array>>,
  terrainApiUrl: string,
  tileX: number,
  tileZ: number,
  loadOriginal: (tileX: number, tileZ: number) => void,
  onFreshLoad?: () => void
): Promise<Uint16Array> {
  const key = tileKey(tileX, tileZ)
  const cached = state.heightmaps.get(key)
  if (cached) return cached

  const inflight = inflightHeightmaps.get(key)
  if (inflight) return inflight

  const promise = (async () => {
    try {
      const url = `${terrainApiUrl}/api/terrain/height/${tileX}/${tileZ}`
      const response = await fetch(url)
      if (!response.ok) {
        throw new Error(
          `HTTP ${response.status} for heightmap (${tileX}, ${tileZ})`
        )
      }
      const buffer = await response.arrayBuffer()
      const data = new Uint16Array(buffer)
      state.heightmaps.set(key, data)
      loadOriginal(tileX, tileZ)
      onFreshLoad?.()
      return data
    } catch (e) {
      console.error(`Failed to load heightmap (${tileX}, ${tileZ}):`, e)
      throw e
    } finally {
      inflightHeightmaps.delete(key)
    }
  })()
  inflightHeightmaps.set(key, promise)
  return promise
}

export async function loadOriginalHeightmap(
  state: TerrainHeightState,
  terrainApiUrl: string,
  tileX: number,
  tileZ: number
): Promise<Uint16Array | null> {
  const key = tileKey(tileX, tileZ)
  if (state.originalHeightmaps.has(key))
    return state.originalHeightmaps.get(key)!
  if (state.missingOriginalTiles.has(key)) return null
  try {
    const url = `${terrainApiUrl}/api/terrain/height-original/${tileX}/${tileZ}`
    const response = await fetch(url)
    if (response.status === 404) {
      state.missingOriginalTiles.add(key)
      return null
    }
    if (!response.ok) return null
    const buffer = await response.arrayBuffer()
    const data = new Uint16Array(buffer)
    state.originalHeightmaps.set(key, data)
    return data
  } catch {
    return null
  }
}

export function ensureOriginalHeightmap(
  state: TerrainHeightState,
  terrainApiUrl: string,
  tileX: number,
  tileZ: number
): void {
  const key = tileKey(tileX, tileZ)
  if (state.originalHeightmaps.has(key)) return
  const current = state.heightmaps.get(key)
  if (!current) return
  state.originalHeightmaps.set(key, new Uint16Array(current))
  state.missingOriginalTiles.delete(key)
  apiFetch(
    `${terrainApiUrl}/api/terrain/height-original/${tileX}/${tileZ}/ensure`,
    { method: 'POST' }
  ).catch(() => {})
}

async function saveTileSet(
  dirtySet: Set<string>,
  dataMap: Map<string, Uint16Array>,
  terrainApiUrl: string,
  urlSegment: string
) {
  const keys = [...dirtySet]
  for (const key of keys) {
    const data = dataMap.get(key)
    if (!data) {
      dirtySet.delete(key)
      continue
    }

    const [txStr, tzStr] = key.split(',')
    const tx = parseInt(txStr)
    const tz = parseInt(tzStr)

    const url = `${terrainApiUrl}/api/terrain/${urlSegment}/${tx}/${tz}`
    const body = (data.buffer as ArrayBuffer).slice(
      data.byteOffset,
      data.byteOffset + data.byteLength
    )

    try {
      await apiFetch(url, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/octet-stream' },
        body,
      })
      dirtySet.delete(key)
    } catch (e) {
      console.error(`Failed to save ${urlSegment} for tile (${tx}, ${tz}):`, e)
    }
  }
}

export async function saveDirtyTiles(
  state: TerrainHeightState,
  terrainApiUrl: string
) {
  await saveTileSet(state.dirtyTiles, state.heightmaps, terrainApiUrl, 'height')
  await saveTileSet(
    state.dirtyOriginalTiles,
    state.originalHeightmaps,
    terrainApiUrl,
    'height-original'
  )
}
