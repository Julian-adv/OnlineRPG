import { get, writable } from 'svelte/store'
import {
  characterPanelVisible,
  inventoryVisible,
  settingsVisible,
  worldMapVisible,
} from './debugStore'
import { shopSession } from './tradeStore'

/** HUD overlays Escape closes. */
export type OverlayId =
  | 'worldMap'
  | 'character'
  | 'inventory'
  | 'trade'
  | 'settings'

/** Escape closes whatever is actually on top, which open order alone cannot
 *  tell us: the panel keys still fire behind a modal, so a bag opened under
 *  the settings overlay would otherwise be the one to go.
 *
 *  `layer` is paint order, not raw z-index. The panels sit inside `.game-hud`,
 *  whose own `z-index: 1` traps their 40/45 in its stacking context, while the
 *  world map (30) and settings (10000) are siblings of that div — so the map
 *  paints over the trade window despite the smaller number. */
const OVERLAYS: Record<OverlayId, { layer: number; close: () => void }> = {
  character: { layer: 0, close: () => characterPanelVisible.set(false) },
  inventory: { layer: 0, close: () => inventoryVisible.set(false) },
  trade: { layer: 1, close: () => shopSession.set(null) },
  worldMap: { layer: 2, close: () => worldMapVisible.set(false) },
  settings: { layer: 3, close: () => settingsVisible.set(false) },
}

/** Open overlays, oldest first. */
export const openOverlays = writable<OverlayId[]>([])

/** Reopening an overlay moves it back to the top of its layer. */
export function withOverlay(open: OverlayId[], id: OverlayId): OverlayId[] {
  return [...open.filter((entry) => entry !== id), id]
}

export function withoutOverlay(open: OverlayId[], id: OverlayId): OverlayId[] {
  return open.filter((entry) => entry !== id)
}

/** Topmost layer wins; the most recently opened breaks a tie. */
export function topOverlay(open: OverlayId[]): OverlayId | undefined {
  let top: OverlayId | undefined
  for (const id of open) {
    if (top === undefined || OVERLAYS[id].layer >= OVERLAYS[top].layer) top = id
  }
  return top
}

/** Derived from the state each overlay already keeps rather than from the
 *  toggle call sites: keys, HUD buttons and the panels' own close buttons all
 *  write the same stores, so watching them keeps the order right without every
 *  caller remembering to report. */
function track(id: OverlayId, open: boolean) {
  openOverlays.update((entries) =>
    open ? withOverlay(entries, id) : withoutOverlay(entries, id)
  )
}

worldMapVisible.subscribe((open) => track('worldMap', open))
characterPanelVisible.subscribe((open) => track('character', open))
inventoryVisible.subscribe((open) => track('inventory', open))
settingsVisible.subscribe((open) => track('settings', open))
shopSession.subscribe((session) => track('trade', session !== null))

const overlayClosers: Partial<Record<OverlayId, () => void>> = {}

/** Overlays whose close does more than reset their state register it here, so
 *  Escape runs the same teardown their own close button does. Pass null on
 *  teardown. */
export function setOverlayCloser(id: OverlayId, close: (() => void) | null) {
  if (close) overlayClosers[id] = close
  else delete overlayClosers[id]
}

/** Closes the topmost overlay. Returns false when none was open, leaving
 *  Escape free for whatever else wants it. */
export function closeTopOverlay(): boolean {
  const top = topOverlay(get(openOverlays))
  if (top === undefined) return false

  const close = overlayClosers[top] ?? OVERLAYS[top].close
  close()
  return true
}
