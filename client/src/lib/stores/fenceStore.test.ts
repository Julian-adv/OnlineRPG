import { beforeEach, describe, expect, it, vi } from 'vitest'
import { get } from 'svelte/store'

vi.mock('../wasm/onlinerpg_shared', () => ({
  passability_set_fences: vi.fn(),
}))

import { passability_set_fences } from '../wasm/onlinerpg_shared'
import { applyFenceVisibility, fences, refreshFenceHeights } from './fenceStore'
import { WORLD_MIN_X, WORLD_MAX_X } from '../terrain/world-wrap'

describe('fence terrain heights', () => {
  beforeEach(() => {
    fences.set(new Map())
    vi.clearAllMocks()
  })

  it('updates rendering and collision together from endpoints and midpoint', () => {
    const fence = {
      edge: { x: 2, z: 1, axis: 'X' as const },
      y: 5,
      owner_id: 1,
    }
    applyFenceVisibility([fence], [])
    const sample = vi.fn((x: number) => (x === 2.5 ? 8 : 10))
    refreshFenceHeights(sample)
    expect(sample.mock.calls.map(([x]) => x)).toEqual([2, 2.5, 3])
    expect([...get(fences).values()]).toEqual([{ ...fence, y: 8 }])
    expect(passability_set_fences).toHaveBeenLastCalledWith([
      { ...fence, y: 8 },
    ])
    vi.mocked(passability_set_fences).mockClear()
    refreshFenceHeights(sample)
    expect(passability_set_fences).not.toHaveBeenCalled()
  })

  it('keeps the server height until all terrain samples are loaded', () => {
    const fence = {
      edge: { x: 2, z: 1, axis: 'Z' as const },
      y: 5,
      owner_id: 1,
    }
    applyFenceVisibility([fence], [])
    vi.mocked(passability_set_fences).mockClear()
    refreshFenceHeights((_x, z) => (z === 2 ? null : 10))
    expect([...get(fences).values()]).toEqual([fence])
    expect(passability_set_fences).not.toHaveBeenCalled()
  })

  it('samples the wrapped endpoint at the world seam', () => {
    const fence = {
      edge: { x: WORLD_MAX_X - 1, z: 1, axis: 'X' as const },
      y: 5,
      owner_id: 1,
    }
    applyFenceVisibility([fence], [])
    const sample = vi.fn((x: number) => (x === WORLD_MIN_X ? 3 : 6))
    refreshFenceHeights(sample)
    expect(sample).toHaveBeenLastCalledWith(WORLD_MIN_X, 1)
    expect([...get(fences).values()][0].y).toBe(3)
  })
})
