import { get, writable } from 'svelte/store'
import { passability_set_furniture } from '../wasm/onlinerpg_shared'
import type { EstateChest, EstateChestState } from '../network/networkTypes'
import { getEstateStorageDef } from '../data/estateFurnitureDefs'
import {
  estateFurniturePlacementError,
  estateFurniturePlacementMode,
  estateFurniturePlacementPending,
  stopEstateFurniturePlacement,
} from './estateFurniturePlacementStore'

export const estateChests = writable(new Map<number, EstateChest>())
export const estateChestMode = estateFurniturePlacementMode
export const estateChestPending = estateFurniturePlacementPending
export const estateChestError = estateFurniturePlacementError
export const openEstateChest = writable<EstateChestState | null>(null)

const syncedBuckets = new Set<string>()

function bucket(chest: EstateChest) {
  return `${Math.floor(chest.position.x / 32)},${Math.floor(chest.position.z / 32)}`
}

function syncCollision(chests: Map<number, EstateChest>) {
  const groups = new Map<string, EstateChest[]>()
  for (const chest of chests.values()) {
    const key = bucket(chest)
    const group = groups.get(key) ?? []
    group.push(chest)
    groups.set(key, group)
  }
  for (const key of new Set([...syncedBuckets, ...groups.keys()])) {
    const placements = (groups.get(key) ?? []).flatMap((chest) => {
      const definition = getEstateStorageDef(chest.item_def_id)
      return definition
        ? [
            {
              id: chest.id,
              type: definition.modelId,
              x: chest.position.x,
              y: chest.position.y,
              z: chest.position.z,
              rotation: chest.rotation_deg,
              floorLevel: chest.floor_level,
            },
          ]
        : []
    })
    passability_set_furniture(`furniture:estate-storage:${key}`, placements)
  }
  syncedBuckets.clear()
  for (const key of groups.keys()) syncedBuckets.add(key)
}

export function applyEstateChestVisibility(
  added: EstateChest[],
  removed: number[]
) {
  const next = new Map(get(estateChests))
  for (const id of removed) next.delete(id)
  for (const chest of added) next.set(chest.id, chest)
  syncCollision(next)
  estateChests.set(next)
  const opened = get(openEstateChest)
  if (opened && removed.includes(opened.chest_id)) openEstateChest.set(null)
}

export function stopEstateChestMode() {
  stopEstateFurniturePlacement()
}

export function resetEstateStorage() {
  for (const key of syncedBuckets)
    passability_set_furniture(`furniture:estate-storage:${key}`, [])
  syncedBuckets.clear()
  estateChests.set(new Map())
  openEstateChest.set(null)
  stopEstateChestMode()
}
