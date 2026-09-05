import type { EquipSlot, ItemInstance } from '../network/networkTypes'
import { ammoAverageDamage, getItemDef, isConsumable } from '../data/itemDefs'

type Category = EquipSlot | 'ammo' | 'dungeon_key' | 'consumable' | 'misc'

// Follow CharacterPanel's slot order, with ammo beside the main hand.
const CATEGORY_ORDER: Category[] = [
  'head',
  'main_hand',
  'ammo',
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
  'dungeon_key',
  'misc',
]

/** Kinds that earn their own run in the bag despite wearing no slot. */
const UNWORN_CATEGORIES: Category[] = ['ammo', 'dungeon_key']
const nameCollator = new Intl.Collator(undefined, { numeric: true })

function categoryOf(itemDefId: string): Category {
  const def = getItemDef(itemDefId)
  if (def?.equipSlot) return def.equipSlot
  const own = UNWORN_CATEGORIES.find((c) => c === def?.category)
  if (own) return own
  if (def && isConsumable(def)) return 'consumable'
  return 'misc'
}

export function inventoryGroupKey(item: ItemInstance): string {
  return getItemDef(item.item_def_id)?.stackable === true
    ? `${item.item_def_id}:${item.enchant}`
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

  const defA = getItemDef(itemDefIdA)
  const defB = getItemDef(itemDefIdB)

  // Sort ammo by damage, matching the default selection.
  const damageDiff =
    (defB ? ammoAverageDamage(defB) : 0) - (defA ? ammoAverageDamage(defA) : 0)
  if (damageDiff !== 0) return damageDiff

  const nameA = defA?.name ?? itemDefIdA
  const nameB = defB?.name ?? itemDefIdB
  const nameDiff = nameCollator.compare(nameA, nameB)
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
