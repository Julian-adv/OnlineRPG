import { get, writable } from 'svelte/store'
import { BGM_TRACKS, bgmFileFor } from '../data/bgmTracks'
import { assetUrl } from '../utils/assetUrl'

const bgmSrc = (file: string) => assetUrl(`/bgm/${file}`)

const BATTLE_CACHE_BYTES = 48 * 1024 * 1024
const battleBlobs = new Map<string, Blob>()
const battleDownloads = new Map<string, Promise<Blob | null>>()
let battleCacheBytes = 0

async function fetchBgmBlob(file: string): Promise<Blob | null> {
  try {
    const res = await fetch(bgmSrc(file))
    return res.ok ? await res.blob() : null
  } catch {
    return null
  }
}

function loadBattleBlob(file: string): Promise<Blob | null> {
  const cached = battleBlobs.get(file)
  if (cached) {
    battleBlobs.delete(file)
    battleBlobs.set(file, cached)
    return Promise.resolve(cached)
  }
  const pending = battleDownloads.get(file)
  if (pending) return pending
  const download = fetchBgmBlob(file).then((blob) => {
    battleDownloads.delete(file)
    if (blob && !disposed && blob.size <= BATTLE_CACHE_BYTES) {
      for (const [oldFile, oldBlob] of battleBlobs) {
        if (battleCacheBytes + blob.size <= BATTLE_CACHE_BYTES) break
        battleBlobs.delete(oldFile)
        battleCacheBytes -= oldBlob.size
      }
      battleBlobs.set(file, blob)
      battleCacheBytes += blob.size
    }
    return blob
  })
  battleDownloads.set(file, download)
  return download
}

// Whole-file fetch avoids Range requests; battle blobs also survive HTTP cache misses.
async function loadBgmSrc(file: string): Promise<string> {
  const blob = await (BATTLE_BGM_FILES.includes(file)
    ? loadBattleBlob(file)
    : fetchBgmBlob(file))
  return blob ? URL.createObjectURL(blob) : bgmSrc(file)
}

function releaseSrc(src: string) {
  if (src.startsWith('blob:')) URL.revokeObjectURL(src)
}

function releaseElement(el: HTMLAudioElement) {
  loads.set(el, (loads.get(el) ?? 0) + 1)
  releaseSrc(el.src)
  el.removeAttribute('src')
  el.load()
}

const loads = new WeakMap<HTMLAudioElement, number>()
let disposed = false

/** Ignore loads superseded by another track or playback state. */
async function attachTrack(
  el: HTMLAudioElement,
  file: string,
  stillWanted: () => boolean
) {
  const seq = (loads.get(el) ?? 0) + 1
  loads.set(el, seq)
  if (disposed || !stillWanted()) return
  const src = await loadBgmSrc(file)
  if (disposed || loads.get(el) !== seq || !stillWanted()) {
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
/** How long heard live notes keep the playlist quiet past the last one. */
const LIVE_NOTES_QUIET_HOLD_MS = 10_000
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
let battleFile: string | null = null

/** Own the element's volume until arrival or abort; report which ended the fade. */
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
  if (disposed || mode !== 'normal') return
  if (getTargetVolume() <= 0 || playlistQuiet()) {
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
      currentBgmTrack.set(audio?.dataset.trackName ?? '')
    })
  }

  applyAudioSettings(audio)
  audio.dataset.trackName = trackName
  void attachTrack(
    audio,
    file,
    () => mode === 'normal' && getTargetVolume() > 0 && !playlistQuiet()
  )
}

let started = false

export function startBgm() {
  if (disposed || started) return
  started = true
  shufflePlaylist()
  playNext()
}

// --- Battle music ---

let battleLingerTimer: ReturnType<typeof setTimeout> | undefined
let battleFadeTimer: ReturnType<typeof setInterval> | undefined
let battleQuietTimer: ReturnType<typeof setTimeout> | undefined

export function startBattleMusic() {
  if (disposed || mode === 'battle') return
  mode = 'battle'
  // Combat ends our performance and pauses a nearby performer's track.
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

  battleFile =
    BATTLE_BGM_FILES[Math.floor(Math.random() * BATTLE_BGM_FILES.length)]

  if (!battleAudio) {
    battleAudio = new Audio()
    battleAudio.loop = true
    battleAudio.addEventListener('playing', () => {
      currentBgmTrack.set(battleAudio?.dataset.trackName ?? '')
    })
  } else {
    battleAudio.pause()
    releaseElement(battleAudio)
  }
  battleAudio.dataset.trackName = battleFile.replace(/\.(mp3|m4a)$/, '')
  applyAudioSettings(battleAudio)
  resumeBattleMusic()

  endedCallback?.()
}

function resumeBattleMusic() {
  if (!battleAudio || !battleFile || getTargetVolume() <= 0) return
  if (battleAudio.src) {
    battleAudio.play().catch(() => {})
    currentBgmTrack.set(battleAudio.dataset.trackName ?? '')
    return
  }
  void attachTrack(
    battleAudio,
    battleFile,
    () => mode === 'battle' && getTargetVolume() > 0
  )
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
  if (!started) return
  if (mode !== 'normal') return
  if (getTargetVolume() <= 0) return
  if (takePerformanceFloor()) return
  if (playlistQuiet()) return
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
  cancelPlaylistFade()
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
  if (disposed || getTargetVolume() <= 0) return
  applyAudioSettings(audio)
  applyAudioSettings(battleAudio)

  if (mode === 'battle') {
    resumeBattleMusic()
    return
  }

  if (takePerformanceFloor()) return

  if (started) {
    resumeNormalBgm()
  }
}

