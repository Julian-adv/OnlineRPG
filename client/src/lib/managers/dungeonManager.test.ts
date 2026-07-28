import { describe, it, expect, vi, beforeEach } from 'vitest'
import { get } from 'svelte/store'

vi.mock('../wasm/onlinerpg_shared', () => ({
  dungeon_layout: () => [],
  dungeon_constants: () => ({
    grid: 56,
    floorHeight: 8,
    wallHeight: 3,
    floorIndexBase: 4,
    shaftW: 2,
    shaftLen: 7,
    landingCells: 1,
    maxDepth: 5,
    pathMaxNodes: 20000,
    eventDeliveryRadius: 60,
  }),
  dungeon_add_passability: () => {},
  dungeon_remove_passability: () => {},
  dungeon_passability_floor_cells: () => null,
  dungeon_rebuild_floor: () => {},
  dungeon_interior_doors: () => [],
  dungeon_floor_height_at: () => NaN,
  dungeon_entrance_ramp_height_at: () => NaN,
  dungeon_shaft_run_pos: () => NaN,
  passability_start_floor_at: () => 0,
}))

vi.mock('../data/dungeonDefs', () => ({ DUNGEON_ENTRANCES: [] }))

const { dungeonManager } = await import('./dungeonManager')
const { playerFloorLevel } = await import('../stores/housingStore')

describe('syncFromFloorLevel surfacing', () => {
  beforeEach(() => {
    playerFloorLevel.set(0)
  })

  it('clears a stale dungeon floor left by waypoint arrival (death respawn)', () => {
    playerFloorLevel.set(8)
    dungeonManager.syncFromFloorLevel(0, 100, 100)
    expect(get(playerFloorLevel)).toBe(0)
  })

  it('leaves housing floors alone', () => {
    playerFloorLevel.set(1)
    dungeonManager.syncFromFloorLevel(0, 100, 100)
    expect(get(playerFloorLevel)).toBe(1)
  })
})
