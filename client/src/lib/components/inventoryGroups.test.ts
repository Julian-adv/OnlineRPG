import { describe, expect, it } from 'vitest'
import {
  createGroupAllocator,
  splitGroupQty,
  type SelectableGroup,
} from './inventoryGroups'

const group: SelectableGroup = {
  key: 'healing_potion:0',
  itemDefId: 'healing_potion',
  enchant: 0,
  totalQty: 5,
  instances: [
    { instanceId: 1, quantity: 3 },
    { instanceId: 2, quantity: 2 },
  ],
}

describe('inventory group allocation', () => {
  it('splits one request across backing instances', () => {
    expect(splitGroupQty(group, 4)).toEqual([
      { instanceId: 1, qty: 3 },
      { instanceId: 2, qty: 1 },
    ])
  })

  it('shares capacity across requests', () => {
    const allocator = createGroupAllocator()

    expect(allocator.take(group, 2)).toEqual([{ instanceId: 1, qty: 2 }])
    expect(allocator.take(group, 3)).toEqual([
      { instanceId: 1, qty: 1 },
      { instanceId: 2, qty: 2 },
    ])
  })
})
