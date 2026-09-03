import { get, writable } from 'svelte/store'
import { BGM_TRACKS, bgmFileFor } from '../data/bgmTracks'
import { assetUrl } from '../utils/assetUrl'

const bgmSrc = (file: string) => assetUrl(`/bgm/${file}`)

/** `<audio>` streams with Range requests, which browsers don't reuse across
 *  page loads even under `immutable`; fetching the whole file first lands a
 *  200 in the HTTP cache, so each browser downloads a track once. */
async function loadBgmSrc(file: string): Promise<string> {
  const url = bgmSrc(file)
  try {
    const res = await fetch(url)
    if (!res.ok) return url
    return URL.createObjectURL(await res.blob())
  } catch {
    return url
  }
}

function releaseSrc(src: string) {
  if (src.startsWith('blob:')) URL.revokeObjectURL(src)
}

/** Drop the media resource; the element itself waits for GC, but a blob or a
 *  half-buffered track should not. */
function releaseElement(el: HTMLAudioElement) {
  releaseSrc(el.src)
  el.removeAttribute('src')
  el.load()
}

const loads = new WeakMap<HTMLAudioElement, number>()

/** Load `file` into `el` and play it, unless a newer load on the same element
 *  or `stillWanted()` says the moment has passed. */
async function attachTrack(
  el: HTMLAudioElement,
  file: string,
  stillWanted: () => boolean
) {
  const seq = (loads.get(el) ?? 0) + 1
  loads.set(el, seq)
  const src = await loadBgmSrc(file)
  if (loads.get(el) !== seq || !stillWanted()) {
    releaseSrc(src)
    return
  }
  releaseSrc(el.src)
  el.src = src
  el.play().catch(() => {})
}

const BATTLE_BGM_FILES = [
  'Blood and Bronze.mp3',
  'Blood and Bronze (1).mp3',
  'Drums of Valor.mp3',
  'Clash of the Iron Realm.m4a',
  'Radiant Vanguard Overdrive.m4a',
  'The Abyssal March.m4a',
  'Triumph of the Vanguard.m4a',
]
const BATTLE_LINGER_MS = 5000
const FADE_OUT_MS = 3000
const FADE_STEP_MS = 50
const BATTLE_QUIET_MIN_SEC = 5
const BATTLE_QUIET_MAX_SEC = 20

const MIN_QUIET_SEC = 0
const MAX_QUIET_SEC = 60

const STORAGE_KEY_VOLUME = 'onlinerpg_bgmVolume'
const STORAGE_KEY_MUTED = 'onlinerpg_bgmMuted'
const DEFAULT_VOLUME = 0.1

/** localStorage can be absent or throw (private mode); settings then just
 *  don't persist. */
function storageGet(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function storageSet(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    // Not persisted.
  }
}

function loadVolume(): number {
  const saved = storageGet(STORAGE_KEY_VOLUME)
  if (saved !== null) {
    const v = parseFloat(saved)
    if (!isNaN(v)) return Math.max(0, Math.min(1, v))
  }
  return DEFAULT_VOLUME
}

export const currentBgmTrack = writable<string>('')
export const bgmVolume = writable<number>(loadVolume())
export const bgmMuted = writable<boolean>(
  storageGet(STORAGE_KEY_MUTED) === 'true'
)

/** What owns the speakers right now. Battle music outranks a `/play_music`
 *  performance, which outranks the playlist. */
type BgmMode = 'normal' | 'battle' | 'performance'

let mode: BgmMode = 'normal'
let audio: HTMLAudioElement | null = null
let playlist: string[] = []
let playlistIndex = 0
let volumeSaveTimer: ReturnType<typeof setTimeout> | undefined

function getTargetVolume(): number {
  return get(bgmMuted) ? 0 : get(bgmVolume)
}

