import { describe, expect, it, beforeEach } from 'vitest'
import { get } from 'svelte/store'
import {
  shopDeals,
  applyDealUpdate,
  setMerchantDeals,
  hasLiveDeal,
} from './tradeStore'

const deal = (item: string, pct: number, secs = 300) => ({
  item_def_id: item,
  kind: 'buy' as const,
  modifier_pct: pct,
  expires_in_secs: secs,
})

describe('hasLiveDeal', () => {
  beforeEach(() => shopDeals.set({}))

  it('ignores other merchants, zero modifiers and lapsed deals', () => {
    setMerchantDeals(7, [deal('torch', 0), deal('rope', -5, 0)])
    setMerchantDeals(9, [deal('dagger', -20)])
    expect(hasLiveDeal(get(shopDeals), 7)).toBe(false)

    setMerchantDeals(7, [deal('bread', -10)])
    expect(hasLiveDeal(get(shopDeals), 7)).toBe(true)
  })

  it('follows a DealUpdated push and its clearing', () => {
    applyDealUpdate(7, 'healing_potion', 'sell', 10, 300)
    expect(hasLiveDeal(get(shopDeals), 7)).toBe(true)
    applyDealUpdate(7, 'healing_potion', 'sell', 0, 300)
    expect(hasLiveDeal(get(shopDeals), 7)).toBe(false)
  })
})
