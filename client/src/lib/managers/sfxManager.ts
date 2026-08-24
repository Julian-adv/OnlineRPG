import { get, writable } from 'svelte/store'
import {
  DEFAULT_MATERIAL_HIT_SOUND_URL,
  DEFAULT_MATERIAL_MISS_SOUND_URL,
  getAllMaterialHitSoundUrls,
  getAllMaterialMissSoundUrls,
} from '../data/materialImpactSounds'
import monsterDefs from '../data/monsterDefs'
import type { Gender } from '../network/networkTypes'

const MONSTER_DEATH_VOLUME = 0.5
const MONSTER_DEATH_POOL_SIZE = 3
const SWORD_HIT_VOLUME = 0.55
const SWORD_MISS_VOLUME = 0.5
const SWORD_HIT_POOL_SIZE = 4
const SWORD_MISS_POOL_SIZE = 4

interface SoundSpec {
  url: string
  volume: number
  pool: number
}
type SoundTable = Readonly<Record<string, SoundSpec>>

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

const PROP_SOUNDS = {
  break: { url: '/sounds/crate-break.ogg', volume: 0.4, pool: 2 },
  chestOpen: { url: '/sounds/chest-open.ogg', volume: 0.5, pool: 2 },
  coinSpill: { url: '/sounds/coin-spill.ogg', volume: 0.5, pool: 2 },
} as const
export type PropSound = keyof typeof PROP_SOUNDS

// Voice on taking damage. Pool 1 each: only one cry ever plays at a time.
const PLAYER_HURT_SOUNDS: Record<Gender, SoundSpec> = {
  female: { url: '/sounds/player-hurt-female.ogg', volume: 0.5, pool: 1 },
  male: { url: '/sounds/player-hurt-male.ogg', volume: 0.5, pool: 1 },
}

// Death cry. Pool 2: two players can fall within a breath of each other.
const PLAYER_DEATH_SOUNDS: Record<Gender, SoundSpec> = {
  female: { url: '/sounds/player-death-female.ogg', volume: 0.5, pool: 2 },
  male: { url: '/sounds/player-death-male.ogg', volume: 0.5, pool: 2 },
}

const DUNGEON_SOUNDS = {
  reset: { url: '/sounds/dungeon-roar.ogg', volume: 0.5, pool: 1 },
} as const
export type DungeonSound = keyof typeof DUNGEON_SOUNDS

const STORAGE_KEY_VOLUME = 'onlinerpg_sfxVolume'
const STORAGE_KEY_MUTED = 'onlinerpg_sfxMuted'
const DEFAULT_SFX_VOLUME = 0.5

// Node ≥22 exposes a localStorage global whose methods are unusable without
// --localstorage-file, so feature-test the method, not the object.
const storage =
  typeof localStorage !== 'undefined' &&
  typeof localStorage.getItem === 'function'
    ? localStorage
    : null

function loadSfxVolume(): number {
  const saved = storage?.getItem(STORAGE_KEY_VOLUME)
  if (saved != null) {
    const v = parseFloat(saved)
    if (!isNaN(v)) return Math.max(0, Math.min(1, v))
  }
  return DEFAULT_SFX_VOLUME
}

export const sfxVolume = writable<number>(loadSfxVolume())
export const sfxMuted = writable<boolean>(
  storage?.getItem(STORAGE_KEY_MUTED) === 'true'
)

let volumeSaveTimer: ReturnType<typeof setTimeout> | undefined

sfxVolume.subscribe((v) => {
  if (!storage) return
  clearTimeout(volumeSaveTimer)
  volumeSaveTimer = setTimeout(
    () => storage.setItem(STORAGE_KEY_VOLUME, String(v)),
    300
  )
})

