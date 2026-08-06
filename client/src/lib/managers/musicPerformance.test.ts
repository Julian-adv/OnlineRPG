import { describe, it, expect, vi, beforeEach } from 'vitest'
import { get } from 'svelte/store'
import { MUSIC_EMOTE_ANIM } from '../stores/emoteStore'

const playPerformance =
  vi.fn<(track: string, onEnded?: () => void) => boolean>()
const stopPerformance = vi.fn()
const fadeOutPerformance = vi.fn()

vi.mock('./bgmManager', () => ({
  playPerformance: (track: string, onEnded?: () => void) =>
    playPerformance(track, onEnded),
  stopPerformance: () => stopPerformance(),
  fadeOutPerformance: () => fadeOutPerformance(),
}))

type Performance = typeof import('./musicPerformance')
let performance: Performance
let emoteStopRequest: typeof import('../stores/emoteStore').emoteStopRequest

/** The callback bgmManager was handed for the Nth started performance. */
const endedCallback = (call: number) => playPerformance.mock.calls[call][1]

beforeEach(async () => {
  vi.resetModules()
  playPerformance.mockReset().mockReturnValue(true)
  stopPerformance.mockReset()
  fadeOutPerformance.mockReset()
  performance = await import('./musicPerformance')
  ;({ emoteStopRequest } = await import('../stores/emoteStore'))
  emoteStopRequest.set(false)
})

describe('musicPerformance', () => {
  it('only the audible performer can stop the music', () => {
    performance.startMusicPerformance(1, 'Twilight Fields', false)

    performance.stopMusicPerformance(2)
    expect(stopPerformance).not.toHaveBeenCalled()

    performance.stopMusicPerformance(1)
    expect(stopPerformance).toHaveBeenCalledTimes(1)
  })

  it('ends the tune when the performer takes up something else', () => {
    performance.startMusicPerformance(1, 'Twilight Fields', false)

    performance.applyInteractionChange(1, MUSIC_EMOTE_ANIM)
    expect(stopPerformance).not.toHaveBeenCalled()

    // Sitting down replaces the strum instead of clearing it.
    performance.applyInteractionChange(1, 'bench')
    expect(stopPerformance).toHaveBeenCalledTimes(1)
  })

  it('fades out only the audible performer, freeing the slot', () => {
    performance.startMusicPerformance(1, 'Twilight Fields', false)

    performance.fadeOutMusicPerformance(2)
    expect(fadeOutPerformance).not.toHaveBeenCalled()

    performance.fadeOutMusicPerformance(1)
    expect(fadeOutPerformance).toHaveBeenCalledTimes(1)

    // The slot already let go — a later stop for the same player is a no-op.
    performance.stopMusicPerformance(1)
    expect(stopPerformance).not.toHaveBeenCalled()
  })

  it('leaves the slot alone when the track cannot be played here', () => {
    performance.startMusicPerformance(1, 'Twilight Fields', false)
    playPerformance.mockReturnValue(false)
    performance.startMusicPerformance(2, 'Muted For Us', false)

    performance.stopMusicPerformance(2)
    expect(stopPerformance).not.toHaveBeenCalled()
  })

  it('asks the emote to end when our own track runs out', () => {
    performance.startMusicPerformance(7, 'Twilight Fields', true)
    endedCallback(0)?.()
    expect(get(emoteStopRequest)).toBe(true)
  })

  it('ignores the end of a performance we already replaced', () => {
    performance.startMusicPerformance(7, 'Twilight Fields', true)
    performance.startMusicPerformance(7, 'Creekside Air', true)

    // Starting the second tune drops the first — that stale end must not
    // cancel the emote the new one just began.
    endedCallback(0)?.()
    expect(get(emoteStopRequest)).toBe(false)

    endedCallback(1)?.()
    expect(get(emoteStopRequest)).toBe(true)
  })
})