function applyAudioSettings(
  el: HTMLAudioElement | null,
  targetVolume = getTargetVolume()
) {
  if (!el) return
  // iOS Safari does not reliably honor programmatic `volume` changes on
  // media elements, so use the native muted flag as the hard mute path.
  el.muted = targetVolume <= 0
  try {
    el.volume = targetVolume
  } catch {
    // Some browsers expose volume as effectively read-only for media elements.
  }
}

let battleAudio: HTMLAudioElement | null = null

/** Walk `el.volume` toward `target` over `ms`, then `onDone(arrived)` —
 *  `arrived` is false when `abort()` cut the fade short. Callers keep the
 *  returned timer as their "fade in flight" sentinel; while one is live they
 *  must not let other code write the element's volume. */
function fadeVolume(
  el: HTMLAudioElement,
  target: number,
  ms: number,
  abort: () => boolean,
  onDone: (arrived: boolean) => void
): ReturnType<typeof setInterval> {
  const step = Math.abs(target - el.volume) / (ms / FADE_STEP_MS)
  const timer = setInterval(() => {
    if (abort()) {
      clearInterval(timer)
      onDone(false)
      return
    }
    const delta = target - el.volume
    if (Math.abs(delta) <= step) {
      clearInterval(timer)
      onDone(true)
      return
    }
    el.volume = Math.min(1, Math.max(0, el.volume + Math.sign(delta) * step))
  }, FADE_STEP_MS)
  return timer
}

function shufflePlaylist() {
  playlist = [...BGM_TRACKS]
  for (let i = playlist.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1))
    ;[playlist[i], playlist[j]] = [playlist[j], playlist[i]]
  }
  playlistIndex = 0
}

let quietTimer: ReturnType<typeof setTimeout> | undefined
let isFirstTrack = true

function playNext() {
  if (audio) releaseElement(audio)
  if (mode !== 'normal') return
  if (!isFirstTrack) {
    const delaySec =
      MIN_QUIET_SEC + Math.random() * (MAX_QUIET_SEC - MIN_QUIET_SEC)
    currentBgmTrack.set('')
    clearTimeout(quietTimer)
    quietTimer = setTimeout(playTrack, delaySec * 1000)
    return
  }
  isFirstTrack = false
  playTrack()
}

function playTrack() {
  if (mode !== 'normal') return
  if (get(bgmMuted) || inBardZone) {
    currentBgmTrack.set('')
    return
  }
  if (playlistIndex >= playlist.length) {
    shufflePlaylist()
  }

  const trackName = playlist[playlistIndex++]
  const file = bgmFileFor(trackName)
  if (!file) return

  if (!audio) {
    audio = new Audio()
    audio.addEventListener('ended', playNext)
    audio.addEventListener('error', playNext)
    audio.addEventListener('playing', () => {
      currentBgmTrack.set(audio!.dataset.trackName ?? '')
    })
  }

  applyAudioSettings(audio)
  audio.dataset.trackName = trackName
  void attachTrack(audio, file, () => mode === 'normal' && !get(bgmMuted))
}

let started = false

export function startBgm() {
  if (started) return
  started = true
  shufflePlaylist()
  playNext()
}

// --- Battle music ---

let battleLingerTimer: ReturnType<typeof setTimeout> | undefined
let battleFadeTimer: ReturnType<typeof setInterval> | undefined
let battleQuietTimer: ReturnType<typeof setTimeout> | undefined