sfxMuted.subscribe((m) => {
  storage?.setItem(STORAGE_KEY_MUTED, String(m))
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

// One pool per url, shared by every group. Playback volume is applied per
// play, so a url two groups happen to share still sounds right in both.
const pools = new Map<string, AudioPool>()

function canUseAudio(): boolean {
  return typeof Audio !== 'undefined'
}

function createAudio(url: string, volume: number): HTMLAudioElement {
  const audio = new Audio(url)
  audio.preload = 'auto'
  audio.volume = volume
  return audio
}

function preloadAudioPool(url: string, volume: number, poolSize: number) {
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

function playAudioFromPool(url: string, volume: number, poolSize: number) {
  preloadAudioPool(url, volume, poolSize)

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
    return audio
  } catch {
    // Browser audio policies can reject playback until the first user gesture.
  }
}

function preloadSounds(table: SoundTable) {
  for (const { url, volume, pool } of Object.values(table)) {
    preloadAudioPool(url, volume, pool)
  }
}

function playSound({ url, volume, pool }: SoundSpec) {
  if (!canUseAudio()) return
  playAudioFromPool(url, volume, pool)
}

export function preloadSwordHitSound() {
  for (const url of getAllMaterialHitSoundUrls()) {
    preloadAudioPool(url, SWORD_HIT_VOLUME, SWORD_HIT_POOL_SIZE)
  }
}

export function preloadSwordMissSound() {
  for (const url of getAllMaterialMissSoundUrls()) {
    preloadAudioPool(url, SWORD_MISS_VOLUME, SWORD_MISS_POOL_SIZE)
  }
}

export function playSwordHitSound(url = DEFAULT_MATERIAL_HIT_SOUND_URL) {
  if (!canUseAudio()) return
  playAudioFromPool(url, SWORD_HIT_VOLUME, SWORD_HIT_POOL_SIZE)
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
  playAudioFromPool(url, SWORD_MISS_VOLUME, SWORD_MISS_POOL_SIZE)
}

/** Monster death cry; only the defs that declare one. */
export function preloadMonsterDeathSounds() {
  for (const def of Object.values(monsterDefs)) {
    if (def.deathSound) {
      preloadAudioPool(
        def.deathSound,
        MONSTER_DEATH_VOLUME,
        MONSTER_DEATH_POOL_SIZE
      )
    }
  }
}

export function playMonsterDeathSound(url: string) {
  if (!canUseAudio()) return
  playAudioFromPool(url, MONSTER_DEATH_VOLUME, MONSTER_DEATH_POOL_SIZE)
}

export function preloadPlayerHurtSounds() {
  preloadSounds(PLAYER_HURT_SOUNDS)
}

// A cry already in the air wins: restarting it mid-breath reads as a stutter
// rather than a second blow.
let hurtVoice: HTMLAudioElement | undefined

/** `delayMs` lines the cry up with the monster's impact frame. */
export function playPlayerHurtSound(gender: Gender, delayMs = 0) {
  const spec = PLAYER_HURT_SOUNDS[gender]
  if (!spec || !canUseAudio()) return
  if (delayMs > 0) {
    window.setTimeout(() => playPlayerHurtSound(gender), delayMs)
    return
  }
  if (hurtVoice && !hurtVoice.paused && !hurtVoice.ended) return
  hurtVoice = playAudioFromPool(spec.url, spec.volume, spec.pool)
}

export function preloadPlayerDeathSounds() {
  preloadSounds(PLAYER_DEATH_SOUNDS)
}

/** Immediate: the collapse starts at the killing-blow message, so the cry
 *  plays with it rather than waiting for the attack's impact frame. */
export function playPlayerDeathSound(gender: Gender) {
  playSound(PLAYER_DEATH_SOUNDS[gender])
}

export function preloadPropSounds() {
  preloadSounds(PROP_SOUNDS)
}

/** `break`: barrel/crate shatter, timed by the caller to the slash contact
 *  frame. `chestOpen`: lid swing on the open broadcast. `coinSpill`: the
 *  pile's pour clip starting. */
export function playPropSound(kind: PropSound) {
  playSound(PROP_SOUNDS[kind])
}

/** Called on dungeon entry, not at world entry: the roar is the heaviest clip
 *  here and only delvers ever hear it. */
export function preloadDungeonSounds() {
  preloadSounds(DUNGEON_SOUNDS)
}

export function playDungeonSound(kind: DungeonSound) {
  playSound(DUNGEON_SOUNDS[kind])
}

export function preloadFishingSounds() {
  preloadSounds(FISHING_SOUNDS)
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
  playSound(FISHING_SOUNDS[kind])
}

/** A cast aborted mid-flight must not splash after the line is back in. */
export function cancelPendingFishingSounds() {
  for (const timer of pendingFishingTimers) window.clearTimeout(timer)
  pendingFishingTimers.clear()
}
