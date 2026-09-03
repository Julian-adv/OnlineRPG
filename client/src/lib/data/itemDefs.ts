import itemsJson from '../../../../data/items.json'
import type { EquipSlot } from '../network/networkTypes'

export const WEAPON_TYPE_LABELS = {
  sword: 'Sword',
  short_sword: 'Short Sword',
  dagger: 'Dagger',
  axe: 'Axe',
  staff: 'Staff',
  spear: 'Spear',
  mace: 'Mace',
  club: 'Club',
  bow: 'Bow',
  crossbow: 'Crossbow',
  torch: 'Torch',
} as const

export type WeaponType = keyof typeof WEAPON_TYPE_LABELS

export interface ItemDefinition {
  id: string
  name: string
  description: string
  weight: number
  /** Absent for non-equippable items (the CSV→JSON step drops empty cells). */
  equipSlot?: EquipSlot | null
  stackable: boolean
  icon: string
  worldModel?: string
  /** Item kind that decides how `dice` is read: "weapon" → damage, "consumable" → healing. */
  category?: string
  weaponType?: WeaponType
  /** Dice notation (e.g. "1d8", "6d4") whose meaning depends on `category`. */
  dice?: string
  material?: string
  /** Base price in the smallest currency unit (copper). */
  basePrice?: number
  /** Guard (AC) bonus granted while equipped. Summed across equipped items. */
  guard?: number
  /** Special effects while equipped: `;`-separated tokens (`cha+1`, `sustenance`). */
  effects?: string
  /** Usable from the bag — the items.csv flag, which the server validates
   * against its `use_effect` dispatch at boot. */
  consumable?: boolean
  /** Satiation restored when eaten (doc/HUNGER.md). */
  nutrition?: number
  /** Phoenix talisman: max-HP percentage restored by a revive. */
  reviveHpPercent?: number
  /** Cloth colour of the procedural cape, e.g. `#6d1720`. Its presence is what
   *  makes a back-slot item a cape rather than, say, a quiver. */
  capeColor?: string
}

const itemDefs = itemsJson as Record<string, ItemDefinition>

export function getItemDef(itemDefId: string): ItemDefinition | undefined {
  return itemDefs[itemDefId]
}

export function weaponTypeLabel(weaponType: WeaponType): string {
  return WEAPON_TYPE_LABELS[weaponType]
}

/** Cloth colour to render the cape in, or undefined when the back-slot item
 *  is not a cape — `capeColor` is the whole test, so a future quiver sits in
 *  the slot without becoming a sheet. A `dye` (the instance's own colour,
 *  doc/CAPE_CUSTOMIZATION.md) overrides the def's, but never makes a cape of
 *  something that isn't one. */
export function capeColorOf(
  itemDefId: string | null | undefined,
  dye?: string | null
): string | undefined {
  const cloth = itemDefId ? getItemDef(itemDefId)?.capeColor : undefined
  return cloth ? (dye ?? cloth) : undefined
}

/** Name as players must see it: the enchant is part of the name, never only a
 *  tooltip line — +0 and +7 are otherwise identical, the cheapest scam there
 *  is. */
export function itemDisplayName(itemDefId: string, enchant = 0): string {
  const def = getItemDef(itemDefId)
  return def ? displayName(def, enchant) : itemDefId
}

export function displayName(def: ItemDefinition, enchant = 0): string {
  return enchant !== 0 ? `+${enchant} ${def.name}` : def.name
}

/** Guard while equipped, with the armor enchant folded in as combat resolves it. */
export function effectiveGuard(def: ItemDefinition, enchant = 0): number {
  return (def.guard ?? 0) + (def.category === 'armor' ? enchant : 0)
}

/** Mean damage roll (dice + enchant); 0 for non-weapons. */
export function averageDamage(def: ItemDefinition, enchant = 0): number {
  const m = def.category === 'weapon' ? def.dice?.match(/^(\d+)d(\d+)$/) : null
  if (!m) return 0
  return (Number(m[1]) * (Number(m[2]) + 1)) / 2 + enchant
}

/** Tooltip lines for what an item does: `guard` (with any armor enchant folded
 *  in, as combat resolves it) then `effects`. */
export function statLabels(def: ItemDefinition, enchant = 0): string[] {
  const guard = effectiveGuard(def, enchant)
  const lines = guard ? [`Guard: +${guard}`] : []
  for (const raw of def.effects?.split(';') ?? []) {
    const token = raw.trim()
    if (!token) continue
    const cha = token.match(/^cha([+-]\d+)$/)
    if (cha) lines.push(`CHA: ${cha[1]}`)
    else if (token === 'sustenance') lines.push('Slows hunger')
    else lines.push(token)
  }
  return lines
}

export interface StatDelta {
  label: string
  /** Candidate minus equipped, in the stat's own unit. */
  delta: number
  better: boolean
}

/** Differences that matter when swapping `def` in for the equipped item:
 *  weight, damage, guard. Equal stats are dropped. */
export function compareStats(
  def: ItemDefinition,
  enchant: number,
  equipped: ItemDefinition,
  equippedEnchant: number
): StatDelta[] {
  const out: StatDelta[] = []
  const push = (label: string, delta: number, lowerIsBetter = false) => {
    if (Math.abs(delta) < 0.05) return
    out.push({ label, delta, better: lowerIsBetter ? delta < 0 : delta > 0 })
  }
  push('Weight', def.weight - equipped.weight, true)
  push(
    'Damage',
    averageDamage(def, enchant) - averageDamage(equipped, equippedEnchant)
  )
  push(
    'Guard',
    effectiveGuard(def, enchant) - effectiveGuard(equipped, equippedEnchant)
  )
  return out
}

export function isConsumable(def: Pick<ItemDefinition, 'consumable'>): boolean {
  return def.consumable === true
}

export default itemDefs
