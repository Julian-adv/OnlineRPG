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

  it('sorts equal names by quantity descending', () => {
    const bag = [
      makeItem({
        instance_id: 1,
        item_def_id: 'healing_potion',
        quantity: 2,
      }),
      makeItem({
        instance_id: 2,
        item_def_id: 'healing_potion',
        quantity: 5,
        enchant: 1,
      }),
    ]

    expect(sortBag(bag).map((item) => item.quantity)).toEqual([5, 2])
  })
})

describe('runs that wear no slot', () => {
  const order = (ids: string[]) =>
    sortBag(
      ids.map((item_def_id, i) => makeItem({ instance_id: i + 1, item_def_id }))
    ).map((item) => item.item_def_id)

  /** Both kinds of arrow and a boot are slotless, so name order alone put the
   *  boot between them. */
  it('keeps the quiver together instead of letting junk split it', () => {
    expect(order(['iron_arrow', 'old_boot', 'steel_arrow'])).toEqual([
      'steel_arrow',
      'iron_arrow',
      'old_boot',
    ])
  })

  it('files rounds with the hand that spends them, ahead of the off hand', () => {
    expect(order(['wooden_shield', 'iron_arrow', 'bow'])).toEqual([
      'bow',
      'iron_arrow',
      'wooden_shield',
    ])
  })

  /** Strongest first: reading the quiver top-down answers what the next shot
   *  fires, which is the strongest the bag holds. */
  it('runs rounds strongest first rather than alphabetically', () => {
    expect(order(['iron_arrow', 'steel_arrow'])).toEqual([
      'steel_arrow',
      'iron_arrow',
    ])
  })

  /** "(10F)" leads "(5F)" under plain collation, which is the only order a
   *  run of keys has. */
  it('orders dungeon keys by floor, not by digit', () => {
    expect(
      order(['ogre_key_15', 'ogre_key_5', 'ogre_key_10', 'crypt_key_5'])
    ).toEqual(['crypt_key_5', 'ogre_key_5', 'ogre_key_10', 'ogre_key_15'])
  })

  it('groups keys away from other slotless odds and ends', () => {
    expect(order(['old_boot', 'ogre_key_5', 'clump_of_kelp'])).toEqual([
      'ogre_key_5',
      'clump_of_kelp',
      'old_boot',
    ])
  })
})
