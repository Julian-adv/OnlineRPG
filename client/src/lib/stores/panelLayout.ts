import { get, writable } from 'svelte/store'

const STORAGE_KEY = 'hudPanelLayout'
export const PANEL_IDS = [
  'chat',
  'character',
  'inventory',
  'friends',
  'party',
  'emotes',
  'trade',
  'playerTrade',
] as const

export type PanelId = (typeof PANEL_IDS)[number]

export interface PanelPos {
  x: number
  y: number
}

export type PanelPositions = Partial<Record<PanelId, PanelPos>>

/** A dragged-off-screen panel keeps this much of itself grabbable. */
const MIN_VISIBLE_X = 48
const HEADER_H = 28
/** Where the consent toasts sit; ConsentToast.svelte reads it. The panel band
 *  ends just below, so adding an id shifts the whole band down on its own. */
export const PANEL_Z_CEILING = 44
const BASE_Z = PANEL_Z_CEILING - PANEL_IDS.length

export function parsePositions(raw: string | null): PanelPositions {
  if (!raw) return {}
  try {
    const data = JSON.parse(raw) as { pos?: PanelPositions }
    const pos: PanelPositions = {}
    for (const id of PANEL_IDS) {
      const p = data.pos?.[id]
      if (p && Number.isFinite(p.x) && Number.isFinite(p.y)) {
        pos[id] = { x: p.x, y: p.y }
      }
    }
    return pos
  } catch {
    return {}
  }
}

/**
 * Keeps the header row reachable; the rest of the panel may leave the viewport.
 * `rightReserve` is the non-grabbable width at the panel's right end (its
 * header buttons), which must stay off the visible sliver on the left edge.
 */
export function clampPanelPos(
  pos: PanelPos,
  size: { width: number; height: number },
  viewportWidth: number,
  viewportHeight: number,
  rightReserve = 0
): PanelPos {
  return {
    x: Math.min(
      Math.max(pos.x, MIN_VISIBLE_X + rightReserve - size.width),
      viewportWidth - MIN_VISIBLE_X
    ),
    y: Math.min(Math.max(pos.y, 0), Math.max(0, viewportHeight - HEADER_H)),
  }
}

/** `order` holds every mounted panel (the action raises on mount). */
export function panelZ(order: readonly PanelId[], id: PanelId): number {
  return BASE_Z + order.indexOf(id)
}

function load(): PanelPositions {
  try {
    return parsePositions(localStorage.getItem(STORAGE_KEY))
  } catch {
    return {}
  }
}

/** Persisted panel positions. */
export const panelPositions = writable<PanelPositions>(load())

let hydrated = false
panelPositions.subscribe((pos) => {
  if (!hydrated) {
    hydrated = true
    return
  }
  try {
    // {pos} envelope kept for back-compat with the legacy {pos, order} shape
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ pos }))
  } catch {
    // unavailable storage; the layout just won't persist
  }
})

/** Session-only raise order, bottom first. */
export const panelOrder = writable<PanelId[]>([])

/** Panel currently being dragged; its stored position is stale until release. */
export const draggingPanel = writable<PanelId | null>(null)

export function savePanelPos(id: PanelId, pos: PanelPos) {
  panelPositions.update((p) => ({ ...p, [id]: pos }))
}

export function raisePanel(id: PanelId) {
  if (get(panelOrder).at(-1) === id) return
  panelOrder.update((o) => [...o.filter((x) => x !== id), id])
}

export function resetPanelLayout() {
  panelPositions.set({})
}
