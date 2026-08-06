import { get, writable } from 'svelte/store'
import { BGM_TRACKS, bgmFileFor } from '../data/bgmTracks'

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
const BATTLE_FADE_OUT_MS = 3000
const BATTLE_FADE_STEP_MS = 50
const BATTLE_QUIET_MIN_SEC = 5
const BATTLE_QUIET_MAX_SEC = 20

const MIN_QUIET_SEC = 0
const MAX_QUIET_SEC = 60

const STORAGE_KEY_VOLUME = 'onlinerpg_bgmVolume'
const STORAGE_KEY_MUTED = 'onlinerpg_bgmMuted'
const DEFAULT_VOLUME = 0.1

function loadVolume(): number {
  const saved = localStorage.getItem(STORAGE_KEY_VOLUME)
  if (saved !== null) {
    const v = parseFloat(saved)
    if (!isNaN(v)) return Math.max(0, Math.min(1, v))
  }
  return DEFAULT_VOLUME
}

export const currentBgmTrack = writable<string>('')
export const bgmVolume = writable<number>(loadVolume())
export const bgmMuted = writable<boolean>(
  localStorage.getItem(STORAGE_KEY_MUTED) === 'true'
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

let isFadingOut = false
let battleAudio: HTMLAudioElement | null = null

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
  if (get(bgmMuted)) {
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
  audio.src = `/bgm/${file}`
  audio.play().catch(() => {})
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
  // A recital cannot outrank a fight: end it, and let the performer's client
  // tell the server so the strum animation stops everywhere.
  const performanceEnded = releasePerformance()
  isFadingOut = false

  // Pause normal BGM
  clearTimeout(quietTimer)
  if (audio) {
    audio.pause()
  }
  currentBgmTrack.set('')

  // Clear any pending linger/fade/quiet from a previous battle
  clearTimeout(battleLingerTimer)
  clearInterval(battleFadeTimer)
  clearTimeout(battleQuietTimer)

  const file =
    BATTLE_BGM_FILES[Math.floor(Math.random() * BATTLE_BGM_FILES.length)]
  const trackName = file.replace(/\.(mp3|m4a)$/, '')

  if (!battleAudio) {
    battleAudio = new Audio()
    battleAudio.loop = true
    battleAudio.addEventListener('playing', () => {
      currentBgmTrack.set(battleAudio!.dataset.trackName ?? '')
    })
  }

  applyAudioSettings(battleAudio)
  battleAudio.dataset.trackName = trackName
  battleAudio.currentTime = 0
  battleAudio.src = `/bgm/${file}`
  if (!get(bgmMuted)) {
    battleAudio.play().catch(() => {})
  }

  performanceEnded?.()
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

  const startVol = battleAudio.volume
  if (startVol === 0) {
    battleAudio.pause()
    currentBgmTrack.set('')
    scheduleNormalBgmResume()
    return
  }

  isFadingOut = true
  const steps = BATTLE_FADE_OUT_MS / BATTLE_FADE_STEP_MS
  const volStep = startVol / steps
  let remaining = steps

  clearInterval(battleFadeTimer)
  battleFadeTimer = setInterval(() => {
    remaining--
    if (remaining <= 0 || !battleAudio) {
      clearInterval(battleFadeTimer)
      isFadingOut = false
      if (battleAudio) {
        battleAudio.pause()
        battleAudio.volume = startVol
      }
      currentBgmTrack.set('')
      scheduleNormalBgmResume()
      return
    }
    battleAudio!.volume = Math.max(0, battleAudio!.volume - volStep)
  }, BATTLE_FADE_STEP_MS)
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
  audio?.pause()
  battleAudio?.pause()
  // The performer's own track keeps running, silenced: its `ended` is what
  // stops the strum animation. A listener's copy just pauses.
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
// everyone. Listeners never load a track they would not hear.

let performanceAudio: HTMLAudioElement | null = null
let performanceEnded: (() => void) | null = null

function applyPerformanceVolume() {
  applyAudioSettings(
    performanceAudio,
    mode === 'performance' ? getTargetVolume() : 0
  )
}

/** Put the running performance back on the speakers — it was silenced by
 *  battle music or by a mute the player has since lifted. */
function takePerformanceFloor(): boolean {
  if (!performanceAudio || getTargetVolume() <= 0) return false
  mode = 'performance'
  applyPerformanceVolume()
  performanceAudio.play().catch(() => {})
  currentBgmTrack.set(performanceAudio.dataset.trackName ?? '')
  return true
}

/** Drop the running performance and hand the speakers back to `mode`'s owner.
 *  Returns the end callback so the caller decides when the performer hears
 *  about it. Never restarts the playlist — callers do, when they want it. */
function releasePerformance(): (() => void) | null {
  if (!performanceAudio) return null
  const onEnded = performanceEnded
  performanceEnded = null
  performanceAudio.pause()
  // Drop the media resource too; the element itself waits for GC, but a
  // half-buffered track should not.
  performanceAudio.removeAttribute('src')
  performanceAudio.load()
  performanceAudio = null
  if (mode === 'performance') {
    mode = 'normal'
    currentBgmTrack.set('')
  }
  return onEnded
}

/** Start `track` for a nearby `/play_music`. `onEnded` marks the local player
 *  as the performer and fires when the track runs out — that is the cue to
 *  stop the emote. Returns false when the track is unknown or inaudible here. */
export function playPerformance(track: string, onEnded?: () => void): boolean {
  const file = bgmFileFor(track)
  if (!file) return false

  const mine = onEnded !== undefined
  // Volume at zero counts as BGM off: no point downloading a track to silence.
  const audible = getTargetVolume() > 0 && mode !== 'battle'
  if (!mine && !audible) return false

  const previousEnded = releasePerformance()
  performanceEnded = onEnded ?? null

  clearTimeout(quietTimer)
  audio?.pause()

  // A fresh element per performance: a late `error` from the one we dropped
  // must not end the tune that replaced it.
  const el = new Audio()
  performanceAudio = el
  const finish = () => {
    if (performanceAudio === el) {
      releasePerformance()?.()
      resumeNormalBgm()
    }
  }
  el.addEventListener('ended', finish)
  el.addEventListener('error', finish)
  el.dataset.trackName = track
  el.src = `/bgm/${file}`
  if (audible) {
    mode = 'performance'
    currentBgmTrack.set(track)
  }
  applyPerformanceVolume()
  el.play().catch(() => {})

  // Whoever was playing lost the floor — tell them, so a performer whose track
  // was cut short leaves the emote instead of strumming in silence.
  previousEnded?.()
  return true
}

/** The performance is over from the outside (the player moved away, left, or
 *  the server said so) — no end callback, that news already travelled. */
export function stopPerformance() {
  releasePerformance()
  resumeNormalBgm()
}

bgmVolume.subscribe((v) => {
  clearTimeout(volumeSaveTimer)
  volumeSaveTimer = setTimeout(
    () => localStorage.setItem(STORAGE_KEY_VOLUME, String(v)),
    300
  )
  applyAudioSettings(audio)
  if (!isFadingOut) applyAudioSettings(battleAudio)
  applyPerformanceVolume()
  if (getTargetVolume() <= 0) {
    pauseForMute()
  } else if (!get(bgmMuted)) {
    resumeAfterUnmute()
  }
})

bgmMuted.subscribe((m) => {
  localStorage.setItem(STORAGE_KEY_MUTED, String(m))
  applyAudioSettings(audio)
  applyAudioSettings(battleAudio)
  applyPerformanceVolume()
  if (m) {
    pauseForMute()
  } else {
    resumeAfterUnmute()
  }
})
