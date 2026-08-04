<script lang="ts">
  import { inventoryStore, playerGold } from '../stores/inventoryStore'
  import type { ItemInstance } from '../stores/inventoryStore'
  import {
    getItemDef,
    isConsumable,
    type ItemDefinition,
  } from '../data/itemDefs'
  import GoldAmount from './GoldAmount.svelte'
  import { networkManager } from '../network/socket'
  import type { CharacterAttributes, EquipSlot } from '../network/networkTypes'
  import {
    dragMeta,
    startDrag,
    isSlotCompatible,
    pointInRect,
    isOverAnyDialog,
    FALLBACK_ICON,
  } from '../stores/dragStore'
  import { itemTooltip } from '../actions/itemTooltip'
  import { buildInventorySlots } from './inventorySlots'
  import { sortBag } from './inventorySort'
  import { groupBagForSelection, splitGroupQty } from './inventoryGroups'
  import QuantityPopup from './QuantityPopup.svelte'
  import { SvelteMap, SvelteSet } from 'svelte/reactivity'

  interface Props {
    visible: boolean
    attributes: CharacterAttributes | null
    onClose: () => void
  }

  let { visible, attributes, onClose }: Props = $props()

  const maxWeight = $derived(attributes ? attributes.str * 15 : 150)

  function itemWeight(item: ItemInstance): number {
    const def = getItemDef(item.item_def_id)
    return (def?.weight ?? 1) * item.quantity
  }

  const currentWeight = $derived.by(() => {
    const inv = $inventoryStore
    let total = 0
    for (const item of inv.bag) total += itemWeight(item)
    for (const item of Object.values(inv.equipped)) {
      if (item) total += itemWeight(item)
    }
    return total
  })

  const slots = $derived(buildInventorySlots(sortBag($inventoryStore.bag)))

  let panelEl = $state<HTMLDivElement | null>(null)
  let pendingDrop = $state<{ slot: ItemInstance; def: ItemDefinition } | null>(
    null
  )

  /** Select mode: bulk-drop several different non-stackable items at once
   *  (junk, unique gear) in a single drag. Stackable stacks keep the
   *  separate single-drag + quantity-popup flow above and are inert here. */
  let selectMode = $state(false)
  const selected = new SvelteSet<number>()
  const bagIds = $derived(
    new Set($inventoryStore.bag.map((item) => item.instance_id))
  )

  // Closing the panel must not leave Select mode (and a stale selection)
  // armed for whenever it's reopened.
  $effect(() => {
    if (!visible) {
      selectMode = false
      selected.clear()
      return
    }
    for (const id of selected) {
      if (!bagIds.has(id)) selected.delete(id)
    }
  })

  function liveBagIds(ids: Iterable<number>): number[] {
    return [...ids].filter((id) => bagIds.has(id))
  }

  function isSelectable(slot: ItemInstance): boolean {
    return getItemDef(slot.item_def_id)?.stackable !== true
  }

  function toggleSelectMode() {
    selectMode = !selectMode
    selected.clear()
  }

  function toggleSelected(slot: ItemInstance) {
    if (!isSelectable(slot)) return
    if (selected.has(slot.instance_id)) selected.delete(slot.instance_id)
    else selected.add(slot.instance_id)
  }

  /** Splits a requested quantity across whichever real bag instances back
   *  this (possibly merged-for-display) stack, since a merged slot's own
   *  instance_id may not itself hold that many units. */
  function dropQty(slot: ItemInstance, qty: number) {
    const stackable = getItemDef(slot.item_def_id)?.stackable === true
    const key = stackable
      ? `${slot.item_def_id}:${slot.enchant}`
      : `unique:${slot.instance_id}`
    const group = groupBagForSelection($inventoryStore.bag).find(
      (g) => g.key === key
    )
    if (!group) return
    const lines = splitGroupQty(group, Math.min(qty, group.totalQty))
    networkManager.sendDropItems(
      lines.map((l) => ({ instance_id: l.instanceId, qty: l.qty }))
    )
  }

  function confirmPendingDrop(qty: number) {
    if (!pendingDrop) return
    dropQty(pendingDrop.slot, qty)
    pendingDrop = null
  }

  function cancelPendingDrop() {
    pendingDrop = null
  }

  function onDblClick(slot: ItemInstance | null) {
    if (!slot || selectMode) return
    const def = getItemDef(slot.item_def_id)
    if (def?.equipSlot) {
      networkManager.sendEquipItem(slot.instance_id)
    } else if (def && isConsumable(def)) {
      networkManager.sendUseItem(slot.instance_id)
    }
  }

  /** Drags the current Select-mode selection as a group, adding the dragged
   *  slot to it first if it wasn't already selected. A plain click (no drag)
   *  toggles the slot instead — routed through `startDrag`'s own click
   *  callback rather than a separate native `onclick`, since pointer capture
   *  keeps the browser's click event targeted here even after a completed
   *  drag, which would otherwise re-select the item this same gesture just
   *  dropped. */
  function onGroupPointerDown(e: PointerEvent, slot: ItemInstance) {
    const def = getItemDef(slot.item_def_id)
    const groupIds = new SvelteSet(liveBagIds(selected))
    groupIds.add(slot.instance_id)
    // Several selected instances can share the same item_def_id (e.g. two
    // separately-caught, non-stackable old_boot entries) — consolidate them
    // into one ghost icon with a summed quantity rather than one icon per
    // instance.
    const byDefId = new SvelteMap<string, { icon: string; quantity: number }>()
    for (const id of groupIds) {
      const item = $inventoryStore.bag.find((i) => i.instance_id === id)
      if (!item) continue
      const existing = byDefId.get(item.item_def_id)
      if (existing) {
        existing.quantity += item.quantity
      } else {
        byDefId.set(item.item_def_id, {
          icon: getItemDef(item.item_def_id)?.icon ?? FALLBACK_ICON,
          quantity: item.quantity,
        })
      }
    }
    const groupItems = [...byDefId.values()]

    startDrag(
      e,
      {
        instanceId: slot.instance_id,
        defId: slot.item_def_id,
        // Group drags never equip, regardless of drop position.
        equipSlot: null,
        source: { type: 'bag' },
        icon: def?.icon ?? FALLBACK_ICON,
        groupItems,
      },
      (x, y) => {
        if (
          panelEl &&
          !pointInRect(x, y, panelEl.getBoundingClientRect()) &&
          !isOverAnyDialog(x, y)
        ) {
          const dropIds = liveBagIds(groupIds)
          if (dropIds.length > 0) {
            networkManager.sendDropItems(
              dropIds.map((instance_id) => ({ instance_id, qty: 1 }))
            )
          }
          selected.clear()
        }
      },
      () => toggleSelected(slot)
    )
  }

  function onPointerDown(e: PointerEvent, slot: ItemInstance) {
    if (e.button !== 0) return
    if (selectMode) {
      if (!isSelectable(slot)) return
      e.preventDefault()
      onGroupPointerDown(e, slot)
      return
    }
    e.preventDefault()
    const def = getItemDef(slot.item_def_id)

    startDrag(
      e,
      {
        instanceId: slot.instance_id,
        defId: slot.item_def_id,
        equipSlot: def?.equipSlot ?? null,
        source: { type: 'bag' },
        icon: def?.icon ?? FALLBACK_ICON,
      },
      (x, y) => {
        for (const slotEl of document.querySelectorAll<HTMLElement>(
          '[data-equip-slot]'
        )) {
          if (pointInRect(x, y, slotEl.getBoundingClientRect())) {
            const targetSlot = slotEl.dataset.equipSlot as EquipSlot
            if (isSlotCompatible(def?.equipSlot ?? null, targetSlot)) {
              networkManager.sendEquipItem(slot.instance_id)
              return
            }
          }
        }
        if (
          panelEl &&
          !pointInRect(x, y, panelEl.getBoundingClientRect()) &&
          !isOverAnyDialog(x, y)
        ) {
          if (slot.quantity > 1 && def) {
            pendingDrop = { slot, def }
          } else {
            networkManager.sendDropItem(slot.instance_id)
          }
        }
      }
    )
  }
