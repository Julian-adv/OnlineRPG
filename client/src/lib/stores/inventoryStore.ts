import { derived, writable } from 'svelte/store'
import type {
  EquipSlot,
  ItemInstance,
  PlayerInventory,
} from '../network/networkTypes'
import { getItemDef } from '../data/itemDefs'
import { activeDebuffs } from './debuffStore'
import { calendarVisible } from './debugStore'
import { armorWeightMult } from '../data/debuffPresentation'
import type { HungerSnapshot } from './hungerStore'

export type { EquipSlot, ItemInstance, PlayerInventory }

const initialState: PlayerInventory = {
  bag: [],
  equipped: {},
}

export const inventoryStore = writable<PlayerInventory>({ ...initialState })

/** The local player's gold in the smallest currency unit (copper). */
export const playerGold = writable(0)

/** The local player's effective guard (base attribute + equipped-gear bonuses),
 *  computed server-side and pushed on join and after each equipment change.
 *  `null` until the first GuardUpdated arrives. */
export const playerGuard = writable<number | null>(null)

/** Item defs that act as a carried light source (mirrors shared TORCH_ITEM_IDS). */
const TORCH_ITEM_IDS = ['torch', 'worn_torch']

export function isTorchItemDefId(id: string | null | undefined): boolean {
  return id != null && TORCH_ITEM_IDS.includes(id)
}

/** True when the local player has a torch equipped in the off-hand slot. */
export const localTorchEquipped = derived(inventoryStore, (inv) => {
  const id = inv.equipped.off_hand?.item_def_id
  return isTorchItemDefId(id)
})

function itemWeight(item: ItemInstance, armorMult: number): number {
  const def = getItemDef(item.item_def_id)
  const mult = def?.category === 'armor' ? armorMult : 1
  return (def?.weight ?? 1) * mult * item.quantity
}

/** Mirrors the server's `calc_total_weight`: soaked armour drags, worn and
 *  packed alike (doc/DEBUFF.md). Per-item tooltips keep the dry weight. */
export const carryWeight = derived(
  [inventoryStore, activeDebuffs],
  ([inv, debuffs]) => {
    const now = Date.now()
    const armorMult = armorWeightMult(
      debuffs.filter((d) => d.until > now).map((d) => d.id)
    )
    let total = 0
    for (const item of inv.bag) total += itemWeight(item, armorMult)
    for (const item of Object.values(inv.equipped)) {
      if (item) total += itemWeight(item, armorMult)
    }
    return total
  }
)

/** Mirrors the server's max_carry_weight: STR × 15 scaled by hunger. */
export const maxCarryWeight = (str: number, hunger: HungerSnapshot | null) =>
  str * 15 * (hunger?.carryMult ?? 1)

export const formatKg = (weight: number) => (weight / 10).toFixed(1)

/** The local player's first revive item (phoenix talisman), offered on the
 *  death dialog together with its def. */
export const reviveItem = derived(inventoryStore, (inv) => {
  for (const item of inv.bag) {
    const def = getItemDef(item.item_def_id)
    if (def?.reviveHpPercent != null) return { item, def }
  }
  return null
})

export function setInventory(inventory: PlayerInventory) {
  inventoryStore.set(inventory)
}

export function resetInventoryStore() {
  inventoryStore.set({ bag: [], equipped: {} })
  playerGold.set(0)
  playerGuard.set(null)
}

const hasTimekeeper = derived(inventoryStore, (inv) =>
  inv.bag.some(
    (item) => getItemDef(item.item_def_id)?.category === 'timekeeper'
  )
)

/** Hour and date beside the sky widget: debug CAL toggle or a carried timekeeper. */
export const calendarShown = derived(
  [calendarVisible, hasTimekeeper],
  ([v, t]) => v || t
)