export function startBattleMusic() {
  if (mode === 'battle') return
  mode = 'battle'
  // A recital cannot outrank a fight: a listener's copy waits to rejoin
  // after; the performer ends it, and their client tells the server so the
  // strum animation stops everywhere.
  let endedCallback: (() => void) | null = null
  if (performanceEnded) endedCallback = dropPerformance()
  else performanceAudio?.pause()

  // Pause normal BGM
  clearTimeout(quietTimer)
  if (audio) {
    audio.pause()
  }
  currentBgmTrack.set('')

  // Clear any pending linger/fade/quiet from a previous battle
  clearTimeout(battleLingerTimer)
  clearInterval(battleFadeTimer)
  battleFadeTimer = undefined
  clearTimeout(battleQuietTimer)

  const file =
    BATTLE_BGM_FILES[Math.floor(Math.random() * BATTLE_BGM_FILES.length)]
  const trackName = file.replace(/\.(mp3|m4a)$/, '')

  if (!battleAudio) {
    battleAudio = new Audio()
    battleAudio.loop = true
    battleAudio.addEventListener('playing', () => {
      currentBgmTrack.set(battleAudio?.dataset.trackName ?? '')
    })
  }
  battleAudio.dataset.trackName = trackName
  applyAudioSettings(battleAudio)
  void attachTrack(battleAudio, file, () => mode === 'battle' && !get(bgmMuted))

  endedCallback?.()
}

export function stopBattleMusic() {
  if (mode !== 'battle') return
  mode = 'normal'

  if (!battleAudio) {
    resumeNormalBgm()
    return
  }

  // Wait a bit before fading out
  clearTimeout(battleLingerTimer)
  battleLingerTimer = setTimeout(fadeOutBattleMusic, BATTLE_LINGER_MS)
}

function fadeOutBattleMusic() {
  if (mode === 'battle' || !battleAudio) return

  const el = battleAudio
  const startVol = el.volume
  clearInterval(battleFadeTimer)
  battleFadeTimer = fadeVolume(
    el,
    0,
    FADE_OUT_MS,
    () => false,
    () => {
      battleFadeTimer = undefined
      el.pause()
      el.volume = startVol
      currentBgmTrack.set('')
      scheduleNormalBgmResume()
    }
  )
}

function scheduleNormalBgmResume() {
  const delaySec =
    BATTLE_QUIET_MIN_SEC +
    Math.random() * (BATTLE_QUIET_MAX_SEC - BATTLE_QUIET_MIN_SEC)
  clearTimeout(battleQuietTimer)
  battleQuietTimer = setTimeout(resumeNormalBgm, delaySec * 1000)
}

function resumeNormalBgm() {
  if (mode !== 'normal') return
  if (get(bgmMuted)) return
  if (takePerformanceFloor()) return
  if (inBardZone) return
  if (!audio) {
    playTrack()
    return
  }
  if (audio.ended || !audio.src) {
    playTrack()
  } else {
    applyAudioSettings(audio)
    audio.play().catch(() => {})
    currentBgmTrack.set(audio.dataset.trackName ?? '')
  }
}

function pauseForMute() {
  cancelBardFade()
  audio?.pause()
  battleAudio?.pause()
  // The performer's own track keeps running, silenced: its `ended` is what
  // stops the strum animation. A listener's copy waits to rejoin later.
  if (mode === 'performance') mode = 'normal'
  applyPerformanceVolume()
  if (!performanceEnded) performanceAudio?.pause()
  currentBgmTrack.set('')
}

function resumeAfterUnmute() {
  applyAudioSettings(audio)
  applyAudioSettings(battleAudio)

  if (mode === 'battle') {
    if (battleAudio) {
      battleAudio.play().catch(() => {})
      currentBgmTrack.set(battleAudio.dataset.trackName ?? '')
    }
    return
  }

  if (takePerformanceFloor()) return

  if (started) {
    resumeNormalBgm()
  }
}

// --- /play_music performance ---
// One slot: a new performance replaces the running one. `performanceAudio`
// doubles as the "a performance exists" flag, and a non-null `performanceEnded`
// marks it as the local player's own. The performer's element is the clock —
// it runs even while silenced, because its `ended` is what ends the emote for
// everyone. `currentPerformance` is the tune on right now, loaded or not: a
// listener never loads a track they would not hear, and rejoins mid-track
// (by wall clock) once the speakers free up.

let performanceAudio: HTMLAudioElement | null = null
let performanceEnded: (() => void) | null = null
let performanceFadeTimer: ReturnType<typeof setInterval> | undefined
let currentPerformance: { track: string; startedAt: number } | null = null

