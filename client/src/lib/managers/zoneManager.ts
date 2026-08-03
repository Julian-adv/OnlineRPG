import { apiFetch, getTerrainApiUrl } from '../utils/networkUtils'
import { regionKey } from '../terrain/terrain-constants'

export interface NoSpawnZone {
  minX: number
  minZ: number
  maxX: number
  maxZ: number
  label?: string
}

export interface MonsterSpawnZone {
  monsterType: string
  maxPerPlayer: number
  maxTotal?: number
  spawnIntervalSecs: number
  minX: number
  minZ: number
  maxX: number
  maxZ: number
}

export interface ZoneData {
  monsterSpawns?: MonsterSpawnZone[]
  noSpawnZones?: NoSpawnZone[]
}

export class ZoneManager {
  private cache = new Map<string, ZoneData>()
  private terrainApiUrl: string

  constructor() {
    this.terrainApiUrl = getTerrainApiUrl()
  }

  async fetchZone(rx: number, rz: number): Promise<ZoneData> {
    const key = regionKey(rx, rz)
    const cached = this.cache.get(key)
    if (cached) return cached

    try {
      const resp = await fetch(
        `${this.terrainApiUrl}/api/terrain/zones/${rx}/${rz}`
      )
      const json = await resp.json()
      const data: ZoneData = {
        monsterSpawns: json.monsterSpawns ?? [],
        noSpawnZones: json.noSpawnZones ?? [],
      }
      this.cache.set(key, data)
      return data
    } catch {
      const data: ZoneData = { monsterSpawns: [], noSpawnZones: [] }
      this.cache.set(key, data)
      return data
    }
  }

  async saveZone(rx: number, rz: number, data: ZoneData): Promise<void> {
    await apiFetch(`${this.terrainApiUrl}/api/terrain/zones/${rx}/${rz}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(data),
    })
    this.cache.set(regionKey(rx, rz), data)
  }

  getCached(rx: number, rz: number): ZoneData | null {
    return this.cache.get(regionKey(rx, rz)) ?? null
  }

  invalidate(rx: number, rz: number): void {
    this.cache.delete(regionKey(rx, rz))
  }
}
