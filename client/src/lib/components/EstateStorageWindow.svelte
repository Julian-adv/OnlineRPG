<script lang="ts">
  import './tradePanel.css'
  import { SvelteMap } from 'svelte/reactivity'
  import { activeDebuffs } from '../stores/debuffStore'
  import { hungerState } from '../stores/hungerStore'
  import {
    carriedItemWeight,
    carryWeight,
    formatKg,
    inventoryStore,
    maxCarryWeight,
  } from '../stores/inventoryStore'
  import {
    estateChestError,
    estateChestPending,
    openEstateChest,
  } from '../stores/estateStorageStore'
  import { armorWeightMult } from '../data/debuffPresentation'
  import { getItemDef, itemDisplayName } from '../data/itemDefs'
  import { isEstateStorageItem } from '../data/estateFurnitureDefs'
  import { networkManager } from '../network/socket'
  import type { BagLineItem, ItemInstance } from '../network/networkTypes'
  import QuantityPopup from './QuantityPopup.svelte'
  import WeightBar from './WeightBar.svelte'
  import { draggablePanel } from '../actions/draggablePanel'
  import { itemTooltip } from '../actions/itemTooltip'
  import { sortBag } from './inventorySort'

  interface Props {
    str: number
  }

  type Direction = 'deposit' | 'withdraw'
  type PendingSelection = {
    direction: Direction
    item: ItemInstance
    max: number
  }

  let { str }: Props = $props()

  const deposits = new SvelteMap<number, number>()
  const withdrawals = new SvelteMap<number, number>()
  let quantityChoice = $state<PendingSelection | null>(null)

  const bag = $derived(sortBag($inventoryStore.bag))
  const stored = $derived(sortBag($openEstateChest?.items ?? []))
  const storageWeight = $derived(
    stored.reduce((sum, item) => sum + baseWeight(item), 0)
  )
  const storageLimit = $derived($openEstateChest?.max_weight ?? 0)
  const armorMultiplier = $derived(
    armorWeightMult(
      $activeDebuffs
        .filter((debuff) => debuff.until > Date.now())
        .map((debuff) => debuff.id)
    )
  )
  const playerLimit = $derived(maxCarryWeight(str, $hungerState))
  const depositStorageWeight = $derived(selectedWeight(deposits, bag, false))
  const depositPlayerWeight = $derived(selectedWeight(deposits, bag, true))
  const withdrawalStorageWeight = $derived(
    selectedWeight(withdrawals, stored, false)
  )
  const withdrawalPlayerWeight = $derived(
    selectedWeight(withdrawals, stored, true)
  )
  const projectedStorageWeight = $derived(
    storageWeight - withdrawalStorageWeight + depositStorageWeight
  )
  const projectedPlayerWeight = $derived(
    $carryWeight - depositPlayerWeight + withdrawalPlayerWeight
  )
  const selectedQuantity = $derived(
    [...deposits.values(), ...withdrawals.values()].reduce(
      (sum, quantity) => sum + quantity,
      0
    )
  )
  const hasIncomingStorage = $derived(deposits.size > 0)
  const hasIncomingPlayer = $derived(withdrawals.size > 0)
  const validSelection = $derived(
    selectedQuantity > 0 &&
      (!hasIncomingStorage || projectedStorageWeight <= storageLimit + 0.001) &&
      (!hasIncomingPlayer || projectedPlayerWeight <= playerLimit + 0.001)
  )

  $effect(() => {
    const bagIds = new Set($inventoryStore.bag.map((item) => item.instance_id))
    const storedIds = new Set(
      ($openEstateChest?.items ?? []).map((item) => item.instance_id)
    )
    for (const id of deposits.keys()) if (!bagIds.has(id)) deposits.delete(id)
    for (const id of withdrawals.keys())
      if (!storedIds.has(id)) withdrawals.delete(id)
  })

  function baseUnitWeight(item: ItemInstance) {
    return getItemDef(item.item_def_id)?.weight ?? 1
  }

  function baseWeight(item: ItemInstance) {
    return baseUnitWeight(item) * item.quantity
  }

  function carriedUnitWeight(item: ItemInstance) {
    return carriedItemWeight({ ...item, quantity: 1 }, armorMultiplier)
  }

  function selectedWeight(
    selection: ReadonlyMap<number, number>,
    items: readonly ItemInstance[],
    carried: boolean
  ) {
    let total = 0
    for (const item of items) {
      const quantity = selection.get(item.instance_id) ?? 0
      total +=
        quantity * (carried ? carriedUnitWeight(item) : baseUnitWeight(item))
    }
    return total
  }

  function isBlockedDeposit(item: ItemInstance) {
    const def = getItemDef(item.item_def_id)
    return isEstateStorageItem(item.item_def_id) || def?.untradeable === true
  }

  function maxSelectable(direction: Direction, item: ItemInstance) {
    if (direction === 'deposit') {
      if (!$openEstateChest?.can_deposit || isBlockedDeposit(item)) return 0
      const unit = baseUnitWeight(item)
      if (unit <= 0) return item.quantity
      return Math.min(
        item.quantity,
        Math.max(
          0,
          Math.floor((storageLimit - projectedStorageWeight + 0.001) / unit)
        )
      )
    }

    const unit = carriedUnitWeight(item)
    if (unit <= 0) return item.quantity
    return Math.min(
      item.quantity,
      Math.max(
        0,
        Math.floor((playerLimit - projectedPlayerWeight + 0.001) / unit)
      )
    )
  }

  function selectionFor(direction: Direction) {
    return direction === 'deposit' ? deposits : withdrawals
  }

  function choose(direction: Direction, item: ItemInstance) {
    if ($estateChestPending) return
    const selection = selectionFor(direction)
    if (selection.has(item.instance_id)) {
      selection.delete(item.instance_id)
      estateChestError.set(null)
      return
    }
    const max = maxSelectable(direction, item)
    if (max < 1) {
      estateChestError.set(
        direction === 'deposit'
          ? 'The storage chest cannot hold more of that item.'
          : 'Your bag cannot hold more of that item.'
      )
      return
    }
    estateChestError.set(null)
    if (max > 1) quantityChoice = { direction, item, max }
    else selection.set(item.instance_id, 1)
  }

  function confirmQuantity(quantity: number) {
    if (!quantityChoice) return
    selectionFor(quantityChoice.direction).set(
      quantityChoice.item.instance_id,
      Math.min(quantity, quantityChoice.max)
    )
    quantityChoice = null
  }

  function candidateLabel(direction: Direction, item: ItemInstance) {
    const selected = selectionFor(direction).get(item.instance_id)
    if (selected) return `Selected ×${selected}`
    const max = maxSelectable(direction, item)
    if (max > 0 && max < item.quantity) return `Max ${max}`
    return ''
  }

  function candidateTitle(direction: Direction, item: ItemInstance) {
    if (direction === 'deposit') {
      if (isBlockedDeposit(item))
        return 'Bound, untradeable, and storage chest items cannot be stored'
      if (!$openEstateChest?.can_deposit) return 'Tax overdue: withdrawal only'
      if (maxSelectable(direction, item) < 1)
        return 'No storage weight remains for this item'
      return `Select ${itemDisplayName(item.item_def_id, item.enchant)} to store`
    }
    if (maxSelectable(direction, item) < 1)
      return 'No bag weight remains for this item'
    return `Select ${itemDisplayName(item.item_def_id, item.enchant)} to take`
  }

  function requestLines(selection: ReadonlyMap<number, number>): BagLineItem[] {
    return [...selection].map(([instance_id, qty]) => ({ instance_id, qty }))
  }

  function submit() {
    const chest = $openEstateChest
    if (!chest || !validSelection || $estateChestPending) return
    estateChestPending.set(true)
    estateChestError.set(null)
    networkManager.sendTransferEstateItems(
      chest.chest_id,
      requestLines(deposits),
      requestLines(withdrawals),
      chest.revision
    )
    deposits.clear()
    withdrawals.clear()
    quantityChoice = null
  }

  function recover() {
    const chest = $openEstateChest
    if (!chest || stored.length > 0 || $estateChestPending) return
    deposits.clear()
    withdrawals.clear()
    quantityChoice = null
    estateChestPending.set(true)
    estateChestError.set(null)
    networkManager.sendRecoverEstateChest(chest.chest_id)
  }

  function close() {
    deposits.clear()
    withdrawals.clear()
    quantityChoice = null
    estateChestError.set(null)
    openEstateChest.set(null)
  }
