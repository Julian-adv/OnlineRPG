<script lang="ts">
  import { networkManager } from '../network/socket'
  import { inventoryStore, playerGold } from '../stores/inventoryStore'
  import { getItemDef } from '../data/itemDefs'
  import { itemTooltip } from '../actions/itemTooltip'
  import { mountOverlay } from '../stores/overlayStack'
  import {
    playerTrade,
    playerTradeError,
    reservedQuantity,
    type PlayerTradeItem,
  } from '../stores/playerTradeStore'
  import { formatGold, parseGold } from '../utils/currency'
  import GoldAmount from './GoldAmount.svelte'
  import QuantityPopup from './QuantityPopup.svelte'

  const trade = $derived($playerTrade)

  // What we've locally staged. Sent whole (never a delta) and replaced by
  // whatever the server echoes back, so the two windows cannot drift.
  let goldText = $state('')
  let pendingAdd = $state<{
    instanceId: number
    name: string
    icon: string
    max: number
  } | null>(null)

  const locked = $derived(trade?.you.locked ?? false)

  $effect(() => {
    // Adopt the server's copper whenever our side is frozen or reset, so the
    // field never shows something the server didn't accept.
    if (locked && trade) goldText = formatGold(trade.you.copper)
  })

  $effect(() => {
    if (!trade) return
    return mountOverlay('playerTrade', () => {
      // Escape is free before the lock and inert after it: locking is the
      // "I mean it" signal, and a reflex key must not undo a negotiation.
      if (!$playerTrade?.you.locked) networkManager.sendPlayerTradeCancel()
    })
  })

  /** The bag, minus what is already on the table. */
  const available = $derived(
    ($inventoryStore?.bag ?? []).map((item) => ({
      item,
      free: item.quantity - reservedQuantity(trade, item.instance_id),
    }))
  )

  function displayName(entry: PlayerTradeItem): string {
    const def = getItemDef(entry.item_def_id)
    const name = def?.name ?? entry.item_def_id
    // Enchant belongs in the name, not a tooltip: nobody hovers mid-trade.
    return entry.enchant !== 0 ? `+${entry.enchant} ${name}` : name
  }

  function pushOffer(items: PlayerTradeItem[], copper: number) {
    networkManager.sendPlayerTradeSetOffer(
      items.map((i) => ({ instance_id: i.instance_id, quantity: i.quantity })),
      copper
    )
  }

  function currentCopper(): number {
    return trade?.you.copper ?? 0
  }

  function offerItem(instanceId: number, quantity: number) {
    if (!trade) return
    const existing = trade.you.items.filter((i) => i.instance_id !== instanceId)
    const previous = trade.you.items.find((i) => i.instance_id === instanceId)
    const total = (previous?.quantity ?? 0) + quantity
    const def = $inventoryStore?.bag.find((i) => i.instance_id === instanceId)
    if (!def) return
    pushOffer(
      [
        ...existing,
        {
          instance_id: instanceId,
          item_def_id: def.item_def_id,
          quantity: total,
          enchant: def.enchant,
        },
      ],
      currentCopper()
    )
  }

  function startAdd(instanceId: number, free: number) {
    if (locked || free <= 0) return
    const item = $inventoryStore?.bag.find((i) => i.instance_id === instanceId)
    if (!item) return
    const def = getItemDef(item.item_def_id)
    if (free === 1) {
      offerItem(instanceId, 1)
      return
    }
    pendingAdd = {
      instanceId,
      name: def?.name ?? item.item_def_id,
      icon: def?.icon ?? 'icon_frame.png',
      max: free,
    }
  }

  function removeOne(entry: PlayerTradeItem) {
    if (!trade || locked) return
    const rest = trade.you.items.filter(
      (i) => i.instance_id !== entry.instance_id
    )
    const left = entry.quantity - 1
    pushOffer(
      left > 0 ? [...rest, { ...entry, quantity: left }] : rest,
      currentCopper()
    )
  }

  function commitGold() {
    if (!trade || locked) return
    const copper = goldText.trim() === '' ? 0 : parseGold(goldText)
    if (copper === null || copper < 0) return
    pushOffer(trade.you.items, Math.min(copper, $playerGold))
  }

  function offerAllGold() {
    if (!trade || locked) return
    goldText = formatGold($playerGold)
    pushOffer(trade.you.items, $playerGold)
  }
</script>

