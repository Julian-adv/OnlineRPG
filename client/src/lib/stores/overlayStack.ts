import { get, writable, type Readable } from 'svelte/store'
import { characterPanelVisible, inventoryVisible } from './debugStore'
import { shopSession } from './tradeStore'

/** HUD overlays Escape interacts with. */
export type OverlayId =
  | 'worldMap'
  | 'character'
  | 'inventory'
  | 'trade'
  | 'settings'
  | 'loading'
  | 'respawn'

/** `layer` is paint order, not raw z-index: `.game-hud`'s z-index:1 stacking
 *  context traps the panels' 40/45 below the root-level dialogs (each 30,
 *  ranked by DOM order) and settings (10000).
 *  Store-backed panels close here; dialogs register their closer via
 *  mountOverlay, and one that registers none (loading) blocks Escape. */
const OVERLAYS: Record<OverlayId, { layer: number; close?: () => void }> = {
  character: { layer: 0, close: () => characterPanelVisible.set(false) },
  inventory: { layer: 0, close: () => inventoryVisible.set(false) },
  trade: { layer: 1, close: () => shopSession.set(null) },
  loading: { layer: 2 },
  respawn: { layer: 3 },
  worldMap: { layer: 4 },
  settings: { layer: 5 },
}

const stack = writable<OverlayId[]>([])

/** Open overlays, oldest first. */
export const openOverlays: Readable<OverlayId[]> = {
  subscribe: stack.subscribe,
}

/** Reopening moves an overlay back to the top of its layer. */
function track(id: OverlayId, open: boolean) {
  stack.update((entries) => {
    const rest = entries.filter((entry) => entry !== id)
    return open ? [...rest, id] : rest
  })
}

// Every open/close path writes these stores, so no call site can forget to report.
characterPanelVisible.subscribe((open) => track('character', open))
inventoryVisible.subscribe((open) => track('inventory', open))
shopSession.subscribe((session) => track('trade', session !== null))

const overlayClosers: Partial<Record<OverlayId, () => void>> = {}

/** For dialogs whose mount is their open state: call from an $effect and
 *  return the teardown. Omit close for dialogs Escape must not dismiss. */
export function mountOverlay(id: OverlayId, close?: () => void): () => void {
  if (close) overlayClosers[id] = close
  track(id, true)
  return () => {
    delete overlayClosers[id]
    track(id, false)
  }
}

/** Topmost layer wins; the most recently opened breaks a tie. */
export function topOverlay(open: OverlayId[]): OverlayId | undefined {
  let top: OverlayId | undefined
  for (const id of open) {
    if (top === undefined || OVERLAYS[id].layer >= OVERLAYS[top].layer) top = id
  }
  return top
}

/** Returns false when nothing was open — leaving Escape free for other uses —
 *  or when the top overlay cannot be closed. */
export function closeTopOverlay(): boolean {
  const top = topOverlay(get(stack))
  if (top === undefined) return false

  const close = overlayClosers[top] ?? OVERLAYS[top].close
  if (!close) return false
  close()
  return true
}
