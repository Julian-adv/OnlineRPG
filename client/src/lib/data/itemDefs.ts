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
}

const itemDefs = itemsJson as Record<string, ItemDefinition>

export function getItemDef(itemDefId: string): ItemDefinition | undefined {
  return itemDefs[itemDefId]
}

/** Categories that can be drunk/used from the bag. Extend as potions are added.
 * Keep in sync with the server's `use_effect` dispatch (server/src/item_defs.rs):
 * eating a fish heals by its dice, opening a coin pouch pays out its copper. */
const CONSUMABLE_CATEGORIES = new Set([
  'healing_potion',
  'return_scroll',
  'enchant_scroll',
  'party_summon_scroll',
  'fish',
  'coin_catch',
])

export function isConsumable(def: ItemDefinition): boolean {
  return def.category !== undefined && CONSUMABLE_CATEGORIES.has(def.category)
}

export default itemDefs
