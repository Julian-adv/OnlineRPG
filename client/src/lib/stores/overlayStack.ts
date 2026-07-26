import { get, writable } from 'svelte/store'
import type { Writable } from 'svelte/store'
import {
  characterPanelVisible,
  inventoryVisible,
  worldMapVisible,
} from './debugStore'

/** HUD overlays Escape closes. */
export type OverlayId = 'character' | 'inventory' | 'worldMap'

const overlayStores: Record<OverlayId, Writable<boolean>> = {
  character: characterPanelVisible,
  inventory: inventoryVisible,
  worldMap: worldMapVisible,
}

/** Open overlays, oldest first. Escape closes the last entry, so stacked
 *  panels come down one press at a time instead of all at once. */
export const openOverlays = writable<OverlayId[]>([])

/** Reopening an overlay moves it back to the top, so it is the next to go. */
export function withOverlay(open: OverlayId[], id: OverlayId): OverlayId[] {
  return [...open.filter((entry) => entry !== id), id]
}

export function withoutOverlay(open: OverlayId[], id: OverlayId): OverlayId[] {
  return open.filter((entry) => entry !== id)
}

/** Derived from the visibility stores rather than the toggle call sites: keys,
 *  HUD buttons and panel close buttons all flip the same stores, so watching
 *  them keeps the order right without every caller remembering to report. */
for (const id of Object.keys(overlayStores) as OverlayId[]) {
  overlayStores[id].subscribe((visible) => {
    openOverlays.update((open) =>
      visible ? withOverlay(open, id) : withoutOverlay(open, id)
    )
  })
}

const overlayClosers: Partial<Record<OverlayId, () => void>> = {}

/** Overlays whose close does more than flip their store register it here, so
 *  Escape runs the same teardown their own close button does. Pass null on
 *  teardown. */
export function setOverlayCloser(id: OverlayId, close: (() => void) | null) {
  if (close) overlayClosers[id] = close
  else delete overlayClosers[id]
}

/** Closes the most recently opened overlay. Returns false when none was open,
 *  leaving Escape free for whatever else wants it. */
export function closeTopOverlay(): boolean {
  const open = get(openOverlays)
  const top = open[open.length - 1]
  if (top === undefined) return false

  const close = overlayClosers[top]
  if (close) close()
  else overlayStores[top].set(false)
  return true
}