const performanceElapsedSecs = () =>
  (performance.now() - (currentPerformance?.startedAt ?? 0)) / 1000

/** Inside a bard NPC's earshot the playlist stays silent — performing or not —
 *  so a performance never has to land on top of the BGM. */
let inBardZone = false
let bardFadeTimer: ReturnType<typeof setInterval> | undefined

/** Stop the playlist fade and put its volume back where the settings say. */
function cancelBardFade() {
  if (bardFadeTimer === undefined) return
  clearInterval(bardFadeTimer)
  bardFadeTimer = undefined
  applyAudioSettings(audio)
}

export function setBardZone(inside: boolean) {
  if (inBardZone === inside) return
  inBardZone = inside
  if (!inside) {
    cancelBardFade()
    resumeNormalBgm()
    return
  }
  if (mode !== 'normal') return
  clearTimeout(quietTimer)
  if (!audio || audio.paused) {
    currentBgmTrack.set('')
    return
  }

  const el = audio
  bardFadeTimer = fadeVolume(
    el,
    0,
    FADE_OUT_MS,
    // Battle music or a performance taking the speakers mid-fade owns
    // `currentBgmTrack` now — back out without touching it.
    () => mode !== 'normal',
    (arrived) => {
      bardFadeTimer = undefined
      if (arrived) {
        el.pause()
        currentBgmTrack.set('')
      }
      applyAudioSettings(el)
    }
  )
}

function applyPerformanceVolume() {
  if (performanceFadeTimer !== undefined) return
  applyAudioSettings(
    performanceAudio,
    mode === 'performance' ? getTargetVolume() : 0
  )
}

/** Put the current performance back on the speakers — it was silenced by
 *  battle music or by a mute the player has since lifted — at wherever the
 *  tune is by now. Returns whether it now owns the speakers. */
function takePerformanceFloor(): boolean {
  if (!currentPerformance || getTargetVolume() <= 0) return false
  if (!performanceAudio) {
    playPerformance(
      currentPerformance.track,
      undefined,
      performanceElapsedSecs()
    )
    return mode === 'performance'
  }
  mode = 'performance'
  applyPerformanceVolume()
  // A listener's copy sat paused while the tune went on; the performer's own
  // kept running.
  if (!performanceEnded && performanceAudio.paused) {
    performanceAudio.currentTime = performanceElapsedSecs()
  }
  performanceAudio.play().catch(() => {})
  currentBgmTrack.set(currentPerformance.track)
  return true
}

/** Drop the running performance and hand the speakers back to `mode`'s owner.
 *  Returns the end callback so the caller decides when the performer hears
 *  about it. Never restarts the playlist — callers do, when they want it. */
function releasePerformance(): (() => void) | null {
  clearInterval(performanceFadeTimer)
  performanceFadeTimer = undefined
  if (!performanceAudio) return null
  const onEnded = performanceEnded
  performanceEnded = null
  performanceAudio.pause()
  releaseElement(performanceAudio)
  performanceAudio = null
  if (mode === 'performance') {
    mode = 'normal'
    currentBgmTrack.set('')
  }
  return onEnded
}

/** The tune is over for good: release the element and forget it. */
function dropPerformance(): (() => void) | null {
  currentPerformance = null
  return releasePerformance()
}

/** Walking into a performance already underway: rise from silence to the set
 *  volume instead of jumping in. */
function startPerformanceFadeIn(el: HTMLAudioElement) {
  el.volume = 0
  clearInterval(performanceFadeTimer)
  performanceFadeTimer = fadeVolume(
    el,
    getTargetVolume(),
    FADE_OUT_MS,
    () => mode !== 'performance',
    () => {
      performanceFadeTimer = undefined
      applyPerformanceVolume()
    }
  )
}

