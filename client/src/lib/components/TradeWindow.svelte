<script lang="ts">
  import { get } from 'svelte/store'
  import { assetUrl } from '../utils/assetUrl'
  import {
    shopSession,
    shopDeals,
    dealKey,
    type BuybackEntry,
    type DealKind,
  } from '../stores/tradeStore'
  import { gameStore } from '../stores/gameStore'
  import { remotePlayerManager } from '../managers/remotePlayerManager'
  import { inventoryStore, playerGold } from '../stores/inventoryStore'
  import { getItemDef, type ItemDefinition } from '../data/itemDefs'
  import { getNpcCapabilities } from '../data/traderDefs'
  import { isStockedByAnyMerchant } from '../data/merchantDefs'
  import {
    DEAL_MAX_HALF_BAND_PCT,
    MAX_TRADE_DISTANCE_METERS,
  } from '../data/tradeConstants'
  import GoldAmount from './GoldAmount.svelte'
  import { itemTooltip } from '../actions/itemTooltip'
  import { draggablePanel } from '../actions/draggablePanel'
  import { networkManager } from '../network/socket'
  import QuantityPopup from './QuantityPopup.svelte'
  import {
    groupBagForSelection,
    createGroupAllocator,
    type SelectableGroup,
  } from './inventoryGroups'

  const session = $derived($shopSession)

  interface CartEntry {
    kind: 'buy' | 'sell' | 'buyback'
    itemDefId: string
    /** Bag group key for a sell entry; backing instances resolve on confirm. */
    groupKey?: string
    /** Buyback entry backing a buyback entry; absent otherwise. */
    entryId?: number
    qty: number
    /** Per-unit price, fixed when the entry is added (prices cannot change
     *  within a shop session). */
    unitPrice: number
    /** Haggled modifier baked into unitPrice. Deal entries are single-use
     *  (the server consumes the deal on the first traded unit), so they
     *  stay at qty 1. */
    dealPct?: number
  }

  /** A row awaiting a quantity choice. */
  interface PendingAdd {
    kind: 'buy' | 'sell'
    itemDefId: string
    groupKey?: string
    def: ItemDefinition
    max: number
    defaultQty?: number
    unitPrice: number
  }

  let cart = $state<CartEntry[]>([])
  let pendingAdd = $state<PendingAdd | null>(null)
  let portraitFailed = $state(false)
  let now = $state(Date.now())

  // Reset the cart only when the merchant actually changes (a different shop
  // opens, or the window closes) — NOT on every ShopState refresh for the same
  // merchant. An NPC can push a refresh via its own dialogue/actions/deals, and
  // that must not wipe the items the player has staged to sell.
  let lastMerchantId: number | null = null
  $effect(() => {
    const id = session?.merchantPlayerId ?? null
    if (id !== lastMerchantId) {
      lastMerchantId = id
      cart = []
      portraitFailed = false
    }
  })

  const portraitSrc = $derived.by(() => {
    if (!session) return null
    const traderId = getNpcCapabilities(session.merchantName).traderId
    return traderId ? assetUrl(`/portraits/${traderId}.webp`) : null
  })

  /** Resident traders (wishlist, real stock) vs merchants (catalog). */
  const isResident = $derived(session !== null && session.wishlist.length > 0)

  // The server rejects trades beyond MAX_TRADE_DISTANCE_METERS; close the
  // window at the same range so the player isn't left with a shop that only
  // errors. A trading NPC is held in place server-side (TradeBusy) while its
  // window is open, so this only triggers when the *player* walks away.
  $effect(() => {
    if (!session) return
    const merchantId = session.merchantPlayerId
    const timer = setInterval(() => {
      now = Date.now()
      const me = get(gameStore).currentPlayer
      const merchant = remotePlayerManager.players.get(merchantId)
      if (!me || !merchant) {
        shopSession.set(null)
        return
      }
      const dx = me.position.x - merchant.position.x
      const dz = me.position.z - merchant.position.z
      if (dx * dx + dz * dz > MAX_TRADE_DISTANCE_METERS ** 2) {
        shopSession.set(null)
      }
    }, 300)
    return () => clearInterval(timer)
  })

  // Residents only buy their wishlist; merchants buy anything priced.
  // Grouped/sorted the same way the bag grid is: same-def stackable stacks
  // (potions, scrolls, food) merge into one row instead of one per fragment.
  const sellEntries = $derived.by((): SelectableGroup[] => {
    if (!session) return []
    const wishlist = session.wishlist
    return groupBagForSelection($inventoryStore.bag).filter((group) => {
      const basePrice = getItemDef(group.itemDefId)?.basePrice ?? 0
      return (
        basePrice > 0 &&
        (wishlist.length === 0 || wishlist.includes(group.itemDefId))
      )
    })
  })

  /** Live haggled modifier for an item, 0 when none (or expired). */
  function dealPct(itemDefId: string, kind: DealKind): number {
    if (!session) return 0
    const deal = $shopDeals[dealKey(session.merchantPlayerId, itemDefId, kind)]
    if (!deal || deal.expiresAt <= now) return 0
    return deal.modifierPct
  }

  /** True when a modifier works against the player (red badge):
   *  paying more on a buy, or being paid less on a sell. */
  function isMarkup(kind: DealKind, pct: number): boolean {
    return kind === 'buy' ? pct > 0 : pct < 0
  }

  // Mirrors the server's integer price math (deals.rs / trade_base_price).
  function indexedBase(def: ItemDefinition): number {
    const base = def.basePrice ?? 0
    if (!session || !def.consumable || !isStockedByAnyMerchant(def.id))
      return base
    return Math.max(1, Math.floor((base * session.priceIndexPercent) / 100))
  }

  function buyPrice(def: ItemDefinition, pct: number): number {
    return Math.max(1, Math.floor((indexedBase(def) * (100 + pct)) / 100))
  }

  function sellPrice(def: ItemDefinition, pct: number): number {
    if (!session) return 0
    const payout = Math.floor(
      ((def.basePrice ?? 0) * session.sellRatePercent * (100 + pct)) / 10000
    )
    // Merchants never pay above the cheapest possible buy (server sell_cap).
    const cap = isResident ? Infinity : buyPrice(def, -DEAL_MAX_HALF_BAND_PCT)
    return Math.max(1, Math.min(payout, cap))
  }

  const buyTotal = $derived(
    cart.reduce(
      (sum, e) => (e.kind !== 'sell' ? sum + e.unitPrice * e.qty : sum),
      0
    )
  )
  const sellTotal = $derived(
    cart.reduce(
      (sum, e) => (e.kind === 'sell' ? sum + e.unitPrice * e.qty : sum),
      0
    )
  )
  /** Net gold the player must pay; negative means the player earns gold.
   *  Residents pay sells out of a finite hidden wallet — the server rejects
   *  the trade ("They cannot afford that right now") when it runs dry. */
  const netCost = $derived(buyTotal - sellTotal)
  const canConfirm = $derived(cart.length > 0 && netCost <= $playerGold)

  /** Pre-selected buy amount; a stock or gold cap below this wins. */
  const DEFAULT_BUY_QTY = 10

  /** Buying a catalog item has no owned "stack" to bound quantity by, so the
   *  popup caps at what the player can currently afford — a UX convenience
   *  only; the server re-validates gold/weight for real on confirm. */
  function affordableQty(unitPrice: number): number {
    return Math.max(1, Math.floor($playerGold / Math.max(1, unitPrice)))
  }

  function addBuy(itemDefId: string, def: ItemDefinition, stockMax?: number) {
    // The first added unit carries any haggled deal (single-use server-side).
    const pct = dealPct(itemDefId, 'buy')
    const hasDealEntry = cart.some(
      (e) => e.kind === 'buy' && e.itemDefId === itemDefId && e.dealPct
    )
    if (pct !== 0 && !hasDealEntry) {
      cart.push({
        kind: 'buy',
        itemDefId,
        qty: 1,
        unitPrice: buyPrice(def, pct),
        dealPct: pct,
      })
      return
    }
    const unitPrice = indexedBase(def)
    const max =
      stockMax !== undefined
        ? Math.max(1, stockMax - reservedBuyQty(itemDefId))
        : affordableQty(unitPrice)
    if (!def.stackable || max <= 1) {
      addBuyUnits(itemDefId, unitPrice, 1)
      return
    }
    pendingAdd = {
      kind: 'buy',
      itemDefId,
      def,
      max,
      defaultQty: Math.min(max, DEFAULT_BUY_QTY, affordableQty(unitPrice)),
      unitPrice,
    }
  }

  function addBuyUnits(itemDefId: string, unitPrice: number, qty: number) {
    const existing = cart.find(
      (e) => e.kind === 'buy' && e.itemDefId === itemDefId && !e.dealPct
    )
    if (existing) {
      existing.qty += qty
    } else {
      cart.push({ kind: 'buy', itemDefId, qty, unitPrice })
    }
  }

  function addSell(group: SelectableGroup, def: ItemDefinition) {
    const pct = dealPct(group.itemDefId, 'sell')
    const hasDealEntry = cart.some(
      (e) => e.kind === 'sell' && e.itemDefId === group.itemDefId && e.dealPct
    )
    if (pct !== 0 && !hasDealEntry) {
      cart.push({
        kind: 'sell',
        itemDefId: group.itemDefId,
        groupKey: group.key,
        qty: 1,
        unitPrice: sellPrice(def, pct),
        dealPct: pct,
      })
      return
    }
    const max = group.totalQty - reservedQty(group.key)
    if (max <= 0) return
    const unitPrice = sellPrice(def, 0)
    if (max <= 1) {
      addSellUnits(group.itemDefId, group.key, unitPrice, 1)
      return
    }
    pendingAdd = {
      kind: 'sell',
      itemDefId: group.itemDefId,
      groupKey: group.key,
      def,
      max,
      unitPrice,
    }
  }

  function addSellUnits(
    itemDefId: string,
    groupKey: string,
    unitPrice: number,
    qty: number
  ) {
    const existing = cart.find(
      (e) => e.kind === 'sell' && e.groupKey === groupKey && !e.dealPct
    )
    if (existing) {
      existing.qty += qty
    } else {
      cart.push({ kind: 'sell', itemDefId, groupKey, qty, unitPrice })
    }
  }

  function confirmPendingAdd(qty: number) {
    if (!pendingAdd) return
    const { kind, itemDefId, groupKey, unitPrice } = pendingAdd
    if (kind === 'buy') {
      addBuyUnits(itemDefId, unitPrice, qty)
    } else if (groupKey !== undefined) {
      addSellUnits(itemDefId, groupKey, unitPrice, qty)
    }
    pendingAdd = null
  }

  function cancelPendingAdd() {
    pendingAdd = null
  }

  function addBuyback(entry: BuybackEntry) {
    if (inCartBuyback(entry.entryId)) return
    cart.push({
      kind: 'buyback',
      itemDefId: entry.itemDefId,
      entryId: entry.entryId,
      qty: 1,
      unitPrice: entry.price,
    })
  }

  /** Each buyback entry is one unit; it can only be staged once. */
  function inCartBuyback(entryId: number): boolean {
    return cart.some((e) => e.kind === 'buyback' && e.entryId === entryId)
  }

  function removeOne(entry: CartEntry) {
    entry.qty -= 1
    if (entry.qty <= 0) {
      cart = cart.filter((e) => e !== entry)
    }
  }

  /** Units of this bag group already reserved in the cart. */
  function reservedQty(groupKey: string): number {
    return cart
      .filter((e) => e.kind === 'sell' && e.groupKey === groupKey)
      .reduce((sum, e) => sum + e.qty, 0)
  }

  /** Buy units of this def already in the cart (caps resident stock buys). */
  function reservedBuyQty(itemDefId: string): number {
    return cart
      .filter((e) => e.kind === 'buy' && e.itemDefId === itemDefId)
      .reduce((sum, e) => sum + e.qty, 0)
  }

  /** Deal entries first, so the server's single-use modifier lands on the
   *  unit the cart priced with it. */
  function dealsFirst(entries: CartEntry[]): CartEntry[] {
    return [...entries].sort(
      (a, b) => Number(Boolean(b.dealPct)) - Number(Boolean(a.dealPct))
    )
  }

  function onConfirm() {
    if (!session || !canConfirm) return
    // A shared allocator so a deal-priced row and a plain row for the same
    // item def (same group) deplete one pool instead of each independently
    // draining the group's full instance list.
    const allocator = createGroupAllocator()
    const sellItems = dealsFirst(cart.filter((e) => e.kind === 'sell'))
      .filter((e) => e.groupKey !== undefined)
      .flatMap((e) => {
        const group = sellEntries.find((g) => g.key === e.groupKey)
        if (!group) return []
        return allocator
          .take(group, e.qty)
          .map((l) => ({ instance_id: l.instanceId, qty: l.qty }))
      })
    const buyItems = dealsFirst(cart.filter((e) => e.kind === 'buy')).map(
      (e) => ({ item_def_id: e.itemDefId, qty: e.qty })
    )
    const buybackIds = cart
      .filter((e) => e.kind === 'buyback' && e.entryId !== undefined)
      .map((e) => e.entryId!)

    // Sells first so their proceeds can fund the buys — each is its own
    // all-or-nothing batch; the connection processes them in send order.
    if (sellItems.length > 0) {
      networkManager.sendSellItems(session.merchantPlayerId, sellItems)
    }
    if (buyItems.length > 0) {
      networkManager.sendBuyItems(session.merchantPlayerId, buyItems)
    }
    if (buybackIds.length > 0) {
      networkManager.sendBuybackItems(session.merchantPlayerId, buybackIds)
    }
    cart = []
  }
