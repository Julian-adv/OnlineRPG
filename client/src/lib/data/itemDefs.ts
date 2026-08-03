import itemsJson from '../../../../data/items.json'
import type { EquipSlot, SkillId } from '../network/networkTypes'

export type PhysicalDamageType = 'untyped' | 'slash' | 'pierce' | 'blunt'

export type ArmorConstruction =
  | 'padded'
  | 'leather'
  | 'mail'
  | 'plate'
  | 'hybrid'

export type RepairFamily = 'cloth' | 'leather' | 'metal' | 'hybrid'

export type EquipmentKind =
  | 'weapon'
  | 'tool'
  | 'clothing'
  | 'body_armor'
  | 'shield'
  | 'accessory'

export type EquipmentLayer = 'held' | 'primary' | 'accessory'

export type GarmentForm =
  | 'helmet'
  | 'cuirass'
  | 'leggings'
  | 'gloves'
  | 'boots'
  | 'hauberk'
  | 'robe'
  | 'coat'

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
  /** Item kind that decides how `dice` is read: weapons deal damage;
   * restorative categories heal. */
  category?: string
  /** Dice notation (e.g. "1d8", "6d4") whose meaning depends on `category`. */
  dice?: string
  /** Authoritative physical channel used by this weapon. */
  damageType?: PhysicalDamageType
  material?: string
  /** Explicit physical construction for worn body armor. */
  armorConstruction?: ArmorConstruction
  /** Explicit role in the equipment rules. */
  equipmentKind?: EquipmentKind
  /** Occupancy layer; currently garments use one primary body layer. */
  equipmentLayer?: EquipmentLayer
  /** Physical garment shape, independent of construction. */
  garmentForm?: GarmentForm
  /** Base price in the smallest currency unit (copper). */
  basePrice?: number
  /** Guard (AC) bonus granted while equipped. Summed across equipped items. */
  guard?: number
  /** Usable from the bag — the items.csv flag, which the server validates
   * against its `use_effect` dispatch at boot. */
  consumable?: boolean
  /** Skill trained by accepted attacks with this main-hand weapon. */
  weaponSkill?: SkillId
  /** Skill trained by accepted monster attacks while this item is equipped. */
  defenseSkill?: SkillId
  /** Skill trained by the server-resolved action performed with this item. */
  useSkill?: SkillId
  /** Satiation restored when eaten (doc/HUNGER.md). */
  nutrition?: number
  /** Maximum condition for per-instance durable equipment. */
  maxDurability?: number
  /** Material family required to repair this armor or supplied by this kit. */
  repairFamily?: RepairFamily
}

const itemDefs = itemsJson as Record<string, ItemDefinition>

export function getItemDef(itemDefId: string): ItemDefinition | undefined {
  return itemDefs[itemDefId]
}

export function isConsumable(def: ItemDefinition): boolean {
  return def.consumable === true
}

export default itemDefs
