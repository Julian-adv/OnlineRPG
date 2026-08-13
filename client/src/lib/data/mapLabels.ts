/**
 * Place-name labels, embedded at build time from data/map_labels.json
 * (generated from data-src/map_labels.csv).
 */
import mapLabelsJson from '../../../../data/map_labels.json'

export type MapLabelKind =
  | 'continent'
  | 'capital'
  | 'city'
  | 'town'
  | 'sea'
  | 'island'
  | 'dungeon'

export interface MapLabelDef {
  id: string
  name: string
  kind: MapLabelKind
  /** World meters. */
  x: number
  z: number
}

export const MAP_LABELS: MapLabelDef[] = Object.values(
  mapLabelsJson as unknown as Record<string, MapLabelDef>
)
