<script lang="ts">
  import { inventoryStore, playerGold } from '../stores/inventoryStore'
  import type { ItemInstance } from '../stores/inventoryStore'
  import { getItemDef, isConsumable } from '../data/itemDefs'
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

  const slots = $derived.by(() => buildInventorySlots($inventoryStore.bag))

  let panelEl = $state<HTMLDivElement | null>(null)

  function onDblClick(slot: ItemInstance | null) {
    if (!slot) return
    const def = getItemDef(slot.item_def_id)
    if (def?.equipSlot) {
      networkManager.sendEquipItem(slot.instance_id)
    } else if (def && isConsumable(def)) {
      networkManager.sendUseItem(slot.instance_id)
    }
  }

  function onPointerDown(e: PointerEvent, slot: ItemInstance) {
    if (e.button !== 0) return
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
          networkManager.sendDropItem(slot.instance_id)
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
      <button class="close-btn" onclick={onClose}>&times;</button>
    </div>

    <div class="bag-grid">
      {#each slots as slot, i (slot?.instance_id ?? `empty-${i}`)}
        {@const def = slot ? getItemDef(slot.item_def_id) : null}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="grid-cell"
          use:itemTooltip={def && slot
            ? { def, item: slot, side: 'left' }
            : null}
          ondblclick={() => onDblClick(slot)}
          onpointerdown={(e: PointerEvent) => {
            if (slot) onPointerDown(e, slot)
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
        </div>
      {/each}
    </div>
  </div>
{/if}

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
