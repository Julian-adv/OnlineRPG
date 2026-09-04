import type { CharacterClass } from '../network/networkTypes'

export enum AnimationName {
  IDLE1 = 'idle1',
  IDLE2 = 'idle2',
  IDLE3 = 'idle3',
  IDLE4 = 'idle4',
  IDLE5 = 'idle5',
  WALK = 'walk',
  JOG = 'jog',
  RUN = 'run',
  JUMP = 'jump',
  SLASH1 = 'slash1',
  SLASH2 = 'slash2',
  SLASH3 = 'slash3',
  SLASH4 = 'slash4',
  ATTACK1 = 'attack1',
  ATTACK2 = 'attack2',
  ATTACK3 = 'attack3',
  ATTACK4 = 'attack4',
  DYING = 'dying',
  HIT = 'hit',
  // Between-swing cooldown breather (and /emote debug); appended after the
  // other combat_melee clips to keep that block contiguous.
  COMBAT_IDLE = 'combat_idle',
}

/** Position of each clip in the ordered `validAnimations` array, which is
 *  built in `AnimationName` declaration order. */
export const AnimationIndex = Object.fromEntries(
  Object.keys(AnimationName).map((key, index) => [key, index])
) as Record<keyof typeof AnimationName, number>

/** Offhand animation clip names — loaded separately, not part of the core ordered array. */
export const OffhandAnimationName = {
  TORCH_IDLE1: 'torch_idle1',
  TORCH_IDLE2: 'torch_idle2',
  TORCH_WALK: 'torch_walk',
  TORCH_RUN: 'torch_run',
} as const

/** All torch idle clip names — picked randomly when the player is idle with a torch. */
export const TORCH_IDLE_CLIP_NAMES = [
  OffhandAnimationName.TORCH_IDLE1,
  OffhandAnimationName.TORCH_IDLE2,
] as const

/** Ranged attack clips, from the combat_ranged pack. `SHOOT` is one whole
 *  draw-and-release, timed like the melee swing (player_attack_impact marks
 *  the loose). Loaded only while a ranged weapon is held; the attack falls
 *  back to the melee slash when the pack lacks the clip. */
export const RangedAnimationName = {
  SHOOT: 'bow_shoot',
} as const

/** Fishing clip names — interaction-state clips from the fishing pack.
 *  The cast plays once on FishingCasted; the idle loops until the line
 *  comes in (bite/fight keep it — only the outcome ends the stance). */
export const FishingAnimationName = {
  CAST: 'fishing_cast',
  IDLE: 'fishing_idle',
} as const

/** Chair interaction. `SIT` is the catalog name the FSM holds; PlayerModel
 *  expands it into the enter clip, the seated loop (with the occasional
 *  talk), and `SIT_TO_STAND` is what the exit swaps in. */
export const SitAnimationName = {
  SIT: 'sit',
  STAND_TO_SIT: 'stand_to_sit',
  IDLE: 'sit_idle',
  TALK: 'sit_talk',
  SIT_TO_STAND: 'sit_to_stand',
} as const

export const SIT_TALK_CHANCE = 0.3

/** Idle clips a class plays instead of the default idles. May mix packs:
 *  the maid holds social poses (static, stretched to 6s) between the shared
 *  locomotion idles. */
export const CLASS_IDLE_CLIP_NAMES: Partial<
  Record<CharacterClass, readonly string[]>
> = {
  maid: [
    'stand_pose2',
    'stand_pose3',
    'stand_pose4',
    'weight_shift',
    'yawn',
    AnimationName.IDLE1,
    AnimationName.IDLE2,
    AnimationName.IDLE3,
    AnimationName.IDLE4,
  ],
}

export type OffhandAnimationKey = keyof typeof OffhandAnimationName
