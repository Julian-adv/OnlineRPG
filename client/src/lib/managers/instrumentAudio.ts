import { get } from 'svelte/store'
import { getInstrumentNote, type InstrumentNote } from '../data/instrumentNotes'
import { sfxMuted, sfxVolume } from './sfxManager'

export const INSTRUMENT_MAX_VOICES = 4

export const INSTRUMENT_DISTANCE_DB_POINTS = [
  [0, 0],
  [3, 0],
  [5, -0.624],
  [7, -2.221],
  [10, -4],
  [15, -4.137],
  [20, -5.16],
  [25, -8.76],
  [28, -15.341],
  [29, -20.916],
  [29.9, -40.522],
  [30, -96.3],
] as const

export function instrumentDistanceGain(distanceMeters: number): number {
  if (!Number.isFinite(distanceMeters)) return 0
  const distance = Math.max(0, distanceMeters)
  if (distance > 30) return 0

  for (let i = 1; i < INSTRUMENT_DISTANCE_DB_POINTS.length; i++) {
    const [rightDistance, rightDb] = INSTRUMENT_DISTANCE_DB_POINTS[i]
    if (distance > rightDistance) continue
    const [leftDistance, leftDb] = INSTRUMENT_DISTANCE_DB_POINTS[i - 1]
    const span = rightDistance - leftDistance
    const amount = span === 0 ? 0 : (distance - leftDistance) / span
    const db = leftDb + (rightDb - leftDb) * amount
    return 10 ** (db / 20)
  }

  return 0
}

export interface InstrumentVoice {
  performerId: number | string
  note: number
  startedAt: number
  stop: () => void
}

export class InstrumentVoicePool {
  private readonly voices = new Set<InstrumentVoice>()

  constructor(readonly maxVoices = INSTRUMENT_MAX_VOICES) {}

  get size(): number {
    return this.voices.size
  }

  add(voice: InstrumentVoice) {
    for (const active of this.voices) {
      if (
        active.performerId === voice.performerId &&
        active.note === voice.note
      ) {
        this.stopVoice(active)
        break
      }
    }

    while (this.voices.size >= this.maxVoices) {
      let oldest: InstrumentVoice | undefined
      for (const active of this.voices) {
        if (!oldest || active.startedAt < oldest.startedAt) oldest = active
      }
      if (!oldest) break
      this.stopVoice(oldest)
    }

    this.voices.add(voice)
  }

  remove(voice: InstrumentVoice) {
    this.voices.delete(voice)
  }

  stopPerformer(performerId: number | string) {
    for (const voice of [...this.voices]) {
      if (voice.performerId === performerId) this.stopVoice(voice)
    }
  }

  stopAll() {
    for (const voice of [...this.voices]) this.stopVoice(voice)
  }

  private stopVoice(voice: InstrumentVoice) {
    if (!this.voices.delete(voice)) return
    voice.stop()
  }
}

interface InstrumentAudioState {
  context: AudioContext
  master: GainNode
  limiter: DynamicsCompressorNode
  buffers: Map<number, AudioBuffer>
  impulse: AudioBuffer
}

let audioState: InstrumentAudioState | null = null
const voicePool = new InstrumentVoicePool()

function audioContextConstructor(): typeof AudioContext | undefined {
  const scope = globalThis as typeof globalThis & {
    webkitAudioContext?: typeof AudioContext
  }
  return scope.AudioContext ?? scope.webkitAudioContext
}

function currentSfxGain(): number {
  return get(sfxMuted) ? 0 : get(sfxVolume)
}

function setMasterGain(state: InstrumentAudioState) {
  const now = state.context.currentTime
  state.master.gain.cancelScheduledValues(now)
  state.master.gain.setTargetAtTime(currentSfxGain(), now, 0.015)
}

function createReverbImpulse(context: AudioContext): AudioBuffer {
  const seconds = 0.38
  const frames = Math.ceil(context.sampleRate * seconds)
  const impulse = context.createBuffer(2, frames, context.sampleRate)

  for (let channel = 0; channel < impulse.numberOfChannels; channel++) {
    const samples = impulse.getChannelData(channel)
    let seed = 0x51f15e ^ (channel * 0x9e3779b9)
    for (let i = 0; i < frames; i++) {
      seed ^= seed << 13
      seed ^= seed >>> 17
      seed ^= seed << 5
      const noise = ((seed >>> 0) / 0xffffffff) * 2 - 1
      const time = i / context.sampleRate
      const decay = Math.exp(-12 * time)
      samples[i] = noise * decay
    }
  }

  return impulse
}

function ensureAudioState(): InstrumentAudioState | null {
  if (audioState) return audioState
  const Context = audioContextConstructor()
  if (!Context) return null

  const context = new Context()
  const master = context.createGain()
  const limiter = context.createDynamicsCompressor()
  master.gain.value = currentSfxGain()
  limiter.threshold.value = -8
  limiter.knee.value = 12
  limiter.ratio.value = 6
  limiter.attack.value = 0.003
  limiter.release.value = 0.15
  master.connect(limiter)
  limiter.connect(context.destination)
  audioState = {
    context,
    master,
    limiter,
    buffers: new Map(),
    impulse: createReverbImpulse(context),
  }
  sfxVolume.subscribe(() => {
    if (audioState) setMasterGain(audioState)
  })
  sfxMuted.subscribe(() => {
    if (audioState) setMasterGain(audioState)
  })
  return audioState
}

