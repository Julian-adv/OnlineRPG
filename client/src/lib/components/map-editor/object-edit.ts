import { get } from 'svelte/store'
import {
  currentEditorRegion,
  currentObjectData,
  selectedObjectPlacementId,
  type ObjectPlacement,
  type ObjectRegionData,
} from '../../stores/editorStore'
import { objectManager } from '../../managers/objectManager'
import { furnitureManager } from '../../managers/furnitureManager'

export function nextPlacementId(data: ObjectRegionData): number {
  return data.placements.reduce((max, p) => Math.max(max, p.id), 0) + 1
}

/** Publish a new placement set to the store and persist it. */
export async function commitPlacements(
  placements: ObjectPlacement[]
): Promise<void> {
  const updated: ObjectRegionData = { placements }
  currentObjectData.set(updated)
  const region = get(currentEditorRegion)
  if (region) await objectManager.saveObject(region.rx, region.rz, updated)
}

export async function deleteSelectedPlacement(): Promise<void> {
  const id = get(selectedObjectPlacementId)
  if (id === null) return
  const data = get(currentObjectData)
  selectedObjectPlacementId.set(null)
  await commitPlacements(data.placements.filter((p) => p.id !== id))
}

/** Copy the selected placement, offset on XZ (a whole cell for solid furniture
 *  so its footprint stays grid-aligned) and select the copy. */
export async function duplicateSelectedPlacement(): Promise<void> {
  const id = get(selectedObjectPlacementId)
  if (id === null) return
  const data = get(currentObjectData)
  const src = data.placements.find((p) => p.id === id)
  if (!src) return
  const step = furnitureManager.isSolid(src.type) ? 1 : 0.5
  const copy = {
    ...src,
    id: nextPlacementId(data),
    x: src.x + step,
    z: src.z + step,
  }
  selectedObjectPlacementId.set(copy.id)
  await commitPlacements([...data.placements, copy])
}