</script>

{#if $openEstateChest}
  <div
    class="trade-window storage-window"
    role="dialog"
    aria-label="Estate storage"
    use:draggablePanel={'storage'}
  >
    <header class="panel-header" data-drag-handle>
      <span class="panel-title">
        {itemDisplayName($openEstateChest.item_def_id)}
      </span>
      <button class="close-btn" onclick={close} aria-label="Close">×</button>
    </header>

    {#if !$openEstateChest.can_deposit}
      <p class="notice">Tax overdue: withdrawal only.</p>
    {/if}
    {#if $estateChestError}<p class="error">{$estateChestError}</p>{/if}

    <div class="trade-columns">
      <section class="trade-column">
        <div class="column-title">Stored items</div>
        <div class="item-list">
          {#each stored as item (item.instance_id)}
            {@const def = getItemDef(item.item_def_id)}
            {@const selected = withdrawals.has(item.instance_id)}
            {@const unavailable =
              !selected && maxSelectable('withdraw', item) < 1}
            <button
              class="item-row"
              class:selected
              class:blocked={unavailable}
              disabled={$estateChestPending || unavailable}
              title={candidateTitle('withdraw', item)}
              onclick={() => choose('withdraw', item)}
              use:itemTooltip={def ? { def, item, side: 'right' } : null}
            >
              {#if def}
                <img
                  class="item-icon"
                  src="/items/{def.icon}"
                  alt=""
                  draggable="false"
                />
              {/if}
              <span class="item-name">
                {itemDisplayName(
                  item.item_def_id,
                  item.enchant
                )}{item.quantity > 1 ? ` ×${item.quantity}` : ''}
              </span>
              {#if candidateLabel('withdraw', item)}
                <span class="selection-label"
                  >{candidateLabel('withdraw', item)}</span
                >
              {/if}
            </button>
          {:else}
            <div class="empty-note">Nothing stored</div>
          {/each}
        </div>
        <WeightBar
          current={storageWeight}
          projected={projectedStorageWeight}
          max={storageLimit}
          label="Storage weight"
        />
      </section>

      <section class="trade-column bag-column">
        <div class="column-title">Your bag</div>
        <div class="item-list">
          {#each bag as item (item.instance_id)}
            {@const def = getItemDef(item.item_def_id)}
            {@const selected = deposits.has(item.instance_id)}
            {@const unavailable =
              !selected && maxSelectable('deposit', item) < 1}
            <button
              class="item-row"
              class:selected
              class:blocked={unavailable}
              disabled={$estateChestPending || unavailable}
              title={candidateTitle('deposit', item)}
              onclick={() => choose('deposit', item)}
              use:itemTooltip={def ? { def, item, side: 'left' } : null}
            >
              {#if def}
                <img
                  class="item-icon"
                  src="/items/{def.icon}"
                  alt=""
                  draggable="false"
                />
              {/if}
              <span class="item-name">
                {itemDisplayName(
                  item.item_def_id,
                  item.enchant
                )}{item.quantity > 1 ? ` ×${item.quantity}` : ''}
              </span>
              {#if candidateLabel('deposit', item)}
                <span class="selection-label"
                  >{candidateLabel('deposit', item)}</span
                >
              {/if}
            </button>
          {:else}
            <div class="empty-note">Your bag is empty</div>
          {/each}
        </div>
        <WeightBar
          current={$carryWeight}
          projected={projectedPlayerWeight}
          max={playerLimit}
          label="Bag weight"
        />
      </section>
    </div>

    <div class="selection-summary">
      <span
        >Storage: {formatKg(Math.max(0, storageLimit - projectedStorageWeight))} kg
        free</span
      >
      <span
        >Bag: {formatKg(Math.max(0, playerLimit - projectedPlayerWeight))} kg free</span
      >
    </div>
    <footer>
      {#if stored.length === 0}
        <button
          class="recover-btn"
          disabled={$estateChestPending}
          title="Return to your bag, or drop here if your bag is overweight"
          onclick={recover}
        >
          Recover
        </button>
      {/if}
      <button
        class="confirm-btn"
        disabled={$estateChestPending || !validSelection}
        onclick={submit}
      >
        Confirm
      </button>
    </footer>
  </div>
{/if}

<QuantityPopup
  visible={quantityChoice !== null}
  itemName={quantityChoice
    ? itemDisplayName(
        quantityChoice.item.item_def_id,
        quantityChoice.item.enchant
      )
    : ''}
  icon={quantityChoice
    ? (getItemDef(quantityChoice.item.item_def_id)?.icon ?? '')
    : ''}
  max={quantityChoice?.max ?? 1}
  onConfirm={confirmQuantity}
  onCancel={() => (quantityChoice = null)}
/>

<style>
  .trade-window {
    z-index: 45;
    width: min(560px, calc(100vw - 32px));
  }

  .panel-header {
    cursor: move;
  }

  .trade-columns {
    min-height: 0;
  }

  .trade-column {
    flex: 1;
    width: auto;
    min-width: 0;
  }

  .trade-column .item-list {
    flex: 1;
    min-height: 0;
  }

  .bag-column {
    padding-left: 16px;
    border-left: 1px solid rgba(255, 255, 255, 0.12);
  }

  .item-row.selected {
    border-color: rgba(240, 192, 64, 0.7);
    background: rgba(240, 192, 64, 0.14);
  }

  .selection-label {
    flex-shrink: 0;
    color: #f0c040;
    font-size: 10px;
    font-weight: 700;
  }

  .notice,
  .error {
    margin: 0 0 8px;
    padding: 7px 9px;
    border-radius: 4px;
  }

  .notice {
    background: rgba(139, 100, 29, 0.28);
    color: #ffd98b;
  }

  .error {
    background: rgba(166, 55, 55, 0.28);
    color: #ffaaaa;
  }

  .selection-summary {
    display: flex;
    justify-content: space-between;
    gap: 16px;
    padding-top: 7px;
    color: #7f95a7;
    font-size: 10px;
  }

  footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 12px;
    padding-top: 8px;
    color: #9fb2c3;
    font-size: 11px;
  }

  .recover-btn {
    margin-top: 4px;
    padding: 4px 14px;
    border: 1px solid rgba(190, 155, 92, 0.45);
    border-radius: 4px;
    background: rgba(83, 66, 41, 0.88);
    color: #ead7b0;
    font-family: inherit;
    font-size: 12px;
    font-weight: 700;
    cursor: pointer;
  }

  .recover-btn:hover:not(:disabled) {
    background: rgba(112, 86, 48, 0.95);
    color: #fff4d6;
  }

  .recover-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }

  @media (max-width: 600px) {
    .trade-columns {
      flex-direction: column;
      overflow-y: auto;
    }

    .trade-column {
      width: auto;
    }

    .item-list {
      flex: none;
      max-height: 22vh;
    }

    .bag-column {
      padding-top: 12px;
      padding-left: 0;
      border-top: 1px solid rgba(255, 255, 255, 0.12);
      border-left: 0;
    }
  }
</style>
