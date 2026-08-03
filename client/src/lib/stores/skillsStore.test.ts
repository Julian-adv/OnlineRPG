import { beforeEach, describe, expect, it, vi } from 'vitest'
import { get } from 'svelte/store'
import { getItemDef } from '../data/itemDefs'
import {
  applySkillXp,
  resetSkillsStore,
  skillDisplayName,
  skillEffectText,
  skillProgressPct,
  skillsStore,
} from './skillsStore'

const wasm = vi.hoisted(() => ({
  armor_skill_guard_bonus: vi.fn((_skill: string, level: number) =>
    Math.min(3, Math.floor((Math.min(level, 30) + 5) / 10))
  ),
  weapon_skill_attack_bonus: vi.fn((_skill: string, level: number) =>
    Math.min(3, Math.floor((Math.min(level, 30) + 5) / 10))
  ),
  shield_skill_guard_bonus: vi.fn((level: number) =>
    Math.min(3, Math.floor((Math.min(level, 30) + 5) / 10))
  ),
  healing_skill_hp_bonus: vi.fn((level: number) =>
    Math.min(3, Math.floor((Math.min(level, 30) + 5) / 10))
  ),
  skill_level_cap: vi.fn(() => 30),
  skill_xp_for_level: vi.fn((level: number) => level * 100),
}))

vi.mock('../wasm/onlinerpg_shared', () => wasm)

describe('trained skills', () => {
  beforeEach(() => {
    resetSkillsStore()
    vi.clearAllMocks()
  })

  it('accepts and displays combat and noncombat progress beside fishing', () => {
    applySkillXp('fishing', 100, 1)
    applySkillXp('one_handed_sword', 500, 2)
    applySkillXp('dagger', 10, 0)
    applySkillXp('spear', 20, 0)
    applySkillXp('shield', 30, 0)
    applySkillXp('healing', 40, 0)
    applySkillXp('leather_armor', 50, 0)

    expect(get(skillsStore).map).toEqual({
      fishing: { level: 1, xp: 100 },
      one_handed_sword: { level: 2, xp: 500 },
      dagger: { level: 0, xp: 10 },
      spear: { level: 0, xp: 20 },
      shield: { level: 0, xp: 30 },
      healing: { level: 0, xp: 40 },
      leather_armor: { level: 0, xp: 50 },
    })
    expect(skillDisplayName('one_handed_sword')).toBe('One-Handed Sword')
    expect(skillDisplayName('dagger')).toBe('Dagger')
    expect(skillDisplayName('spear')).toBe('Spear')
    expect(skillDisplayName('shield')).toBe('Shield')
    expect(skillDisplayName('healing')).toBe('Healing')
    expect(skillDisplayName('leather_armor')).toBe('Leather Armor')
  })

  it('uses the shared WASM formula for weapon accuracy text', () => {
    expect(skillEffectText('fishing', 30)).toBeNull()
    expect(skillEffectText('one_handed_sword', 0)).toBe('Accuracy +0')
    expect(skillEffectText('one_handed_sword', 5)).toBe('Accuracy +1')
    expect(skillEffectText('one_handed_sword', 15)).toBe('Accuracy +2')
    expect(skillEffectText('one_handed_sword', 25)).toBe('Accuracy +3')
    expect(skillEffectText('one_handed_sword', 999)).toBe('Accuracy +3')
    expect(skillEffectText('dagger', 15)).toBe('Accuracy +2')
    expect(skillEffectText('spear', 25)).toBe('Accuracy +3')
    expect(wasm.weapon_skill_attack_bonus).toHaveBeenLastCalledWith('spear', 25)
    expect(skillEffectText('shield', 15)).toBe('Guard +2')
    expect(wasm.shield_skill_guard_bonus).toHaveBeenLastCalledWith(15)
    expect(skillEffectText('healing', 25)).toBe('Bandage healing +3 HP')
    expect(wasm.healing_skill_hp_bonus).toHaveBeenLastCalledWith(25)
    expect(skillEffectText('leather_armor', 15)).toBe('Guard +2')
    expect(wasm.armor_skill_guard_bonus).toHaveBeenLastCalledWith(
      'leather_armor',
      15
    )
  })

  it('calculates in-level progress and completes the bar at level 30', () => {
    const level = 2
    const start = wasm.skill_xp_for_level(level)
    const next = wasm.skill_xp_for_level(level + 1)
    expect(skillProgressPct({ level, xp: start + (next - start) / 2 })).toBe(50)
    expect(
      skillProgressPct({ level: 30, xp: wasm.skill_xp_for_level(30) })
    ).toBe(100)
  })
})

