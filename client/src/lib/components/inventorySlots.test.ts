import { describe, expect, it } from 'vitest'
import type { ItemInstance } from '../network/networkTypes'
import { buildInventorySlots } from './inventorySlots'

function makeItem(instanceId: number): ItemInstance {
  return {
    instance_id: instanceId,
    item_def_id: `item-${instanceId}`,
    quantity: 1,
    enchant: 0,
  }
}

describe('buildInventorySlots', () => {
  it('renders 50 empty slots for an empty inventory', () => {
    expect(buildInventorySlots([])).toEqual(new Array(50).fill(null))
  })

  it('keeps a minimum of 50 slots for short inventories', () => {
    const bag = Array.from({ length: 49 }, (_, index) => makeItem(index + 1))
    const slots = buildInventorySlots(bag)

    expect(slots).toHaveLength(50)
    expect(slots.slice(0, bag.length)).toEqual(bag)
    expect(slots[49]).toBeNull()
  })

  it('keeps exactly 50 items without adding another row', () => {
    const bag = Array.from({ length: 50 }, (_, index) => makeItem(index + 1))
    const slots = buildInventorySlots(bag)

    expect(slots).toHaveLength(50)
    expect(slots).toEqual(bag)
  })

  it.each([
    [51, 55],
    [56, 60],
  ])(
    'expands %i items to %i slots without truncation',
    (itemCount, slotCount) => {
      const bag = Array.from({ length: itemCount }, (_, index) =>
        makeItem(index + 1)
      )
      const slots = buildInventorySlots(bag)

      expect(slots).toHaveLength(slotCount)
      expect(slots.slice(0, bag.length)).toEqual(bag)
      expect(slots[itemCount - 1]).toBe(bag[itemCount - 1])
      expect(slots.slice(itemCount)).toEqual(
        new Array(slotCount - itemCount).fill(null)
      )
    }
  )
})
