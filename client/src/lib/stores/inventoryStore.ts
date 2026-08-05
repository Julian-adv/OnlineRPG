import { derived, writable } from 'svelte/store'
import type {
  EquipmentBurden,
  EquipSlot,
  ItemInstance,
  PlayerInventory,
  SkillId,
} from '../network/networkTypes'
import {
  applyBodyCoverage,
  BODY_REGIONS,
  bodyCoveragePercent,
  getItemDef,
  itemBodyCoverage,
  type ArmorConstruction,
  type BodyRegion,
  type PhysicalProtection,
} from '../data/itemDefs'

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

/** Server-authored equipped-load tier and effective movement speed. */
export const equipmentBurden = writable<EquipmentBurden | null>(null)

export type PrimaryArmorDefense = {
  itemDefId: string
  name: string
  construction: ArmorConstruction
  defenseSkill: SkillId | null
  functional: boolean
  coveredRegions: BodyRegion[]
  missingRegions: BodyRegion[]
  coveragePercent: number
  protection: PhysicalProtection
  effectiveProtection: PhysicalProtection
}

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

/** Equipped primary chest profile for display; Guard remains server-authored. */
export const primaryArmorDefense = derived(inventoryStore, (inv) => {
  const item = inv.equipped.chest
  if (!item) return null

  const def = getItemDef(item.item_def_id)
  if (
    !def ||
    def.equipmentKind !== 'body_armor' ||
    def.equipmentLayer !== 'primary' ||
    def.equipSlot !== 'chest' ||
    !def.armorConstruction
  ) {
    return null
  }

  const covered = new Set<BodyRegion>()
  for (const equippedItem of Object.values(inv.equipped)) {
    if (!equippedItem || equippedItem.durability === 0) continue
    const equippedDef = getItemDef(equippedItem.item_def_id)
    if (equippedDef?.equipmentKind !== 'body_armor') continue
    for (const region of itemBodyCoverage(equippedDef)) covered.add(region)
  }
  const coveredRegions = BODY_REGIONS.filter((region) => covered.has(region))
  const missingRegions = BODY_REGIONS.filter((region) => !covered.has(region))
  const coveragePercent = bodyCoveragePercent(coveredRegions)
  const functional = item.durability == null || item.durability > 0
  const protection = {
    slash: def.slashProtection ?? 0,
    pierce: def.pierceProtection ?? 0,
    blunt: def.bluntProtection ?? 0,
  }

  return {
    itemDefId: def.id,
    name: def.name,
    construction: def.armorConstruction,
    defenseSkill: def.defenseSkill ?? null,
    functional,
    coveredRegions,
    missingRegions,
    coveragePercent,
    protection,
    effectiveProtection: functional
      ? applyBodyCoverage(protection, coveragePercent)
      : { slash: 0, pierce: 0, blunt: 0 },
  } satisfies PrimaryArmorDefense
})

export function setInventory(inventory: PlayerInventory) {
  inventoryStore.set(inventory)
}

export function setEquipmentBurden(burden: EquipmentBurden) {
  equipmentBurden.set(burden)
}

export function resetInventoryStore() {
  inventoryStore.set({ bag: [], equipped: {} })
  playerGold.set(0)
  playerGuard.set(null)
  equipmentBurden.set(null)
}
