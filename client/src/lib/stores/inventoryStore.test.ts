import { get } from 'svelte/store'
import { beforeEach, describe, expect, it } from 'vitest'
import {
  equipmentBurden,
  resetInventoryStore,
  setEquipmentBurden,
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