{#if trade}
  <div
    class="player-trade"
    role="dialog"
    aria-label="Trade with {trade.them.name}"
    data-panel="playerTrade"
  >
    <div class="panel-header">
      <span class="panel-title">Trade with {trade.them.name}</span>
      <button
        class="close-btn"
        onclick={() => networkManager.sendPlayerTradeCancel()}
      >
        ×
      </button>
    </div>

    {#if $playerTradeError}
      <p class="trade-error">{$playerTradeError}</p>
    {/if}

    <div class="tables">
      <section class="side" class:locked>
        <h3>
          You offer
          {#if trade.you.locked}<span class="badge">locked</span>{/if}
          {#if trade.you.confirmed}<span class="badge ok">confirmed</span>{/if}
        </h3>
        <ul class="offer">
          {#each trade.you.items as entry (entry.instance_id)}
            {@const def = getItemDef(entry.item_def_id)}
            <li>
              <button
                class="row"
                disabled={locked}
                onclick={() => removeOne(entry)}
                use:itemTooltip={def ? { def, side: 'right' } : null}
              >
                <img
                  class="item-icon"
                  src="/items/{def?.icon}"
                  alt=""
                  draggable="false"
                />
                <span class="item-name">{displayName(entry)}</span>
                {#if entry.quantity > 1}<span class="qty"
                    >×{entry.quantity}</span
                  >{/if}
              </button>
            </li>
          {:else}
            <li class="empty-note">Click bag items to offer</li>
          {/each}
        </ul>
        <div class="gold-row">
          <input
            type="text"
            bind:value={goldText}
            disabled={locked}
            placeholder="0c"
            onchange={commitGold}
            onblur={commitGold}
          />
          <button disabled={locked} onclick={offerAllGold}>All</button>
        </div>
      </section>

      <section class="side them">
        <h3>
          {trade.them.name} offers
          {#if trade.them.locked}<span class="badge">locked</span>{/if}
          {#if trade.them.confirmed}<span class="badge ok">confirmed</span>{/if}
        </h3>
        <ul class="offer">
          {#each trade.them.items as entry (entry.instance_id)}
            {@const def = getItemDef(entry.item_def_id)}
            <li>
              <span
                class="row static"
                use:itemTooltip={def ? { def, side: 'left' } : null}
              >
                <img
                  class="item-icon"
                  src="/items/{def?.icon}"
                  alt=""
                  draggable="false"
                />
                <span class="item-name">{displayName(entry)}</span>
                {#if entry.quantity > 1}<span class="qty"
                    >×{entry.quantity}</span
                  >{/if}
              </span>
            </li>
          {:else}
            <li class="empty-note">Nothing offered</li>
          {/each}
        </ul>
        <div class="gold-row static">
          <GoldAmount copper={trade.them.copper} />
        </div>
      </section>
    </div>

    {#if !locked}
      <section class="bag">
        <h3>Your bag</h3>
        <ul class="offer">
          {#each available as { item, free } (item.instance_id)}
            {@const def = getItemDef(item.item_def_id)}
            <li>
              <button
                class="row"
                disabled={free <= 0}
                onclick={() => startAdd(item.instance_id, free)}
                use:itemTooltip={def ? { def, side: 'right' } : null}
              >
                <img
                  class="item-icon"
                  src="/items/{def?.icon}"
                  alt=""
                  draggable="false"
                />
                <span class="item-name">
                  {item.enchant !== 0 ? `+${item.enchant} ` : ''}{def?.name ??
                    item.item_def_id}
                </span>
                <span class="qty">{free}/{item.quantity}</span>
              </button>
            </li>
          {:else}
            <li class="empty-note">Bag is empty</li>
          {/each}
        </ul>
      </section>
    {/if}

    <footer>
      {#if !trade.you.locked}
        <button
          class="primary"
          onclick={() => networkManager.sendPlayerTradeLock(trade.revision)}
        >
          Lock offer
        </button>
      {:else if !trade.you.confirmed}
        <button
          class="ghost"
          onclick={() => networkManager.sendPlayerTradeUnlock()}
        >
          Unlock
        </button>
        <button
          class="primary"
          disabled={!trade.them.locked}
          onclick={() => networkManager.sendPlayerTradeConfirm(trade.revision)}
        >
          {trade.them.locked ? 'Confirm trade' : 'Waiting for them to lock'}
        </button>
      {:else}
        <span class="waiting">Waiting for {trade.them.name} to confirm…</span>
      {/if}
    </footer>
  </div>

  <QuantityPopup
    visible={pendingAdd !== null}
    itemName={pendingAdd?.name ?? ''}
    icon={pendingAdd?.icon ?? ''}
    max={pendingAdd?.max ?? 1}
    onConfirm={(qty) => {
      if (pendingAdd) offerItem(pendingAdd.instanceId, qty)
      pendingAdd = null
    }}
    onCancel={() => (pendingAdd = null)}
  />
{/if}

<style>
  .player-trade {
    position: fixed;
    left: 50%;
    top: 45%;
    transform: translate(-50%, -50%);
    z-index: 45;
    display: flex;
    flex-direction: column;
    width: min(600px, calc(100vw - 32px));
    max-height: 70vh;
    backdrop-filter: blur(4px);
    padding: 10px;
    border: 1px solid rgba(255, 255, 255, 0.18);
    border-radius: 10px;
    background: rgba(6, 10, 14, 0.88);
    color: #e6edf3;
    font-family: 'Courier New', monospace;
    font-size: 12px;
    pointer-events: auto;
  }

  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
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

  .trade-error {
    margin: 0 0 8px;
    padding: 4px 6px;
    border-radius: 4px;
    background: rgba(120, 60, 60, 0.85);
    color: #f0b8b8;
  }

  .tables {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
    min-height: 0;
    overflow: hidden;
  }

  .side {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 120px;
    padding: 8px;
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 6px;
  }

  .side.locked {
    border-color: rgba(240, 192, 64, 0.5);
  }

  h3 {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 0 4px;
    font-size: 12px;
    font-weight: 700;
    color: #9fb2c3;
  }

  .badge {
    flex-shrink: 0;
    padding: 0 4px;
    border-radius: 3px;
    font-weight: 700;
    background: rgba(120, 100, 40, 0.85);
    color: #f0d8a0;
  }

  .badge.ok {
    background: rgba(60, 110, 60, 0.85);
    color: #b8f0b8;
  }

  .offer {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    scrollbar-width: none;
  }

  .offer::-webkit-scrollbar {
    display: none;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 3px 4px;
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 4px;
    background: none;
    color: inherit;
    font-family: inherit;
    font-size: inherit;
    text-align: left;
    cursor: pointer;
    flex-shrink: 0;
    transition:
      background 150ms ease,
      border-color 150ms ease;
  }

  .row:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.08);
    border-color: rgba(255, 255, 255, 0.3);
  }

  .row:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .row.static {
    cursor: default;
  }

  .item-icon {
    width: 28px;
    height: 28px;
    image-rendering: pixelated;
    flex-shrink: 0;
  }

  .item-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .qty {
    flex-shrink: 0;
    color: #9fb2c3;
    font-variant-numeric: tabular-nums;
  }

  .empty-note {
    color: #6b7d8d;
    padding: 6px 4px;
  }

  .gold-row {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid rgba(255, 255, 255, 0.15);
  }

  .gold-row input {
    flex: 1;
    min-width: 0;
    padding: 3px 6px;
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 4px;
    background: rgba(0, 0, 0, 0.35);
    color: #ffd700;
    font-family: inherit;
    font-size: inherit;
  }

  .gold-row input:disabled {
    opacity: 0.4;
  }

  .gold-row button {
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

  .gold-row button:hover:not(:disabled) {
    color: #fff;
    border-color: rgba(255, 255, 255, 0.4);
  }

  .gold-row button:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .bag {
    display: flex;
    flex-direction: column;
    min-height: 0;
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid rgba(255, 255, 255, 0.15);
  }

  .bag .offer {
    max-height: 30vh;
  }

  footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid rgba(255, 255, 255, 0.15);
  }

  footer button {
    padding: 4px 14px;
    border-radius: 4px;
    border: 1px solid rgba(255, 255, 255, 0.2);
    background: none;
    color: #9fb2c3;
    font-family: inherit;
    font-size: 12px;
    font-weight: 700;
    cursor: pointer;
    transition:
      background 150ms ease,
      color 150ms ease;
  }

  footer button:hover:not(:disabled) {
    color: #fff;
    border-color: rgba(255, 255, 255, 0.4);
  }

  footer button.primary {
    background: rgba(60, 90, 60, 0.85);
    border-color: rgba(140, 220, 140, 0.35);
    color: #d6f0d6;
  }

  footer button.primary:hover:not(:disabled) {
    background: rgba(80, 120, 80, 0.95);
    color: #fff;
  }

  footer button:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .waiting {
    color: #9fb2c3;
  }

  @media (max-width: 600px), (pointer: coarse) {
    .player-trade {
      top: 40%;
      max-height: 60vh;
    }

    .tables {
      gap: 10px;
    }

    .row {
      min-height: 36px;
    }

    .close-btn {
      min-width: 32px;
      min-height: 32px;
      font-size: 22px;
    }

    footer button {
      min-height: 30px;
    }
  }
</style>