</script>

{#if session}
  <div
    class="trade-window"
    role="dialog"
    aria-label="Trade"
    data-panel="trade"
    use:draggablePanel={'trade'}
  >
    {#if portraitSrc && !portraitFailed}
      <img
        class="merchant-portrait"
        src={portraitSrc}
        alt={session.merchantName}
        draggable="false"
        onerror={() => (portraitFailed = true)}
      />
    {/if}
    <div class="panel-header" data-drag-handle>
      <span class="panel-title">
        {isResident
          ? `Trade with ${session.merchantName}`
          : `${session.merchantName}'s Shop`}
      </span>
      <button class="close-btn" onclick={() => shopSession.set(null)}
        >&times;</button
      >
    </div>

    <div class="trade-columns">
      <div class="trade-column">
        <div class="column-title">Buy</div>
        <div class="item-list">
          {#each session.catalog as itemDefId (itemDefId)}
            {@const def = getItemDef(itemDefId)}
            {#if def}
              {@const pct = dealPct(itemDefId, 'buy')}
              <button
                class="item-row"
                onclick={() => addBuy(itemDefId, def)}
                use:itemTooltip={{ def, side: 'left' }}
              >
                <img
                  class="item-icon"
                  src="/items/{def.icon}"
                  alt=""
                  draggable="false"
                />
                <span class="item-name">{def.name}</span>
                {#if pct !== 0}
                  <span class="deal-badge" class:markup={isMarkup('buy', pct)}
                    >{pct > 0 ? '+' : ''}{pct}%</span
                  >
                {/if}
                <span class="item-price"
                  ><GoldAmount copper={buyPrice(def, pct)} /></span
                >
              </button>
            {/if}
          {/each}
          {#each session.stock as entry (entry.itemDefId)}
            {@const def = getItemDef(entry.itemDefId)}
            {#if def}
              {@const pct = dealPct(entry.itemDefId, 'buy')}
              <button
                class="item-row"
                disabled={reservedBuyQty(entry.itemDefId) >= entry.quantity}
                onclick={() => addBuy(entry.itemDefId, def, entry.quantity)}
                use:itemTooltip={{ def, side: 'left' }}
              >
                <img
                  class="item-icon"
                  src="/items/{def.icon}"
                  alt=""
                  draggable="false"
                />
                <span class="item-name">
                  {def.name}{entry.quantity > 1 ? ` ×${entry.quantity}` : ''}
                </span>
                {#if pct !== 0}
                  <span class="deal-badge" class:markup={isMarkup('buy', pct)}
                    >{pct > 0 ? '+' : ''}{pct}%</span
                  >
                {/if}
                <span class="item-price"
                  ><GoldAmount copper={buyPrice(def, pct)} /></span
                >
              </button>
            {/if}
          {:else}
            {#if isResident}
              <div class="empty-note">Nothing for sale</div>
            {/if}
          {/each}
          {#if session.buyback.length > 0}
            <div class="column-title buyback-title">Buy back</div>
            {#each session.buyback as entry (entry.entryId)}
              {@const def = getItemDef(entry.itemDefId)}
              {#if def}
                <button
                  class="item-row"
                  disabled={inCartBuyback(entry.entryId)}
                  onclick={() => addBuyback(entry)}
                  use:itemTooltip={{ def, side: 'left' }}
                >
                  <img
                    class="item-icon"
                    src="/items/{def.icon}"
                    alt=""
                    draggable="false"
                  />
                  <span class="item-name">
                    {entry.enchant > 0 ? `+${entry.enchant} ` : ''}{def.name}
                  </span>
                  <span class="item-price"
                    ><GoldAmount copper={entry.price} /></span
                  >
                </button>
              {/if}
            {/each}
          {/if}
        </div>
      </div>

      <div class="trade-column cart-column">
        <div class="cart-line cart-current">
          <span class="cart-label">Current</span>
          <GoldAmount copper={$playerGold} />
        </div>
        <div class="column-title">Cart</div>
        <div class="item-list">
          {#each cart as entry (entry.kind + ':' + (entry.groupKey ?? entry.entryId ?? entry.itemDefId) + (entry.dealPct ? ':deal' : ''))}
            {@const def = getItemDef(entry.itemDefId)}
            {#if def}
              <button
                class="item-row"
                onclick={() => removeOne(entry)}
                use:itemTooltip={{ def, side: 'left' }}
              >
                <span class="cart-kind {entry.kind}">
                  {entry.kind === 'sell' ? 'S' : 'B'}
                </span>
                <img
                  class="item-icon"
                  src="/items/{def.icon}"
                  alt=""
                  draggable="false"
                />
                <span class="item-name">
                  {def.name}{entry.qty > 1 ? ` ×${entry.qty}` : ''}
                </span>
                {#if entry.dealPct}
                  <span
                    class="deal-badge"
                    class:markup={isMarkup(
                      entry.kind === 'sell' ? 'sell' : 'buy',
                      entry.dealPct
                    )}
                  >
                    {entry.dealPct > 0 ? '+' : ''}{entry.dealPct}%
                  </span>
                {/if}
                <span class="item-price {entry.kind}">
                  {entry.kind === 'sell' ? '+' : '−'}<GoldAmount
                    copper={entry.unitPrice * entry.qty}
                  />
                </span>
              </button>
            {/if}
          {:else}
            <div class="empty-note">Click items to add</div>
          {/each}
        </div>
        <div class="cart-footer">
          <div class="cart-line">
            <span class="cart-label">Total</span>
            <span class="cart-total" class:earn={netCost < 0}>
              {netCost === 0 ? '' : netCost < 0 ? '+' : '−'}<GoldAmount
                copper={Math.abs(netCost)}
              />
            </span>
          </div>
          <div class="cart-line">
            <span class="cart-label">After</span>
            <GoldAmount copper={$playerGold - netCost} />
          </div>
          <button
            class="confirm-btn"
            disabled={!canConfirm}
            onclick={onConfirm}
          >
            Confirm
          </button>
        </div>
      </div>

      <div class="trade-column">
        <div class="column-title">Sell ({session.sellRatePercent}%)</div>
        <div class="item-list">
          {#each sellEntries as group (group.key)}
            {@const def = getItemDef(group.itemDefId)}
            {#if def}
              {@const reserved = reservedQty(group.key)}
              {@const pct = dealPct(group.itemDefId, 'sell')}
              <button
                class="item-row"
                disabled={reserved >= group.totalQty}
                onclick={() => addSell(group, def)}
                use:itemTooltip={{
                  def,
                  item: {
                    instance_id: group.instances[0].instanceId,
                    item_def_id: group.itemDefId,
                    quantity: group.totalQty,
                    enchant: group.enchant,
                  },
                  side: 'right',
                }}
              >
                <img
                  class="item-icon"
                  src="/items/{def.icon}"
                  alt=""
                  draggable="false"
                />
                <span class="item-name">
                  {def.name}{group.totalQty > 1 ? ` ×${group.totalQty}` : ''}
                </span>
                {#if pct !== 0}
                  <span class="deal-badge" class:markup={isMarkup('sell', pct)}
                    >{pct > 0 ? '+' : ''}{pct}%</span
                  >
                {/if}
                <span class="item-price"
                  ><GoldAmount copper={sellPrice(def, pct)} /></span
                >
              </button>
            {/if}
          {:else}
            <div class="empty-note">Nothing to sell</div>
          {/each}
        </div>
      </div>
    </div>
  </div>
{/if}

<QuantityPopup
  visible={pendingAdd !== null}
  itemName={pendingAdd?.def.name ?? ''}
  icon={pendingAdd?.def.icon ?? ''}
  max={pendingAdd?.max ?? 1}
  defaultQty={pendingAdd?.defaultQty}
  onConfirm={confirmPendingAdd}
  onCancel={cancelPendingAdd}
/>

<style>
  .trade-window {
    position: fixed;
    left: 50%;
    top: 45%;
    transform: translate(-50%, -50%);
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
    max-height: 70vh;
  }

  .merchant-portrait {
    position: absolute;
    left: 0;
    bottom: 100%;
    width: 160px;
    pointer-events: none;
    user-select: none;
    filter: drop-shadow(0 4px 8px rgba(0, 0, 0, 0.5));
  }

  .panel-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
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

  .trade-columns {
    display: flex;
    gap: 16px;
    overflow: hidden;
  }

  .trade-column {
    display: flex;
    flex-direction: column;
    width: 230px;
    min-width: 0;
  }

  .cart-column {
    width: 210px;
    padding: 0 10px;
    border-left: 1px solid rgba(255, 255, 255, 0.12);
    border-right: 1px solid rgba(255, 255, 255, 0.12);
  }

  .column-title {
    font-size: 12px;
    font-weight: 700;
    color: #9fb2c3;
    padding-bottom: 4px;
  }

  .buyback-title {
    margin-top: 8px;
    padding-top: 6px;
    border-top: 1px solid rgba(255, 255, 255, 0.15);
  }

  .item-list {
    overflow-y: auto;
    overscroll-behavior: contain;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 50vh;
    scrollbar-width: none;
  }

  .item-list::-webkit-scrollbar {
    display: none;
  }

  .item-row {
    display: flex;
    align-items: center;
    gap: 6px;
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

  .item-row:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.08);
    border-color: rgba(255, 255, 255, 0.3);
  }

  .item-row:disabled {
    opacity: 0.4;
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

  .item-price {
    color: #ffd700;
    flex-shrink: 0;
  }

  .item-price.buy,
  .item-price.buyback {
    color: #ff9a8a;
  }

  .item-price.sell {
    color: #8ae29a;
  }

  .cart-kind {
    flex-shrink: 0;
    width: 14px;
    font-weight: 700;
    text-align: center;
  }

  .cart-kind.buy,
  .cart-kind.buyback {
    color: #ff9a8a;
  }

  .cart-kind.sell {
    color: #8ae29a;
  }

  .deal-badge {
    flex-shrink: 0;
    padding: 0 4px;
    border-radius: 3px;
    font-weight: 700;
    background: rgba(60, 110, 60, 0.85);
    color: #b8f0b8;
  }

  .deal-badge.markup {
    background: rgba(120, 60, 60, 0.85);
    color: #f0b8b8;
  }

  .cart-current {
    padding-bottom: 4px;
    margin-bottom: 4px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.15);
  }

  .cart-footer {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 4px;
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid rgba(255, 255, 255, 0.15);
  }

  .cart-line {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
  }

  .cart-label {
    color: #9fb2c3;
    font-weight: 700;
  }

  .cart-total {
    font-weight: 700;
    color: #ff9a8a;
  }

  .cart-total.earn {
    color: #8ae29a;
  }

  .confirm-btn {
    margin-top: 4px;
    background: rgba(60, 90, 60, 0.85);
    color: #d6f0d6;
    border: 1px solid rgba(140, 220, 140, 0.35);
    border-radius: 4px;
    padding: 4px 14px;
    font-family: inherit;
    font-size: 12px;
    font-weight: 700;
    cursor: pointer;
    transition:
      background 150ms ease,
      color 150ms ease;
  }

  .confirm-btn:hover:not(:disabled) {
    background: rgba(80, 120, 80, 0.95);
    color: #fff;
  }

  .confirm-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .empty-note {
    color: #6b7d8d;
    padding: 6px 4px;
  }

  @media (max-width: 600px), (pointer: coarse) {
    .trade-window {
      top: 40%;
      max-height: 60vh;
    }

    .merchant-portrait {
      display: none;
    }

    .trade-columns {
      gap: 10px;
    }

    .trade-column {
      width: 170px;
    }

    .cart-column {
      width: 165px;
      padding: 0 6px;
    }

    .item-row {
      min-height: 36px;
    }

    .confirm-btn {
      min-height: 30px;
    }

    .close-btn {
      min-width: 32px;
      min-height: 32px;
      font-size: 22px;
    }
  }
</style>
