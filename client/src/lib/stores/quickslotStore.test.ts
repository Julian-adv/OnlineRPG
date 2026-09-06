import { get } from 'svelte/store'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  assignQuickslot,
  loadQuickslots,
  quickslotAction,
  quickslots,
  resetQuickslots,
  resolveQuickslot,
  type CarriedItem,
} from './quickslotStore'

// The node test environment has no localStorage.
const storage = new Map<string, string>()
vi.stubGlobal('localStorage', {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => void storage.set(key, value),
})

beforeEach(() => {
  storage.clear()
  resetQuickslots()
})

function item(
  instance_id: number,
  item_def_id: string,
  enchant = 0,
  quantity = 1
): CarriedItem {
  return { instance_id, item_def_id, enchant, quantity }
}

const SHIELD = { equipSlot: 'off_hand' as const }
const SWORD = { equipSlot: 'main_hand' as const }
const MAIL = { equipSlot: 'chest' as const }
const POTION = { consumable: true }
const STORAGE = { useAction: 'estate_storage' as const }

describe('resolveQuickslot', () => {
  it('acts on exactly the bound level when it is carried (#148)', () => {
    const bag = [item(1, 'shield', 0), item(2, 'shield', 3)]
    const plus3 = resolveQuickslot(
      { defId: 'shield', enchant: 3 },
      SHIELD,
      {},
      bag
    )
    expect(plus3.enchant).toBe(3)
    expect(plus3.bagItem?.instance_id).toBe(2)
    expect(plus3.qty).toBe(1)
    const plain = resolveQuickslot(
      { defId: 'shield', enchant: 0 },
      SHIELD,
      {},
      bag
    )
    expect(plain.enchant).toBe(0)
    expect(plain.bagItem?.instance_id).toBe(1)
  })

  it('toggles off the worn item at the bound level', () => {
    const resolved = resolveQuickslot(
      { defId: 'shield', enchant: 3 },
      SHIELD,
      { off_hand: item(2, 'shield', 3) },
      [item(1, 'shield', 0)]
    )
    expect(resolved.wornSlot).not.toBeNull()
    expect(resolved.bagItem).toBeUndefined()
  })

  it('follows the worn copy when it was enchanted in place, even past a stronger spare', () => {
    const resolved = resolveQuickslot(
      { defId: 'iron_sword', enchant: 2 },
      SWORD,
      { main_hand: item(9, 'iron_sword', 3) },
      [item(5, 'iron_sword', 5)]
    )
    expect(resolved.enchant).toBe(3)
    expect(resolved.wornSlot).not.toBeNull()
  })

  it('still toggles off a drifted worn copy when a bag spare sits at the bound level', () => {
    const resolved = resolveQuickslot(
      { defId: 'iron_sword', enchant: 3 },
      SWORD,
      { main_hand: item(9, 'iron_sword', 4) },
      [item(5, 'iron_sword', 3)]
    )
    expect(resolved.enchant).toBe(4)
    expect(resolved.wornSlot).not.toBeNull()
    expect(resolved.bagItem).toBeUndefined()
  })

  it('picks the bag copy closest to the bound level, not the strongest', () => {
    const resolved = resolveQuickslot(
      { defId: 'iron_sword', enchant: 2 },
      SWORD,
      {},
      [item(5, 'iron_sword', 5), item(9, 'iron_sword', 3)]
    )
    expect(resolved.enchant).toBe(3)
    expect(resolved.bagItem?.instance_id).toBe(9)
  })

  it('keeps an any-level binding on the worn copy over a fresh plain spare', () => {
    const resolved = resolveQuickslot(
      { defId: 'chain_mail', enchant: null },
      MAIL,
      { chest: item(9, 'chain_mail', 4) },
      [item(5, 'chain_mail', 0)]
    )
    expect(resolved.enchant).toBe(4)
    expect(resolved.wornSlot).not.toBeNull()
  })

  it('sends an any-level binding to the best bag copy when nothing is worn', () => {
    const resolved = resolveQuickslot(
      { defId: 'iron_sword', enchant: null },
      SWORD,
      {},
      [item(5, 'iron_sword', 1), item(9, 'iron_sword', 4)]
    )
    expect(resolved.enchant).toBe(4)
    expect(resolved.bagItem?.instance_id).toBe(9)
  })

  it('goes inert but keeps the bound level when the def is gone entirely', () => {
    const resolved = resolveQuickslot(
      { defId: 'iron_sword', enchant: 2 },
      SWORD,
      {},
      [item(1, 'shield', 2)]
    )
    expect(resolved).toEqual({
      enchant: 2,
      wornSlot: null,
      bagItem: undefined,
      qty: 0,
    })
  })

  it('sums bag quantity only at the resolved level', () => {
    const resolved = resolveQuickslot(
      { defId: 'healing_potion', enchant: 0 },
      POTION,
      {},
      [item(1, 'healing_potion', 0, 5), item(2, 'healing_potion', 0, 2)]
    )
    expect(resolved.qty).toBe(7)
  })
})