/** Start `track` for a nearby `/play_music`. `onEnded` marks the local player
 *  as the performer and fires when the track runs out — that is the cue to
 *  stop the emote. `offsetSecs` > 0 joins a tune mid-track (the listener just
 *  came into earshot) and fades it in. Returns false when the track is
 *  unknown. */
export function playPerformance(
  track: string,
  onEnded?: () => void,
  offsetSecs = 0
): boolean {
  const file = bgmFileFor(track)
  if (!file) return false

  const mine = onEnded !== undefined
  const audible = getTargetVolume() > 0 && mode !== 'battle'
  const previousEnded = releasePerformance()
  currentPerformance = {
    track,
    startedAt: performance.now() - offsetSecs * 1000,
  }

  // Volume at zero counts as BGM off: no point downloading a track to
  // silence. Rejoin when the speakers come back.
  if (!mine && !audible) {
    previousEnded?.()
    return true
  }
  performanceEnded = onEnded ?? null

  clearTimeout(quietTimer)
  audio?.pause()

  // A fresh element per performance: a late `error` from the one we dropped
  // must not end the tune that replaced it.
  const el = new Audio()
  performanceAudio = el
  const finish = () => {
    if (performanceAudio === el) {
      dropPerformance()?.()
      resumeNormalBgm()
    }
  }
  el.addEventListener('ended', finish)
  el.addEventListener('error', finish)
  el.dataset.trackName = track
  if (audible) {
    mode = 'performance'
    currentBgmTrack.set(track)
  }
  applyPerformanceVolume()
  if (offsetSecs > 0) {
    // A late listener should hear the tune now, and a blob only plays once
    // the whole file is down: stream it and seek instead.
    el.addEventListener(
      'loadedmetadata',
      () => {
        // Past the end just clamps and fires `ended` — a stale performance
        // cleans itself up.
        el.currentTime = offsetSecs
      },
      { once: true }
    )
    el.src = bgmSrc(file)
    if (audible) startPerformanceFadeIn(el)
    el.play().catch(() => {})
  } else {
    void attachTrack(el, file, () => performanceAudio === el)
  }

  // Whoever was playing lost the floor — tell them, so a performer whose track
  // was cut short leaves the emote instead of strumming in silence.
  previousEnded?.()
  return true
}

/** The performance is over from the outside (the player moved away, left, or
 *  the server said so) — no end callback, that news already travelled. */
export function stopPerformance() {
  dropPerformance()
  resumeNormalBgm()
}

/** A listener crossed out of the performance's delivery circle (either side
 *  moved): fade the tune down, then let it go. An inaudible or unloaded copy
 *  just stops. */
export function fadeOutPerformance() {
  if (!performanceAudio || mode !== 'performance') {
    stopPerformance()
    return
  }
  clearInterval(performanceFadeTimer)
  performanceFadeTimer = fadeVolume(
    performanceAudio,
    0,
    FADE_OUT_MS,
    // A mode change mid-fade (mute took the speakers) ends it early; a
    // replacement performance cannot get here — releasePerformance cancels.
    () => mode !== 'performance',
    () => stopPerformance()
  )
}

bgmVolume.subscribe((v) => {
  clearTimeout(volumeSaveTimer)
  volumeSaveTimer = setTimeout(
    () => storageSet(STORAGE_KEY_VOLUME, String(v)),
    300
  )
  if (bardFadeTimer === undefined) applyAudioSettings(audio)
  if (battleFadeTimer === undefined) applyAudioSettings(battleAudio)
  applyPerformanceVolume()
  if (getTargetVolume() <= 0) {
    pauseForMute()
  } else if (!get(bgmMuted)) {
    resumeAfterUnmute()
  }
})

bgmMuted.subscribe((m) => {
  storageSet(STORAGE_KEY_MUTED, String(m))
  applyAudioSettings(audio)
  applyAudioSettings(battleAudio)
  applyPerformanceVolume()
  if (m) {
    pauseForMute()
  } else {
    resumeAfterUnmute()
  }
})
