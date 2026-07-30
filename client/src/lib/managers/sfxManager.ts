import { get, writable } from 'svelte/store'
import {
  DEFAULT_MATERIAL_HIT_SOUND_URL,
  DEFAULT_MATERIAL_MISS_SOUND_URL,
  getAllMaterialHitSoundUrls,
  getAllMaterialMissSoundUrls,
} from '../data/materialImpactSounds'

const SWORD_HIT_VOLUME = 0.55
const SWORD_MISS_VOLUME = 0.5
const SWORD_HIT_POOL_SIZE = 4
const SWORD_MISS_POOL_SIZE = 4

// The reel fires on every reel-stance engage, so it gets a deeper pool; the
// rest are one-shot moments.
const FISHING_SOUNDS = {
  cast: { url: '/sounds/fishing-cast.ogg', volume: 0.45, pool: 2 },
  splash: { url: '/sounds/fishing-splash.ogg', volume: 0.5, pool: 2 },
  plop: { url: '/sounds/fishing-plop.ogg', volume: 0.6, pool: 2 },
  reel: { url: '/sounds/fishing-reel.ogg', volume: 0.4, pool: 4 },
  snap: { url: '/sounds/fishing-snap.ogg', volume: 0.5, pool: 2 },
  catch: { url: '/sounds/fishing-catch.ogg', volume: 0.45, pool: 2 },
} as const
export type FishingSound = keyof typeof FISHING_SOUNDS

const STORAGE_KEY_VOLUME = 'onlinerpg_sfxVolume'
const STORAGE_KEY_MUTED = 'onlinerpg_sfxMuted'
const DEFAULT_SFX_VOLUME = 1

function loadSfxVolume(): number {
  if (typeof localStorage === 'undefined') return DEFAULT_SFX_VOLUME
  const saved = localStorage.getItem(STORAGE_KEY_VOLUME)
  if (saved !== null) {
    const v = parseFloat(saved)
    if (!isNaN(v)) return Math.max(0, Math.min(1, v))
  }
  return DEFAULT_SFX_VOLUME
}

export const sfxVolume = writable<number>(loadSfxVolume())
export const sfxMuted = writable<boolean>(
  typeof localStorage !== 'undefined' &&
    localStorage.getItem(STORAGE_KEY_MUTED) === 'true'
)

let volumeSaveTimer: ReturnType<typeof setTimeout> | undefined

sfxVolume.subscribe((v) => {
  if (typeof localStorage === 'undefined') return
  clearTimeout(volumeSaveTimer)
  volumeSaveTimer = setTimeout(
    () => localStorage.setItem(STORAGE_KEY_VOLUME, String(v)),
    300
  )
})

sfxMuted.subscribe((m) => {
  if (typeof localStorage === 'undefined') return
  localStorage.setItem(STORAGE_KEY_MUTED, String(m))
})

// Multiplier applied on top of each sound's baseline volume so the Settings
// SFX slider/mute scales all effects uniformly.
function getSfxMultiplier(): number {
  return get(sfxMuted) ? 0 : get(sfxVolume)
}

interface AudioPool {
  audios: HTMLAudioElement[]
  index: number
}

const swordHitPools = new Map<string, AudioPool>()
const swordMissPools = new Map<string, AudioPool>()
const fishingPools = new Map<string, AudioPool>()

function canUseAudio(): boolean {
  return typeof Audio !== 'undefined'
}

function createAudio(url: string, volume: number): HTMLAudioElement {
  const audio = new Audio(url)
  audio.preload = 'auto'
  audio.volume = volume
  return audio
}

function preloadAudioPool(
  pools: Map<string, AudioPool>,
  url: string,
  volume: number,
  poolSize: number
) {
  if (!canUseAudio() || pools.has(url)) return

  const pool = {
    audios: Array.from({ length: poolSize }, () => createAudio(url, volume)),
    index: 0,
  }

  for (const audio of pool.audios) {
    audio.load()
  }

  pools.set(url, pool)
}

function playAudioFromPool(
  pools: Map<string, AudioPool>,
  url: string,
  volume: number,
  poolSize: number
) {
  preloadAudioPool(pools, url, volume, poolSize)

  const pool = pools.get(url)
  if (!pool) return

  const audio = pool.audios[pool.index]
  pool.index = (pool.index + 1) % pool.audios.length

  const effectiveVolume = volume * getSfxMultiplier()
  if (effectiveVolume <= 0) return

  try {
    audio.currentTime = 0
    audio.volume = effectiveVolume
    audio.play().catch(() => {})
  } catch {
    // Browser audio policies can reject playback until the first user gesture.
  }
}

export function preloadSwordHitSound() {
  for (const url of getAllMaterialHitSoundUrls()) {
    preloadAudioPool(swordHitPools, url, SWORD_HIT_VOLUME, SWORD_HIT_POOL_SIZE)
  }
}

export function preloadSwordMissSound() {
  for (const url of getAllMaterialMissSoundUrls()) {
    preloadAudioPool(
      swordMissPools,
      url,
      SWORD_MISS_VOLUME,
      SWORD_MISS_POOL_SIZE
    )
  }
}

export function playSwordHitSound(url = DEFAULT_MATERIAL_HIT_SOUND_URL) {
  if (!canUseAudio()) return
  playAudioFromPool(swordHitPools, url, SWORD_HIT_VOLUME, SWORD_HIT_POOL_SIZE)
}

export function playSwordMissSound(
  url = DEFAULT_MATERIAL_MISS_SOUND_URL,
  delayMs = 0
) {
  if (!canUseAudio()) return
  if (delayMs > 0) {
    window.setTimeout(() => playSwordMissSound(url), delayMs)
    return
  }
  playAudioFromPool(
    swordMissPools,
    url,
    SWORD_MISS_VOLUME,
    SWORD_MISS_POOL_SIZE
  )
}

export function preloadFishingSounds() {
  for (const { url, volume, pool } of Object.values(FISHING_SOUNDS)) {
    preloadAudioPool(fishingPools, url, volume, pool)
  }
}

const pendingFishingTimers = new Set<number>()

/** `delayMs` lets e.g. the splash line up with the bobber landing rather
 *  than the swing that threw it (same pattern as `playSwordMissSound`). */
export function playFishingSound(kind: FishingSound, delayMs = 0) {
  if (!canUseAudio()) return
  if (delayMs > 0) {
    const timer = window.setTimeout(() => {
      pendingFishingTimers.delete(timer)
      playFishingSound(kind)
    }, delayMs)
    pendingFishingTimers.add(timer)
    return
  }
  const { url, volume, pool } = FISHING_SOUNDS[kind]
  playAudioFromPool(fishingPools, url, volume, pool)
}

/** A cast aborted mid-flight must not splash after the line is back in. */
export function cancelPendingFishingSounds() {
  for (const timer of pendingFishingTimers) window.clearTimeout(timer)
  pendingFishingTimers.clear()
}
