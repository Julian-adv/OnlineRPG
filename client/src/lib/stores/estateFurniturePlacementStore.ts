import { writable } from 'svelte/store'
import type { EstatePlot } from '../terrain/estatePlacement'

export type EstateFurniturePlacementMode = {
  instance_id: number
  item_def_id: string
  owner_id: number
  plots: EstatePlot[]
}

export const estateFurniturePlacementMode =
  writable<EstateFurniturePlacementMode | null>(null)
export const estateFurniturePlacementPending = writable(false)
export const estateFurniturePlacementError = writable<string | null>(null)

export function stopEstateFurniturePlacement() {
  estateFurniturePlacementMode.set(null)
  estateFurniturePlacementPending.set(false)
  estateFurniturePlacementError.set(null)
}
