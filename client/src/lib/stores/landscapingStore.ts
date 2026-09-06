import { derived, get, writable } from 'svelte/store'
import type { FencePlot } from '../terrain/fenceEdges'
import type { LandscapingTool } from '../terrain/landscaping'
import { inventoryStore } from './inventoryStore'
import { splatLayer } from './editorStore'

export interface LandscapingMode {
  owner_id: number
  plots: FencePlot[]
  palette: number[]
  has_toolbox: boolean
  tool: LandscapingTool
}

export const landscapingMode = writable<LandscapingMode | null>(null)
export const landscapingPending = writable(false)
export const landscapingError = writable<string | null>(null)
export const landscapingHint = writable<string | null>(null)
export const landscapingRoadStart = writable<[number, number] | null>(null)
export const hasLandscapingToolbox = derived(inventoryStore, (inventory) =>
  inventory.bag.some(
    (item) => item.item_def_id === 'landscaping_toolbox' && item.quantity > 0
  )
)

export function openLandscapingMode(mode: LandscapingMode) {
  landscapingError.set(null)
  landscapingHint.set(null)
  landscapingRoadStart.set(null)
  if (!mode.palette.includes(get(splatLayer)))
    splatLayer.set(mode.palette[0] ?? 0)
  landscapingMode.set(mode)
}

export function selectLandscapingTool(tool: LandscapingTool) {
  landscapingMode.update((mode) => {
    if (!mode || (tool !== 'Fence' && !get(hasLandscapingToolbox))) return mode
    return { ...mode, tool }
  })
  landscapingRoadStart.set(null)
  landscapingError.set(null)
  landscapingHint.set(null)
}

export function stopLandscapingMode() {
  landscapingMode.set(null)
  landscapingRoadStart.set(null)
  landscapingError.set(null)
  landscapingHint.set(null)
}