describe('weapon skill item metadata', () => {
  it('maps sword, dagger, and Spear definitions used by tooltips', () => {
    for (const id of [
      'iron_sword',
      'worn_iron_sword',
      'goblin_sword',
      'small_sword',
    ]) {
      const def = getItemDef(id)
      expect(def?.weaponSkill).toBe('one_handed_sword')
      expect(skillDisplayName(def!.weaponSkill!)).toBe('One-Handed Sword')
    }
    expect(getItemDef('dagger')?.weaponSkill).toBe('dagger')
    expect(getItemDef('spear')?.weaponSkill).toBe('spear')
    for (const id of [
      'iron_sword',
      'worn_iron_sword',
      'dagger',
      'goblin_sword',
      'small_sword',
    ]) {
      expect(getItemDef(id)?.damageType).toBe('slash')
    }
    expect(getItemDef('spear')?.damageType).toBe('pierce')
    expect(getItemDef('torch')?.damageType).toBe('blunt')
    expect(getItemDef('fishing_rod')?.damageType).toBeUndefined()
    for (const id of ['torch', 'fishing_rod']) {
      expect(getItemDef(id)?.weaponSkill).toBeUndefined()
    }
  })
})

describe('defense skill item metadata', () => {
  it('keeps shield and construction-specific armor mappings distinct', () => {
    for (const id of ['wooden_shield', 'raven_shield']) {
      const def = getItemDef(id)
      expect(def?.defenseSkill).toBe('shield')
      expect(skillDisplayName(def!.defenseSkill!)).toBe('Shield')
      expect(def?.armorConstruction).toBeUndefined()
    }
    for (const id of ['torch', 'leather_helmet', 'ring_of_protection']) {
      expect(getItemDef(id)?.defenseSkill).toBeUndefined()
    }
    expect(getItemDef('leather_armor')?.defenseSkill).toBe('leather_armor')
    expect(getItemDef('leather_armor')?.guard).toBe(2)
    for (const id of [
      'leather_helmet',
      'leather_armor',
      'leather_gloves',
      'leather_pants',
      'leather_boots',
    ]) {
      expect(getItemDef(id)?.armorConstruction).toBe('leather')
    }
    expect(getItemDef('chain_mail')?.armorConstruction).toBe('mail')
    expect(getItemDef('chain_mail')?.guard).toBe(5)
    expect(getItemDef('chain_mail')?.defenseSkill).toBeUndefined()
    expect(getItemDef('breastplate')?.armorConstruction).toBe('plate')
    expect(getItemDef('breastplate')?.guard).toBe(7)
    expect(getItemDef('breastplate')?.defenseSkill).toBeUndefined()
    expect(getItemDef('padded_battle_robe')?.armorConstruction).toBe('padded')
    expect(getItemDef('padded_battle_robe')?.repairFamily).toBe('cloth')
    expect(getItemDef('leather_armor')?.repairFamily).toBe('leather')
    expect(getItemDef('brigandine_coat')?.armorConstruction).toBe('hybrid')
    expect(getItemDef('brigandine_coat')?.repairFamily).toBe('hybrid')
    expect(getItemDef('brigandine_coat')?.guard).toBe(2)
    expect(getItemDef('chain_mail')?.repairFamily).toBe('metal')
    expect(getItemDef('breastplate')?.repairFamily).toBe('metal')
    for (const [id, repairFamily] of [
      ['cloth_repair_kit', 'cloth'],
      ['leather_repair_kit', 'leather'],
      ['metal_repair_kit', 'metal'],
      ['hybrid_repair_kit', 'hybrid'],
    ] as const) {
      expect(getItemDef(id)?.repairFamily).toBe(repairFamily)
    }
    for (const id of [
      'traveler_robe',
      'padded_battle_robe',
      'brigandine_coat',
    ]) {
      const def = getItemDef(id)
      expect(def?.equipmentLayer).toBe('primary')
      expect(def?.defenseSkill).toBeUndefined()
    }
    expect(getItemDef('traveler_robe')).toMatchObject({
      equipmentKind: 'clothing',
      garmentForm: 'robe',
    })
    expect(getItemDef('traveler_robe')?.armorConstruction).toBeUndefined()
    expect(getItemDef('padded_battle_robe')).toMatchObject({
      equipmentKind: 'body_armor',
      garmentForm: 'robe',
    })
    expect(getItemDef('brigandine_coat')).toMatchObject({
      equipmentKind: 'body_armor',
      garmentForm: 'coat',
    })
  })
})

describe('use skill item metadata', () => {
  it('maps Healing to bandaging rather than finished products', () => {
    const bandage = getItemDef('bandage')
    expect(bandage?.useSkill).toBe('healing')
    expect(skillDisplayName(bandage!.useSkill!)).toBe('Healing')
    const potion = getItemDef('healing_potion')
    expect(potion?.useSkill).toBeUndefined()
    for (const id of ['raw_minnow', 'raw_trout', 'scroll_of_return']) {
      expect(getItemDef(id)?.useSkill).toBeUndefined()
    }
  })
})
