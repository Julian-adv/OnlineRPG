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

    expect(buildInventorySlots(bag)).toEqual([...bag, null])
  })

  it('keeps exactly 50 items without adding another row', () => {
    const bag = Array.from({ length: 50 }, (_, index) => makeItem(index + 1))

    expect(buildInventorySlots(bag)).toEqual(bag)
  })

  it.each([
    [51, 55],
    [55, 55],
  ])(
    'expands %i items to %i slots without truncation',
    (itemCount, slotCount) => {
      const bag = Array.from({ length: itemCount }, (_, index) =>
        makeItem(index + 1)
      )

      expect(buildInventorySlots(bag)).toEqual([
        ...bag,
        ...new Array(slotCount - itemCount).fill(null),
      ])
    }
  )
})
