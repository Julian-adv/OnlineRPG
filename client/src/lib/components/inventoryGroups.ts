import type { ItemInstance } from '../network/networkTypes'
import { getItemDef } from '../data/itemDefs'
import { CATEGORY_ORDER, categoryOf } from './inventorySort'

/** One backing bag entry behind a displayed group. The server may hold the
 *  same item_def_id/enchant as several separate stacks (e.g. repeated
 *  purchases don't always merge), so a group's displayed total can span
 *  multiple real instance ids. */
export interface SelectableGroup {
  key: string
  itemDefId: string
  enchant: number
  totalQty: number
  instances: { instanceId: number; quantity: number }[]
}

/** Groups the bag the same way `sortBag` does for display (merging same
 *  item_def_id/enchant stacks, sorted by category/name/qty), but keeps the
 *  backing instance ids and their individual quantities instead of
 *  collapsing to one representative instance. Selection UIs that let a
 *  player request "N of this pile" need that breakdown to split the request
 *  across however many real stacks actually back it. */
export function groupBagForSelection(
  bag: readonly ItemInstance[]
): SelectableGroup[] {
  const groups = new Map<string, SelectableGroup>()
  for (const item of bag) {
    const stackable = getItemDef(item.item_def_id)?.stackable === true
    const key = stackable
      ? `${item.item_def_id}:${item.enchant}`
      : `unique:${item.instance_id}`
    const existing = groups.get(key)
    if (existing) {
      existing.totalQty += item.quantity
      existing.instances.push({
        instanceId: item.instance_id,
        quantity: item.quantity,
      })
    } else {
      groups.set(key, {
        key,
        itemDefId: item.item_def_id,
        enchant: item.enchant,
        totalQty: item.quantity,
        instances: [{ instanceId: item.instance_id, quantity: item.quantity }],
      })
    }
  }

  return [...groups.values()].sort((a, b) => {
    const categoryDiff =
      CATEGORY_ORDER.indexOf(categoryOf(a.itemDefId)) -
      CATEGORY_ORDER.indexOf(categoryOf(b.itemDefId))
    if (categoryDiff !== 0) return categoryDiff

    const nameA = getItemDef(a.itemDefId)?.name ?? a.itemDefId
    const nameB = getItemDef(b.itemDefId)?.name ?? b.itemDefId
    const nameDiff = nameA.localeCompare(nameB)
    if (nameDiff !== 0) return nameDiff

    return b.totalQty - a.totalQty
  })
}

/** Splits a requested quantity across a group's backing instances,
 *  greedily draining each stack in turn. `qty` must not exceed
 *  `group.totalQty` minus anything already reserved by the caller.
 *
 *  Only safe for a single request per group. If several separate requests
 *  can target the *same* group (e.g. a haggled-deal cart row and a plain
 *  cart row for the same item def), calling this once per request each
 *  starts from the group's full, undrained instance list and can
 *  double-allocate the same instance's capacity across requests — use
 *  `createGroupAllocator` for that case instead. */
export function splitGroupQty(
  group: SelectableGroup,
  qty: number
): { instanceId: number; qty: number }[] {
  const lines: { instanceId: number; qty: number }[] = []
  let remaining = qty
  for (const inst of group.instances) {
    if (remaining <= 0) break
    const take = Math.min(remaining, inst.quantity)
    if (take > 0) lines.push({ instanceId: inst.instanceId, qty: take })
    remaining -= take
  }
  return lines
}

/** Tracks remaining per-instance capacity across several `take()` calls
 *  against the same set of groups, so multiple cart rows for one group
 *  (e.g. a deal-priced row plus a plain row) deplete a shared pool instead
 *  of each independently draining the group's full, undrained instances. */
export function createGroupAllocator(groups: readonly SelectableGroup[]) {
  const remaining = new Map<number, number>()
  for (const group of groups) {
    for (const inst of group.instances) {
      remaining.set(inst.instanceId, inst.quantity)
    }
  }
  return {
    take(
      group: SelectableGroup,
      qty: number
    ): { instanceId: number; qty: number }[] {
      const lines: { instanceId: number; qty: number }[] = []
      let need = qty
      for (const inst of group.instances) {
        if (need <= 0) break
        const avail = remaining.get(inst.instanceId) ?? 0
        const take = Math.min(need, avail)
        if (take > 0) {
          lines.push({ instanceId: inst.instanceId, qty: take })
          remaining.set(inst.instanceId, avail - take)
          need -= take
        }
      }
      return lines
    },
  }
}
