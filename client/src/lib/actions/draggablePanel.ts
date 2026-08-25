import {
  clampPanelPos,
  draggingPanel,
  panelOrder,
  panelPositions,
  panelZ,
  raisePanel,
  savePanelPos,
  type PanelId,
  type PanelPos,
} from '../stores/panelLayout'

const DRAG_THRESHOLD_SQ = 16

/** Drags a `position: fixed` panel by its `[data-drag-handle]`; persists position, tracks stacking. */
export function draggablePanel(node: HTMLElement, id: PanelId) {
  const handle = node.querySelector<HTMLElement>('[data-drag-handle]')
  if (!handle) return

  handle.style.touchAction = 'none'
  handle.style.cursor = 'grab'
  handle.style.userSelect = 'none'
  // Header buttons trail the title; they must never be the only part left on screen.
  const firstButton = handle.querySelector('button')

  let stored: PanelPos | undefined
  let endDrag: (() => void) | null = null

  function place(x: number, y: number) {
    node.style.inset = `${y}px auto auto ${x}px`
    node.style.transform = 'none'
  }

  function rightReserve(rect: DOMRect) {
    return firstButton
      ? rect.right - firstButton.getBoundingClientRect().left
      : 0
  }

  function apply() {
    if (endDrag) return
    if (!stored) {
      node.style.inset = ''
      node.style.transform = ''
      return
    }
    const rect = node.getBoundingClientRect()
    const { x, y } = clampPanelPos(
      stored,
      rect,
      window.innerWidth,
      window.innerHeight,
      rightReserve(rect)
    )
    place(x, y)
  }

  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0 || endDrag) return
    if ((e.target as HTMLElement).closest('button')) return

    raisePanel(id)
    const rect = node.getBoundingClientRect()
    const reserve = rightReserve(rect)
    const vw = window.innerWidth
    const vh = window.innerHeight
    const pointerId = e.pointerId
    const startX = e.clientX
    const startY = e.clientY
    let moved = false
    let last: PanelPos = { x: rect.left, y: rect.top }

    function onMove(me: PointerEvent) {
      if (me.pointerId !== pointerId) return
      const dx = me.clientX - startX
      const dy = me.clientY - startY
      if (!moved) {
        if (dx * dx + dy * dy < DRAG_THRESHOLD_SQ) return
        moved = true
        draggingPanel.set(id)
        // Hand over from the CSS defaults (right / translateY) without a jump.
        place(rect.left, rect.top)
      }
      last = clampPanelPos(
        { x: rect.left + dx, y: rect.top + dy },
        rect,
        vw,
        vh,
        reserve
      )
      // Compositor-only move: no layout pass while the 3D scene keeps rendering.
      node.style.transform = `translate3d(${last.x - rect.left}px, ${
        last.y - rect.top
      }px, 0)`
    }

    function onUp(ue: PointerEvent) {
      if (ue.pointerId !== pointerId) return
      stop()
      if (moved) savePanelPos(id, last)
    }

    function stop() {
      if (moved) draggingPanel.set(null)
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onUp)
      endDrag = null
    }

    endDrag = stop
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onUp)
  }

  raisePanel(id)
  const unsubOrder = panelOrder.subscribe((order) => {
    node.style.zIndex = String(panelZ(order, id))
  })
  const unsubPos = panelPositions.subscribe((pos) => {
    stored = pos[id]
    apply()
  })

  handle.addEventListener('pointerdown', onPointerDown)
  window.addEventListener('resize', apply)

  return {
    destroy() {
      endDrag?.()
      unsubOrder()
      unsubPos()
      handle.removeEventListener('pointerdown', onPointerDown)
      window.removeEventListener('resize', apply)
    },
  }
}
