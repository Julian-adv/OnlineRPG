import {
  clampPanelPos,
  panelLayout,
  panelZ,
  raisePanel,
  savePanelPos,
  type PanelId,
  type PanelPos,
} from '../stores/panelLayout'

const DRAG_THRESHOLD_SQ = 16

/**
 * Makes a `position: fixed` HUD panel draggable by its `[data-drag-handle]`
 * descendant, persisting position and stacking order.
 *
 * A press on a button inside the handle never starts a drag, so the existing
 * close/collapse buttons keep behaving exactly as before. The panel's CSS
 * position (including `right` and `translateY`) is only overridden once the
 * panel has actually been moved.
 */
export function draggablePanel(node: HTMLElement, id: PanelId) {
  const handle = node.querySelector<HTMLElement>('[data-drag-handle]')
  if (!handle) return

  handle.style.touchAction = 'none'
  handle.style.cursor = 'grab'
  // Otherwise dragging selects the title text instead of moving the panel.
  handle.style.userSelect = 'none'

  let stored: PanelPos | undefined
  let dragging = false

  function place(x: number, y: number) {
    node.style.left = `${x}px`
    node.style.top = `${y}px`
    node.style.right = 'auto'
    node.style.transform = 'none'
  }

  function apply() {
    if (dragging) return
    if (!stored) {
      node.style.left = ''
      node.style.top = ''
      node.style.right = ''
      node.style.transform = ''
      return
    }
    const rect = node.getBoundingClientRect()
    const { x, y } = clampPanelPos(
      stored,
      rect,
      window.innerWidth,
      window.innerHeight
    )
    place(x, y)
  }

  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0) return
    if ((e.target as HTMLElement).closest('button')) return

    raisePanel(id)
    // Anchor on the on-screen rect so handing over from the CSS defaults
    // (right / translateY) causes no visual jump.
    const rect = node.getBoundingClientRect()
    place(rect.left, rect.top)

    const startX = e.clientX
    const startY = e.clientY
    let moved = false
    let last: PanelPos = { x: rect.left, y: rect.top }

    function onMove(me: PointerEvent) {
      const dx = me.clientX - startX
      const dy = me.clientY - startY
      if (!moved && dx * dx + dy * dy < DRAG_THRESHOLD_SQ) return
      moved = true
      dragging = true
      last = clampPanelPos(
        { x: rect.left + dx, y: rect.top + dy },
        rect,
        window.innerWidth,
        window.innerHeight
      )
      // Compositor-only move: no layout pass while the 3D scene keeps rendering.
      node.style.transform = `translate3d(${last.x - rect.left}px, ${
        last.y - rect.top
      }px, 0)`
    }

    function onUp() {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onUp)
      dragging = false
      if (moved) {
        place(last.x, last.y)
        savePanelPos(id, last)
      }
    }

    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onUp)
  }

  const unsubscribe = panelLayout.subscribe((layout) => {
    stored = layout.pos[id]
    const z = panelZ(layout.order, id)
    node.style.zIndex = z === null ? '' : String(z)
    apply()
  })

  handle.addEventListener('pointerdown', onPointerDown)
  window.addEventListener('resize', apply)

  return {
    destroy() {
      unsubscribe()
      handle.removeEventListener('pointerdown', onPointerDown)
      window.removeEventListener('resize', apply)
    },
  }
}
