import { mount, unmount } from 'svelte'
import { get } from 'svelte/store'
import ItemTooltip from '../components/ItemTooltip.svelte'
import { dragMeta } from '../stores/dragStore'
import { displacedByEquip } from '../stores/inventoryStore'
import { getItemDef, type ItemDefinition } from '../data/itemDefs'
import type { ItemInstance } from '../network/networkTypes'

export interface ItemTooltipParams {
  def: ItemDefinition
  /** The hovered instance, when one exists — supplies per-instance display
   * data such as the +N enchant. Omit for def-only surfaces (shop catalog). */
  item?: ItemInstance
  /** Enchant to show when no instance backs the surface (e.g. a quickslot
   * whose bound item is depleted). Ignored when `item` is present. */
  enchant?: number
  side?: 'left' | 'right'
}

/**
 * Shows an ItemTooltip next to the element while hovered. Pass `null` to
 * disable (e.g. an empty inventory slot).
 *
 * The tooltip is mounted at document.body so ancestor overflow/transform
 * containing blocks cannot clip it. The anchor rect is measured once on
 * mouseenter, so the tooltip hides on any scroll/resize (rect goes stale)
 * and while an item drag is in progress.
 */
export function itemTooltip(
  node: HTMLElement,
  params: ItemTooltipParams | null
) {
  let instance: object | null = null
  let unsubDrag: (() => void) | null = null

  function show() {
    if (!params || instance || get(dragMeta)) return
    const displaced = displacedByEquip(params.def, params.item)
    const displacedDef = displaced && getItemDef(displaced.item_def_id)
    instance = mount(ItemTooltip, {
      target: document.body,
      props: {
        def: params.def,
        enchant: params.item?.enchant ?? params.enchant,
        compare: displacedDef
          ? { def: displacedDef, enchant: displaced.enchant }
          : undefined,
        side: params.side,
        anchor: node.getBoundingClientRect(),
      },
    })
    unsubDrag = dragMeta.subscribe((meta) => {
      if (meta) hide()
    })
    window.addEventListener('scroll', hide, true)
    window.addEventListener('resize', hide)
  }

  function hide() {
    unsubDrag?.()
    unsubDrag = null
    window.removeEventListener('scroll', hide, true)
    window.removeEventListener('resize', hide)
    if (instance) {
      unmount(instance)
      instance = null
    }
  }

  node.addEventListener('mouseenter', show)
  node.addEventListener('mouseleave', hide)

  return {
    update(next: ItemTooltipParams | null) {
      params = next
      if (!next) hide()
    },
    destroy() {
      hide()
      node.removeEventListener('mouseenter', show)
      node.removeEventListener('mouseleave', hide)
    },
  }
}
