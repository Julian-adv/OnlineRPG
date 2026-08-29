import { writable } from 'svelte/store'
import type { EquipSlot } from '../network/networkTypes'

export const QUICKSLOT_COUNT = 10

/**
 * One quickslot binding: an item *definition* id plus an enchant level. Not an
 * instance id — those are reissued every login, so a binding must survive on
 * def + enchant alone. `enchant: null` means "any level": bindings saved
 * before enchant levels were stored carry no level intent.
 */
export type QuickslotEntry = {
  defId: string
  enchant: number | null
}

/** Quickslot assignments; `null` means the slot is empty. */
export const quickslots = writable<(QuickslotEntry | null)[]>(
  new Array(QUICKSLOT_COUNT).fill(null)
)

/** The inventory fields quickslot resolution reads. */
export type CarriedItem = {
  instance_id: number
  item_def_id: string
  enchant: number
  quantity: number
}

/**
 * The bag copy a binding equips or consumes: the bound level while a copy at
 * that level is in the bag — that is what keeps a "+3 shield" binding off the
 * plain shield — then the level closest to the bound one (ties to the
 * higher), so a binding whose level no longer exists degrades to the nearest
 * copy instead of an unrelated spare. Any-level bindings take the best copy.
 */
function pickBagLevel(bound: number | null, levels: number[]): number | null {
  if (levels.length === 0) return null
  if (bound !== null && levels.includes(bound)) return bound
  if (bound === null) return Math.max(...levels)
  return levels.reduce((a, b) => {
    const da = Math.abs(a - bound)
    const db = Math.abs(b - bound)
    return db < da || (db === da && b > a) ? b : a
  })
}

export type ResolvedQuickslot = {
  /** Level the slot displays and acts on; the stored intent (possibly null)
   *  when nothing of the def is carried. */
  enchant: number | null
  /** Pressing the slot unequips the worn item (toggle off). */
  unequip: boolean
  /** Bag item a press equips or consumes, when not unequipping. */
  bagItem: CarriedItem | undefined
  /** Bag quantity at the resolved level. */
  qty: number
}

/**
 * What a quickslot press acts on. `worn` is the occupant of the bound def's
 * own equip slot (undefined for consumables or an empty slot).
 *
 * A `worn` item of the bound def always resolves to a toggle-off at its own
 * level, whatever level is stored: enchant scrolls raise a worn item's level
 * in place, so the worn copy IS the bound item after it drifts — and with one
 * binding per def, "unequip what I'm wearing" is the only consistent meaning
 * for the key. The bound level picks which bag copy to equip, nothing more.
 */
export function resolveQuickslot(
  entry: QuickslotEntry,
  worn: CarriedItem | undefined,
  bag: CarriedItem[]
): ResolvedQuickslot {
  const bagMatches = bag.filter((i) => i.item_def_id === entry.defId)
  const wornMatch = worn?.item_def_id === entry.defId ? worn : undefined
  const level = wornMatch
    ? wornMatch.enchant
    : pickBagLevel(
        entry.enchant,
        bagMatches.map((i) => i.enchant)
      )
  if (level === null)
    return {
      enchant: entry.enchant,
      unequip: false,
      bagItem: undefined,
      qty: 0,
    }
  const atLevel = bagMatches.filter((i) => i.enchant === level)
  return {
    enchant: level,
    unequip: wornMatch !== undefined,
    bagItem: wornMatch ? undefined : atLevel[0],
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
  def: { equipSlot?: EquipSlot | null; consumable?: boolean },
  resolved: ResolvedQuickslot
): QuickslotAction {
  const slot = def.equipSlot
  if (slot && resolved.unequip) return { kind: 'unequip', slot }
  if (!resolved.bagItem) return null
  if (slot) return { kind: 'equip', instanceId: resolved.bagItem.instance_id }
  if (def.consumable === true)
    return { kind: 'use', instanceId: resolved.bagItem.instance_id }
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
    // One binding per item type: level fallbacks make two bindings of the
    // same def collapse into indistinguishable slots, so re-dragging a def
    // moves its binding instead.
    for (let i = 0; i < next.length; i++) {
      if (next[i]?.defId === entry.defId) next[i] = null
    }
    next[index] = { defId: entry.defId, enchant: entry.enchant }
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
