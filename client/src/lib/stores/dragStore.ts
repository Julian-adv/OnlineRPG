import { writable } from 'svelte/store'
import type { EquipSlot } from '../network/networkTypes'
import { assignQuickslot } from './quickslotStore'

export const FALLBACK_ICON = 'icon_frame.png'

export type DragMeta = {
  instanceId: number
  defId: string
  enchant: number
  equipSlot: EquipSlot | null
  source: { type: 'bag' } | { type: 'equipped'; slot: EquipSlot }
  icon: string
  /** Set when dragging a multi-item Select-mode selection instead of a
   *  single slot, so the ghost can show one icon + quantity per item
   *  instead of the single main icon. */
  groupItems?: { icon: string; quantity: number }[]
}

export const dragMeta = writable<DragMeta | null>(null)
export const dragPos = writable({ x: 0, y: 0 })

export function isSlotCompatible(
  itemSlot: EquipSlot | null,
  targetSlot: EquipSlot
): boolean {
  if (!itemSlot) return false
  if (itemSlot === targetSlot) return true
  if (itemSlot === 'ring' && targetSlot === 'ring_left') return true
  return false
}

export function pointInRect(x: number, y: number, r: DOMRect): boolean {
  return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
}

/** Grow a rect outward by `m` px on every side (for forgiving hit-testing). */
export function inflateRect(r: DOMRect, m: number): DOMRect {
  return new DOMRect(r.x - m, r.y - m, r.width + 2 * m, r.height + 2 * m)
}

/**
 * The quickslot index under the pointer, or -1. Treats the whole bar (incl.
 * gaps, with a little slack) as one drop zone and snaps to the nearest slot by
 * 2D distance — so a multi-row bar targets the right row, not just the right
 * column. Shared by the drag highlight and the drop handler so they agree.
 */
export function quickslotAt(x: number, y: number): number {
  // Rect-test the bar before listing the slots: this runs on every pointermove
  // and usually misses.
  const bar =
    document.querySelector<HTMLElement>('[data-quickslot]')?.parentElement
  if (
    !bar ||
    !pointInRect(x, y, inflateRect(bar.getBoundingClientRect(), 12))
  ) {
    return -1
  }
  let best = -1
  let bestDist = Infinity
  for (const el of bar.querySelectorAll<HTMLElement>('[data-quickslot]')) {
    const r = el.getBoundingClientRect()
    const dx = (r.left + r.right) / 2 - x
    const dy = (r.top + r.bottom) / 2 - y
    const dist = dx * dx + dy * dy
    if (dist < bestDist) {
      bestDist = dist
      best = Number(el.dataset.quickslot)
    }
  }
  return best
}

export function isOverAnyDialog(x: number, y: number): boolean {
  for (const dialog of document.querySelectorAll('[role="dialog"]')) {
    if (pointInRect(x, y, dialog.getBoundingClientRect())) return true
  }
  return false
}

const DRAG_THRESHOLD_SQ = 64

export function startDrag(
  e: PointerEvent,
  meta: DragMeta,
  onDrop: (x: number, y: number) => void,
  /** Fired instead of `onDrop` when the pointer never moved past the drag
   *  threshold (a plain tap/click). Callers must not *also* bind a native
   *  `click` handler for this: pointer capture keeps the browser's own click
   *  event targeted at this element even after a real drag ends elsewhere,
   *  so a separate click listener fires a spurious extra toggle right after
   *  a completed drag — this callback is gated on this gesture's own
   *  `started` flag instead, which never has that false positive. */
  onClick?: () => void
) {
  const target = e.currentTarget as HTMLElement
  target.setPointerCapture(e.pointerId)
  const startX = e.clientX
  const startY = e.clientY
  let started = false
  const pos = { x: 0, y: 0 }

  function onMove(me: PointerEvent) {
    me.preventDefault()
    const dx = me.clientX - startX
    const dy = me.clientY - startY
    if (!started && dx * dx + dy * dy < DRAG_THRESHOLD_SQ) return
    if (!started) {
      started = true
      dragMeta.set(meta)
    }
    pos.x = me.clientX
    pos.y = me.clientY
    dragPos.set(pos)
  }

  function removeListeners() {
    target.removeEventListener('pointermove', onMove)
    target.removeEventListener('pointerup', onEnd)
    target.removeEventListener('pointercancel', onEnd)
    target.removeEventListener('lostpointercapture', onLostCapture)
  }

  function onEnd(ue: PointerEvent) {
    removeListeners()
    if (target.hasPointerCapture(ue.pointerId)) {
      target.releasePointerCapture(ue.pointerId)
    }
    if (ue.type !== 'pointercancel') {
      if (started) {
        // Quickslot drops work from every drag source and win over the
        // source's own targets, so resolve them here — the bar's highlight
        // uses the same test, and no call site can implement (or forget) it
        // differently.
        const qsIndex = quickslotAt(ue.clientX, ue.clientY)
        if (qsIndex < 0) {
          onDrop(ue.clientX, ue.clientY)
        } else if (meta.groupItems === undefined) {
          assignQuickslot(qsIndex, { defId: meta.defId, enchant: meta.enchant })
        }
      } else {
        onClick?.()
      }
    }
    dragMeta.set(null)
  }

  function onLostCapture() {
    removeListeners()
    dragMeta.set(null)
  }

  target.addEventListener('pointermove', onMove)
  target.addEventListener('pointerup', onEnd)
  target.addEventListener('pointercancel', onEnd)
  target.addEventListener('lostpointercapture', onLostCapture)
}