function randomSequence(seedValue: number) {
  let seed = seedValue | 0
  return () => {
    seed ^= seed << 13
    seed ^= seed >>> 17
    seed ^= seed << 5
    return ((seed >>> 0) / 0xffffffff) * 2 - 1
  }
}

function instrumentPlaybackRate(
  context: AudioContext,
  note: InstrumentNote
): number {
  const period = Math.max(2, Math.round(context.sampleRate / note.frequencyHz))
  return (note.frequencyHz * (period + 0.5)) / context.sampleRate
}

function createPluckedStringBuffer(
  context: AudioContext,
  note: InstrumentNote
): AudioBuffer {
  const period = Math.max(2, Math.round(context.sampleRate / note.frequencyHz))
  const playbackRate = instrumentPlaybackRate(context, note)
  const frames = Math.ceil(
    note.durationSeconds * context.sampleRate * playbackRate
  )
  const buffer = context.createBuffer(2, frames, context.sampleRate)
  const ring = new Float32Array(period)
  const random = randomSequence(0x6d2b79f5 ^ (note.index * 0x45d9f3b))

  let previousNoise = 0
  for (let i = 0; i < period; i++) {
    const noise = random()
    ring[i] = noise - previousNoise * 0.35
    previousNoise = noise
  }

  const left = buffer.getChannelData(0)
  const right = buffer.getChannelData(1)
  const damping = Math.exp(Math.log(0.0025) / (frames / period))
  let cursor = 0
  let dc = 0

  for (let i = 0; i < frames; i++) {
    const current = ring[cursor]
    const next = ring[(cursor + 1) % period]
    ring[cursor] = (current + next) * 0.5 * damping
    cursor = (cursor + 1) % period

    dc += 0.0025 * (current - dc)
    const time = i / (context.sampleRate * playbackRate)
    const attack = Math.min(1, time / 0.004)
    const tail = Math.min(1, (note.durationSeconds - time) / 0.025)
    const sample = (current - dc) * attack * Math.max(0, tail) * 0.52
    left[i] = sample
    right[i] = sample * 0.97 + (i >= 11 ? left[i - 11] * 0.03 : 0)
  }

  return buffer
}

function noteBuffer(
  state: InstrumentAudioState,
  note: InstrumentNote
): AudioBuffer {
  let buffer = state.buffers.get(note.index)
  if (!buffer) {
    buffer = createPluckedStringBuffer(state.context, note)
    state.buffers.set(note.index, buffer)
  }
  return buffer
}

export function playInstrumentNote(
  noteIndex: number,
  performerId: number | string,
  volume = 1
): boolean {
  const note = getInstrumentNote(noteIndex)
  const state = ensureAudioState()
  if (!note || !state || !Number.isFinite(volume) || volume <= 0) return false

  const { context } = state
  if (context.state === 'suspended') context.resume().catch(() => {})

  const source = context.createBufferSource()
  const body = context.createBiquadFilter()
  const dry = context.createGain()
  const convolver = context.createConvolver()
  const wet = context.createGain()
  const output = context.createGain()
  const level = Math.min(1, volume)

  source.buffer = noteBuffer(state, note)
  source.playbackRate.value = instrumentPlaybackRate(context, note)
  body.type = 'lowpass'
  body.frequency.value = Math.min(7200, note.frequencyHz * 18)
  body.Q.value = 0.45
  dry.gain.value = 0.72 * level
  convolver.buffer = state.impulse
  wet.gain.value = 0.065 * level
  output.gain.value = 1

  source.connect(body)
  body.connect(dry)
  body.connect(convolver)
  dry.connect(output)
  convolver.connect(wet)
  wet.connect(output)
  output.connect(state.master)

  let stopped = false
  const disconnect = () => {
    source.disconnect()
    body.disconnect()
    dry.disconnect()
    convolver.disconnect()
    wet.disconnect()
    output.disconnect()
  }
  const stop = () => {
    if (stopped) return
    stopped = true
    const now = context.currentTime
    output.gain.cancelScheduledValues(now)
    output.gain.setValueAtTime(output.gain.value, now)
    output.gain.linearRampToValueAtTime(0, now + 0.012)
    try {
      source.stop(now + 0.014)
    } catch {
      disconnect()
    }
  }

  const voice: InstrumentVoice = {
    performerId,
    note: noteIndex,
    startedAt: context.currentTime,
    stop,
  }
  source.onended = () => {
    voicePool.remove(voice)
    disconnect()
  }
  voicePool.add(voice)
  source.start()
  return true
}

export function stopInstrumentPerformer(performerId: number | string) {
  voicePool.stopPerformer(performerId)
}

export function stopAllInstrumentAudio() {
  voicePool.stopAll()
}
