import type { EquipSlot, ItemInstance } from '../network/networkTypes'
import { getItemDef, isConsumable } from '../data/itemDefs'

type Category = EquipSlot | 'consumable' | 'misc'

// Mirrors the equip-slot ordering used in CharacterPanel's paperdoll layout.
const CATEGORY_ORDER: Category[] = [
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

function categoryOf(itemDefId: string): Category {
  const def = getItemDef(itemDefId)
  if (def?.equipSlot) return def.equipSlot
  if (def && isConsumable(def)) return 'consumable'
  return 'misc'
}

export function inventoryGroupKey(item: ItemInstance): string {
  return getItemDef(item.item_def_id)?.stackable === true
    ? `${item.item_def_id}:${item.enchant}:${item.durability ?? 'none'}`
    : `unique:${item.instance_id}`
}

export function compareInventoryOrder(
  itemDefIdA: string,
  quantityA: number,
  itemDefIdB: string,
  quantityB: number
): number {
  const categoryDiff =
    CATEGORY_ORDER.indexOf(categoryOf(itemDefIdA)) -
    CATEGORY_ORDER.indexOf(categoryOf(itemDefIdB))
  if (categoryDiff !== 0) return categoryDiff

  const nameA = getItemDef(itemDefIdA)?.name ?? itemDefIdA
  const nameB = getItemDef(itemDefIdB)?.name ?? itemDefIdB
  const nameDiff = nameA.localeCompare(nameB)
  return nameDiff !== 0 ? nameDiff : quantityB - quantityA
}

/** Merges same-item/enchant stackable stacks, then groups by equip slot,
 *  name, and quantity. Client-side only; does not touch server state. */
export function sortBag(bag: readonly ItemInstance[]): ItemInstance[] {
  const merged = new Map<string, ItemInstance>()
  for (const item of bag) {
    const key = inventoryGroupKey(item)
    const existing = merged.get(key)
    if (existing) {
      existing.quantity += item.quantity
    } else {
      merged.set(key, { ...item })
    }
  }

  return [...merged.values()].sort((a, b) =>
    compareInventoryOrder(a.item_def_id, a.quantity, b.item_def_id, b.quantity)
  )
}
