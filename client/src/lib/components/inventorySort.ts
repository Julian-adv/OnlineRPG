import type { EquipSlot, ItemInstance } from '../network/networkTypes'
import { getItemDef, isConsumable } from '../data/itemDefs'

export type Category = EquipSlot | 'consumable' | 'misc'

// Mirrors the equip-slot ordering used in CharacterPanel's paperdoll layout.
export const CATEGORY_ORDER: Category[] = [
  'head',
  'main_hand',
  'off_hand',
  'chest',
  'ear',
  'neck',
  'belt',
  'pants',
  'boots',
  'ring',
  'ring_left',
  'hands',
  'back',
  'shirt',
  'consumable',
  'misc',
]

export function categoryOf(itemDefId: string): Category {
  const def = getItemDef(itemDefId)
  if (def?.equipSlot) return def.equipSlot
  if (def && isConsumable(def)) return 'consumable'
  return 'misc'
}

/** Merges same-item/enchant stackable stacks, then groups by equip slot,
 *  name, and quantity. Client-side only; does not touch server state. */
export function sortBag(bag: readonly ItemInstance[]): ItemInstance[] {
  const merged = new Map<string, ItemInstance>()
  for (const item of bag) {
    const stackable = getItemDef(item.item_def_id)?.stackable === true
    const key = stackable
      ? `${item.item_def_id}:${item.enchant}`
      : `unique:${item.instance_id}`
    const existing = merged.get(key)
    if (existing) {
      existing.quantity += item.quantity
    } else {
      merged.set(key, { ...item })
    }
  }

  return [...merged.values()].sort((a, b) => {
    const categoryDiff =
      CATEGORY_ORDER.indexOf(categoryOf(a.item_def_id)) -
      CATEGORY_ORDER.indexOf(categoryOf(b.item_def_id))
    if (categoryDiff !== 0) return categoryDiff

    const nameA = getItemDef(a.item_def_id)?.name ?? a.item_def_id
    const nameB = getItemDef(b.item_def_id)?.name ?? b.item_def_id
    const nameDiff = nameA.localeCompare(nameB)
    if (nameDiff !== 0) return nameDiff

    return b.quantity - a.quantity
  })
}
