import { apiFetch, getTerrainApiUrl } from '../utils/networkUtils'

export interface NpcScheduleEntry {
  at: string
  pos: [number, number, number]
  rotation: number
  floor_level: number
  label?: string
  action?: string
  object_id?: number
  waypoints: [number, number, number][]
}

export interface NpcScheduleData {
  schedule: NpcScheduleEntry[]
}

export class NpcScheduleManager {
  private baseUrl: string

  constructor() {
    this.baseUrl = getTerrainApiUrl()
  }

  async listNpcs(): Promise<string[]> {
    const res = await fetch(`${this.baseUrl}/api/npcs`)
    if (!res.ok) throw new Error(`Failed to list NPCs: ${res.status}`)
    return res.json()
  }

  async fetchSchedule(name: string): Promise<NpcScheduleData> {
    const res = await fetch(
      `${this.baseUrl}/api/npcs/${encodeURIComponent(name)}/schedule`
    )
    if (!res.ok)
      throw new Error(`Failed to fetch schedule for '${name}': ${res.status}`)
    return res.json()
  }

  async saveSchedule(name: string, data: NpcScheduleData): Promise<void> {
    const res = await apiFetch(
      `${this.baseUrl}/api/npcs/${encodeURIComponent(name)}/schedule`,
      {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
      }
    )
    if (!res.ok)
      throw new Error(`Failed to save schedule for '${name}': ${res.status}`)
  }
}
