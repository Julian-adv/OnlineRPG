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
  type EquipSlot,
  type ItemInstance,
  type PrimaryArmorDefense,
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
      coveredRegions: ['torso', 'arms'],
      missingRegions: ['head', 'hands', 'legs', 'feet'],
      coveragePercent: 55,
      protection: { slash: 2, pierce: 2, blunt: 2 },
      effectiveProtection: { slash: 2, pierce: 2, blunt: 2 },
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
      coveredRegions: [],
      missingRegions: ['head', 'torso', 'arms', 'hands', 'legs', 'feet'],
      coveragePercent: 0,
      protection: { slash: 1, pierce: 0, blunt: 2 },
      effectiveProtection: { slash: 0, pierce: 0, blunt: 0 },
    })
  })

  it('unions equipped regions and restores the complete plate profile', () => {
    setInventory({
      bag: [],
      equipped: {
        head: {
          instance_id: 4,
          item_def_id: 'plate_helmet',
          quantity: 1,
          enchant: 0,
          durability: null,
        },
        chest: {
          instance_id: 5,
          item_def_id: 'breastplate',
          quantity: 1,
          enchant: 0,
          durability: 120,
        },
        hands: {
          instance_id: 6,
          item_def_id: 'plate_gauntlets',
          quantity: 1,
          enchant: 0,
          durability: null,
        },
        pants: {
          instance_id: 7,
          item_def_id: 'plate_greaves',
          quantity: 1,
          enchant: 0,
          durability: null,
        },
        boots: {
          instance_id: 8,
          item_def_id: 'plate_boots',
          quantity: 1,
          enchant: 0,
          durability: null,
        },
      },
    })

    expect(get(primaryArmorDefense)).toMatchObject({
      coveragePercent: 85,
      coveredRegions: ['head', 'torso', 'hands', 'legs', 'feet'],
      missingRegions: ['arms'],
      effectiveProtection: { slash: 3, pierce: 3, blunt: 1 },
    })

    const plateInventory = get(inventoryStore)
    setInventory({
      ...plateInventory,
      equipped: {
        ...plateInventory.equipped,
        chest: {
          instance_id: 9,
          item_def_id: 'chain_mail',
          quantity: 1,
          enchant: 0,
          durability: 90,
        },
      },
    })
    expect(get(primaryArmorDefense)).toMatchObject({
      coveragePercent: 100,
      coveredRegions: ['head', 'torso', 'arms', 'hands', 'legs', 'feet'],
      missingRegions: [],
      effectiveProtection: { slash: 2, pierce: 1, blunt: 0 },
    })
  })

  it('matches the current armor loadout contract', () => {
    const cases: Array<{
      name: string
      loadout: Array<[EquipSlot, string]>
      expected: Partial<PrimaryArmorDefense> | null
    }> = [
      {
        name: 'clothing',
        loadout: [['chest', 'traveler_robe']],
        expected: null,
      },
      {
        name: 'padded',
        loadout: [['chest', 'padded_battle_robe']],
        expected: {
          itemDefId: 'padded_battle_robe',
          construction: 'padded',
          defenseSkill: 'padded_armor',
          functional: true,
          coveredRegions: ['torso', 'arms', 'legs'],
          missingRegions: ['head', 'hands', 'feet'],
          coveragePercent: 75,
          protection: { slash: 1, pierce: 0, blunt: 2 },
          effectiveProtection: { slash: 1, pierce: 0, blunt: 2 },
        },
      },
      {
        name: 'leather',
        loadout: [
          ['head', 'leather_helmet'],
          ['chest', 'leather_armor'],
          ['hands', 'leather_gloves'],
          ['pants', 'leather_pants'],
          ['boots', 'leather_boots'],
        ],
        expected: {
          itemDefId: 'leather_armor',
          construction: 'leather',
          defenseSkill: 'leather_armor',
          functional: true,
          coveredRegions: ['head', 'torso', 'hands', 'legs', 'feet'],
          missingRegions: ['arms'],
          coveragePercent: 85,
          protection: { slash: 1, pierce: 1, blunt: 1 },
          effectiveProtection: { slash: 1, pierce: 1, blunt: 1 },
        },
      },
      {
        name: 'mail',
        loadout: [
          ['head', 'iron_helmet'],
          ['chest', 'chain_mail'],
          ['hands', 'iron_gauntlets'],
          ['boots', 'iron_boots'],
        ],
        expected: {
          itemDefId: 'chain_mail',
          construction: 'mail',
          defenseSkill: 'mail_armor',
          functional: true,
          coveredRegions: ['head', 'torso', 'arms', 'hands', 'legs', 'feet'],
          missingRegions: [],
          coveragePercent: 100,
          protection: { slash: 2, pierce: 1, blunt: 0 },
          effectiveProtection: { slash: 2, pierce: 1, blunt: 0 },
        },
      },
      {
        name: 'plate',
        loadout: [
          ['head', 'plate_helmet'],
          ['chest', 'breastplate'],
          ['hands', 'plate_gauntlets'],
          ['pants', 'plate_greaves'],
          ['boots', 'plate_boots'],
        ],
        expected: {
          itemDefId: 'breastplate',
          construction: 'plate',
          defenseSkill: 'plate_armor',
          functional: true,
          coveredRegions: ['head', 'torso', 'hands', 'legs', 'feet'],
          missingRegions: ['arms'],
          coveragePercent: 85,
          protection: { slash: 3, pierce: 3, blunt: 1 },
          effectiveProtection: { slash: 3, pierce: 3, blunt: 1 },
        },
      },
      {
        name: 'hybrid',
        loadout: [['chest', 'brigandine_coat']],
        expected: {
          itemDefId: 'brigandine_coat',
          construction: 'hybrid',
          defenseSkill: 'hybrid_armor',
          functional: true,
          coveredRegions: ['torso', 'arms'],
          missingRegions: ['head', 'hands', 'legs', 'feet'],
          coveragePercent: 55,
          protection: { slash: 2, pierce: 2, blunt: 2 },
          effectiveProtection: { slash: 2, pierce: 2, blunt: 2 },
        },
      },
    ]

    let nextInstanceId = 10
    for (const loadoutCase of cases) {
      const equipped = Object.fromEntries(
        loadoutCase.loadout.map(([slot, itemDefId]) => [
          slot,
          {
            instance_id: nextInstanceId++,
            item_def_id: itemDefId,
            quantity: 1,
            enchant: 0,
            durability: null,
          },
        ])
      ) as Partial<Record<EquipSlot, ItemInstance>>
      setInventory({ bag: [], equipped })

      const profile = get(primaryArmorDefense)
      if (loadoutCase.expected === null) {
        expect(profile, loadoutCase.name).toBeNull()
      } else {
        expect(profile, loadoutCase.name).toMatchObject(loadoutCase.expected)
      }
    }
  })
})
