import { writable } from 'svelte/store'

/** One purchasable item in a non-merchant trader's real inventory. */
export interface StockEntry {
  itemDefId: string
  quantity: number
}

/** One unit the player recently sold to a merchant, repurchasable at the
 *  exact payout received (sold units otherwise vanish — merchants keep no
 *  stock). */
export interface BuybackEntry {
  entryId: number
  itemDefId: string
  enchant: number
  price: number
}

/** An open shop session with a trading NPC, driven by ServerMessage::ShopState.
 *  Set to a session to open the trade window, to null to close it.
 *  Merchants fill `catalog` (unlimited stock); resident traders fill
 *  `wishlist`/`stock` instead and pay out of a finite wallet the server
 *  keeps hidden (like another player's gold). */
export interface ShopSession {
  merchantPlayerId: number
  merchantName: string
  catalog: string[]
  sellRatePercent: number
  /** Non-merchants only buy these item defs; empty = buys anything priced. */
  wishlist: string[]
  /** Non-merchant real-inventory stock; merchants use `catalog`. */
  stock: StockEntry[]
  /** Units this player recently sold to this merchant; empty for
   *  non-merchants, whose bought units stay visible in `stock`. */
  buyback: BuybackEntry[]
}

export const shopSession = writable<ShopSession | null>(null)

/** Merchant id the player explicitly asked to trade with (sendOpenShop),
 *  so unsolicited NPC-pushed ShopStates can be told apart from replies. */
let requestedShop: { merchantPlayerId: number; at: number } | null = null
const SHOP_REQUEST_TTL_MS = 5_000

export function markShopRequested(merchantPlayerId: number) {
  requestedShop = { merchantPlayerId, at: Date.now() }
}

/** True if the player recently requested this merchant's shop themselves. */
export function wasShopRequested(merchantPlayerId: number): boolean {
  return (
    requestedShop !== null &&
    requestedShop.merchantPlayerId === merchantPlayerId &&
    Date.now() - requestedShop.at < SHOP_REQUEST_TTL_MS
  )
}

/** An NPC-pushed (open_trade) shop the player hasn't accepted yet. The
 *  trade window covers much of the screen, so unsolicited pushes only show
 *  a small offer toast until the player opts in. */
export interface PendingTradeOffer {
  session: ShopSession
  offeredAt: number
}

export const pendingTradeOffer = writable<PendingTradeOffer | null>(null)

export function acceptTradeOffer(offer: PendingTradeOffer) {
  pendingTradeOffer.set(null)
  shopSession.set(offer.session)
}

export function declineTradeOffer() {
  pendingTradeOffer.set(null)
}

/** Which side of a trade a haggled deal applies to (mirrors DealKind). */
export type DealKind = 'buy' | 'sell'

/** A haggled price modifier granted by an LLM NPC (economy phase 2).
 *  Single-use: the server consumes it on the first traded unit. */
export interface ShopDeal {
  /** Percentage points added to the normal price (negative = buy discount,
   *  positive = sell bonus). */
  modifierPct: number
  /** Epoch ms when the deal lapses (server enforces the real expiry). */
  expiresAt: number
}

/** Active deals keyed by `dealKey(...)`. */
export const shopDeals = writable<Record<string, ShopDeal>>({})

export function dealKey(
  merchantPlayerId: number,
  itemDefId: string,
  kind: DealKind
): string {
  return `${merchantPlayerId}|${itemDefId}|${kind}`
}

/** Apply a DealUpdated message; a modifier of 0 clears the deal. */
export function applyDealUpdate(
  merchantPlayerId: number,
  itemDefId: string,
  kind: DealKind,
  modifierPct: number,
  expiresInSecs: number
) {
  shopDeals.update((deals) => {
    const key = dealKey(merchantPlayerId, itemDefId, kind)
    const next = { ...deals }
    if (modifierPct === 0) {
      delete next[key]
    } else {
      next[key] = { modifierPct, expiresAt: Date.now() + expiresInSecs * 1000 }
    }
    return next
  })
}

/** Replace all deals for one merchant (from ShopState.active_deals). */
export function setMerchantDeals(
  merchantPlayerId: number,
  deals: {
    item_def_id: string
    kind: DealKind
    modifier_pct: number
    expires_in_secs: number
  }[]
) {
  shopDeals.update((existing) => {
    const next: Record<string, ShopDeal> = {}
    for (const [key, deal] of Object.entries(existing)) {
      if (!key.startsWith(`${merchantPlayerId}|`)) next[key] = deal
    }
    for (const d of deals) {
      next[dealKey(merchantPlayerId, d.item_def_id, d.kind)] = {
        modifierPct: d.modifier_pct,
        expiresAt: Date.now() + d.expires_in_secs * 1000,
      }
    }
    return next
  })
}

/** Whether this merchant is holding a live haggled price for the player.
 *  The amounts belong to the trade window's badges and the server's chat
 *  notice; this only marks that one is waiting. */
export function hasLiveDeal(
  deals: Record<string, ShopDeal>,
  merchantPlayerId: number
): boolean {
  const prefix = `${merchantPlayerId}|`
  const now = Date.now()
  return Object.entries(deals).some(
    ([key, deal]) =>
      key.startsWith(prefix) && deal.modifierPct !== 0 && deal.expiresAt > now
  )
}
