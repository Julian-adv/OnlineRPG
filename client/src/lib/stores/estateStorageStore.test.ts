import { beforeEach, describe, expect, it, vi } from 'vitest'
import { get } from 'svelte/store'

vi.mock('../wasm/onlinerpg_shared', () => ({
  passability_set_furniture: vi.fn(),
}))

import { passability_set_furniture } from '../wasm/onlinerpg_shared'
import {
  applyEstateChestVisibility,
  estateChests,
  openEstateChest,
  resetEstateStorage,
} from './estateStorageStore'

const chest = {
  id: 7,
  estate_id: 2,
  owner_id: 3,
  item_def_id: 'storage_chest',
  position: { x: 2.5, y: 5, z: 2.5 },
  rotation_deg: 90,
  floor_level: 0,
  overdue: false,
  revision: 0,
}

describe('estate storage visibility', () => {
  beforeEach(() => {
    resetEstateStorage()
    vi.clearAllMocks()
  })

  it('keeps contents private while syncing the visible solid chest', () => {
    applyEstateChestVisibility([chest], [])

    expect([...get(estateChests).values()]).toEqual([chest])
    expect(get(openEstateChest)).toBeNull()
    expect(passability_set_furniture).toHaveBeenLastCalledWith(
      'furniture:estate-storage:0,0',
      [
        {
          id: 7,
          type: 'chest_animated',
          x: 2.5,
          y: 5,
          z: 2.5,
          rotation: 90,
          floorLevel: 0,
        },
      ]
    )
  })

  it('removal clears collision and closes the matching window', () => {
    applyEstateChestVisibility([chest], [])
    openEstateChest.set({
      chest_id: chest.id,
      item_def_id: 'storage_chest',
      revision: 0,
      max_weight: 500,
      can_deposit: true,
      items: [],
    })
    vi.mocked(passability_set_furniture).mockClear()

    applyEstateChestVisibility([], [chest.id])

    expect(get(estateChests).size).toBe(0)
    expect(get(openEstateChest)).toBeNull()
    expect(passability_set_furniture).toHaveBeenLastCalledWith(
      'furniture:estate-storage:0,0',
      []
    )
  })
})
