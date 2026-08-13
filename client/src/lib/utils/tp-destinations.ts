/**
 * Named /tp destinations for admins: hand-curated landmarks plus entries
 * derived from data (dungeon entrances and boss rooms from the registry +
 * shared layout generator, cities from the world-map labels), so the list
 * tracks content changes without hand-kept coordinates.
 */
import { dungeon_layout } from '../wasm/onlinerpg_shared'
import {
  dungeonCellCenter,
  type DungeonFloorLayout,
} from '../managers/dungeonManager'
import { DUNGEON_ENTRANCES } from '../data/dungeonDefs'
import { MAP_LABELS, type MapLabelKind } from '../data/mapLabels'
import worldJson from '../../../../data-src/world.json'

export interface TpDestination {
  name: string
  label: string
  x: number
  y: number
  z: number
}

/** Surface entries use y=0: the client snaps to terrain height on arrival. */
const STATIC_DESTINATIONS: TpDestination[] = [
  {
    name: 'spawn',
    label: 'Aldermark village (start)',
    x: worldJson.spawnPosition.x,
    y: 0,
    z: worldJson.spawnPosition.z,
  },
  {
    name: 'snowpeak',
    label: 'Snow mountain summit (1438m)',
    x: -1078,
    y: 0,
    z: 5067,
  },
]

const CITY_KINDS = new Set<MapLabelKind>(['capital', 'city', 'town'])

let cached: TpDestination[] | null = null

export function tpDestinations(): TpDestination[] {
  if (cached) return cached

  const out = [...STATIC_DESTINATIONS]

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
      ...dungeonCellCenter(e, last.depth, boss),
    })
  }

  for (const label of MAP_LABELS) {
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
