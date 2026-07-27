import type { ItemInstance } from '../network/networkTypes'

const INVENTORY_COLUMNS = 5
const INVENTORY_MIN_ROWS = 10

export function buildInventorySlots(
  bag: readonly ItemInstance[]
): (ItemInstance | null)[] {
  const rows = Math.max(
    INVENTORY_MIN_ROWS,
    Math.ceil(bag.length / INVENTORY_COLUMNS)
  )

  return Array.from(
    { length: rows * INVENTORY_COLUMNS },
    (_, index) => bag[index] ?? null
  )
}
