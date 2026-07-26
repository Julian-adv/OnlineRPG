import type { ItemInstance } from '../network/networkTypes'

const INVENTORY_COLUMNS = 5
const INVENTORY_MIN_SLOTS = 50

export function buildInventorySlots(
  bag: readonly ItemInstance[]
): (ItemInstance | null)[] {
  const slotCount = Math.max(
    INVENTORY_MIN_SLOTS,
    Math.ceil(bag.length / INVENTORY_COLUMNS) * INVENTORY_COLUMNS
  )

  return Array.from({ length: slotCount }, (_, index) => bag[index] ?? null)
}