</script>

{#if visible}
  <div
    class="inventory-panel"
    class:drop-target={$dragMeta?.source.type === 'equipped'}
    role="dialog"
    aria-label="Inventory"
    data-panel="inventory"
    bind:this={panelEl}
  >
    <div class="panel-header">
      <span class="panel-title">Inventory</span>
      <span class="gold-display"><GoldAmount copper={$playerGold} /></span>
      <span class="weight-display">
        {(currentWeight / 10).toFixed(1)} / {(maxWeight / 10).toFixed(1)} kg
      </span>
      <button
        class="select-btn"
        class:active={selectMode}
        title="Select multiple items to drop"
        onclick={toggleSelectMode}
      >
        {selectMode ? 'Cancel' : 'Select'}
      </button>
      <button class="close-btn" onclick={onClose}>&times;</button>
    </div>

    <div class="bag-grid">
      {#each slots as slot, i (slot?.instance_id ?? `empty-${i}`)}
        {@const def = slot ? getItemDef(slot.item_def_id) : null}
        {@const locked = selectMode && slot !== null && !isSelectable(slot)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="grid-cell"
          class:selected={slot !== null && selected.has(slot.instance_id)}
          class:locked
          use:itemTooltip={def && slot
            ? { def, item: slot, side: 'left' }
            : null}
          ondblclick={() => onDblClick(slot)}
          onpointerdown={(e: PointerEvent) => {
            if (slot && !locked) onPointerDown(e, slot)
          }}
        >
          {#if def}
            <img
              class="item-icon"
              src="/items/{def.icon}"
              alt=""
              draggable="false"
            />
          {/if}
          {#if slot && slot.enchant > 0}
            <span class="item-enchant">+{slot.enchant}</span>
          {/if}
          {#if slot && slot.quantity > 1}
            <span class="item-qty">{slot.quantity}</span>
          {/if}
          {#if slot !== null && selected.has(slot.instance_id)}
            <span class="item-selected-check">&check;</span>
          {/if}
        </div>
      {/each}
    </div>
  </div>
{/if}

<QuantityPopup
  visible={pendingDrop !== null}
  itemName={pendingDrop?.def.name ?? ''}
  icon={pendingDrop?.def.icon ?? ''}
  max={pendingDrop?.slot.quantity ?? 1}
  onConfirm={confirmPendingDrop}
  onCancel={cancelPendingDrop}
/>

<style>
  .inventory-panel {
    --inventory-slot-size: 64px;
    --inventory-slot-gap: 6px;
    --inventory-visible-rows: 10;
    position: fixed;
    right: 9px;
    top: 45%;
    transform: translateY(-50%);
    z-index: 40;
    display: flex;
    flex-direction: column;
    backdrop-filter: blur(4px);
    padding: 10px;
    border: 1px solid rgba(255, 255, 255, 0.18);
    border-radius: 10px;
    background: rgba(6, 10, 14, 0.88);
    color: #e6edf3;
    font-family: 'Courier New', monospace;
    font-size: 12px;
    pointer-events: auto;
    max-width: calc(100vw - 32px);
  }

  .inventory-panel.drop-target {
    border-color: rgba(88, 255, 88, 0.5);
    box-shadow: inset 0 0 12px rgba(88, 255, 88, 0.15);
  }

  .panel-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding-bottom: 8px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.15);
    margin-bottom: 8px;
  }

  .panel-title {
    font-size: 14px;
    font-weight: 700;
    color: #f0c040;
  }

  .close-btn {
    background: none;
    border: none;
    color: #9fb2c3;
    font-size: 18px;
    cursor: pointer;
    padding: 0 2px;
    line-height: 1;
  }

  .close-btn:hover {
    color: #fff;
  }

  .weight-display {
    font-size: 11px;
    color: #9fb2c3;
  }

  .gold-display {
    font-size: 11px;
    font-weight: 700;
    color: #ffd700;
  }

  .select-btn {
    background: none;
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 4px;
    color: #9fb2c3;
    font-family: inherit;
    font-size: 11px;
    font-weight: 700;
    cursor: pointer;
    padding: 2px 6px;
  }

  .select-btn:hover {
    color: #fff;
    border-color: rgba(255, 255, 255, 0.4);
  }

  .select-btn.active {
    background: rgba(120, 60, 60, 0.85);
    border-color: rgba(220, 140, 140, 0.45);
    color: #fff;
  }

  .bag-grid {
    display: grid;
    grid-template-columns: repeat(5, var(--inventory-slot-size));
    grid-auto-rows: var(--inventory-slot-size);
    gap: var(--inventory-slot-gap);
    max-height: calc(
      var(--inventory-slot-size) * var(--inventory-visible-rows) +
        var(--inventory-slot-gap) * (var(--inventory-visible-rows) - 1)
    );
    overflow-y: auto;
    overflow-x: hidden;
    overscroll-behavior: contain;
    scrollbar-width: thin;
    scrollbar-color: rgba(113, 128, 150, 0.5) transparent;
  }

  .bag-grid::-webkit-scrollbar {
    width: 8px;
  }

  .bag-grid::-webkit-scrollbar-track {
    background: transparent;
  }

  .bag-grid::-webkit-scrollbar-thumb {
    background: rgba(113, 128, 150, 0.5);
    border-radius: 999px;
  }

  .bag-grid::-webkit-scrollbar-thumb:hover {
    background: rgba(160, 174, 192, 0.7);
  }

  .grid-cell {
    position: relative;
    box-sizing: border-box;
    width: var(--inventory-slot-size);
    height: var(--inventory-slot-size);
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-radius: 4px;
  }

  .grid-cell.selected {
    border-color: rgba(220, 140, 140, 0.7);
    background: rgba(120, 60, 60, 0.25);
  }

  .grid-cell.locked {
    opacity: 0.35;
    pointer-events: none;
  }

  .item-selected-check {
    position: absolute;
    top: 2px;
    right: 4px;
    font-size: 12px;
    font-weight: 700;
    color: #f0b8b8;
    text-shadow: 0 0 3px rgba(0, 0, 0, 0.8);
  }

  .item-icon {
    /* Slightly inset and centred so edge-to-edge icons (sword, spear) stay
       inside the slot's border instead of spilling over it. */
    position: absolute;
    inset: 0;
    margin: auto;
    width: 90%;
    height: 90%;
    object-fit: contain;
    image-rendering: pixelated;
  }

  .item-qty {
    position: absolute;
    bottom: 2px;
    right: 4px;
    font-size: 11px;
    font-weight: 700;
    color: #fff;
    text-shadow: 0 0 3px rgba(0, 0, 0, 0.8);
  }

  .item-enchant {
    position: absolute;
    top: 2px;
    left: 4px;
    font-size: 11px;
    font-weight: 700;
    color: #7ec8ff;
    text-shadow: 0 0 3px rgba(0, 0, 0, 0.8);
  }

  @media (max-width: 600px), (pointer: coarse) {
    .inventory-panel {
      --inventory-slot-size: 48px;
      --inventory-slot-gap: 5px;
      --inventory-visible-rows: 3;
      right: calc(8px + env(safe-area-inset-right));
      top: auto;
      bottom: calc(72px + env(safe-area-inset-bottom));
      transform: none;
      padding: 8px;
      border-radius: 8px;
    }

    .panel-header {
      padding-bottom: 6px;
      margin-bottom: 6px;
      gap: 8px;
    }

    .panel-title {
      font-size: 13px;
    }

    .weight-display {
      font-size: 10px;
    }

    .close-btn {
      min-width: 32px;
      min-height: 32px;
      font-size: 22px;
    }

    .bag-grid {
      -webkit-overflow-scrolling: touch;
    }
  }

  @media (max-width: 340px) {
    .inventory-panel {
      --inventory-slot-size: 44px;
      --inventory-slot-gap: 4px;
    }
  }
</style>