// --- /play_music performance ---
// Our performance keeps time while muted; listeners defer loading and rejoin by elapsed time.

let performanceAudio: HTMLAudioElement | null = null
let performanceEnded: (() => void) | null = null
let performanceFadeTimer: ReturnType<typeof setInterval> | undefined
let currentPerformance: { track: string; startedAt: number } | null = null

const performanceElapsedSecs = () =>
  (performance.now() - (currentPerformance?.startedAt ?? 0)) / 1000

/** Inside a bard NPC's earshot the playlist stays silent — performing or not —
 *  so a performance never has to land on top of the BGM. */
let inBardZone = false
/** Live free play holds the playlist the same way: the local player's own
 *  open panel, and notes heard from a nearby performer. */
let livePanelQuiet = false
let liveNotesQuiet = false
let liveNotesTimer: ReturnType<typeof setTimeout> | undefined
let playlistFadeTimer: ReturnType<typeof setInterval> | undefined

function playlistQuiet(): boolean {
  return inBardZone || livePanelQuiet || liveNotesQuiet
}

/** Stop the playlist fade and put its volume back where the settings say. */
function cancelPlaylistFade() {
  if (playlistFadeTimer === undefined) return
  clearInterval(playlistFadeTimer)
  playlistFadeTimer = undefined
  applyAudioSettings(audio)
}

function enterPlaylistQuiet() {
  if (mode !== 'normal') return
  if (playlistFadeTimer !== undefined) return
  clearTimeout(quietTimer)
  if (!audio || audio.paused) {
    currentBgmTrack.set('')
    return
  }

  const el = audio
  playlistFadeTimer = fadeVolume(
    el,
    0,
    FADE_OUT_MS,
    // Battle music or a performance taking the speakers mid-fade owns
    // `currentBgmTrack` now — back out without touching it.
    () => mode !== 'normal',
    (arrived) => {
      playlistFadeTimer = undefined
      if (arrived) {
        el.pause()
        currentBgmTrack.set('')
      }
      applyAudioSettings(el)
    }
  )
}

function leavePlaylistQuiet() {
  if (playlistQuiet()) return
  cancelPlaylistFade()
  resumeNormalBgm()
}

export function setBardZone(inside: boolean) {
  if (inBardZone === inside) return
  inBardZone = inside
  if (inside) enterPlaylistQuiet()
  else leavePlaylistQuiet()
}

/** The local player's own free-play panel: quiet while it stays open. */
export function setLiveInstrumentQuiet(active: boolean) {
  if (livePanelQuiet === active) return
  livePanelQuiet = active
  if (active) enterPlaylistQuiet()
  else leavePlaylistQuiet()
}

/** A note heard from a nearby live performer: quiet now, release after the
 *  hold. Keying on heard notes rather than a start message also covers
 *  listeners who walked into earshot mid-performance. */
export function holdLiveInstrumentQuiet(holdMs = LIVE_NOTES_QUIET_HOLD_MS) {
  clearTimeout(liveNotesTimer)
  liveNotesTimer = setTimeout(() => {
    liveNotesQuiet = false
    leavePlaylistQuiet()
  }, holdMs)
  if (!liveNotesQuiet) {
    liveNotesQuiet = true
    enterPlaylistQuiet()
  }
}

function applyPerformanceVolume() {
  if (performanceFadeTimer !== undefined) return
  applyAudioSettings(
    performanceAudio,
    mode === 'performance' ? getTargetVolume() : 0
  )
}

/** Resume the current performance at its elapsed time, taking the speakers. */
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

/** Release audio and return the end callback; callers control notification and resume. */
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

/** Start a known performance; `onEnded` marks our own, `offsetSecs` joins mid-track. */
export function playPerformance(
  track: string,
  onEnded?: () => void,
  offsetSecs = 0
): boolean {
  if (disposed) return false
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

const unsubscribeVolume = bgmVolume.subscribe((v) => {
  clearTimeout(volumeSaveTimer)
  volumeSaveTimer = setTimeout(
    () => storageSet(STORAGE_KEY_VOLUME, String(v)),
    300
  )
  if (playlistFadeTimer === undefined) applyAudioSettings(audio)
  if (battleFadeTimer === undefined) applyAudioSettings(battleAudio)
  applyPerformanceVolume()
  if (getTargetVolume() <= 0) {
    pauseForMute()
  } else if (!get(bgmMuted)) {
    resumeAfterUnmute()
  }
})

const unsubscribeMuted = bgmMuted.subscribe((m) => {
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

export function disposeBgm() {
  disposed = true
  battleBlobs.clear()
  battleDownloads.clear()
  battleCacheBytes = 0
  battleFile = null
  started = false
  unsubscribeVolume()
  unsubscribeMuted()
  clearTimeout(volumeSaveTimer)
  clearTimeout(quietTimer)
  clearTimeout(battleLingerTimer)
  clearTimeout(battleQuietTimer)
  clearTimeout(liveNotesTimer)
  clearInterval(battleFadeTimer)
  clearInterval(playlistFadeTimer)
  dropPerformance()
  if (audio) {
    audio.removeEventListener('ended', playNext)
    audio.removeEventListener('error', playNext)
  }
  for (const el of [audio, battleAudio]) {
    if (!el) continue
    el.pause()
    releaseElement(el)
  }
  audio = null
  battleAudio = null
  currentBgmTrack.set('')
}

if (import.meta.hot) import.meta.hot.dispose(disposeBgm)
