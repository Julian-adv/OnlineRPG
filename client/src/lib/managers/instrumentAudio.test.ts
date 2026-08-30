import { describe, expect, it, vi } from 'vitest'
import {
  INSTRUMENT_MAX_VOICES,
  InstrumentVoicePool,
  instrumentDistanceGain,
  type InstrumentVoice,
} from './instrumentAudio'

function voice(
  performerId: number | string,
  note: number,
  startedAt: number
): InstrumentVoice {
  return { performerId, note, startedAt, stop: vi.fn() }
}

describe('instrumentDistanceGain', () => {
  it('holds full gain through 3 m and interpolates in dB', () => {
    expect(instrumentDistanceGain(0)).toBe(1)
    expect(instrumentDistanceGain(3)).toBe(1)
    expect(instrumentDistanceGain(6)).toBeCloseTo(10 ** (-1.4225 / 20), 8)
  })

  it('follows the measured far-distance points and cuts beyond 30 m', () => {
    expect(instrumentDistanceGain(10)).toBeCloseTo(10 ** (-4 / 20), 8)
    expect(instrumentDistanceGain(29)).toBeCloseTo(10 ** (-20.916 / 20), 8)
    expect(instrumentDistanceGain(30)).toBeCloseTo(10 ** (-96.3 / 20), 10)
    expect(instrumentDistanceGain(30.001)).toBe(0)
    expect(instrumentDistanceGain(Number.NaN)).toBe(0)
  })
})

describe('InstrumentVoicePool', () => {
  it('replaces the same performer and note', () => {
    const pool = new InstrumentVoicePool()
    const first = voice(1, 3, 1)
    const second = voice(1, 3, 2)

    pool.add(first)
    pool.add(second)

    expect(first.stop).toHaveBeenCalledOnce()
    expect(second.stop).not.toHaveBeenCalled()
    expect(pool.size).toBe(1)
  })

  it('allows the same note from different performers', () => {
    const pool = new InstrumentVoicePool()
    pool.add(voice(1, 3, 1))
    pool.add(voice(2, 3, 2))
    expect(pool.size).toBe(2)
  })

  it('steals the oldest voice when the fifth begins', () => {
    const pool = new InstrumentVoicePool()
    const voices = Array.from(
      { length: INSTRUMENT_MAX_VOICES + 1 },
      (_, index) => voice(1, index, index)
    )
    for (const active of voices) pool.add(active)

    expect(voices[0].stop).toHaveBeenCalledOnce()
    expect(pool.size).toBe(INSTRUMENT_MAX_VOICES)
  })

  it('stops one performer without touching the others', () => {
    const pool = new InstrumentVoicePool()
    const a = voice('a', 0, 1)
    const b = voice('b', 1, 2)
    pool.add(a)
    pool.add(b)

    pool.stopPerformer('a')
    expect(a.stop).toHaveBeenCalledOnce()
    expect(b.stop).not.toHaveBeenCalled()
    expect(pool.size).toBe(1)
  })
})
