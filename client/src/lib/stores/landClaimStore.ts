import { get, writable } from 'svelte/store'

export interface LandClaim {
  instance_id: number
  tile_x: number
  tile_z: number
  quadrant: number
  status: 'confirm' | 'pending' | 'claimed' | 'rejected'
  reason?: string
  refreshing?: boolean
}

export const landClaimDialog = writable<LandClaim | null>(null)

let refreshInstanceId: number | null = null

export function refreshLandClaimPreview(send: (instanceId: number) => void) {
  const claim = get(landClaimDialog)
  if (
    !claim ||
    refreshInstanceId !== null ||
    claim.status === 'pending' ||
    claim.status === 'claimed'
  )
    return
  refreshInstanceId = claim.instance_id
  landClaimDialog.set({ ...claim, refreshing: true })
  send(claim.instance_id)
}

export function applyLandClaimPreview(
  data: Omit<LandClaim, 'status' | 'refreshing'>
) {
  const claim = get(landClaimDialog)
  if (refreshInstanceId === data.instance_id) {
    refreshInstanceId = null
    if (
      !claim ||
      claim.instance_id !== data.instance_id ||
      claim.status === 'claimed'
    )
      return
  }
  if (claim?.status !== 'pending') {
    landClaimDialog.set({
      ...data,
      status: data.reason ? 'rejected' : 'confirm',
    })
  }
}

export function resetLandClaimPreview() {
  refreshInstanceId = null
  landClaimDialog.set(null)
}
