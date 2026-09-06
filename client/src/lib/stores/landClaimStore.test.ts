import { afterEach, describe, expect, it, vi } from 'vitest'
import { get } from 'svelte/store'
import {
  applyLandClaimPreview,
  landClaimDialog,
  refreshLandClaimPreview,
  resetLandClaimPreview,
} from './landClaimStore'

const preview = { instance_id: 1, tile_x: 0, tile_z: 0, quadrant: 3 }
afterEach(resetLandClaimPreview)

describe('land preview refresh', () => {
  it('updates the plot and eligibility without sending duplicate requests', () => {
    applyLandClaimPreview(preview)
    const send = vi.fn()
    refreshLandClaimPreview(send)
    refreshLandClaimPreview(send)
    expect(send).toHaveBeenCalledExactlyOnceWith(1)
    expect(get(landClaimDialog)?.refreshing).toBe(true)
    applyLandClaimPreview({ ...preview, tile_x: 1, reason: 'Crown land' })
    expect(get(landClaimDialog)).toMatchObject({
      tile_x: 1,
      status: 'rejected',
      reason: 'Crown land',
    })
    expect(get(landClaimDialog)?.refreshing).toBeFalsy()
    refreshLandClaimPreview(send)
    applyLandClaimPreview({ ...preview, tile_x: 2 })
    expect(get(landClaimDialog)).toMatchObject({ tile_x: 2, status: 'confirm' })
  })

  it('does not reopen a closed preview when its refresh arrives', () => {
    applyLandClaimPreview(preview)
    refreshLandClaimPreview(vi.fn())
    landClaimDialog.set(null)
    applyLandClaimPreview({ ...preview, tile_x: 1 })
    expect(get(landClaimDialog)).toBeNull()
  })

  it.each(['pending', 'claimed'] as const)(
    'does not refresh a %s claim',
    (status) => {
      landClaimDialog.set({ ...preview, status })
      const send = vi.fn()
      refreshLandClaimPreview(send)
      expect(send).not.toHaveBeenCalled()
    }
  )
})
