/**
 * Named /tp destinations for admins. Static landmarks plus entries derived
 * from data: dungeon entrances and final-floor boss rooms come from the
 * registry + shared wasm layout generator, cities from the world-map labels —
 * so the list tracks content changes without hand-kept coordinates.
 */
import { dungeon_constants, dungeon_layout } from '../wasm/onlinerpg_shared'
import type {
  DungeonConstants,
  DungeonFloorLayout,
} from '../managers/dungeonManager'
import { DUNGEON_ENTRANCES } from '../data/dungeonDefs'
import mapLabelsJson from '../../../../data/map_labels.json'

export interface TpDestination {
  name: string
  label: string
  x: number
  y: number
  z: number
}

interface MapLabel {
  id: string
  name: string
  kind: string
  x: number
  z: number
}

/** Surface entries use y=0: the client snaps to terrain height on arrival. */
const STATIC_DESTINATIONS: TpDestination[] = [
  {
    name: 'spawn',
    label: 'Aldermark village (start)',
    x: -1475.2,
    y: 0,
    z: 4741.6,
  },
  {
    name: 'snowpeak',
    label: 'Snow mountain summit (1438m)',
    x: -1078,
    y: 0,
    z: 5067,
  },
]

const CITY_KINDS = new Set(['capital', 'city', 'town'])

let cached: TpDestination[] | null = null

export function tpDestinations(): TpDestination[] {
  if (cached) return cached

  const out = [...STATIC_DESTINATIONS]
  const consts = dungeon_constants() as DungeonConstants

  for (const e of DUNGEON_ENTRANCES) {
    const short = e.id.split('_').pop() ?? e.id
    out.push({
      name: short,
      label: `${e.name} entrance`,
      x: e.x,
      y: 0,
      z: e.z,
    })

    const layouts = dungeon_layout(e.id) as DungeonFloorLayout[]
    const last = layouts[layouts.length - 1]
    const boss = last?.spawns.find((s) => s.isBoss)
    if (!last || !boss) continue
    out.push({
      name: `${short}-boss`,
      label: `${e.name} floor ${last.depth} boss room`,
      x: Math.floor(e.x) - consts.grid / 2 + boss.x + 0.5,
      y: e.y - last.depth * consts.floorHeight,
      z: Math.floor(e.z) - consts.grid / 2 + boss.z + 0.5,
    })
  }

  for (const label of Object.values(
    mapLabelsJson as unknown as Record<string, MapLabel>
  )) {
    // Aldermark duplicates the spawn entry.
    if (!CITY_KINDS.has(label.kind) || label.id === 'aldermark') continue
    out.push({
      name: label.id,
      label: `${label.name} (${label.kind})`,
      x: label.x,
      y: 0,
      z: label.z,
    })
  }

  cached = out
  return out
}
