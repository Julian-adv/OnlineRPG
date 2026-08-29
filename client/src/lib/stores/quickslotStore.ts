import { writable } from 'svelte/store'
import type { EquipSlot, ItemInstance } from '../network/networkTypes'
import { isConsumable, type ItemDefinition } from '../data/itemDefs'
import { wornOfDef } from './inventoryStore'

export const QUICKSLOT_COUNT = 10

/** A binding is def + enchant level, never an instance id (reissued each
 *  login). `enchant: null` = any level (entries saved before levels existed). */
export type QuickslotEntry = {
  defId: string
  enchant: number | null
}

/** Quickslot assignments; `null` means the slot is empty. */
export const quickslots = writable<(QuickslotEntry | null)[]>(
  new Array(QUICKSLOT_COUNT).fill(null)
)

export type CarriedItem = Pick<
  ItemInstance,
  'instance_id' | 'item_def_id' | 'enchant' | 'quantity'
>

type QuickslotDef = Pick<ItemDefinition, 'equipSlot' | 'consumable'>

/** Bound level if carried, else the nearest (ties high); null bound = highest. */
function pickBagLevel(bound: number | null, levels: number[]): number | null {
  if (levels.length === 0) return null
  const target = bound ?? Infinity
  return levels.reduce((a, b) => {
    const da = Math.abs(a - target)
    const db = Math.abs(b - target)
    return db < da || (db === da && b > a) ? b : a
  })
}

export type ResolvedQuickslot = {
  /** Level shown and acted on; the stored intent when nothing is carried. */
  enchant: number | null
  /** Slot the bound def is worn in — a press toggles it off. */
  wornSlot: EquipSlot | null
  /** Bag item a press equips or consumes, when nothing is worn. */
  bagItem: CarriedItem | undefined
  /** Bag quantity at the resolved level. */
  qty: number
}

/** A worn copy of the bound def always toggles off at its own level (enchant
 *  scrolls raise it in place); the bound level only picks which bag copy to equip. */
export function resolveQuickslot(
  entry: QuickslotEntry,
  def: QuickslotDef,
  equipped: Partial<Record<EquipSlot, CarriedItem>>,
  bag: CarriedItem[]
): ResolvedQuickslot {
  const bagMatches = bag.filter((i) => i.item_def_id === entry.defId)
  const worn = wornOfDef(entry.defId, def.equipSlot, equipped)
  const level = worn
    ? worn.item.enchant
    : pickBagLevel(
        entry.enchant,
        bagMatches.map((i) => i.enchant)
      )
  const atLevel = bagMatches.filter((i) => i.enchant === level)
  return {
    enchant: level ?? entry.enchant,
    wornSlot: worn?.slot ?? null,
    bagItem: worn ? undefined : atLevel[0],
    qty: atLevel.reduce((total, i) => total + i.quantity, 0),
  }
}

export type QuickslotAction =
  | { kind: 'unequip'; slot: EquipSlot }
  | { kind: 'equip'; instanceId: number }
  | { kind: 'use'; instanceId: number }
  | null

/** The network action a quickslot press dispatches, if any. */
export function quickslotAction(
  def: QuickslotDef,
  resolved: ResolvedQuickslot
): QuickslotAction {
  if (resolved.wornSlot) return { kind: 'unequip', slot: resolved.wornSlot }
  if (!resolved.bagItem) return null
  const instanceId = resolved.bagItem.instance_id
  if (def.equipSlot) return { kind: 'equip', instanceId }
  if (isConsumable(def)) return { kind: 'use', instanceId }
  return null
}

/** localStorage key for the active character; null until a character loads. */
let storageKey: string | null = null

function persist(slots: (QuickslotEntry | null)[]) {
  if (!storageKey) return
  try {
    localStorage.setItem(storageKey, JSON.stringify(slots))
  } catch {
    /* storage full or unavailable — quickslots just won't persist */
  }
}

/** Entries saved before enchant levels were stored are plain def ids. */
function parseEntry(raw: unknown): QuickslotEntry | null {
  if (typeof raw === 'string') return { defId: raw, enchant: null }
  if (typeof raw === 'object' && raw !== null) {
    const { defId, enchant } = raw as QuickslotEntry
    if (
      typeof defId === 'string' &&
      (typeof enchant === 'number' || enchant === null)
    )
      return { defId, enchant }
  }
  return null
}

/** Load this character's saved quickslots (call when a character is selected). */
export function loadQuickslots(characterId: number) {
  storageKey = `quickslots:${characterId}`
  const next: (QuickslotEntry | null)[] = new Array(QUICKSLOT_COUNT).fill(null)
  try {
    const raw = localStorage.getItem(storageKey)
    const parsed = raw ? JSON.parse(raw) : null
    if (Array.isArray(parsed)) {
      for (let i = 0; i < QUICKSLOT_COUNT; i++) next[i] = parseEntry(parsed[i])
    }
  } catch {
    /* corrupt entry — fall back to empty */
  }
  quickslots.set(next)
}

export function assignQuickslot(index: number, entry: QuickslotEntry) {
  if (index < 0 || index >= QUICKSLOT_COUNT) return
  quickslots.update((slots) => {
    const next = [...slots]
    // One binding per item type; re-dragging a def moves it.
    for (let i = 0; i < next.length; i++) {
      if (next[i]?.defId === entry.defId) next[i] = null
    }
    next[index] = entry
    persist(next)
    return next
  })
}

export function clearQuickslot(index: number) {
  if (index < 0 || index >= QUICKSLOT_COUNT) return
  quickslots.update((slots) => {
    const next = [...slots]
    next[index] = null
    persist(next)
    return next
  })
}

export function resetQuickslots() {
  storageKey = null
  quickslots.set(new Array(QUICKSLOT_COUNT).fill(null))
}
