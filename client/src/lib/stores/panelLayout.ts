import { writable } from 'svelte/store'

export type PanelId = 'character' | 'inventory' | 'friends' | 'party'

export interface PanelPos {
  x: number
  y: number
}

export interface PanelLayout {
  pos: Partial<Record<PanelId, PanelPos>>
  /** Raise order, bottom first. Only raised panels get an inline z-index. */
  order: PanelId[]
}

const STORAGE_KEY = 'hudPanelLayout'
const PANEL_IDS: PanelId[] = ['character', 'inventory', 'friends', 'party']

/** A dragged-off-screen panel keeps this much of itself grabbable. */
const MIN_VISIBLE_X = 48
const HEADER_H = 28
/** 41..44 stays under the trade windows' z-index 45. */
const BASE_Z = 41

const EMPTY: PanelLayout = { pos: {}, order: [] }

export function parseLayout(raw: string | null): PanelLayout {
  if (!raw) return EMPTY
  try {
    const data = JSON.parse(raw) as Partial<PanelLayout>
    const pos: PanelLayout['pos'] = {}
    for (const id of PANEL_IDS) {
      const p = data.pos?.[id]
      if (p && Number.isFinite(p.x) && Number.isFinite(p.y)) {
        pos[id] = { x: p.x, y: p.y }
      }
    }
    const order = Array.isArray(data.order)
      ? data.order.filter(
          (id, i, all) => PANEL_IDS.includes(id) && all.indexOf(id) === i
        )
      : []
    return { pos, order }
  } catch {
    return EMPTY
  }
}

/** Keeps the header row reachable; the rest of the panel may leave the viewport. */
export function clampPanelPos(
  pos: PanelPos,
  size: { width: number; height: number },
  viewportWidth: number,
  viewportHeight: number
): PanelPos {
  return {
    x: Math.min(
      Math.max(pos.x, MIN_VISIBLE_X - size.width),
      viewportWidth - MIN_VISIBLE_X
    ),
    y: Math.min(Math.max(pos.y, 0), Math.max(0, viewportHeight - HEADER_H)),
  }
}

/** null for a panel that was never raised: it keeps its CSS z-index. */
export function panelZ(order: PanelId[], id: PanelId): number | null {
  const index = order.indexOf(id)
  return index < 0 ? null : BASE_Z + index
}

function load(): PanelLayout {
  try {
    return parseLayout(localStorage.getItem(STORAGE_KEY))
  } catch {
    return EMPTY
  }
}

export const panelLayout = writable<PanelLayout>(load())

panelLayout.subscribe((layout) => {
  try {
    if (layout.order.length === 0 && Object.keys(layout.pos).length === 0) {
      localStorage.removeItem(STORAGE_KEY)
    } else {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(layout))
    }
  } catch {
    // unavailable storage; the layout just won't persist
  }
})

export function savePanelPos(id: PanelId, pos: PanelPos) {
  panelLayout.update((l) => ({ ...l, pos: { ...l.pos, [id]: pos } }))
}

export function raisePanel(id: PanelId) {
  panelLayout.update((l) =>
    l.order.at(-1) === id
      ? l
      : { ...l, order: [...l.order.filter((o) => o !== id), id] }
  )
}

export function resetPanelLayout() {
  panelLayout.set(EMPTY)
}
