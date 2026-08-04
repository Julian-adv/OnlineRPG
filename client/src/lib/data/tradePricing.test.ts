import { describe, expect, it, vi } from 'vitest'
import type { ItemInstance } from '../network/networkTypes'
import { getItemDef } from './itemDefs'
import { npcSellPrice } from './tradePricing'

const wasm = vi.hoisted(() => ({
  durability_value_percent: vi.fn((current: number, max: number) =>
    max === 0 ? undefined : 25 + Math.floor((75 * Math.min(current, max)) / max)
  ),
}))

vi.mock('../wasm/onlinerpg_shared', () => wasm)

function item(itemDefId: string, durability?: number | null): ItemInstance {
  return {
    instance_id: 1,
    item_def_id: itemDefId,
    quantity: 1,
    enchant: 0,
    durability,
  }
}

describe('NPC sell pricing', () => {
  it('applies smooth condition value after the normal merchant offer', () => {
    const leather = getItemDef('leather_armor')!
    expect(npcSellPrice(leather, item('leather_armor', 17), 40, 0)).toBe(1104)
    expect(npcSellPrice(leather, item('leather_armor', 17), 40, 25)).toBe(1380)
    expect(wasm.durability_value_percent).toHaveBeenLastCalledWith(17, 60)
  })

  it('keeps full gear at full value and broken gear at the salvage floor', () => {
    const leather = getItemDef('leather_armor')!
    expect(npcSellPrice(leather, item('leather_armor', 60), 40, 0)).toBe(2400)
    expect(npcSellPrice(leather, item('leather_armor', 0), 40, 0)).toBe(600)
  })

  it('does not discount non-durable or legacy missing-condition items', () => {
    const sword = getItemDef('iron_sword')!
    expect(npcSellPrice(sword, item('iron_sword'), 40, 0)).toBe(4000)

    const leather = getItemDef('leather_armor')!
    expect(npcSellPrice(leather, item('leather_armor', null), 40, 0)).toBe(2400)
  })
})