describe('quickslotAction', () => {
  it('unequips the worn copy even when a bag spare matches the bound level', () => {
    const resolved = resolveQuickslot(
      { defId: 'iron_sword', enchant: 3 },
      SWORD,
      { main_hand: item(9, 'iron_sword', 4) },
      [item(5, 'iron_sword', 3)]
    )
    expect(quickslotAction(SWORD, resolved)).toEqual({
      kind: 'unequip',
      slot: 'main_hand',
    })
  })

  it('equips exactly the bound instance (#148)', () => {
    const resolved = resolveQuickslot(
      { defId: 'shield', enchant: 3 },
      SHIELD,
      {},
      [item(1, 'shield', 0), item(2, 'shield', 3)]
    )
    expect(quickslotAction(SHIELD, resolved)).toEqual({
      kind: 'equip',
      instanceId: 2,
    })
  })

  it('consumes from the bag and goes inert when nothing is carried', () => {
    const resolved = resolveQuickslot(
      { defId: 'healing_potion', enchant: 0 },
      POTION,
      {},
      [item(1, 'healing_potion', 0, 5)]
    )
    expect(quickslotAction(POTION, resolved)).toEqual({
      kind: 'use',
      instanceId: 1,
    })
    const empty = resolveQuickslot(
      { defId: 'healing_potion', enchant: 0 },
      POTION,
      {},
      []
    )
    expect(quickslotAction(POTION, empty)).toBeNull()
  })

  it('uses a placement item from its data action', () => {
    const resolved = resolveQuickslot(
      { defId: 'storage_chest', enchant: 0 },
      STORAGE,
      {},
      [item(8, 'storage_chest')]
    )
    expect(quickslotAction(STORAGE, resolved)).toEqual({
      kind: 'use',
      instanceId: 8,
    })
  })
})

describe('assignQuickslot', () => {
  it('keeps one binding per item type, replacing any level of the same def', () => {
    assignQuickslot(0, { defId: 'shield', enchant: 3 })
    assignQuickslot(4, { defId: 'shield', enchant: 0 })
    const slots = get(quickslots)
    expect(slots[0]).toBeNull()
    expect(slots[4]).toEqual({ defId: 'shield', enchant: 0 })
  })

  it('leaves other defs alone', () => {
    assignQuickslot(0, { defId: 'shield', enchant: 3 })
    assignQuickslot(1, { defId: 'iron_sword', enchant: 3 })
    expect(get(quickslots)[0]).toEqual({ defId: 'shield', enchant: 3 })
  })
})

describe('loadQuickslots', () => {
  it('round-trips assignments through storage', () => {
    loadQuickslots(7)
    assignQuickslot(2, { defId: 'shield', enchant: 3 })
    quickslots.set([])
    loadQuickslots(7)
    expect(get(quickslots)[2]).toEqual({ defId: 'shield', enchant: 3 })
  })

  it('reads pre-enchant entries (plain def ids) as any-level bindings', () => {
    storage.set(
      'quickslots:7',
      JSON.stringify(['healing_potion', null, 'shield'])
    )
    loadQuickslots(7)
    const slots = get(quickslots)
    expect(slots[0]).toEqual({ defId: 'healing_potion', enchant: null })
    expect(slots[1]).toBeNull()
    expect(slots[2]).toEqual({ defId: 'shield', enchant: null })
  })

  it('drops corrupt or unrecognized entries', () => {
    storage.set(
      'quickslots:7',
      JSON.stringify([42, { defId: 'shield' }, { defId: 'sword', enchant: 1 }])
    )
    loadQuickslots(7)
    const slots = get(quickslots)
    expect(slots[0]).toBeNull()
    expect(slots[1]).toBeNull()
    expect(slots[2]).toEqual({ defId: 'sword', enchant: 1 })
  })
})
