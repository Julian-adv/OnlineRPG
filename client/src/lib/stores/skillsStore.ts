import { writable } from 'svelte/store'
import type { SkillId, Skills } from '../network/networkTypes'

export type { SkillId, Skills }

/** Player-facing skill names (mirrors shared `SkillId::display_name`). */
export const SKILL_DISPLAY_NAMES: Record<SkillId, string> = {
  fishing: 'Fishing',
}

/** The local player's trained skills, pushed by the server on join
 *  (`SkillsUpdate`) and advanced by `SkillXpGained`. Empty map until the
 *  first skill is trained — panels render nothing for an empty map. */
export const skillsStore = writable<Skills>({ map: {} })

export function setSkills(skills: Skills) {
  skillsStore.set(skills)
}

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
