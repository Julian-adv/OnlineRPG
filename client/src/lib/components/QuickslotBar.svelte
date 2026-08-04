<script lang="ts">
  import { inventoryStore } from '../stores/inventoryStore'
  import { getItemDef, isConsumable } from '../data/itemDefs'
  import { networkManager } from '../network/socket'
  import {
    quickslots,
    QUICKSLOT_COUNT,
    loadQuickslots,
    clearQuickslot,
  } from '../stores/quickslotStore'
  import { dragMeta, dragPos, quickslotAt } from '../stores/dragStore'
  import { itemTooltip } from '../actions/itemTooltip'

  interface Props {
    /** Active character id — used to load that character's saved quickslots. */
    characterId: number | null
  }

  let { characterId }: Props = $props()

  $effect(() => {
    if (characterId != null) loadQuickslots(characterId)
  })

  /** Total quantity of an item def currently sitting in the bag. */
  function bagQuantity(defId: string): number {
    let total = 0
    for (const item of $inventoryStore.bag) {
      if (item.item_def_id === defId) total += item.quantity
    }
    return total
  }

  const slots = $derived(
    $quickslots.map((defId) => {
      if (!defId) return null
      const def = getItemDef(defId)
      if (!def) return null
      return { defId, def, qty: bagQuantity(defId) }
    })
  )

  // While an item is dragged, the slot it would drop into (-1 otherwise).
  // Uses the same snap logic as the drop handler so highlight and drop agree.
  const dropIndex = $derived(
    $dragMeta && $dragMeta.groupItems === undefined
      ? quickslotAt($dragPos.x, $dragPos.y)
      : -1
  )

  /**
   * Use the item bound to a quickslot: for equippables, toggle equip/unequip
   * (pressing again unequips the same item — e.g. a torch turns its light off);
   * for consumables, use one from the bag.
   */
  function useSlot(index: number) {
    const entry = slots[index]
    if (!entry) return
    const slot = entry.def.equipSlot
    // Already wearing this exact item in its slot → unequip (toggle off).
    if (slot && $inventoryStore.equipped[slot]?.item_def_id === entry.defId) {
      networkManager.sendUnequipItem(slot)
      return
    }
    const inst = $inventoryStore.bag.find((b) => b.item_def_id === entry.defId)
    if (!inst) return // none left in bag
    if (slot) networkManager.sendEquipItem(inst.instance_id)
    else if (isConsumable(entry.def))
      networkManager.sendUseItem(inst.instance_id)
  }

  // Digit1..Digit9 → slots 0..8, Digit0 → slot 9.
  function handleKeydown(event: KeyboardEvent) {
    if (event.ctrlKey || event.altKey || event.metaKey) return
    const tag = (document.activeElement?.tagName ?? '').toLowerCase()
    if (tag === 'input' || tag === 'textarea') return
    const match = /^Digit(\d)$/.exec(event.code)
    if (!match) return
    const digit = Number(match[1])
    const index = digit === 0 ? 9 : digit - 1
    if (index >= QUICKSLOT_COUNT) return
    event.preventDefault()
    useSlot(index)
  }

  // The 1-based key label shown on each slot (last slot is "0").
  function keyLabel(index: number): string {
    return index === 9 ? '0' : String(index + 1)
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="quickslot-bar" role="toolbar" aria-label="Quickslots">
  {#each slots as entry, i (i)}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="quickslot"
      class:empty={!entry}
      class:drop-target={i === dropIndex}
      data-quickslot={i}
      use:itemTooltip={entry ? { def: entry.def, side: 'right' } : null}
      onclick={() => useSlot(i)}
      oncontextmenu={(e) => {
        e.preventDefault()
        clearQuickslot(i)
      }}
    >
      <span class="key-label">{keyLabel(i)}</span>
      {#if entry}
        <img
          class="item-icon"
          class:depleted={entry.qty === 0}
          src="/items/{entry.def.icon}"
          alt=""
          draggable="false"
        />
        {#if entry.qty !== 1}
          <span class="item-qty" class:zero={entry.qty === 0}>{entry.qty}</span>
        {/if}
      {/if}
    </div>
  {/each}
</div>

<style>
  .quickslot-bar {
    /* Wide-screen single-row slot size (~70% of the original 56px). The
       wrap/phone media queries below shrink it for narrow viewports. */
    --quickslot-size: 40px;
    --quickslot-gap: 4px;
    display: flex;
    flex-direction: row;
    gap: var(--quickslot-gap);
    /* No padding or border: the bar's box is exactly the slots, so its bottom
       edge lines up with the chat panel and menu buttons. */
    border-radius: 10px;
    font-family: 'Courier New', monospace;
    pointer-events: auto;
    max-width: calc(100vw - 32px);
  }

  .quickslot {
    position: relative;
    box-sizing: border-box;
    width: var(--quickslot-size);
    height: var(--quickslot-size);
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-radius: 4px;
    background: rgba(6, 10, 14, 0.55);
    backdrop-filter: blur(4px);
  }

  /* Outline keeps the slot's box (and the bar's alignment) unchanged. */
  .quickslot.drop-target {
    border-color: rgba(88, 255, 88, 0.7);
    outline: 2px solid rgba(88, 255, 88, 0.7);
    outline-offset: 1px;
    box-shadow: 0 0 10px rgba(88, 255, 88, 0.35);
  }

  .key-label {
    position: absolute;
    top: 2px;
    left: 4px;
    font-size: 11px;
    font-weight: 700;
    color: #9fb2c3;
    text-shadow: 0 0 3px rgba(0, 0, 0, 0.9);
    pointer-events: none;
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

  .item-icon.depleted {
    filter: grayscale(1) brightness(0.5);
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

  .item-qty.zero {
    color: #e06c6c;
  }

  /* Very narrow (<1000px): wrap the 10 slots into exactly two rows of five.
     The width is pinned to five slots wide and the action cluster is rigid
     (flex-shrink:0 in GameHud), so the bar can never be squeezed into a third
     or fourth row — the chat panel takes all the shrinking instead. */
  @media (max-width: 999.98px) {
    .quickslot-bar {
      flex-wrap: wrap;
      justify-content: center;
      --quickslot-size: 40px;
      /* Exactly five slots + four gaps per row (+1px guards against rounding
         bumping the fifth slot to a new row). */
      width: calc(5 * var(--quickslot-size) + 4 * var(--quickslot-gap) + 1px);
      max-width: calc(100vw - 18px);
    }
  }

  /* Phone / narrow: keep the two-row layout but let each slot shrink so the
     five-wide rows still fit (with the menu) without overflowing the screen. */
  @media (max-width: 600px), (pointer: coarse) {
    .quickslot-bar {
      --quickslot-size: min(40px, calc(20vw - 36px));
    }
  }
</style>
