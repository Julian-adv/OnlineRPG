import { durability_value_percent } from '../wasm/onlinerpg_shared'
import type { ItemInstance } from '../network/networkTypes'
import type { ItemDefinition } from './itemDefs'

/** Mirrors the server's integer sell math, including durable-item value. */
export function npcSellPrice(
  def: ItemDefinition,
  item: ItemInstance,
  sellRatePercent: number,
  modifierPercent: number
): number {
  const standardPayout = Math.max(
    1,
    Math.floor(
      ((def.basePrice ?? 0) * sellRatePercent * (100 + modifierPercent)) / 10000
    )
  )
  const conditionPercent =
    def.maxDurability && item.durability != null
      ? (durability_value_percent(item.durability, def.maxDurability) ?? 100)
      : 100
  return Math.max(1, Math.floor((standardPayout * conditionPercent) / 100))
}
