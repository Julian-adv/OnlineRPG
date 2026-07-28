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
  SLASH5 = 'slash5',
  ATTACK1 = 'attack1',
  ATTACK2 = 'attack2',
  ATTACK3 = 'attack3',
  ATTACK4 = 'attack4',
  DYING = 'dying',
}

export enum AnimationIndex {
  IDLE1 = 0,
  IDLE2 = 1,
  IDLE3 = 2,
  IDLE4 = 3,
  IDLE5 = 4,
  WALK = 5,
  JOG = 6,
  RUN = 7,
  JUMP = 8,
  SLASH1 = 9,
  SLASH2 = 10,
  SLASH3 = 11,
  SLASH4 = 12,
  ATTACK1 = 13,
  ATTACK2 = 14,
  ATTACK3 = 15,
  ATTACK4 = 16,
  DYING = 17,
}

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

/** Fishing clip names — interaction-state clips from the fishing pack.
 *  The cast plays once on FishingCasted; the idle loops until the line
 *  comes in (bite/fight keep it — only the outcome ends the stance). */
export const FishingAnimationName = {
  CAST: 'fishing_cast',
  IDLE: 'fishing_idle',
} as const

export type OffhandAnimationKey = keyof typeof OffhandAnimationName
