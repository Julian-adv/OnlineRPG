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
export const SWORD_MISS_DELAY_MS = 450

const FISHING_CAST_SOUND_URL = '/sounds/fishing-cast.ogg'
const FISHING_SPLASH_SOUND_URL = '/sounds/fishing-splash.ogg'
const FISHING_PLOP_SOUND_URL = '/sounds/fishing-plop.ogg'
const FISHING_REEL_SOUND_URL = '/sounds/fishing-reel.ogg'
const FISHING_SNAP_SOUND_URL = '/sounds/fishing-snap.ogg'
const FISHING_CATCH_SOUND_URL = '/sounds/fishing-catch.ogg'
const FISHING_CAST_VOLUME = 0.45
const FISHING_SPLASH_VOLUME = 0.5
const FISHING_PLOP_VOLUME = 0.6
const FISHING_REEL_VOLUME = 0.4
const FISHING_SNAP_VOLUME = 0.5
const FISHING_CATCH_VOLUME = 0.45
// The reel fires every struggle round, so it gets a deeper pool; the rest are
// one-shot moments.
const FISHING_REEL_POOL_SIZE = 4
const FISHING_POOL_SIZE = 2

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
  poolSize: number,
  delayMs = 0
) {
  preloadAudioPool(pools, url, volume, poolSize)
  if (delayMs > 0) {
    window.setTimeout(
      () => playAudioFromPool(pools, url, volume, poolSize),
      delayMs
    )
    return
  }

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
  preloadAudioPool(
    fishingPools,
    FISHING_CAST_SOUND_URL,
    FISHING_CAST_VOLUME,
    FISHING_POOL_SIZE
  )
  preloadAudioPool(
    fishingPools,
    FISHING_SPLASH_SOUND_URL,
    FISHING_SPLASH_VOLUME,
    FISHING_POOL_SIZE
  )
  preloadAudioPool(
    fishingPools,
    FISHING_PLOP_SOUND_URL,
    FISHING_PLOP_VOLUME,
    FISHING_POOL_SIZE
  )
  preloadAudioPool(
    fishingPools,
    FISHING_REEL_SOUND_URL,
    FISHING_REEL_VOLUME,
    FISHING_REEL_POOL_SIZE
  )
  preloadAudioPool(
    fishingPools,
    FISHING_SNAP_SOUND_URL,
    FISHING_SNAP_VOLUME,
    FISHING_POOL_SIZE
  )
  preloadAudioPool(
    fishingPools,
    FISHING_CATCH_SOUND_URL,
    FISHING_CATCH_VOLUME,
    FISHING_POOL_SIZE
  )
}

export function playFishingCastSound() {
  if (!canUseAudio()) return
  playAudioFromPool(
    fishingPools,
    FISHING_CAST_SOUND_URL,
    FISHING_CAST_VOLUME,
    FISHING_POOL_SIZE
  )
}

/** `delayMs` lets the caller line the splash up with the bobber landing
 *  rather than the swing that threw it. */
export function playFishingSplashSound(delayMs = 0) {
  if (!canUseAudio()) return
  playAudioFromPool(
    fishingPools,
    FISHING_SPLASH_SOUND_URL,
    FISHING_SPLASH_VOLUME,
    FISHING_POOL_SIZE,
    delayMs
  )
}

export function playFishingPlopSound() {
  if (!canUseAudio()) return
  playAudioFromPool(
    fishingPools,
    FISHING_PLOP_SOUND_URL,
    FISHING_PLOP_VOLUME,
    FISHING_POOL_SIZE
  )
}

export function playFishingReelSound() {
  if (!canUseAudio()) return
  playAudioFromPool(
    fishingPools,
    FISHING_REEL_SOUND_URL,
    FISHING_REEL_VOLUME,
    FISHING_REEL_POOL_SIZE
  )
}

export function playFishingSnapSound() {
  if (!canUseAudio()) return
  playAudioFromPool(
    fishingPools,
    FISHING_SNAP_SOUND_URL,
    FISHING_SNAP_VOLUME,
    FISHING_POOL_SIZE
  )
}

export function playFishingCatchSound() {
  if (!canUseAudio()) return
  playAudioFromPool(
    fishingPools,
    FISHING_CATCH_SOUND_URL,
    FISHING_CATCH_VOLUME,
    FISHING_POOL_SIZE
  )
}
