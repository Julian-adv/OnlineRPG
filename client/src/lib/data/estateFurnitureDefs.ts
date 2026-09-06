import estateStorageJson from '../../../../data/estate_storage.json'
import type { EstateFurniturePlacementDefinition } from '../terrain/estatePlacement'

interface EstateStorageData {
  id: string
  modelId: string
  capacityKg: number
  snapStep: number
  rotationStep: number
  footprintWidth: number
  footprintDepth: number
  minFloor: number
  maxFloor: number
  floorEdgeClearance: number
  indoorCollisionRadius: number
  outdoorCollisionRadius: number
}

export interface EstateStorageDefinition extends EstateFurniturePlacementDefinition {
  itemDefId: string
  modelId: string
  capacityKg: number
}

export const estateStorageDefs = new Map<string, EstateStorageDefinition>(
  Object.values(estateStorageJson as Record<string, EstateStorageData>).map(
    (data) => [
      data.id,
      {
        itemDefId: data.id,
        modelId: data.modelId,
        capacityKg: data.capacityKg,
        modelUrl: `/models/objects/${data.modelId}.glb`,
        snapStep: data.snapStep,
        rotationStep: data.rotationStep,
        footprint: {
          width: data.footprintWidth,
          depth: data.footprintDepth,
        },
        floorEdgeClearance: data.floorEdgeClearance,
        minFloor: data.minFloor,
        maxFloor: data.maxFloor,
      },
    ]
  )
)

export function getEstateStorageDef(itemDefId: string | null | undefined) {
  return itemDefId ? estateStorageDefs.get(itemDefId) : undefined
}

export function isEstateStorageItem(itemDefId: string) {
  return estateStorageDefs.has(itemDefId)
}
