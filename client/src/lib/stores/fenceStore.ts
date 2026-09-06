import { derived, get, writable } from 'svelte/store'
import { passability_set_fences } from '../wasm/onlinerpg_shared'
import { inventoryStore } from './inventoryStore'
import {
  landscapingMode,
  landscapingPending,
  stopLandscapingMode,
} from './landscapingStore'
import { wrapWorldX } from '../terrain/world-wrap'
import { fenceKey, type Fence, type FenceEdge } from '../terrain/fenceEdges'

export const fences = writable(new Map<string, Fence>())
export const fenceMode = derived(landscapingMode, (mode) =>
  mode?.tool === 'Fence' ? mode : null
)
export const fencePending = writable(false)
export const fenceError = writable<string | null>(null)
export const fenceTarget = writable<{
  edge: FenceEdge
  valid: boolean
  removing: boolean
  reason: string | null
} | null>(null)
export const fenceCount = derived(inventoryStore, (inventory) =>
  inventory.bag.reduce(
    (count, item) =>
      count + (item.item_def_id === 'wooden_fence' ? item.quantity : 0),
    0
  )
)

export function applyFenceVisibility(added: Fence[], removed: FenceEdge[]) {
  const next = new Map(get(fences))
  for (const edge of removed) next.delete(fenceKey(edge))
  for (const fence of added) next.set(fenceKey(fence.edge), fence)
  passability_set_fences([...next.values()])
  fences.set(next)
}

export function refreshFenceHeights(
  sampleHeight: (x: number, z: number) => number | null
) {
  const changed: Fence[] = []
  for (const fence of get(fences).values()) {
    const heights = [0, 0.5, 1].map((t) =>
      sampleHeight(
        wrapWorldX(fence.edge.x + (fence.edge.axis === 'X' ? t : 0)),
        fence.edge.z + (fence.edge.axis === 'Z' ? t : 0)
      )
    )
    if (heights.some((y) => y === null)) continue
    const y = Math.min(...(heights as number[]))
    if (y !== fence.y) changed.push({ ...fence, y })
  }
  if (changed.length) applyFenceVisibility(changed, [])
}

export function stopFenceMode() {
  stopLandscapingMode()
  fenceTarget.set(null)
  fenceError.set(null)
}

export function resetFences() {
  if (get(fences).size) passability_set_fences([])
  fences.set(new Map())
  fencePending.set(false)
  landscapingPending.set(false)
  stopFenceMode()
}
