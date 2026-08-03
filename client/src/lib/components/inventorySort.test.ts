import { describe, expect, it } from 'vitest'
import type { ItemInstance } from '../network/networkTypes'
import { sortBag } from './inventorySort'

function makeItem(
  overrides: Partial<ItemInstance> & { instance_id: number }
): ItemInstance {
  return {
    item_def_id: 'iron_sword',
    quantity: 1,
    enchant: 0,
    ...overrides,
  }
}

describe('sortBag', () => {
  it('merges stackable stacks of the same item and enchant', () => {
    const bag = [
      makeItem({ instance_id: 1, item_def_id: 'healing_potion', quantity: 30 }),
      makeItem({ instance_id: 2, item_def_id: 'healing_potion', quantity: 70 }),
    ]

    const result = sortBag(bag)

    expect(result).toHaveLength(1)
    expect(result[0].quantity).toBe(100)
  })

  it('keeps different enchant levels of the same item separate', () => {
    const bag = [
      makeItem({ instance_id: 1, item_def_id: 'iron_sword', enchant: 0 }),
      makeItem({ instance_id: 2, item_def_id: 'iron_sword', enchant: 1 }),
    ]

    const result = sortBag(bag)

    expect(result).toHaveLength(2)
  })

  it('does not merge duplicate non-stackable items', () => {
    const bag = [
      makeItem({ instance_id: 1, item_def_id: 'iron_sword' }),
      makeItem({ instance_id: 2, item_def_id: 'iron_sword' }),
    ]

    const result = sortBag(bag)

    expect(result).toHaveLength(2)
    expect(result.every((item) => item.quantity === 1)).toBe(true)
  })

  it('groups by equip slot category before consumables/misc', () => {
    const bag = [
      makeItem({ instance_id: 1, item_def_id: 'healing_potion', quantity: 1 }),
      makeItem({ instance_id: 2, item_def_id: 'leather_helmet' }),
      makeItem({ instance_id: 3, item_def_id: 'iron_sword' }),
    ]

    const result = sortBag(bag)

    expect(result.map((item) => item.item_def_id)).toEqual([
      'leather_helmet',
      'iron_sword',
      'healing_potion',
    ])
  })

  it('sorts same-category items alphabetically by name', () => {
    const bag = [
      makeItem({ instance_id: 1, item_def_id: 'worn_iron_sword' }),
      makeItem({ instance_id: 2, item_def_id: 'iron_sword' }),
    ]

    const result = sortBag(bag)

    expect(result.map((item) => item.item_def_id)).toEqual([
      'iron_sword',
      'worn_iron_sword',
    ])
  })
})
