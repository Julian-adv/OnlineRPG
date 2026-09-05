import type { CharacterClass, Gender } from '../network/networkTypes'

export const KNIGHT_CHARACTER_MODEL_PATH = '/models/characters/knight.glb'
export const FEMALE_KNIGHT_CHARACTER_MODEL_PATH =
  '/models/characters/female_knight.glb'
export const ROGUE_CHARACTER_MODEL_PATH = '/models/characters/rogue.glb'
export const FEMALE_ROGUE_CHARACTER_MODEL_PATH =
  '/models/characters/female_rogue.glb'
export const MERCHANT_CHARACTER_MODEL_PATH = '/models/characters/npc_woman.glb'
export const BARBARIAN_CHARACTER_MODEL_PATH = '/models/characters/barbarian.glb'
export const FEMALE_BARBARIAN_CHARACTER_MODEL_PATH =
  '/models/characters/female_barbarian.glb'
export const GUARD_CHARACTER_MODEL_PATH = '/models/characters/guard.glb'
export const CAVEMAN_CHARACTER_MODEL_PATH = '/models/characters/caveman.glb'
export const CAVEWOMAN_CHARACTER_MODEL_PATH = '/models/characters/cavewoman.glb'
export const VALKYRIE_CHARACTER_MODEL_PATH = '/models/characters/valkyrie.glb'
export const RANGER_CHARACTER_MODEL_PATH = '/models/characters/ranger.glb'
export const PRIEST_CHARACTER_MODEL_PATH = '/models/characters/priest.glb'
export const FEMALE_PRIEST_CHARACTER_MODEL_PATH =
  '/models/characters/female_priest.glb'
export const FEMALE_BARD_CHARACTER_MODEL_PATH =
  '/models/characters/female_bard.glb'
export const NIGHT_MERCHANT_CHARACTER_MODEL_PATH =
  '/models/characters/night_merchant.glb'
export const MAID_CHARACTER_MODEL_PATH = '/models/characters/maid.glb'
export const PINK_MAID_CHARACTER_MODEL_PATH = '/models/characters/pink_maid.glb'
export const STEWARD_CHARACTER_MODEL_PATH = '/models/characters/steward.glb'

export const CHARACTER_ANIMATION_PACK_PATHS = {
  locomotion: '/models/animations/locomotion.glb',
  combatMelee: '/models/animations/combat_melee.glb',
  combatRanged: '/models/animations/combat_ranged.glb',
  social: '/models/animations/social.glb',
  offhand: '/models/animations/offhand.glb',
  fishing: '/models/animations/fishing.glb',
} as const

export function getWeaponModelPath(worldModel: string): string {
  return `/models/${worldModel}`
}

/** URL for an object-catalog GLB. `model` is the catalog `ObjectDef.model`,
 *  already relative to /models/ (e.g. "objects/bed.glb", "weapons/sword.glb").
 *  All object-model loaders route through here so the base path lives in one
 *  place. */
export function getObjectModelPath(model: string): string {
  return `/models/${model}`
}

const CLASS_GENDER_MODELS: Partial<
  Record<CharacterClass, Partial<Record<Gender, string>>>
> = {
  knight: {
    male: KNIGHT_CHARACTER_MODEL_PATH,
    female: FEMALE_KNIGHT_CHARACTER_MODEL_PATH,
  },
  barbarian: {
    male: BARBARIAN_CHARACTER_MODEL_PATH,
    female: FEMALE_BARBARIAN_CHARACTER_MODEL_PATH,
  },
  rogue: {
    male: ROGUE_CHARACTER_MODEL_PATH,
    female: FEMALE_ROGUE_CHARACTER_MODEL_PATH,
  },
  caveman: {
    male: CAVEMAN_CHARACTER_MODEL_PATH,
    female: CAVEWOMAN_CHARACTER_MODEL_PATH,
  },
  valkyrie: { female: VALKYRIE_CHARACTER_MODEL_PATH },
  ranger: { male: RANGER_CHARACTER_MODEL_PATH },
  priest: {
    male: PRIEST_CHARACTER_MODEL_PATH,
    female: FEMALE_PRIEST_CHARACTER_MODEL_PATH,
  },
  bard: { female: FEMALE_BARD_CHARACTER_MODEL_PATH },
  merchant: { female: MERCHANT_CHARACTER_MODEL_PATH },
  guard: { male: GUARD_CHARACTER_MODEL_PATH },
  maid: { female: MAID_CHARACTER_MODEL_PATH },
}

/** Dedicated looks for named official NPCs, overriding the class model.
 *  Keyed by NPC character name, same as getMerchantByNpcName. */
const NPC_MODEL_OVERRIDES: Record<string, string> = {
  Wick: NIGHT_MERCHANT_CHARACTER_MODEL_PATH,
  Cocoly: PINK_MAID_CHARACTER_MODEL_PATH,
  Aldwin: STEWARD_CHARACTER_MODEL_PATH,
}

export function getNpcModelPath(npcName: string): string | undefined {
  return NPC_MODEL_OVERRIDES[npcName]
}

const NPC_CLASSES: CharacterClass[] = ['merchant', 'guard', 'maid']
export const PLAYER_CLASSES = (
  Object.keys(CLASS_GENDER_MODELS) as CharacterClass[]
).filter((cls) => !NPC_CLASSES.includes(cls))

export function getAvailableGenders(characterClass: CharacterClass): Gender[] {
  const genders = CLASS_GENDER_MODELS[characterClass]
  if (!genders) return ['male', 'female']
  return Object.keys(genders) as Gender[]
}

export function getCharacterModelPath(
  characterClass: CharacterClass,
  gender?: Gender
): string {
  const genders = CLASS_GENDER_MODELS[characterClass]
  if (genders) {
    if (gender && genders[gender]) return genders[gender]
    return Object.values(genders)[0]
  }
  return KNIGHT_CHARACTER_MODEL_PATH
}
