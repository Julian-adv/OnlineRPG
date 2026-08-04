import type { ItemInstance } from '../network/networkTypes'
import { compareInventoryOrder, inventoryGroupKey } from './inventorySort'

/** A displayed group and its backing bag entries. */
export interface SelectableGroup {
  key: string
  itemDefId: string
  enchant: number
  durability?: number | null
  totalQty: number
  instances: { instanceId: number; quantity: number }[]
}

/** Groups and sorts the bag while retaining each backing instance. */
export function groupBagForSelection(
  bag: readonly ItemInstance[]
): SelectableGroup[] {
  const groups = new Map<string, SelectableGroup>()
  for (const item of bag) {
    const key = inventoryGroupKey(item)
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
        durability: item.durability,
        totalQty: item.quantity,
        instances: [{ instanceId: item.instance_id, quantity: item.quantity }],
      })
    }
  }

  return [...groups.values()].sort((a, b) =>
    compareInventoryOrder(a.itemDefId, a.totalQty, b.itemDefId, b.totalQty)
  )
}

/** Allocates one request. Reuse `createGroupAllocator` for shared groups. */
export function splitGroupQty(
  group: SelectableGroup,
  qty: number
): { instanceId: number; qty: number }[] {
  return createGroupAllocator().take(group, qty)
}

/** Tracks per-instance capacity across multiple `take()` calls. */
export function createGroupAllocator() {
  const remaining = new Map<number, number>()
  return {
    take(
      group: SelectableGroup,
      qty: number
    ): { instanceId: number; qty: number }[] {
      const lines: { instanceId: number; qty: number }[] = []
      let need = qty
      for (const inst of group.instances) {
        if (need <= 0) break
        const avail = remaining.get(inst.instanceId) ?? inst.quantity
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
