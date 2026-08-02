import itemsJson from '../../../../data/items.json'
import type { EquipSlot } from '../network/networkTypes'

export interface ItemDefinition {
  id: string
  name: string
  description: string
  weight: number
  /** Absent for non-equippable items (the CSV→JSON step drops empty cells). */
  equipSlot?: EquipSlot | null
  stackable: boolean
  icon: string
  worldModel?: string
  /** Item kind that decides how `dice` is read: "weapon" → damage, "consumable" → healing. */
  category?: string
  /** Dice notation (e.g. "1d8", "6d4") whose meaning depends on `category`. */
  dice?: string
  material?: string
  /** Base price in the smallest currency unit (copper). */
  basePrice?: number
  /** Guard (AC) bonus granted while equipped. Summed across equipped items. */
  guard?: number
  /** Usable from the bag — the items.csv flag, which the server validates
   * against its `use_effect` dispatch at boot. */
  consumable?: boolean
  /** Satiation restored when eaten (doc/HUNGER.md). */
  nutrition?: number
}

const itemDefs = itemsJson as Record<string, ItemDefinition>

export function getItemDef(itemDefId: string): ItemDefinition | undefined {
  return itemDefs[itemDefId]
}

export function isConsumable(def: ItemDefinition): boolean {
  return def.consumable === true
}

export default itemDefs
