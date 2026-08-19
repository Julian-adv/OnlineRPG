import { xp_for_level } from '../wasm/onlinerpg_shared'

export interface LevelProgress {
  neededXp: number
  gainedXp: number
  progress: number
  percent: number
}

export function levelProgress(level: number, totalXp: number): LevelProgress {
  const start = xp_for_level(level)
  const neededXp = Math.max(1, xp_for_level(level + 1) - start)
  const gainedXp = Math.min(neededXp, Math.max(0, totalXp - start))
  const progress = gainedXp / neededXp
  return { neededXp, gainedXp, progress, percent: Math.floor(progress * 100) }
}
