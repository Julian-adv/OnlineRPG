import { get } from 'svelte/store'
import { beforeEach, describe, expect, it } from 'vitest'
import {
  equipmentBurden,
  isTorchItemDefId,
  inventoryStore,
  primaryArmorDefense,
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

describe('primary armor defense profile', () => {
  beforeEach(() => resetInventoryStore())

  it('excludes empty chest slots and ordinary clothing', () => {
    expect(get(primaryArmorDefense)).toBeNull()

    setInventory({
      bag: [],
      equipped: {
        chest: {
          instance_id: 1,
          item_def_id: 'traveler_robe',
          quantity: 1,
          enchant: 0,
          durability: null,
        },
      },
    })

    expect(get(primaryArmorDefense)).toBeNull()
  })

  it('projects the equipped chest definition into one active profile', () => {
    setInventory({
      bag: [],
      equipped: {
        chest: {
          instance_id: 2,
          item_def_id: 'brigandine_coat',
          quantity: 1,
          enchant: 0,
          durability: 75,
        },
      },
    })

    expect(get(primaryArmorDefense)).toEqual({
      itemDefId: 'brigandine_coat',
      name: 'Brigandine Coat',
      construction: 'hybrid',
      defenseSkill: 'hybrid_armor',
      functional: true,
      protection: { slash: 2, pierce: 2, blunt: 2 },
    })
  })

  it('keeps the authored profile visible but marks Broken armor inactive', () => {
    setInventory({
      bag: [],
      equipped: {
        chest: {
          instance_id: 3,
          item_def_id: 'padded_battle_robe',
          quantity: 1,
          enchant: 0,
          durability: 0,
        },
      },
    })

    expect(get(primaryArmorDefense)).toMatchObject({
      itemDefId: 'padded_battle_robe',
      functional: false,
      protection: { slash: 1, pierce: 0, blunt: 2 },
    })
  })
})
