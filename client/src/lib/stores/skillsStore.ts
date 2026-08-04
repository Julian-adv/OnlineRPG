import { writable } from 'svelte/store'
import type { SkillId, Skills } from '../network/networkTypes'
import {
  armor_skill_guard_bonus,
  healing_skill_hp_bonus,
  skill_level_cap,
  skill_xp_for_level,
  shield_skill_guard_bonus,
  weapon_skill_attack_bonus,
} from '../wasm/onlinerpg_shared'

export type { SkillId, Skills }

/** Player-facing skill names (mirrors shared `SkillId::display_name`). */
export const SKILL_DISPLAY_NAMES: Record<SkillId, string> = {
  fishing: 'Fishing',
  one_handed_sword: 'One-Handed Sword',
  dagger: 'Dagger',
  spear: 'Spear',
  shield: 'Shield',
  healing: 'Healing',
  leather_armor: 'Leather Armor',
  mail_armor: 'Mail Armor',
  plate_armor: 'Plate Armor',
  padded_armor: 'Padded Armor',
  hybrid_armor: 'Hybrid Armor',
}

export function skillDisplayName(skill: SkillId): string {
  return SKILL_DISPLAY_NAMES[skill] ?? skill
}

export function skillEffectText(skill: SkillId, level: number): string | null {
  if (skill === 'fishing') return null
  if (skill === 'shield') return `Guard +${shield_skill_guard_bonus(level)}`
  if (skill === 'healing')
    return `Bandage healing +${healing_skill_hp_bonus(level)} HP`
  if (
    skill === 'leather_armor' ||
    skill === 'mail_armor' ||
    skill === 'plate_armor' ||
    skill === 'padded_armor' ||
    skill === 'hybrid_armor'
  )
    return `Guard +${armor_skill_guard_bonus(skill, level)}`
  return `Accuracy +${weapon_skill_attack_bonus(skill, level)}`
}

export function skillProgressPct(progress: { level: number; xp: number }) {
  if (progress.level >= skill_level_cap()) return 100
  const start = skill_xp_for_level(progress.level)
  const next = skill_xp_for_level(progress.level + 1)
  return Math.min(100, ((progress.xp - start) / (next - start)) * 100)
}

/** The local player's trained skills, pushed by the server on join
 *  (`SkillsUpdate`) and advanced by `SkillXpGained`. Empty map until the
 *  first skill is trained — panels render nothing for an empty map. */
export const skillsStore = writable<Skills>({ map: {} })

export function applySkillXp(
  skill: SkillId,
  totalXp: number,
  newLevel: number
) {
  skillsStore.update((skills) => ({
    map: { ...skills.map, [skill]: { level: newLevel, xp: totalXp } },
  }))
}

export function resetSkillsStore() {
  skillsStore.set({ map: {} })
}
