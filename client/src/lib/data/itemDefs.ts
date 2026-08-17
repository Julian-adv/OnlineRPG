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
  /** Special effects while equipped: `;`-separated tokens (`cha+1`, `sustenance`). */
  effects?: string
  /** Usable from the bag — the items.csv flag, which the server validates
   * against its `use_effect` dispatch at boot. */
  consumable?: boolean
  /** Satiation restored when eaten (doc/HUNGER.md). */
  nutrition?: number
  /** Cloth colour of the procedural cape, e.g. `#6d1720`. Its presence is what
   *  makes a back-slot item a cape rather than, say, a quiver. */
  capeColor?: string
}

const itemDefs = itemsJson as Record<string, ItemDefinition>

export function getItemDef(itemDefId: string): ItemDefinition | undefined {
  return itemDefs[itemDefId]
}

/** Cloth colour of the cape a back-slot item is, or undefined when it is not
 *  one — `capeColor` is the whole test, so a future quiver sits in the slot
 *  without becoming a sheet. */
export function capeColorOf(
  itemDefId: string | null | undefined
): string | undefined {
  return itemDefId ? getItemDef(itemDefId)?.capeColor : undefined
}

/** Tooltip lines for what an item does: `guard` (with any armor enchant folded
 *  in, as combat resolves it) then `effects`. */
export function statLabels(def: ItemDefinition, enchant = 0): string[] {
  const guard = (def.guard ?? 0) + (def.category === 'armor' ? enchant : 0)
  const lines = guard ? [`Guard: +${guard}`] : []
  for (const raw of def.effects?.split(';') ?? []) {
    const token = raw.trim()
    if (!token) continue
    const cha = token.match(/^cha([+-]\d+)$/)
    if (cha) lines.push(`CHA: ${cha[1]}`)
    else if (token === 'sustenance') lines.push('Slows hunger')
    else lines.push(token)
  }
  return lines
}

export function isConsumable(def: ItemDefinition): boolean {
  return def.consumable === true
}

export default itemDefs
