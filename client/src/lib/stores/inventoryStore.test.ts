import { get } from 'svelte/store'
import { beforeEach, describe, expect, it } from 'vitest'
import {
  equipmentBurden,
  isTorchItemDefId,
  inventoryStore,
  resetInventoryStore,
  setEquipmentBurden,
  setInventory,
} from './inventoryStore'

describe('equipment burden state', () => {
  beforeEach(() => resetInventoryStore())

  it('stores the server-authored tier and effective movement speed', () => {
    setEquipmentBurden({
      equipped_weight: 43,
      max_carry_weight: 150,
      tier: 'light',
      movement_speed: 2.7,
    })

    expect(get(equipmentBurden)).toEqual({
      equipped_weight: 43,
      max_carry_weight: 150,
      tier: 'light',
      movement_speed: 2.7,
    })
  })

  it('clears stale burden state on disconnect', () => {
    setEquipmentBurden({
      equipped_weight: 80,
      max_carry_weight: 150,
      tier: 'heavy',
      movement_speed: 2.1,
    })
    resetInventoryStore()
    expect(get(equipmentBurden)).toBeNull()
  })
})

describe('isTorchItemDefId', () => {
  it('recognizes every carried torch variant', () => {
    expect(isTorchItemDefId('torch')).toBe(true)
    expect(isTorchItemDefId('worn_torch')).toBe(true)
  })

  it('rejects missing and unrelated item definitions', () => {
    expect(isTorchItemDefId(undefined)).toBe(false)
    expect(isTorchItemDefId(null)).toBe(false)
    expect(isTorchItemDefId('dagger')).toBe(false)
  })
})

describe('item condition state', () => {
  beforeEach(() => resetInventoryStore())

  it('keeps server-authored durability on bag and equipped instances', () => {
    setInventory({
      bag: [
        {
          instance_id: 1,
          item_def_id: 'leather_repair_kit',
          quantity: 2,
          enchant: 0,
          durability: null,
        },
      ],
      equipped: {
        chest: {
          instance_id: 2,
          item_def_id: 'leather_armor',
          quantity: 1,
          enchant: 0,
          durability: 17,
        },
      },
    })

    expect(get(inventoryStore).equipped.chest?.durability).toBe(17)
    expect(get(inventoryStore).bag[0].durability).toBeNull()
  })
})
